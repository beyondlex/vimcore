//! Round-30 probes (II): `.` and Ex boundaries, macros/q1/m1, search offsets, visual and undo.
//!
//! Methodology: every vim expectation was first verified with the local vim 9.1 `-s`
//! typeahead (no arrow keys, closing with Esc:wq — see the channel lessons in the
//! probe_round30.rs header).

mod common;

use common::{edit, Fixture};

// ---------------------------------------------------------------- `.` and Ex

/// Probe W1: `.` does not repeat `:s` (vim 9.1 evidence: after `:1,1s/a/B/`, `j.` leaves
/// line 2 unchanged — `.` only replays normal-mode changes; replaying a substitution is
/// `&`/`:&`'s job).
#[test]
fn dot_does_not_repeat_substitute() {
    let f = edit(
        "aaa\naaa\naaa\n",
        0,
        0,
        &[":", "1", ",", "1", "s", "/", "a", "/", "B", "/", "<CR>", "j", ".", "<CR>"],
    );
    // `j` moves; on line 2 `.` has no normal change to replay (:s does not enter last_change)
    assert_eq!(f.text(), "Baa\naaa\naaa\n");
}

// ---------------------------------------------------------------- macros and registers

/// Probe W2: `q1` digit-register recording (vim docs: q{0-9a-zA-Z"}; 9.1 probe: keys
/// execute while recording, and the replay `@1` is the real test — x inserted twice).
#[test]
fn digit_register_macro_roundtrip() {
    let mut f = edit("abc\n", 0, 0, &["q", "1", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q", "@", "1"]);
    assert_eq!(f.text(), "xxabc\n", "recording executes x once (IME path), @1 replays it once more");
}

/// Probe W3: `q/` invalid register rings the bell and does not start recording (same in
/// vim; the following / opens the search prompt).
#[test]
fn q_invalid_register_bells_and_stays_idle() {
    let mut f = edit("abc\n", 0, 0, &["q", "/"]);
    assert!(f.host.bells > 0, "q/ should ring the bell");
    // / still opens the search prompt (we are not in register-pending state), b goes into the prompt normally
    f.feed(["/", "b", "<CR>"]);
    assert_eq!(f.cursor(), "a".len(), "/b searches normally, landing on b");
}

/// Probe W4: `m1` invalid mark name — vim 9.1 silently no-ops (file unchanged), and the
/// engine likewise keeps no record (marks.set accepts only letters plus ^ .).
#[test]
fn m_digit_mark_is_rejected() {
    let f = edit("abc\n", 0, 0, &["m", "1"]);
    assert_eq!(f.text(), "abc\n");
    // jumping with '1 reports E20 (mark not set)
    let f = edit("abc\n", 0, 0, &["m", "1", "'", "1"]);
    assert_eq!(f.cursor(), 0, "after m1, '1 must have no landing point");
}

// ---------------------------------------------------------------- search and highlight

/// Probe W5: search skips the match at the cursor (vim: search starts after
/// the cursor) plus an empty `/` + Enter re-searches the last pattern from the
/// current position forward (with wrap). Engine measured sequence: `/foo<CR>`
/// lands at 8 (skipping the current match at 0), `n` wraps to 0, an empty `/`
/// + CR from 0 lands at 8 again.
#[test]
fn empty_search_enter_reuses_last_pattern() {
    let mut f = edit("foo bar foo\n", 0, 0, &["/", "f", "o", "o", "<CR>"]);
    assert_eq!(f.cursor(), 8, "search skips the match at the cursor");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 0, "n wraps to the first match");
    // empty / + CR: re-search the last pattern, direction follows the current prompt
    f.feed(["/", "<CR>"]);
    assert_eq!(f.cursor(), 8, "the re-search goes forward from the current position");
}

// ---------------------------------------------------------------- visual and undo

/// Probe W6: after `o<Esc>` (opened empty line), `u` undoes the opened line (vim: the line disappears).
#[test]
fn empty_open_line_undo() {
    let f = edit("ab\n", 0, 0, &["o", "<Esc>", "u"]);
    assert_eq!(f.text(), "ab\n", "u removes the empty line");
}

/// Probe W7: visual-line `Vj>` indents two lines; a single undo restores both (group semantics).
#[test]
fn visual_line_indent_undo_group() {
    let f = edit("a\nb\nc\n", 0, 0, &["V", "j", ">", "u"]);
    assert_eq!(f.text(), "a\nb\nc\n", "one u restores both lines' indent");
}

/// Probe W8: `cc` inserts on an empty line; `.` replays at the right position (vim: `.`
/// executes at the replay point).
#[test]
fn cc_on_empty_line_then_dot() {
    let mut f = edit("\nab\n", 0, 0, &["c", "c"]);
    f.type_text("X");
    f.feed(["<Esc>", "j", "."]);
    // line 2 "ab" was cleared by cc and X inserted; the `.` replay acts on line 2 (the old ab line)
    assert_eq!(f.text(), "X\nX\n");
}

/// Probe W9: `diw` on a single-char buffer — vim deletes the char and leaves an empty line.
#[test]
fn diw_on_single_char_line() {
    let f = edit("x\n", 0, 0, &["d", "i", "w"]);
    assert_eq!(f.text(), "\n");
}

/// Probe W10: `ci(` without a matching paren — rings the bell and does not enter insert.
#[test]
fn ci_on_unbalanced_parens_bells() {
    let f = edit("abc\n", 0, 0, &["c", "i", "("]);
    let bells = f.host.bells;
    assert!(bells > 0, "unbalanced parens should ring the bell");
    assert!(
        !matches!(f.vim.mode(), vimcore::mode::Mode::Insert),
        "must not enter insert mode"
    );
}

// ---------------------------------------------------------------- Ex misc boundaries

/// Probe W11: after `:%d` empties the buffer there is still one line (the empty-line
/// invariant), cursor at 0.
#[test]
fn percent_delete_leaves_single_empty_line() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "%", "d", "<CR>"]);
    assert_eq!(f.text(), "");
    assert_eq!(f.cursor(), 0);
}

/// Probe W12: `:1,2d a` goes into the register + `"ap` pastes it back. p pastes **after**
/// the cursor line (after the delete the cursor sits on the l3 line), so the restored
/// order is l3/l1/l2.
#[test]
fn ex_delete_into_named_register() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "1", ",", "2", "d", " ", "a", "<CR>", "\"", "a", "p"]);
    assert_eq!(f.text(), "l3\nl1\nl2\n");
}

/// Probe W13: the `:set ts?` query feedback goes through status_message.
#[test]
fn set_query_reports_value() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=4")),
        "actual: {:?}",
        f.host.statuses
    );
}
