//! Round-34 probes: regressions clearing the round-33 audit ledger (2026-10-08).
//!
//! Covers three fix groups:
//! * Unterminated search addresses (`:/pat`/`:?pat`) — the rest of the line is the pattern
//!   (ledger item 5; oracle: `:?,3d` → `E486: Pattern not found: ,3d`, the command does not
//!   execute; on a hit the cursor lands on the first non-blank of the matching line; an empty
//!   pattern reuses the previous search, with no history it reports E35).
//! * The vim layout of `:reg` (ledger item 6): `  l  ""   second line^J` — two spaces + type
//!   char + two spaces + register name + three spaces + content (linewise ends displayed as ^J).
//! * `:undo` at the bottom: a pure bell, no message (the 9.1 oracle has an empty v:errmsg —
//!   ledger item closed).

mod common;

use common::Fixture;

fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

#[test]
fn open_search_address_consumes_the_rest_of_the_line() {
    // `:? ,3d`: `,3d` as a whole becomes the pattern, the search fails with E486 and nothing runs
    let mut f = Fixture::at("aaa\nbbb\nccc\n", 0, 0);
    ex(&mut f, "?,3d");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E486: Pattern not found: ,3d"),
        "vim 9.1: everything to the end of the line is the pattern after an unterminated ?"
    );
    assert_eq!(f.text(), "aaa\nbbb\nccc\n", "the command was never parsed, the buffer is untouched");
    assert_eq!(f.vim.cursor_offset(), 0, "the cursor stays put");

    // `:/ba` hits: lands on the first non-blank of the matching line; a following `n` reuses the
    // pattern
    let mut f = Fixture::at("aaa\nbar fizz\nccc\n", 0, 0);
    ex(&mut f, "/ba");
    assert_eq!(f.vim.cursor_offset(), 4, "lands at the start of the bar line (offset 4)");
    assert!(f.host.statuses.is_empty(), "a hit reports no message");
    f.feed(["n"]);
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E486")),
        "the pattern is active, so n must not report an E35/E486 class error (wrap or bell when no further match)"
    );

    // Empty pattern: no history → E35
    let mut f = Fixture::at("aaa\nbbb\n", 0, 0);
    ex(&mut f, "/");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E35: No previous regular expression"),
        "vim 9.1: :/ with no history reports E35"
    );

    // Empty pattern: with history → reuse; when there is no further matching line forward it
    // wraps to the first matching line
    let mut f = Fixture::at("foo\nbar foo\n", 0, 0);
    ex(&mut f, "/bar");
    assert_eq!(f.vim.cursor_offset(), 4, "/bar lands on the bar line");
    ex(&mut f, "/");
    assert_eq!(
        f.vim.cursor_offset(),
        4,
        ":// reuses the pattern bar; no further matching line forward → wrap to the first matching line (the same line)"
    );
}

#[test]
fn undo_at_oldest_change_is_silent() {
    // 9.1 oracle: at the oldest change `:undo` leaves v:errmsg empty — a pure bell is correct
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["x"]);
    ex(&mut f, "undo");
    assert_eq!(f.text(), "abc\n");
    ex(&mut f, "undo");
    let noisy = f
        .host
        .statuses
        .iter()
        .any(|s| s.contains("E") && !s.contains("E492"));
    assert!(!noisy, "vim 9.1: :undo at the bottom emits no message ({:?})", f.host.statuses);
}

#[test]
fn reg_listing_uses_vim_layout() {
    let mut f = Fixture::at("hello world\nsecond line\n", 0, 0);
    f.feed(["y", "y", "j", "\"", "a", "d", "d"]);
    ex(&mut f, "reg");
    // `  l  ""   second line^J` (two spaces + type + two spaces + register name + three spaces)
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("  l  \"\"   second line^J")),
        "vim 9.1: the :reg line layout (unnamed register, linewise ^J tail), actual {:?}",
        f.host.statuses
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("  l  \"a   second line")),
        "named register a uses the same layout"
    );
}

#[test]
fn redo_unsets_marks_undo_restores_again() {
    // 9.1 oracle: `ma dd u` restores the mark (the last char of the line), `<C-r>` **unsets** it
    // (getpos("'a") = 0,0), and `u` again restores it. The old behavior left the mark stranded at
    // the restored position (a column-drift shape).
    let mut f = Fixture::at("  foo\nbar\nbaz\n", 0, 2);
    f.feed(["m", "a", "d", "d", "u"]);
    assert_eq!(f.text(), "  foo\nbar\nbaz\n");
    // After u the cursor is at the undo hint position (line start); the mark itself is restored to
    // the last char of the line
    assert_eq!(f.vim.marks.resolve('a', &f.buf), Some(4), "E4: mark restored");
    f.feed(["<C-r>"]); // C-r
    assert_eq!(f.text(), "bar\nbaz\n", "redo deletes again");
    f.feed(["`", "a"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "after redo mark 'a is unset (vim 9.1 oracle getpos = 0,0)"
    );
    f.feed(["u"]);
    f.feed(["`", "a"]);
    assert_eq!(f.cursor(), 4, "undoing again restores it (oracle 1,3 = the last char of the line)");
}
