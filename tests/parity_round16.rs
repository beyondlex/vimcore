//! Round-16 regression: empty-text linewise count repeat (`3o<Esc>`), the autoindent empty-line
//! lifecycle (vim's did_ai flag: the Esc/CR/BS three-way strip), the bells of the failed-delete
//! family, the mode guard of the host drag-select API, and showcmd's char-arg command letter.
//! All expected behaviors are evidenced by vim 9.1 probes (probe scripts and output: NOTES round
//! sixteen, the P/Q/R/S series).

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

// ---- Empty-text counted repeat (probes Q3-Q5 / R1 / R7 / Q2) ----------------

/// PROBE R1/R7: vim 9.1 `3o<Esc>` on an indented line → three **plain empty lines** (the ai
/// indent does not travel with the empty copy); the cursor lands on the last copied line. The
/// old engine abstained as soon as the text was empty and opened only one line.
#[test]
fn empty_count_repeat_opens_lines() {
    let f = edit("    ind\n", 0, 0, &["3", "o", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n\n\n", "R1: three PLAIN empty lines");
    assert_eq!(f.line(), 3, "cursor on the last copy");

    let f = edit("x\n", 0, 0, &["2", "o", "<Esc>"]);
    assert_eq!(f.text(), "x\n\n\n", "R7: 2o<Esc> → two lines");

    let f = edit("    ind\n", 0, 0, &["3", "O", "<Esc>"]);
    assert_eq!(f.text(), "\n\n\n    ind\n", "3O<Esc> opens three above");
}

/// PROBE Q3/Q4/Q5: a charwise empty session (`3i`/`3A`/`3R` + Esc) repeats nothing.
#[test]
fn empty_charwise_repeat_is_noop() {
    let f = edit("x\n", 0, 0, &["3", "i", "<Esc>"]);
    assert_eq!(f.text(), "x\n", "Q3: 3i<Esc>");
    let f = edit("abc\n", 0, 0, &["3", "A", "<Esc>"]);
    assert_eq!(f.text(), "abc\n", "Q5: 3A<Esc>");
    let f = edit("abc\n", 0, 0, &["3", "R", "<Esc>"]);
    assert_eq!(f.text(), "abc\n", "Q4: 3R<Esc>");
}

/// The abstaining motions apply to an empty session as well: `2o<Down><Esc>` does not replicate.
#[test]
fn navigated_empty_repeat_does_not_replicate() {
    let mut f = edit("a\nb\n", 0, 0, &["2", "o"]);
    f.feed(["<Down>", "<Esc>"]);
    assert_eq!(f.text(), "a\n\nb\n");
}

// ---- The autoindent empty-line lifecycle (probes S1-S8) ---------------------

/// PROBE S1: `o<Esc>` leaves a **plain empty line** (not indented), cursor at the line start
/// (2,1).
#[test]
fn open_line_esc_strips_autoindent() {
    let mut f = edit("    ind\n", 0, 0, &["o"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n", "S1");
    assert_eq!(f.cursor(), 8, "cursor at line start like vim (2,1)");
}

/// PROBE S2: `o<Down><Esc>` — a line that was never typed on **keeps** its indent once the
/// cursor leaves it (Esc strips only the current line).
#[test]
fn open_line_then_down_keeps_indent() {
    let mut f = edit("    ind\nzz\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.feed(["<Down>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n    \nzz\n", "S2");
}

/// PROBE S3: `o<CR><Esc>` leaves both lines empty — CR strips the stale indent of the line being
/// split before splitting it, then Esc strips the new line's.
#[test]
fn open_line_cr_esc_leaves_both_empty() {
    let mut f = edit("    ind\n", 0, 0, &["o"]);
    f.feed(["<CR>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n\n", "S3");
}

/// PROBE S4: typing everything then deleting it all (`ofoo<BS><BS><BS><Esc>`) **keeps** the
/// indent — typing clears did_ai.
#[test]
fn typed_then_deleted_keeps_indent() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.type_text("foo");
    f.feed(["<BS>", "<BS>", "<BS>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n    \n", "S4");
}

/// PROBE S5: `o<BS>` deletes the whole indent run in one stroke (not one char).
#[test]
fn backspace_on_untouched_indent_deletes_it_all() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n", "S5");
}

/// PROBE S6/S8: `O<Esc>` and `cc<Esc>` are of the same family — a plain empty line.
#[test]
fn open_above_and_cc_strip_too() {
    let mut f = edit("    ind\n", 0, 0, &["O"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\n    ind\n", "S6");
    let f = edit("    ind\n", 0, 0, &["c", "c", "<Esc>"]);
    assert_eq!(f.text(), "\n", "S8");
}

/// Typed text keeps its indent as usual (R2: `3ofoo<Esc>` gives three copies all indented).
#[test]
fn typed_open_line_keeps_indent_and_replicates_it() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "3", "o"]);
    f.type_text("foo");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ind\n    foo\n    foo\n    foo\n", "R2");
    assert_eq!(f.line(), 3, "cursor on the last copy");
}

/// Behavior is unchanged under noai: `o` never inserts an indent there, so Esc has nothing to
/// strip.
#[test]
fn noautoindent_open_line_unchanged() {
    let f = edit(
        "    ind\n",
        0,
        0,
        &[":", "s", "e", "t", " ", "n", "o", "a", "i", "<CR>", "o", "<Esc>"],
    );
    assert_eq!(f.text(), "    ind\n\n");
}

// ---- Bells of the failed-delete family (same as vim) ------------------------

/// `x` on an empty line, `X` at the line start, `D` on an empty line, `J` at EOF: vim bells, the
/// old engine was silent.
#[test]
fn failed_deletes_and_joins_bell() {
    let f = edit("a\n\nb\n", 1, 0, &["x"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1, "x on an empty line must bell");

    let f = edit("ab\n", 0, 0, &["X"]);
    assert_eq!(f.text(), "ab\n");
    assert_eq!(f.host.bells, 1, "X at column zero must bell");

    let f = edit("a\n\nb\n", 1, 0, &["D"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1, "D on an empty line must bell");

    let f = edit("a\nb\n", 1, 0, &["J"]);
    assert_eq!(f.text(), "a\nb\n");
    assert_eq!(f.host.bells, 1, "J at EOF must bell");

    let f = edit("a\nb\n", 1, 0, &["g", "J"]);
    assert_eq!(f.text(), "a\nb\n");
    assert_eq!(f.host.bells, 1, "gJ at EOF must bell");
}

/// Probe R3: when the counted join cannot reach the end of the buffer, the reachable joins still
/// complete normally. `3J` at EOF-1: only one of the two seams is left, and vim bells for the
/// failed one; `2J` (a single seam) fully succeeds without a bell (probe R3 showed no bell).
#[test]
fn partial_join_bells_after_doing_its_work() {
    let f = edit("a\nb\nc\n", 1, 0, &["2", "J"]);
    assert_eq!(f.text(), "a\nb c\n", "the reachable join still runs");
    assert_eq!(f.host.bells, 0, "2J has exactly one seam — no failure");

    let f = edit("a\nb\nc\n", 1, 0, &["3", "J"]);
    assert_eq!(f.text(), "a\nb c\n", "the reachable join still runs");
    assert_eq!(f.host.bells, 1, "the second seam is impossible — bell");
}

/// `s` on an empty line: the delete half fails and bells, but the insert half still enters.
#[test]
fn substitute_char_on_empty_line_bells_then_inserts() {
    let f = edit("a\n\nb\n", 1, 0, &["s"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1);
    assert!(
        f.vim.mode_indicator().contains("INSERT"),
        "s still enters insert mode"
    );
}

// ---- Mode guard of the host drag-select API ---------------------------------

/// Drag-select is ignored during an insert session / while the prompt is open: the mode machine
/// must not split into two halves (the old implementation rewrote mode to Visual while the
/// insert_session was still alive).
#[test]
fn host_visual_range_ignored_outside_normal_and_visual() {
    let mut f = Fixture::new("alpha\nbeta\n");
    f.feed_raw(Key::char('i'));
    f.vim.set_visual_range(&f.buf, 0, 5);
    assert!(
        f.vim.mode_indicator().contains("INSERT"),
        "drag during insert must not change the mode"
    );
    assert!(f.vim.visual_selection().is_none());
    // The session is still intact: Esc exits normally
    f.feed_raw(Key::escape());
    assert_eq!(f.vim.mode_indicator(), "");

    f.feed_raw(Key::char(':'));
    f.vim.set_visual_range(&f.buf, 0, 5);
    assert_eq!(
        f.vim.mode_indicator(),
        "CMDLINE",
        "drag during the prompt must not change the mode"
    );
}

/// A click while the prompt is open does not move the buffer cursor (the default address of an
/// Ex range reads it).
#[test]
fn host_click_during_cmdline_ignored() {
    let mut f = Fixture::new("alpha\nbeta\n");
    let before = f.vim.cursor_offset();
    f.feed_raw(Key::char(':'));
    f.vim.set_cursor_offset(&f.buf, 8);
    assert_eq!(
        f.vim.cursor_offset(),
        before,
        "click at the prompt must not move the buffer cursor"
    );
}

// ---- showcmd completion -----------------------------------------------------

/// While waiting for a char-argument, showcmd displays the command letter (blank in the old
/// implementation); a leading count is displayed too and the argument is appended once it
/// arrives.
#[test]
fn showcmd_shows_pending_char_arg_command() {
    let mut f = Fixture::new("abc\n");
    f.feed(["3"]);
    assert_eq!(f.vim.showcmd(), "3", "count shows alone");
    f.feed(["r"]);
    assert_eq!(f.vim.showcmd(), "3r", "count then r");
    f.feed(["x"]);
    assert_eq!(f.vim.showcmd(), "", "completed command clears showcmd");

    let mut f = Fixture::new("abc\n");
    f.feed(["f"]);
    assert_eq!(f.vim.showcmd(), "f");
    let mut f = Fixture::new("abc\n");
    f.feed(["'", ]);
    assert_eq!(f.vim.showcmd(), "'");
    let mut f = Fixture::new("abc\n");
    f.feed(["@"]);
    assert_eq!(f.vim.showcmd(), "@");
}
