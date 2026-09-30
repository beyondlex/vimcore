//! Round-5 review regressions. Semantic cases re-probed against vim 9.1
//! (headless `-es` scripts; probe outputs quoted in the docs). One engine
//! invariant (mapping expansions re-entering mapping resolution) is covered
//! at both the trie and pipeline level.

mod common;

use common::{edit, Fixture};

// ---- visual mode: counts and feedback ---------------------------------------

/// vim 9.1 probes (`Vj3>` with sw=4 expandtab → 12 spaces per line,
/// `Vj2>` → 8): a count BEFORE the indent operator multiplies the shift.
/// The old engine ignored the count entirely (always one shiftwidth).
#[test]
fn visual_indent_applies_the_count() {
    let f = edit("l1\nl2\n", 0, 0, &["V", "j", "3", ">"]);
    assert_eq!(
        f.text(),
        "            l1\n            l2\n",
        "Vj3> shifts three shiftwidths on every selected line"
    );
    // in NORMAL mode the count means LINES, not shiftwidths (vim `3<<`
    // outdents three lines by one shiftwidth each)
    let f = edit(
        "        l1\n        l2\n        l3\n",
        0,
        0,
        &["3", "<", "<"],
    );
    assert_eq!(
        f.text(),
        "    l1\n    l2\n    l3\n",
        "normal 3<< outdents three lines one shiftwidth each"
    );
}

/// An unmapped key in visual mode cancels a pending count (vim cancels the
/// whole pending state). The old engine rang the bell but kept the count, so
/// the NEXT motion silently scaled with it (`V 3 & j` jumped three lines).
#[test]
fn visual_unknown_key_cancels_the_pending_count() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
    f.feed(["V", "3", "&", "j"]);
    assert_eq!(f.line(), 1, "j after the canceled count moves one line");
}

// ---- no-op commands never enter the changelist -------------------------------

/// vim 9.1 probe (`jx gg x g;g;` on an empty-first-line buffer: both `g;`
/// land on the changed line): a no-op `x` must not create a change entry.
/// The old engine bumped the changelist unconditionally, so the second `g;`
/// parked on the empty line where nothing had ever changed.
#[test]
fn noop_x_does_not_pollute_the_changelist() {
    let mut f = Fixture::new("\nfoo\n");
    f.feed(["j", "x", "g", "g", "x", "g", "g", "g", ";"]);
    assert_eq!(f.line(), 1, "first g; lands on the real change");
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 1, "second g; has nowhere older to go (vim E662)");
}

/// The whole no-op family shares the guard: `x`/`X` at a line boundary,
/// `p` with an empty register, `J` at EOF, `~` past the line end — none of
/// them may record a change position (or move `'.`).
#[test]
fn noop_commands_leave_the_change_mark_alone() {
    // p with an empty register (bell, no paste)
    let mut f = Fixture::new("abc\ndef\n");
    f.feed(["j", "x", "g", "g", "p", "g", ";"]);
    assert_eq!(f.line(), 1, "noop p is not a change");

    // ~ on an empty line
    let mut f = Fixture::new("\nfoo\n");
    f.feed(["j", "x", "g", "g", "~", "g", ";"]);
    assert_eq!(f.line(), 1, "noop ~ is not a change");

    // J at the last line
    let mut f = Fixture::new("\nfoo\n");
    f.feed(["j", "x", "J", "g", ";"]);
    assert_eq!(f.line(), 1, "noop J is not a change");
}

// ---- search / substitute feedback ----------------------------------------------

/// vim 9.1: `/<CR>` with no previous pattern sets v:errmsg to
/// "E35: No previous regular expression". The engine stayed silent.
#[test]
fn search_enter_without_a_pattern_reports_e35() {
    let mut f = Fixture::new("abc\n");
    f.feed(["/", "<CR>"]);
    assert_eq!(
        f.host.statuses,
        vec!["E35: No previous regular expression".to_owned()]
    );
    // the same for the :s empty-pattern reuse
    let mut f = Fixture::new("abc\n");
    f.feed([":", "s", "/", "/", "x", "/", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E35")),
        ":s//x/ without a previous pattern reports E35"
    );
}

// ---- :yank with a register AND a count -----------------------------------------

/// vim 9.1 probe (`:1y a 2` into `"a`, then `"ap`): the count after the
/// register extends the range. The old parser ignored it and yanked one
/// line.
#[test]
fn ex_yank_honors_count_after_register() {
    let mut f = Fixture::new("l1\nl2\nl3\n");
    f.feed(["g", "g", ":", "1", "y", " ", "a", " ", "2", "<CR>"]);
    f.feed(["G", "\"", "a", "P"]);
    assert_eq!(f.text(), "l1\nl2\nl1\nl2\nl3\n", ":1y a 2 yanks two lines");
}

// ---- `&` and bare `:s` repeat the last substitute --------------------------------

/// `&` re-runs the last `:s` on the current line; bare `:s` does the same;
/// without a previous substitute both report vim's E33.
#[test]
fn ampersand_repeats_the_last_substitute() {
    let mut f = Fixture::new("foo bar\nbaz foo\n");
    f.feed([":", "s", "/", "f", "o", "o", "/", "x", "/", "<CR>"]);
    f.feed(["j", "&"]);
    assert_eq!(f.text(), "x bar\nbaz x\n", "& repeats the substitution");

    let mut f = Fixture::new("foo\n");
    f.feed(["&"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E33")),
        "& without a previous substitute reports E33"
    );
    let mut f = Fixture::new("foo\n");
    f.feed([":", "s", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E33")),
        "bare :s without a previous substitute reports E33"
    );
}

// ---- mappings completing mid-queue ----------------------------------------------

/// A mapping whose LHS is a LEAF in the trie must fire even when keys queued
/// behind it (an expansion, a macro, a `.` replay) — the old trie walk
/// reported a miss for the whole queue and processed the mapping's keys as
/// literals. vim 9.1 (`:imap a <Esc>` + `:imap q ax`, type q): the chain
/// q→ax→a→<Esc> exits insert and the `x` runs as a normal command.
#[test]
fn mapping_fires_when_its_keys_arrive_mid_queue() {
    let mut f = Fixture::new("abc\n");
    f.vim
        .keymaps_mut()
        .map_str(vimcore::keymap::ModeClass::Insert, "a", "<Esc>");
    f.vim
        .keymaps_mut()
        .map_str(vimcore::keymap::ModeClass::Insert, "q", "ax");
    f.feed(["i", "q"]);
    assert_eq!(
        f.vim.mode(),
        vimcore::mode::Mode::Normal,
        "the a→<Esc> mapping fired mid-queue"
    );
    assert_eq!(
        f.text(),
        "bc\n",
        "the trailing x ran as a normal-mode command"
    );
}

/// The leaf rule must not hijack the tuned builtin-vs-mapping ambiguity: a
/// mapping WITH children (`j` of `jk`) followed by a non-matching key still
/// falls through to the builtin, exactly as before.
#[test]
fn mapping_prefix_with_children_still_falls_through() {
    let mut f = Fixture::new("foo bar\n");
    f.vim
        .keymaps_mut()
        .map_str(vimcore::keymap::ModeClass::Normal, "j", "x");
    // `j` is a builtin motion AND a mapping; `jd` can grow into neither a
    // j-mapping continuation nor... the engine resolves the builtin the
    // moment the input stops being a mapping prefix
    f.feed(["j"]);
    assert_eq!(f.text(), "oo bar\n", "the j mapping fires on its own");
}

// ---- TCK reference implementation contract ---------------------------------------

/// The TCK's String-backed reference buffer must honor its own `char_at`
/// contract: non-boundary offsets read as `None`, never panic. The engine's
/// `floor_to_char_boundary` probes non-boundary offsets ON PURPOSE, so hosts
/// copying the old reference impl took slice panics from legitimate engine
/// probes (caught by the round-5 review; `floor_to_char_boundary` on "中文"
/// offset 1 used to panic the copied impl).
#[test]
fn tck_reference_impl_reads_non_boundaries_as_none() {
    use vimcore::buffer::floor_to_char_boundary;
    use vimcore::tck::buffer_read_contract;

    let buf = vimcore::tck::TckStrBuf::from_text("中文mix\n行2");
    buffer_read_contract(&buf).unwrap();
    // "中文" occupies bytes 0..6; boundaries are 0, 3, 6
    assert_eq!(floor_to_char_boundary(&buf, 1), 0, "floors onto a boundary");
    assert_eq!(floor_to_char_boundary(&buf, 2), 0, "inside 中 → floor to 0");
    assert_eq!(floor_to_char_boundary(&buf, 3), 3);
    // past the end floors onto the LAST CHARACTER's start (the loop stops
    // at the first surviving boundary below the offset)
    assert_eq!(floor_to_char_boundary(&buf, 99), 13);
}

// ---- new Ex commands: :sort / :join -----------------------------------------------

/// `:{range}sort[!] [i] [u]` — sort the range's lines (`!` reverses, `i`
/// ignores case, `u` dedupes after sorting). vim's `:sort` used to fall into
/// the substitute parser and come out as a bare bell with no feedback.
#[test]
fn ex_sort_sorts_reverses_and_dedupes() {
    let mut f = Fixture::new("pear\napple\nbanana\n");
    f.feed([":", "%", "s", "o", "r", "t", "<CR>"]);
    assert_eq!(f.text(), "apple\nbanana\npear\n");

    let mut f = Fixture::new("pear\napple\nbanana\n");
    f.feed([":", "2", ",", "3", "s", "o", "r", "t", "!", "<CR>"]);
    assert_eq!(
        f.text(),
        "pear\nbanana\napple\n",
        ":2,3sort! reverses lines 2-3"
    );
    // a single-line range has nothing to reorder (vim same)
    let mut f = Fixture::new("pear\napple\nbanana\n");
    f.feed([":", "2", "s", "o", "r", "t", "!", "<CR>"]);
    assert_eq!(
        f.text(),
        "pear\napple\nbanana\n",
        ":2sort! is a no-op on one line"
    );

    let mut f = Fixture::new("b\na\nb\n");
    f.feed([":", "%", "s", "o", "r", "t", " ", "u", "<CR>"]);
    assert_eq!(f.text(), "a\nb\n", ":sort u dedupes");

    let mut f = Fixture::new("B\na\nC\n");
    f.feed([":", "%", "s", "o", "r", "t", " ", "i", "<CR>"]);
    assert_eq!(f.text(), "a\nB\nC\n", ":sort i ignores case");
}

/// `:{range}j[oin][!]` — join the range's lines; a one-line range joins with
/// the NEXT line (bare `:j`); `!` concatenates verbatim (vim `:j!` ≈ `gJ`).
#[test]
fn ex_join_joins_the_range() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "1", ",", "3", "j", "<CR>"]);
    assert_eq!(f.text(), "a b c\n");

    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "j", "<CR>"]);
    assert_eq!(f.text(), "a b\nc\n", "bare :j joins with the next line");

    let mut f = Fixture::new("a  \n   b\nc\n");
    f.feed([":", "1", ",", "2", "j", "!", "<CR>"]);
    assert_eq!(f.text(), "a     b\nc\n", ":j! joins verbatim");
}
