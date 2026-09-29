//! Round-3 review regressions: each case was probed against vim 9.1 (or is a
//! pure engine-invariant case where noted) before changing the engine.

mod common;

use common::edit;
use vimcore::buffer::VimBuffer as _;
use vimcore::mode::Mode;

// ---- operator doubling without noise --------------------------------------

/// `gugu` / `gUgU` / `g~g~` / `gqgq` execute via the operator-doubling retry
/// (the trie path dies in operator-pending phase, the queue re-feeds the
/// trigger key). That retry used to ring the bell on the intermediate `g` —
/// vim completes the doubling silently.
#[test]
fn doubled_multi_key_operators_do_not_bell() {
    for (start, keys, want) in [
        ("HELLO\nWORLD\n", "g u g u", "hello\nWORLD\n"),
        ("hello\nworld\n", "g U g U", "HELLO\nworld\n"),
        ("hello\nworld\n", "g ~ g ~", "HELLO\nworld\n"), // count=1: line 0 only
        // gq with textwidth 78 keeps short lines as they are
        ("hello\nworld\n", "g q g q", "hello\nworld\n"),
    ] {
        let mut f = common::Fixture::at(start, 0, 0);
        f.feed(keys.split_whitespace());
        assert_eq!(f.text(), want, "text after {keys}");
        assert_eq!(f.host.bells, 0, "bell after {keys}");
    }
}

/// The retry must keep beeping for genuinely bad input after a pending
/// operator (`dgx` = no such motion; vim probe: buffer unchanged, `x`
/// swallowed) — the fix is bell-suppression only when the re-fed key
/// completes the doubling.
#[test]
fn bad_motion_after_operator_still_bells() {
    let f = edit("hello\nworld\n", 0, 0, &["d", "g", "x"]);
    assert!(f.host.bells > 0, "dgx must bell");
    assert_eq!(f.text(), "hello\nworld\n", "x is swallowed, like vim");
}

// ---- jump context marks ('' / `` / '.) -------------------------------------

/// `''` / `` `` `` jump back to where the last jump started (linewise /
/// exact), probed against vim 9.1: G → gg → '' lands on the pre-gg line.
#[test]
fn backtick_backtick_returns_to_last_jump_origin() {
    let mut f = edit("a\nb\nc\nd\n", 0, 0, &["G", "g", "g"]);
    assert_eq!(f.line(), 0);
    f.feed(["'", "'"]);
    assert_eq!(
        f.line(),
        3,
        "'' returns to the line of the last jump origin"
    );
    f.feed(["`", "`"]);
    assert_eq!(f.line(), 0, "`` returns to the exact origin of that jump");
}

/// `'.` jumps to the line of the last change.
#[test]
fn quote_dot_jumps_to_last_change_line() {
    let mut f = edit("a\nb\nc\nd\n", 3, 0, &["i"]);
    f.type_text("X");
    f.feed(["<Esc>", "g", "g", "'", "."]);
    assert_eq!(f.line(), 3);
}

/// With no previous jump/change the marks are unset: bell, no crash.
#[test]
fn context_marks_unset_bell_without_jump() {
    let f = edit("a\nb\nc\n", 0, 0, &["'", "'", "`", "`", "'", "."]);
    assert!(f.host.bells >= 3);
    assert_eq!(f.line(), 0);
}

// ---- changelist & jumplist follow edits -------------------------------------

/// `g;` must land on the position the change has AFTER later edits shifted
/// the text, not on the raw pre-edit offset (vim adjusts the changelist).
#[test]
fn changelist_positions_track_edits() {
    // change on line 1, then insert a line ABOVE it, then g; must land on
    // the (shifted) changed line, not one line up.
    let mut f = edit("a\nb\nc\n", 1, 0, &["c", "w"]);
    f.type_text("B");
    f.feed(["<Esc>", "g", "g", "O"]);
    f.type_text("new");
    f.feed(["<Esc>", "g", ";"]);
    assert_eq!(f.line(), 2, "g; follows the shifted change position");
    assert_eq!(f.text(), "new\na\nB\nc\n");
}

/// Same rule for the jumplist (`C-o`/`C-i` after an edit above a jump
/// target). gg→G records origin 6 / dest 10 in a 4-line buffer; opening a
/// line above the G target shifts both by 4 bytes, so walking back and
/// forward must land on the SHIFTED positions.
#[test]
fn jumplist_positions_track_edits() {
    let mut f = edit("a\nb\nc\nd\n", 3, 0, &["g", "g"]);
    assert_eq!(f.line(), 0);
    f.feed(["G", "k", "O"]); // G jumped to line 3; open a line above line 2
    f.type_text("new");
    f.feed(["<Esc>", "<C-o>"]);
    assert_eq!(f.line(), 0, "C-o walks back to the (stable) gg target");
    f.feed(["<C-i>"]);
    // the G target was the last line ('d'); the opened line above pushed it
    // down one row
    assert_eq!(f.line(), 4, "C-i follows the shifted G target");
}

// ---- empty linewise registers paste an empty line ----------------------------

/// vim 9.1 probe: `yy` on an empty line then `p` inserts ONE empty line
/// below (`dd` likewise); the engine used to treat the empty register as
/// "nothing to paste".
#[test]
fn empty_linewise_register_pastes_empty_line() {
    let f = edit("one\n\ntwo\n", 1, 0, &["y", "y", "j", "p"]);
    assert_eq!(f.text(), "one\n\ntwo\n\n");
    assert_eq!(f.line(), 3);

    let f = edit("one\n\ntwo\n", 1, 0, &["d", "d", "p"]);
    assert_eq!(f.text(), "one\ntwo\n\n");

    let f = edit("one\n\ntwo\n", 1, 0, &["y", "y", "k", "P"]);
    assert_eq!(f.text(), "\none\n\ntwo\n");

    // counts scale: `3p` of an empty linewise register = three empty lines
    let f = edit("one\n\ntwo\n", 1, 0, &["y", "y", "j", "3", "p"]);
    assert_eq!(f.text(), "one\n\ntwo\n\n\n\n");
}

// ---- insert sessions reach the changelist ------------------------------------

/// vim 9.1 probe: after `i`-typing at line 2, `'.` is that line (first
/// non-blank) and `g;` is the exact first-typed offset. Plain insert
/// sessions used to bypass `bump` entirely, so both were stale.
#[test]
fn insert_session_records_change_position() {
    let mut f = common::Fixture::at("aaaa\nbbbb\ncccc\n", 1, 2);
    f.feed(["i"]);
    f.type_text("xy");
    f.feed(["<Esc>", "g", "g"]);
    f.feed(["'", "."]);
    assert_eq!(f.line(), 1, "'. marks the line of the typed change");
    f.feed(["g", ";"]);
    let bb = f.buf.line_start(1);
    assert_eq!(f.cursor(), bb + 2, "g; lands on the first typed offset");
    assert_eq!(f.text(), "aaaa\nbbxybb\ncccc\n");
}

/// `o` + Esc (a line created without typing) is still a change.
#[test]
fn open_line_records_change_position() {
    let mut f = common::Fixture::at("a\nb\n", 0, 0);
    f.feed(["o", "<Esc>", "g", "g"]);
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 1);
}

// ---- search failure feedback --------------------------------------------------

/// `/` + Enter with no match: vim prints E486 and rings; the engine used to
/// ring silently. `n` before any search: E35.
#[test]
fn search_failures_report_e486_and_e35() {
    let f = edit("abc\n", 0, 0, &["/", "z", "z", "<Enter>"]);
    assert_eq!(f.text(), "abc\n");
    assert!(f.host.bells > 0);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E486")),
        "statuses: {:?}",
        f.host.statuses
    );

    let f = edit("abc\n", 0, 0, &["n"]);
    assert!(f.host.statuses.iter().any(|s| s.starts_with("E35")));
    assert!(f.host.bells > 0);
}

// ---- cmdline backspace at empty prompt ----------------------------------------

/// vim probe (mode() after feedkeys): <BS> on an empty `:` prompt does NOT
/// leave cmdline mode — the engine used to cancel the prompt.
#[test]
fn backspace_on_empty_prompt_stays_in_cmdline() {
    let mut f = common::Fixture::new("abc\n");
    f.feed([":"]);
    assert!(matches!(f.vim.mode(), Mode::CommandLine { .. }));
    f.feed(["<BS>"]);
    assert!(
        matches!(f.vim.mode(), Mode::CommandLine { .. }),
        "BS on empty prompt must keep the prompt open"
    );
    f.feed(["<BS>", "<BS>"]);
    assert!(matches!(f.vim.mode(), Mode::CommandLine { .. }));
    f.feed(["<Esc>"]);
    assert_eq!(f.vim.mode(), Mode::Normal);
}

// ---- runaway replay & paste counts (engine-invariant, no vim probe) -----------

/// `.` after a count-insert re-runs the EnterInsert path with its count —
/// the replay inserts one copy at the cursor, then the exit replication
/// appends the rest (vim probe on "Xaa": `3iab<Esc>.` → `ababaabababbXaa`,
/// byte-identical to the engine).
#[test]
fn dot_repeat_after_count_insert_repeats_count() {
    let mut f = edit("aa\n", 0, 0, &["3", "i"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababaa\n");
    f.feed(["."]);
    assert_eq!(f.text(), "ababaabababbaa\n");
}

/// A mapping that expands to itself must hit the depth guard, not hang.
#[test]
fn self_recursive_mapping_is_bounded() {
    let mut f = common::Fixture::new("x\n");
    f.vim
        .keymaps_mut()
        .map_str(vimcore::keymap::ModeClass::Normal, "x", "x");
    f.feed(["x"]);
    assert_eq!(f.text(), "x\n");
    assert!(f.host.bells > 0, "guard trips with a bell");
}

/// A macro that plays itself nests forever at a constant queue size; the
/// pipeline guard must drop it and survive.
#[test]
fn self_playing_macro_is_bounded() {
    let mut f = edit("word\n", 0, 0, &["q", "a", "@", "a", "q"]);
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "word\n", "guard dropped the runaway replay");
}

/// `:1,2y` then `p`: the Ex yank feeds normal-mode paste.
#[test]
fn ex_yank_then_put_roundtrip() {
    let mut f = common::Fixture::at("a\nb\nc\n", 2, 0);
    f.feed([":"]);
    for c in "1,2y".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["<Enter>", "p"]);
    assert_eq!(
        f.text(),
        "a\nb\nc\na\nb\n",
        "p pastes below the cursor line"
    );
}

/// `''` after a search jump returns to the line the search started from.
#[test]
fn context_mark_after_search_jump() {
    let mut f = common::Fixture::at("top\nmid\nneedle\nbottom\n", 0, 0);
    f.feed(["/", "n", "e", "e", "d", "l", "e", "<Enter>"]);
    assert_eq!(f.line(), 2);
    f.feed(["'", "'"]);
    assert_eq!(f.line(), 0, "'' returns to the pre-search line");
}

/// Wide-char replace argument: `r` accepts multi-byte chars and keeps the
/// cursor on the replaced char. (Fixture `col` is a BYTE offset: 3 = on 文.)
#[test]
fn replace_char_with_wide_char() {
    let f = edit("abc\n", 0, 0, &["r", "中"]);
    assert_eq!(f.text(), "中bc\n");
    assert_eq!(f.cursor(), 0);
    let f = edit("中文\n", 0, 3, &["r", "x"]);
    assert_eq!(f.text(), "中x\n");
    assert_eq!(f.cursor(), "中".len());
}

/// A visual operator that deletes its own selection must record clamped
/// `last_visual` bounds — the raw anchor can point past the new end (fuzz
/// found `Vd` on the whole buffer storing `0..len_before`).
#[test]
fn visual_delete_records_clamped_last_visual() {
    let mut f = common::Fixture::new("你好, world 123 -45\n#tag\"\n");
    // a drag to the very end clamps ONTO the last character (vim never parks
    // the cursor past it), so the charwise delete leaves the final newline
    f.vim.set_visual_range(&f.buf, 0, f.buf.len());
    f.feed(["d"]);
    assert_eq!(f.text(), "\n", "charwise wipe up to the last char");
    // the LINEWISE whole-buffer wipe does remove everything
    let mut f = common::Fixture::new("你好, world 123 -45\n#tag\"\n");
    f.vim.set_visual_range(&f.buf, 0, 0);
    f.feed(["V", "G", "d"]);
    assert_eq!(f.text(), "", "whole-buffer linewise selection wiped");
    let (lo, hi) = f.vim.marks.last_visual.expect("last_visual recorded");
    assert!(hi <= f.buf.len(), "last_visual {lo}..{hi} past the new end");
    // gv after the wipe must not panic and stays in-bounds
    f.feed(["g", "v"]);
    assert!(f.cursor() <= f.buf.len());
}

// ---- runaway replay & paste counts (engine-invariant, no vim probe) -----------

/// `.` with an absurd count must clamp the queued work, not OOM the host:
/// the eager replay queue is bounded by the same budget as the pipeline
/// guard. (Pre-guard this test allocates ~1e9 keys and runs for minutes.)
#[test]
fn repeat_change_huge_count_is_bounded() {
    let mut f = edit("abc\n", 0, 0, &["x"]);
    f.feed(["9", "9", "9", "9", "9", "9", "9", "9", "9", "."]);
    // surviving instantly with a small buffer IS the assertion
    assert!(f.text().len() <= "abc\n".len());
}

/// Same bound for macro replay with a count.
#[test]
fn macro_huge_count_is_bounded() {
    let mut f = edit("abc\ndef\n", 0, 0, &["q", "a", "x", "q"]);
    f.feed(["9", "9", "9", "9", "9", "9", "9", "9", "9", "@", "a"]);
    // 100k budgeted x-replays consume "abc" and then no-op at the line end
    assert_eq!(f.text(), "\ndef\n");
}

/// `p` with an absurd count clamps the pasted size instead of allocating
/// register × count bytes.
#[test]
fn put_huge_count_is_bounded() {
    let f = edit(
        "abc\n",
        0,
        0,
        &[
            "y", "y", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "p",
        ],
    );
    let text = f.text();
    assert!(text.len() < 100_000_000, "pasted {} bytes", text.len());
}

// ---- count-repeat insert ------------------------------------------------------

/// vim's count-repeat insert: `3ifoo<Esc>` types foo three times, `3ofoo<Esc>`
/// opens three lines, `2a!` doubles the `!`. One undo step; the cursor ends
/// one left of the LAST copy.
#[test]
fn count_repeat_insert() {
    let mut f = edit("hello\n", 0, 0, &["3", "i"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababhello\n");
    assert_eq!(f.cursor(), "ababab".len() - 1);
    f.feed(["u"]);
    assert_eq!(f.text(), "hello\n", "one undo step restores everything");

    let mut f = edit("hello\n", 0, 0, &["3", "o"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hello\nab\nab\nab\n");
    assert_eq!(f.line(), 3);

    let mut f = edit("hello\n", 0, 0, &["2", "a"]);
    f.type_text("!");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "h!!ello\n");

    // tab-indented o copies keep the indent
    let mut f = edit("\thello\n", 0, 0, &["2", "o"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\thello\n\tx\n\tx\n");
}

/// Sessions that navigated (cursor left the typed-text end) or typed a
/// newline don't replicate — vim repeats literal input, we approximate by
/// only repeating plain type-then-escape.
#[test]
fn count_repeat_insert_skips_navigated_sessions() {
    // cursor moved off the typed-text end: no replication
    let mut f = edit("hello\n", 0, 0, &["3", "i"]);
    f.type_text("a");
    f.feed(["<Left>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ahello\n");

    // newline inside the session: no replication
    let mut f = edit("x\n", 0, 0, &["3", "i"]);
    f.type_text("a");
    f.feed(["<Enter>"]);
    f.type_text("b");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a\nbx\n");

    // count with c belongs to the motion (2cw changes two words), and the
    // cw trim still keeps trailing whitespace out
    let mut f = edit("word rest\n", 0, 0, &["2", "c", "w"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Z\n");
    let mut f = edit("word rest\n", 0, 0, &["c", "w"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Z rest\n");
}

// ---- visual r{char} -------------------------------------------------------------

/// vim 9.1 probes: charwise `vll r 0` replaces the covered chars; linewise
/// `V r -` fills each selected line to its own char count; blockwise fills
/// each row's covered span. Cursor lands on the selection start.
#[test]
fn visual_r_replaces_selection() {
    // charwise: "abcdef", select "bcd" (l then v2l), r0 → "a000ef"
    let f = edit("abcdef\n", 0, 1, &["v", "2", "l", "r", "0"]);
    assert_eq!(f.text(), "a000ef\n");
    assert_eq!(f.cursor(), "a".len());

    // linewise: each line filled to its own char count
    let f = edit("abcdef\nxy\n", 0, 0, &["V", "j", "r", "-"]);
    assert_eq!(f.text(), "------\n--\n");

    // wide chars count by CHAR, not display width (vim probe: 中文ab → ----)
    let f = edit("中文ab\n", 0, 0, &["V", "r", "-"]);
    assert_eq!(f.text(), "----\n");

    // blockwise 3 columns wide (vim probe: rows fill their covered span,
    // rows run out of chars keep their tail: "xy" fills 2 of 3 columns)
    let f = edit("abcdef\nxyz\n", 0, 0, &["<C-v>", "l", "j", "l", "r", "0"]);
    assert_eq!(f.text(), "000def\n000\n");
    let f = edit("abcdef\nxy\n", 0, 0, &["<C-v>", "l", "j", "r", "0"]);
    assert_eq!(f.text(), "00cdef\n00\n");

    // empty selection is impossible, but a single-char selection works
    let f = edit("abc\n", 0, 0, &["v", "r", "z"]);
    assert_eq!(f.text(), "zbc\n");
}

// ---- :yank -----------------------------------------------------------------------

/// `:{range}y[ank] [reg]` yanks the range's lines, like `:d` deletes them.
#[test]
fn ex_yank_range() {
    let mut f = common::Fixture::at("a\nb\nc\n", 2, 0);
    f.feed([":"]);
    for c in "1,2y".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["<Enter>"]);
    assert_eq!(f.text(), "a\nb\nc\n", ":y does not modify the buffer");
    let reg = f.vim.registers.get('"').expect("unnamed set");
    assert_eq!(reg.text, "a\nb\n");
    assert!(reg.kind == vimcore::registers::RegisterKind::Linewise);

    // explicit register + default range = current line
    let mut f = common::Fixture::at("a\nb\nc\n", 1, 0);
    f.feed([":"]);
    for c in "y b".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["<Enter>"]);
    let reg = f.vim.registers.get('b').expect("named register");
    assert_eq!(reg.text, "b\n");

    // a numeric argument is a line count (`:2y 2` yanks lines 2-3)
    let mut f = common::Fixture::at("a\nb\nc\n", 1, 0);
    f.feed([":"]);
    for c in "2y 2".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["<Enter>"]);
    assert_eq!(f.vim.registers.get('"').unwrap().text, "b\nc\n");
}

// ---- misc parity notes -------------------------------------------------------------

/// `G`/`gg` with count > line count clamps (vim parity), and a no-move jump
/// (`1G` already on line 1) must not corrupt the jumplist.
#[test]
fn goto_line_clamps_and_noop_jump_is_safe() {
    let f = edit("a\nb\nc\n", 0, 0, &["9", "G"]);
    assert_eq!(f.line(), 2);
    let f = edit("a\nb\nc\n", 0, 0, &["1", "G", "<C-o>", "<C-o>"]);
    assert_eq!(f.line(), 0);
    assert!(f.host.bells > 0, "walking past the jumplist start bells");
}
