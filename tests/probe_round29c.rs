//! Round-29 probes (III): mapping × macro × replay combinations, Ex boundaries, host API misuse.

mod common;

use common::{edit, Fixture};
use vimcore::key::{parse_key_sequence, Key};
use vimcore::keymap::ModeClass;

fn f_with_map(initial: &str, lhs: &str, rhs: &str) -> Fixture {
    let mut f = Fixture::at(initial, 0, 0);
    f.vim
        .keymaps_mut()
        .map_str(ModeClass::Normal, lhs, rhs);
    f
}

/// Probe N: when a mapping is recorded into a macro the raw key is stored; replay
/// expands it again through the pipeline (vim 9.1 typeahead evidence: with
/// `:nmap x dd`, the macro register from `qaxjyyq` is "xjyy", without expanded keys).
#[test]
fn probe_macro_records_raw_keys_mapping_replays() {
    // after :nmap x dd, record qaxq: macro = [x]; during recording x already ran dd once.
    let mut f = f_with_map("row1\nrow2\nrow3\n", "x", "dd");
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "the macro records the raw key x");
    assert_eq!(f.text(), "row2\nrow3\n", "the mapping already executed during recording");
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "row3\n", "on replay x expands through the mapping to dd, deleting one more line");
}

/// Probe O: a noremap expansion is not re-mapped recursively.
#[test]
fn probe_noremap_no_recursion() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Normal, "a", "x", true);
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "b", "a");
    f.feed(["b"]); // b → a (noremap) → x, no further expansion
    assert_eq!(f.text(), "bc\n", "the noremap expansion runs as the literal x");
}

/// Probe P: when a mapping LHS collides with a builtin prefix, the builtin combination
/// still wins (gg is not blocked by a gX mapping).
#[test]
fn probe_builtin_beats_mapping_prefix() {
    let mut f = f_with_map("a\nb\nc\n", "gx", "dd");
    f.feed(["g", "g"]); // gg is builtin: jumps to the first line immediately, no wait for gx
    assert_eq!(f.line(), 0);
    assert_eq!(f.host.bells, 0);
}

/// Probe Q: insert-mode mapping `jk` → Esc.
#[test]
fn probe_insert_mapping_jk_escape() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Insert, "jk", "<Esc>", true);
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["j"]);
    assert_eq!(f.vim.mode_indicator(), "INSERT", "j is a mapping prefix, still pending");
    f.feed(["k"]);
    assert_eq!(f.vim.mode_indicator(), "", "jk fires Esc and leaves insert");
    assert_eq!(f.text(), "Xabc\n");
}

/// Probe R: the multi-line copy of block insert is one undo group (a single u undoes all).
#[test]
fn probe_block_insert_single_undo_group() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    let groups_before = f.host.group_count;
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Xab\nXcd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ab\ncd\n", "one u undoes the copy across all lines");
    assert_eq!(f.host.group_count, groups_before + 1, "exactly one undo group");
}

/// Probe S: `.` replay after `"ap` keeps the register prefix.
#[test]
fn probe_dot_replays_register_put() {
    let mut f = Fixture::at("x\naa\nbb\n", 0, 0);
    f.feed(["\"", "a", "y", "y"]); // "a = "x\n"
    f.feed(["j"]);
    f.feed(["\"", "a", "p"]); // insert the x line below line 2 (aa)
    f.feed(["j"]);
    f.feed(["."]); // insert again below the cursor line (bb)
    assert_eq!(f.text(), "x\naa\nx\nbb\nx\n");
}

/// Probe T: `:s/a//` (empty replacement) deletes the match.
#[test]
fn probe_substitute_empty_replacement_deletes() {
    let f = {
        let mut f = Fixture::at("bar foo bar\n", 0, 0);
        f.feed([":", "s", "/", "b", "a", "r", "/", "/", "<CR>"]);
        f
    };
    assert_eq!(f.text(), " foo bar\n", "without the g flag only the first match per line is substituted");
}

/// Probe U: `:set foo ts=8` — the unknown item raises E518 and stops; the later ts=8
/// does not take effect.
#[test]
fn probe_set_unknown_stops_before_later_items() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed([":", "s", "e", "t", " ", "f", "o", "o", " ", "t", "s", "=", "8", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E518")),
        "should report E518"
    );
    f.feed([":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=4")),
        "ts=8 must not take effect, got {:?}",
        f.host.statuses
    );
}

/// Probe V: `@:` replays the previous Ex command.
#[test]
fn probe_at_colon_replays_ex() {
    let mut f = Fixture::at("zaa\nzb\nzc\n", 0, 0);
    f.feed([":", "s", "/", "z", "/", "w", "/", "<CR>"]);
    assert_eq!(f.text(), "waa\nzb\nzc\n");
    f.feed(["j"]);
    f.feed(["@", ":"]);
    assert_eq!(f.text(), "waa\nwb\nzc\n");
}

/// Probe W: running `:noh` inside visual — highlights are cleared and it falls back to
/// normal (selection closed).
#[test]
fn probe_visual_noh_closes_selection() {
    let mut f = Fixture::at("foo bar\n", 0, 0);
    f.feed(["/"]); 
    f.feed(["b", "a", "r", "<CR>"]);
    f.feed(["v"]); // enter visual
    f.feed([":"]);
    for c in "noh".chars() {
        f.feed(&[c.to_string()]);
    }
    f.feed(["<CR>"]);
    assert_eq!(f.vim.mode_indicator(), "", "leaves visual after execution");
    assert!(f.host.highlights.is_empty(), "noh clears the highlights");
}

/// Probe X: host API misuse — visual drag with anchor > cursor (dragging upwards).
#[test]
fn probe_set_visual_range_drag_up() {
    let mut f = Fixture::at("abcd\nefgh\nij\n", 2, 1);
    f.vim.set_visual_range(&f.buf, 7, 0); // anchor behind, cursor in front
    let (a, c, _) = f.vim.visual_selection().unwrap();
    assert_eq!((a, c), (7, 0), "visual_selection returns the raw anchor/cursor");
    f.feed(["d"]);
    // the selection is (0,0)..(1,7) (inclusive of h): deletes "abcd\nefg", leaving "h\nij\n".
    assert_eq!(f.text(), "h\nij\n", "the whole upward-dragged selection is deleted (not the rest of the anchor line)");
}

/// Probe Y: host API misuse — a drag during an insert session is ignored.
#[test]
fn probe_set_visual_range_ignored_in_insert() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["i"]);
    f.vim.set_visual_range(&f.buf, 0, 4);
    assert_eq!(f.vim.mode_indicator(), "INSERT", "a drag in insert must not change the mode");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
}

/// Probe Z: host API — an ime replace_range outside an insert session forms its own
/// undo group.
#[test]
fn probe_replace_range_outside_session_closes_group() {
    let mut f = Fixture::at("hello world\n", 0, 0);
    let before = f.host.group_count;
    f.ime_replace(0..5, "goodbye");
    assert_eq!(f.text(), "goodbye world\n");
    f.feed(["x"]);
    f.feed(["u"]);
    assert_eq!(f.text(), "goodbye world\n", "u undoes the x, not the ime replacement");
    f.feed(["u"]);
    assert_eq!(f.text(), "hello world\n", "the second u undoes the ime replacement");
    assert!(f.host.group_count >= before + 2, "two independent undo groups");
}

/// Probe AA: `2fa` with only one a fails as a whole (no partial motion).
#[test]
fn probe_count_find_beyond_line_fails_whole() {
    let f = edit("a b a\n", 0, 0, &["2", "f", "b"]);
    assert_eq!(f.host.bells, 1, "should ring when the targets run short");
    // the cursor does not move.
    assert_eq!(f.cursor(), 0);
}

/// Probe AB: `dit` takes the innermost tag.
#[test]
fn probe_dit_innermost_tag() {
    let f = edit("<div><p>x</p></div>", 0, 9, &["d", "i", "t"]);
    assert_eq!(f.text(), "<div><p></p></div>", "dit only deletes the innermost content");
}

/// Probe AC: a failed `f<CR>` does not overwrite last_find (; still searches for a).
#[test]
fn probe_failed_f_cr_keeps_last_find() {
    let f = edit("axa\n", 0, 0, &["f", "a", "f", "<CR>", ";"]);
    // f a lands on the 2nd a (col 2); f<CR> fails; ; searches a again from the line end —
    // nothing more in the line, so it fails with a bell (f does not wrap across lines),
    // cursor stays put.
    assert_eq!(f.cursor(), 2, "; repeats the search in the f a direction");
    assert_eq!(f.host.bells, 2, "f<CR> and the following ; each ring once");
}

/// Probe AD: a huge repeat `999999999J` stops promptly at end of buffer (no hang).
#[test]
fn probe_huge_join_count_terminates() {
    let f = edit("a\nb\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "J"]);
    assert_eq!(f.text(), "a b\n", "J joins up to the last line and inserts a space at the seam");
    assert_eq!(f.host.bells, 1, "one bell when there are not enough lines");
}

/// Probe AE: parse_key_sequence of a pure-char `Key` sequence agrees with the key pipeline.
#[test]
fn probe_parse_key_sequence_angles() {
    let keys = parse_key_sequence("<Esc>ab<C-a>");
    assert_eq!(keys.len(), 4);
    assert_eq!(keys[0], Key::escape());
    assert_eq!(keys[3], Key::ctrl_char('a'));
    // an unclosed < stays literal.
    let broken = parse_key_sequence("a<b");
    assert_eq!(broken.len(), 3);
    assert_eq!(broken[1], Key::char('<'));
    assert_eq!(broken[2], Key::char('b'));
}

/// Probe AF: VimState borrow-checking through `Ctx` — `.` first, then `u`, in one fixture.
#[test]
fn probe_dot_then_undo() {
    let mut f = Fixture::at("word\n", 0, 0);
    f.feed(["x"]);
    f.feed(["."]);
    assert_eq!(f.text(), "rd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ord\n", "u undoes the second x");
    f.feed(["u"]);
    assert_eq!(f.text(), "word\n");
}

