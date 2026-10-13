//! Round-29 probes: abnormal key sequences and boundary data, catching regressions.

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

/// Probe 1: after a macro recording session ends, `.` must not replay the `q{reg}` prefix.
///
/// The old implementation recorded `q`/`a` together into `recording`; at exit_insert
/// commit time they leaked into `last_change`; replaying [q, a, …] with `.` would reopen
/// a macro recording session that never stops (the status UI shows recording and later
/// keys are swallowed into the macro).
#[test]
fn probe_dot_after_macro_session_does_not_rearm_recording() {
    // qaxix<Esc>q: enter macro recording, i enters insert, type x, Esc exits, stop recording.
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["q", "a", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q"]);
    assert_eq!(f.text(), "xab\ncd\n");
    assert_eq!(f.vim.macro_len('a'), 3, "the macro should record the 3 steps [i, Text(x), Esc]");
    assert!(
        f.vim.macro_recording().is_none(),
        "must not still be recording after the macro stops"
    );

    // `.` replays the insert: must not re-enter a macro recording session, macro content
    // stays as-is. (The `q`/`a` prefix is cleared from recording at macro start by the
    // end_command at the tail of complete_char_arg, so last_change holds only
    // [i, Text, Esc].)
    f.feed(["."]);
    assert_eq!(f.text(), "xxab\ncd\n", "`.` should repeat the insertion of x");
    assert!(
        f.vim.macro_recording().is_none(),
        "`.` replay must not restart macro recording"
    );
    assert_eq!(f.vim.macro_len('a'), 3, "`.` must not clear/overwrite macro a");

    // the x after `.` is a plain delete (the cursor rests on the first char of the replayed insert).
    f.feed(["x"]);
    assert_eq!(f.text(), "xab\ncd\n", "the x after `.` is a plain delete");
    assert_eq!(f.vim.macro_len('a'), 3, "macro a should not capture keys after `.`");
}

/// Probe 1b: a macro without edits (`qaq`) must not pollute the `.` replay record.
#[test]
fn probe_empty_macro_keeps_last_change() {
    let mut f = Fixture::at("foo\n", 0, 0);
    f.feed(["x"]); // last_change = [x]
    f.feed(["q", "a", "q"]); // empty macro
    f.feed(["G"]); // move the cursor
    f.feed(["."]);
    assert_eq!(f.text(), "o\n", "`.` should repeat [x] (foo → oo → o), not be overwritten by the empty macro");
}

/// Probe 2: incsearch preview with an emptied pattern.
///
/// After `/foo` press BS three times to clear the prompt: an empty regex matches zero-
/// width at every byte position, and the old implementation published tens of thousands
/// of zero-width matches to the host. vim's preview falls back to the previous search's
/// highlights.
#[test]
fn probe_incsearch_empty_pattern_restores_previous_highlights() {
    let mut f = Fixture::at("foo bar foo\n", 0, 0);
    f.feed(["/"]); // open the prompt
    f.feed(["f", "o", "o"]);
    let preview_count = f.host.highlights.len();
    assert!(preview_count >= 2, "the incsearch preview should show matches of foo");

    // clear the prompt: the preview must fall back (no publishing of all zero-width matches).
    f.feed(["<BS>", "<BS>", "<BS>"]);
    assert!(
        f.host.highlights.iter().all(|r| !r.is_empty()),
        "the empty-pattern preview must not publish zero-width matches, got {:?}",
        f.host.highlights
    );
    assert_eq!(
        f.host.highlights.len(),
        0,
        "with no previously accepted pattern, the empty preview should clear the highlights"
    );
}

/// Probe 3: `dgn` with no pattern quietly reports E35 without panicking.
#[test]
fn probe_dgn_without_pattern_reports_e35() {
    let f = edit("abc\n", 0, 0, &["d", "g", "n"]);
    assert_eq!(f.text(), "abc\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E35")),
        "should report E35, got {:?}",
        f.host.statuses
    );
}

/// Probe 4: hex/binary prefix boundary data for `<C-a>`.
#[test]
fn probe_increment_radix_edges() {
    // cursor on the 0 of `0x` with no hex digit after: 0x is not a valid literal, fall back to decimal increment of 0.
    let f = edit("0x tail\n", 0, 0, &["<C-a>"]);
    assert_eq!(f.text(), "1x tail\n");

    // cursor on x: the 0 is before the cursor, vim reports E18 and does not move.
    let f = edit("0x tail\n", 0, 1, &["<C-a>"]);
    assert_eq!(f.text(), "0x tail\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E18")),
        "no digit after the cursor should report E18, got {:?}",
        f.host.statuses
    );

    // 0b prefix followed by 2 (not a binary digit): not treated as binary.
    let f = edit("0b2 tail\n", 0, 0, &["<C-a>"]);
    assert_eq!(f.text(), "1b2 tail\n");

    // huge numbers saturate without panicking.
    let f = edit("99999999999999999999999\n", 0, 0, &["<C-a>"]);
    assert!(!f.text().is_empty());
}

/// Probe 5: pressing Enter in Replace mode replaces the cursor char with a newline (vim: splits the line).
#[test]
fn probe_replace_mode_enter_splits_line() {
    let f = {
        let mut f = Fixture::at("abc\n", 0, 1);
        f.feed(["R"]);
        f.feed_raw(Key::named("enter"));
        f
    };
    assert_eq!(f.text(), "a\nc\n", "R<CR> should turn the cursor char into a newline");
}

/// Probe 6: Esc immediately after visual-block `I` (no input) — must not copy, must not panic.
#[test]
fn probe_block_insert_empty_escape() {
    let f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", "I", "<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
}

/// Probe 7: `:reg` after `"Ayy` uppercase-register append does not panic, and pasting
/// yields the merged result.
#[test]
fn probe_uppercase_append_yank() {
    let mut f = Fixture::at("one\ntwo\n", 0, 0);
    f.feed(["\"", "a", "y", "y"]); // first put one into "a
    f.feed(["j"]);
    f.feed(["\"", "A", "y", "y"]); // "A appends two
    f.feed(["G"]);
    f.feed(["\"", "a", "p"]);
    assert_eq!(f.text(), "one\ntwo\none\ntwo\n", "the append should yield the two lines one+two");
}

/// Probe 8: after blockwise visual `p` replaces using the block register, `gv` restores
/// the original block range.
#[test]
fn probe_block_put_gv_restores_bounds() {
    let mut f = Fixture::at("abcd\nefgh\n", 0, 0);
    // block-yank the first column (2 lines × 1 column), then block-put it back.
    f.feed(["<C-v>", "j", "y"]);
    f.feed(["<C-v>", "j", "l", "p"]);
    assert_eq!(f.text(), "acd\negh\n", "block register lines [a, e] replace [ab, ef] row by row");
}
