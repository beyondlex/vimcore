//! Round-29 probes (part two): block-insert sessions, boundary buffers, `~`/E18 feedback, macro
//! append.

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

/// Probe A: the block-visual `c` session — typing, backspace, and after Esc the replica shrinks
/// by the net typing of the typed line.
#[test]
fn probe_block_change_backspace_shrinks_replica() {
    let mut f = Fixture::at("abcd\nefgh\n", 0, 1);
    f.feed(["<C-v>", "j", "c"]); // select one column on each of the two lines (b/f), enter block insert
    f.type_text("XY");
    f.feed_raw(Key::backspace()); // remove Y → net input X
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "aXcd\neXgh\n", "the replica must match the net input of the typed line");
}

/// Probe B: block-visual `c` with an immediate Esc — the selected columns are deleted, the lines
/// stay, no replica, no panic.
#[test]
fn probe_block_change_empty_escape() {
    let f = {
        let mut f = Fixture::at("abcd\nefgh\n", 0, 1);
        f.feed(["<C-v>", "j", "c", "<Esc>"]);
        f
    };
    assert_eq!(f.text(), "acd\negh\n", "the selected columns (b/f) are deleted, no replica at all");
}

/// Probe C: `$` block delete empties the selected lines (vim: deletes 0..end of line content, not
/// the line itself); the `.` redo on an empty line is safe (the selection covers the same two
/// lines, there is nothing to delete, no panic).
#[test]
fn probe_dot_after_dollar_block_delete() {
    let mut f = Fixture::at("aaa bbb\nccc ddd\neee fff\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "d"]);
    assert_eq!(f.text(), "\n\neee fff\n");
    f.feed(["."]);
    assert_eq!(f.text(), "\n\neee fff\n", "replaying `.` onto an empty line is a safe no-op");
}

/// Probe D: `~` on an empty line is a no-op and does not panic.
#[test]
fn probe_tilde_on_empty_line_noop() {
    let f = edit("\nfoo\n", 0, 0, &["~"]);
    assert_eq!(f.text(), "\nfoo\n");
}

/// Probe E: `<C-r>` in insert mode pasting a linewise register.
/// The register text ("one\n") lands at the line-end append point: the line becomes "twoone", the
/// register's own newline starts a new line, and the buffer's original line terminator leaves one
/// more empty line — same as vim.
#[test]
fn probe_insert_ctrl_r_linewise_register() {
    let mut f = Fixture::at("one\ntwo\n", 0, 0);
    f.feed(["y", "y"]); // "0 = "one\n"
    f.feed(["j", "$"]);
    f.feed(["a"]); // append at the line end
    f.feed(["<C-r>", "0"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one\ntwoone\n\n");
}

/// Probe F: `:0d` deletes the first line (vim semantics: address 0 acts on the first line,
/// equivalent to line 1).
#[test]
fn probe_ex_zero_delete() {
    let f = {
        let mut f = Fixture::at("a\nb\nc\n", 0, 0);
        f.feed([":", "0", "d", "<CR>"]);
        f
    };
    assert_eq!(f.text(), "b\nc\n");
}

/// Probe G: `<C-a>` on `-0`: the minus sign is dropped when the result is non-negative (vim 9.1
/// semantics).
#[test]
fn probe_increment_negative_zero_drops_sign() {
    let f = edit("-0\n", 0, 1, &["<C-a>"]);
    assert_eq!(f.text(), "1\n");
}

/// Probe H: `qA` append-recording then `@@` replays the whole macro; the content holds the steps
/// of two sessions.
#[test]
fn probe_qa_append_then_atat() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["q", "a", "x", "q"]); // a = [x]
    f.feed(["j"]);
    f.feed(["q", "A", "x", "q"]); // a = [x, x]
    assert_eq!(f.vim.macro_len('a'), 2);
    f.feed(["@@", "@@"]); // replay twice in a row (2 steps each)
    assert_eq!(f.text(), "b\nd\n", "@@ replays macro a: two x");
}

/// Probe I: Esc after `g`/`gu` — a quiet cancel, no bell.
#[test]
fn probe_pending_prefix_escape_is_quiet() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["g", "<Esc>"]);
    f.feed(["g", "u", "<Esc>"]);
    f.feed(["3", "<Esc>"]);
    assert_eq!(f.host.bells, 0, "cancelling a prefix must be quiet, got {} bells", f.host.bells);
    assert_eq!(f.text(), "abc\n");
    // After a cancel the count must not leak into the next command.
    f.feed(["x"]);
    assert_eq!(f.text(), "bc\n", "after Esc cancels, x deletes only 1 char");
}

/// Probe J: `v$` + `d` on an empty line — the visual selection of an empty line covers its newline,
/// so d deletes the empty line itself (vim's `vd` empty-line delete semantics), no panic.
#[test]
fn probe_visual_dollar_on_empty_line() {
    let f = edit("a\n\nb\n", 1, 0, &["v", "$", "d"]);
    assert_eq!(f.text(), "a\nb\n", "the empty line's selection covers the newline, d deletes that empty line");
}

/// Probe K: `:%s/./x/` on an empty buffer — E486, no panic.
#[test]
fn probe_substitute_on_empty_buffer() {
    let mut f = Fixture::new("");
    f.feed([":", "%", "s", "/", ".", "/", "x", "<CR>"]);
    assert_eq!(f.text(), "");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E486")),
        "no match must report E486, got {:?}",
        f.host.statuses
    );
}

/// Probe L: `ciw` on a single-char buffer.
#[test]
fn probe_ciw_single_char_buffer() {
    let mut f = Fixture::at("x", 0, 0);
    f.feed(["c", "i", "w"]);
    f.type_text("y");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "y");
}

/// Probe M: `.` replays a delete with an explicit register prefix (`"add` → `.` still goes into "a).
#[test]
fn probe_dot_replays_named_register_delete() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["\"", "a", "d", "d"]);
    f.feed(["."]);
    assert_eq!(f.text(), "cc\n", "`.` repeats the \"ad delete of two lines");
    // Register a holds the content of the second delete.
    f.feed(["G"]);
    f.feed(["\"", "a", "P"]);
    assert_eq!(f.text(), "bb\ncc\n", "\"aP puts the lines deleted by `.`");
}
