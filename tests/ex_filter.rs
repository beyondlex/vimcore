//! `:{range}!{filter}` Ex filtering and the bare `:!{cmd}` shell escape (2026-10-09, added so tk's
//! Bench can spell `:%!<cmd>`). The assertion standard matches the repository: **evidenced by vim
//! 9.1** (same oracle channel as the audit3_b comments). Filtering through the `!` prompt
//! (`!!`/`!{motion}`) is already covered by audit3_b B3; only the Ex form is tested here.
//!
//! Checked divergence: in vim's `:!` the `|` belongs to the shell (`:!echo a|b` prints "a|b"), and
//! the engine's `!` branch is ordered before the `|` bar split, so they agree.

mod common;

use common::{edit, Fixture};

/// `:%!tr a-z A-Z`: the whole buffer goes through the filter. This is the main case of tk Bench.
#[test]
fn percent_filter_whole_buffer() {
    let f = edit(
        "hello\nworld\n",
        0,
        0,
        &[":", "%", "!", "t", "r", " ", "a", "-", "z", " ", "A", "-", "Z", "<CR>"],
    );
    assert_eq!(f.text(), "HELLO\nWORLD\n", "vim: %!tr filters the whole buffer");
}

/// `:2!rev`: filtering over a single-line range, other lines untouched.
#[test]
fn single_line_range_filter() {
    let f = edit(
        "ab\ncd\n",
        1,
        0,
        &[":", "2", "!", "r", "e", "v", "<CR>"],
    );
    assert_eq!(f.text(), "ab\ndc\n", "vim: 2!rev filters only the second line");
}

/// Clamping inside a range (E16, then an explicit clamp to tolerate `:2,99!...`) — no: vim reports
/// E16 for an out-of-range address plus a command and runs nothing; what this verifies is exactly
/// that the E16 path is not swallowed by the `!` branch first.
#[test]
fn out_of_buffer_range_still_e16() {
    let f = edit(
        "ab\ncd\n",
        0,
        0,
        &[":", "2", ",", "9", "!", "r", "e", "v", "<CR>"],
    );
    assert_eq!(f.text(), "ab\ncd\n", "vim: out-of-range plus command = E16, buffer untouched");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "must report E16: {:?}",
        f.host.statuses
    );
}

/// Bare `:!{cmd}`: a shell escape — the buffer is untouched and the output goes to the status line
/// (vim shows the output in the message area; the engine's status is a single line, taking the
/// first line plus a line-count hint).
#[test]
fn bare_bang_is_shell_escape() {
    let f = edit(
        "hello\n",
        0,
        0,
        &[":", "!", "e", "c", "h", "o", " ", "h", "i", "<CR>"],
    );
    assert_eq!(f.text(), "hello\n", "vim: a bare :! does not change the buffer");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("hi")),
        "the output must reach the status line: {:?}",
        f.host.statuses
    );
}

/// Empty `:!`: E471 (vim: Argument required), buffer untouched.
#[test]
fn bare_bang_empty_argument_e471() {
    let f = edit("hello\n", 0, 0, &[":", "!", "<CR>"]);
    assert_eq!(f.text(), "hello\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E471")),
        "must report E471: {:?}",
        f.host.statuses
    );
}

/// The filter participates in undo: after `:%!tr`, `u` returns to the original text (the same
/// run_shell_filter undo-group semantics as the `!!` prompt path).
#[test]
fn percent_filter_is_undoable() {
    let mut f = edit(
        "hello\nworld\n",
        0,
        0,
        &[
            ":", "%", "!", "t", "r", " ", "a", "-", "z", " ", "A", "-", "Z", "<CR>",
        ],
    );
    assert_eq!(f.text(), "HELLO\nWORLD\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "hello\nworld\n", "vim: the filter is undone in one step");
}

/// The `|` inside `:` belongs to the shell: the output of `:!echo 'a|b'` contains `|` (vim does no
/// bar splitting for `:!` — that is precisely why the `!` branch must be ordered before the bar
/// split; unquoted, the shell interprets the pipeline itself, and vim and the engine agree).
#[test]
fn bang_bar_belongs_to_shell() {
    let f = edit(
        "x\n",
        0,
        0,
        &[
            ":", "!", "e", "c", "h", "o", " ", "'", "a", "|", "b", "'", "<CR>",
        ],
    );
    assert_eq!(f.text(), "x\n", "buffer untouched");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("a|b")),
        "the pipe char inside quotes must reach the shell verbatim: {:?}",
        f.host.statuses
    );
}

/// Compile-time placeholder: keeps the Fixture import used (prevents an unused warning before the
/// first test).
#[allow(dead_code)]
fn _uses(f: &Fixture) {
    let _ = f.text();
}
