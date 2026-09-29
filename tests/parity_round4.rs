//! Round-4 review regressions. Every vim-semantics case here was probed
//! against vim 9.1 (headless `-es` scripts) before the engine change; the
//! probes' outputs are quoted in the test docs.

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer as _;

// ---- dw/cw on whitespace-only lines ----------------------------------------

/// vim 9.1 probes (`dw@0` / `dw@2` on `AAAA / "   " / BBBB`):
/// `dw` from col 0 of a whitespace-only line deletes the spaces and KEEPS the
/// newline (`["AAAA", "", "BBBB"]`); from the last space it deletes just that
/// char. The old engine promoted any blank start line to a LINEWISE delete,
/// which also removed the newline.
#[test]
fn dw_on_whitespace_only_line_keeps_the_newline() {
    // cursor on the first space: all three spaces go, the line survives
    let f = edit("AAAA\n   \nBBBB\n", 1, 0, &["d", "w"]);
    assert_eq!(f.text(), "AAAA\n\nBBBB\n", "dw@0 deletes the run, not the line");
    // cursor on the last space: exactly that char goes
    let f = edit("AAAA\n   \nBBBB\n", 1, 2, &["d", "w"]);
    assert_eq!(f.text(), "AAAA\n  \nBBBB\n", "dw@2 deletes one char");
}

/// `cw` on a whitespace-only line changes only the covered span (the `dw`
/// span minus trailing whitespace): from col 0 the line becomes `Z`, from the
/// last space the leading spaces survive (`  Z`). vim probes `cw@0`/`cw@2`.
#[test]
fn cw_on_whitespace_only_line_matches_the_dw_span() {
    let mut f = edit("AAAA\n   \nBBBB\n", 1, 0, &["c", "w"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "AAAA\nZ\nBBBB\n", "cw@0 replaces the run");
    let mut f = edit("AAAA\n   \nBBBB\n", 1, 2, &["c", "w"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "AAAA\n  Z\nBBBB\n", "cw@2 replaces one char");
}

/// `dw` on an EMPTY line still deletes the line (vim probe `dw@empty`:
/// `["AAAA", "BBBB"]`) — the linewise promotion survives for the empty case.
#[test]
fn dw_on_empty_line_still_deletes_the_line() {
    let f = edit("AAAA\n\nBBBB\n", 1, 0, &["d", "w"]);
    assert_eq!(f.text(), "AAAA\nBBBB\n");
}

/// `d2w` from a whitespace-only line spans the newline (multi-`w` crossing):
/// vim probe `d2w@0` on `AAAA / "   " / BBBB` leaves just `["AAAA"]`.
#[test]
fn d2w_from_whitespace_line_spans_newline() {
    let f = edit("AAAA\n   \nBBBB\n", 1, 0, &["d", "2", "w"]);
    assert_eq!(f.text(), "AAAA\n", "d2w removes spaces + next line");
}

// ---- `^`/linewise landing on whitespace-only lines --------------------------

/// vim probe: `^` on `"   "` parks the cursor on the LAST space (col 3),
/// never on the newline. The old `first_non_blank` returned the line end,
/// and a cursor sitting on the `\n` made later `diw`/`dw` swallow it.
#[test]
fn caret_on_whitespace_line_lands_on_last_char() {
    // buffer "   \nfoo\n": the three spaces sit at offsets 0..3
    let mut f = Fixture::new("   \nfoo\n");
    f.feed(["g", "g", "^"]);
    assert_eq!(f.vim.cursor_offset(), 2, "^ lands on the last space");

    // the landmine this fixes: diw at that position must not eat the newline
    let f = edit("   \nfoo\n", 0, 2, &["d", "i", "w"]);
    assert_eq!(f.text(), "\nfoo\n", "diw clears the spaces, keeps the line");

    let mut f = edit("   \nfoo\n", 0, 2, &["c", "i", "w"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Z\nfoo\n", "ciw replaces the run");
}

/// Same landing rule for the other linewise first-non-blank moves (vim probe:
/// `G` onto a trailing `"   "` line sits on its last space).
#[test]
fn linewise_moves_land_on_last_char_of_blank_lines() {
    let mut f = Fixture::new("foo\n   \n");
    f.feed(["G"]);
    // "   " occupies offsets 4..7; the last space is 6, the newline 7
    assert_eq!(f.vim.cursor_offset(), 6, "G lands on the last space");

    let mut f = Fixture::new("\nfoo\n");
    f.feed(["g", "g"]);
    assert_eq!(f.vim.cursor_offset(), 0, "empty line keeps the line start");
}

/// vim probe: `I` on `"   "` types AFTER the blanks (`"   Z"`), while a plain
/// `i` at `^` inserts before the last space (`"  Z "`).
#[test]
fn insert_first_non_blank_on_blank_line_appends_at_end() {
    let mut f = Fixture::at("   \nfoo\n", 0, 0);
    f.feed(["I"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "   Z\nfoo\n", "I inserts at the line end");

    let mut f = edit("   \nfoo\n", 0, 0, &["^", "i"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "  Z \nfoo\n", "i after ^ inserts before the last space");
}

// ---- cursor placement never parks on the newline ----------------------------

/// A host click past the end of a line must land ON the last character (vim
/// never places the normal-mode cursor on the `\n`). The old clamp kept the
/// offset at the newline, where `dw`/`diw` would swallow the line break.
#[test]
fn host_click_at_line_end_lands_on_last_char() {
    let mut f = Fixture::new("foo bar\nbaz\n");
    f.vim.set_cursor_offset(&f.buf, 7); // the '\n' of line 0
    assert_eq!(f.vim.cursor_offset(), 6, "click past EOL sits on 'r'");
    f.feed(["d", "w"]);
    assert_eq!(f.text(), "foo ba\nbaz\n", "dw from there deletes only 'r'");

    // an empty line has no char to sit on — the start stays
    let mut f = Fixture::new("foo\n\nbaz\n");
    f.vim.set_cursor_offset(&f.buf, 4); // the empty line's newline
    assert_eq!(f.vim.cursor_offset(), 4);
}

/// Same rule for the drag endpoint (the selection cursor, not the anchor).
#[test]
fn visual_drag_endpoint_clamps_off_the_newline() {
    let mut f = Fixture::new("foo bar\nbaz\n");
    f.vim.set_visual_range(&f.buf, 0, 7);
    let (_, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(cursor, 6, "drag endpoint sits on the last char");
}

/// Undo restoring an insert-era cursor position (which may legitimately be a
/// line end) must not park the block cursor on the `\n`.
#[test]
fn undo_cursor_restore_stays_off_the_newline() {
    let mut f = Fixture::at("abc\n", 0, 3);
    f.feed(["A"]); // insert session at the line end
    f.type_text("d");
    f.feed(["<Esc>"]); // "abcd" — undo group captured cursor at 4 (the \n)
    f.feed(["u"]);
    assert_eq!(
        f.vim.cursor_offset(),
        2,
        "undo lands on a character, not the newline"
    );
}
