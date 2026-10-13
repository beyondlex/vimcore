//! Round-32 independent audit (group E): visual mode / blockwise visual / marks (2026-10-07).
//!
//! Every `#[test]` corresponds to one new finding; the assertions state **the expected results
//! verified against vim 9.1 (`-Nu NONE -N -i NONE -s` typeahead oracle)**, so they must fail on the
//! current working tree. Each test comment gives: severity, reproduction (buffer / (line,col) /
//! keys), vim's expectation vs the engine's current behavior, and the evidence (oracle output, or a
//! note that it still needs re-checking). The known-divergence ledger (the numbered NOTES.md entries
//! and the 50 items of the round-31 audit) was excluded item by item.

mod common;

use common::{edit, Fixture};
use vimcore::mode::Mode;

// ---------------------------------------------------------------------------
// Finding 1 (P1): blockwise-visual `o` swaps the corners by bytes -- vim's semantics swap the
// rectangle by (row, virtual column)
//
// Repro A: buffer "abcdef\nabcdef\n", (0,0), keys <C-v> j l o d.
//   vim 9.1 (oracle: `abcdef\nabcdef\n` + \026jlod): the block is unchanged (both rows lose "ab")
//   → "cdef\ncdef\n". Engine: after o the byte columns of anchor/cursor become the two block edges
//   and the right edge's column is lost → each row deletes only the 'b' at col 1
//   → "acdef\nacdef\n".
// Repro B: same buffer (0,0), <C-v> l o d (a single-row block).
//   vim (oracle o22): deletes col 0..2 → "cdef\nabcdef\n".
//   Engine: both corners are on row 0 and the columns collapse to a width of 1 → "acdef\nabcdef\n".
// Repro C: buffer "abcdef\nab\n", (0,3), <C-v> j o d (the virtual column diverges after j moves the
// cursor onto the short row).
//   vim (oracle o3): the block keeps the single column 3 → "abef\nab\n".
//   Engine: after the swap the anchor takes the clamped byte column 1 → the block becomes
//   col 1..4 → "aef\na\n".
#[test]
fn block_o_swaps_raw_bytes_instead_of_row_col_corners() {
    // A: a two-row block is still two rows by two columns after o
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "l", "o", "d"]);
    assert_eq!(
        f.text(),
        "cdef\ncdef\n",
        "vim 9.1: o leaves the block rectangle unchanged, d deletes col 0..2 of both rows"
    );
    // B: a single-row block still covers both selected columns after o
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "l", "o", "d"]);
    assert_eq!(
        f.text(),
        "cdef\nabcdef\n",
        "vim 9.1: on a single-row block o only swaps the column ends, d deletes col 0..2"
    );
    // C: after j diverges the virtual column on the short row, o must not drag the left edge onto
    // the clamped byte column
    let f = edit("abcdef\nab\n", 0, 3, &["<C-v>", "j", "o", "d"]);
    assert_eq!(
        f.text(),
        "abef\nab\n",
        "vim 9.1: the block keeps col 3 and deletes only the 'd' of the first row"
    );
}

// ---------------------------------------------------------------------------
// Finding 2 (P1): blockwise-visual `O` does not refresh block_cursor_col -- the stale virtual column
// shifts the rectangle to the right
//
// Repro: buffer "abcdef\nabcdef\n", (0,0), keys <C-v> j l O d.
// vim 9.1 (oracle o4): O only moves the cursor to the other corner of the same row, the rectangle is
// unchanged → both rows lose "ab" → "cdef\ncdef\n". Engine: SwapEndsKeepCol rearranges the bytes of
// the two corners, but block_cursor_col is still the 1 left after `l`, so the span reads col 1..2 →
// only 'b' is deleted on both rows → "acdef\nacdef\n".
#[test]
fn block_O_leaves_stale_virtual_column() {
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "l", "O", "d"]);
    assert_eq!(
        f.text(),
        "cdef\ncdef\n",
        "vim 9.1: O keeps the rectangle, d deletes col 0..2 of both rows"
    );
}

// ---------------------------------------------------------------------------
// Finding 3 (P2): `` `> ``/`'> lands one position past the last selected character (charwise selection)
//
// Repro: buffer "hello world\nsecond\n", (0,0), keys v l l l y ` >.
// vim 9.1 (oracle o5: col("'>")=4, and col(".")=4 after `` `> ``): '> sits on the last selected
// character (the final 'l' of "hell", 1-based col 4 = offset 3). The engine uses the exclusive end
// byte (offset 4) as the mark → `` `> `` stops on the 'o' (offset 4).
#[test]
fn tick_gt_charwise_lands_one_past_last_selected_char() {
    let f = edit("hello world\nsecond\n", 0, 0, &["v", "l", "l", "l", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: `> stops on the last selected character 'l' (offset 3)"
    );
}

// ---------------------------------------------------------------------------
// Finding 4 (P2): the `'> column of a linewise selection stops at the line start -- vim stops on the
// last character of the final row
//
// Repro A: buffer "aaa\nbbbb\nccc\n", (0,0), V j y ` >.
//   vim 9.1 (oracle o6a: '>=(2,5), col(".")=4 after `>): `> stops on the last character of the final
//   row "bbbb" (1-based col 4 = offset 7). Engine: offset 5 (line1 col1).
// Repro B: same buffer (0,0), V y ` >.
//   vim (oracle o6b: '>=(1,4), col(".")=3): offset 2. Engine: offset 0.
#[test]
fn tick_gt_linewise_lands_at_line_start_instead_of_last_char() {
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "j", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        7,
        "vim 9.1: after Vj `> stops on the last character 'b' of the final row (offset 7)"
    );
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        2,
        "vim 9.1: after Vy `> stops on the last selected character 'a' of the range row (offset 2)"
    );
}

// ---------------------------------------------------------------------------
// Finding 5 (P1): undo does not restore the marks of a deleted row -- vim's undo block restores the
// marks together with the text
//
// Repro: buffer "  foo\nbar\nbaz\n", (0,2) (on the 'f'), keys m a d d u ` a.
// vim 9.1 (oracle r8a/o7b: getpos("'a")=(1,5) after u, col(".")=5 after `a): the mark comes back to
// row 1 with the undo (vim 9.1 puts it on the last character, 1-based col 5 = offset 4).
// Engine: dd's adjust_delete folds the mark onto the deletion point 0 and undo only swaps the text
// without touching the mark → `a lands at (1,1) (offset 0).
#[test]
fn undo_does_not_restore_marks_of_deleted_line() {
    let f = edit("  foo\nbar\nbaz\n", 0, 2, &["m", "a", "d", "d", "u", "`", "a"]);
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: after undo `a returns to the deleted row (oracle col=5 → offset 4)"
    );
}

// ---------------------------------------------------------------------------
// Finding 6 (P1): once dd deletes the whole row holding the selection, gv reselects across the line
// break and d joins two rows
//
// Repro: buffer "aa\nbb\ncc\ndd\n", (1,0), keys v l y d d g v d.
// vim 9.1 (oracle o8b: after dd '<=(2,1) '>=(2,2); o8: after gvd getline = ["aa","","dd"]): the marks
// collapse onto the surviving row "cc" at col 1..2, gv reselects "cc" and d empties that row's
// content → "aa\n\ndd\n". Engine: the stored `> end (3,3) runs through floor(hi-1)+clamp and stops on
// the last character of the previous row, so the reselection spans the newline (sel anchor=3
// cursor=1) → d deletes 1..4 and joins the rows → "ac\ndd\n".
#[test]
fn gv_after_line_delete_reselects_across_line_break() {
    let f = edit("aa\nbb\ncc\ndd\n", 1, 0, &["v", "l", "y", "d", "d", "g", "v", "d"]);
    assert_eq!(
        f.text(),
        "aa\n\ndd\n",
        "vim 9.1: gv reselects the surviving row's content and d empties that row (line structure kept)"
    );
}

// ---------------------------------------------------------------------------
// Finding 7 (P2): `'[`/`']`/`` `[ ``/`` `] `` are unbound -- vim jumps to the first/last character of
// the most recent yank/change
//
// Repro: buffer "foo bar\n", (0,0), press ` ] (or ']) after y i w.
// vim 9.1 (oracle o9a/o9b: `[ → col 1, `] → col 3): `] stops on the last yanked character 'o'
// (offset 2). Engine: trie Miss rings and stays put (`] lands at offset 0).
#[test]
fn bracket_marks_unbound() {
    let f = edit("foo bar\n", 0, 0, &["y", "i", "w", "`", "]"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: `] stops on the last yanked character (offset 2)");
    assert_eq!(f.host.bells, 0, "vim 9.1: `] is a real command and does not ring");

    let f = edit("foo bar\n", 0, 0, &["y", "i", "w", "`", "["]);
    assert_eq!(
        f.host.bells, 0,
        "vim 9.1: `[ jumps to the first yanked character (here offset 0, the cursor itself), no bell"
    );
}

// ---------------------------------------------------------------------------
// Finding 8 (P2): `<C-o>` in visual mode should drop back to Normal (the cursor staying on the visual
// cursor end) -- the engine silently hands the key to the host and leaves the selection dangling
//
// Repro: buffer "aaa\nbbb\nccc\nddd\neee\nfff\n", (0,0), G v k k <C-o>.
// vim 9.1 (oracle r3c/r3h/r3i: mode()="n", line/col = the visual cursor end (6,1), visualmode() still
// returns "v"): <C-o> in visual mode returns to Normal, the cursor stays on the current end and the
// jumplist is not used. Engine: the visual trie has no such key → ProcessOutcome::Unknown hands it
// back to the host (zero bells) and the mode stays Visual{Char}.
#[test]
fn ctrl_o_in_visual_should_exit_to_normal_mode() {
    let f = edit("aaa\nbbb\nccc\nddd\neee\nfff\n", 0, 0, &["G", "v", "k", "k", "<C-o>"]);
    assert_eq!(
        f.vim.mode(),
        Mode::Normal,
        "vim 9.1: C-o in visual mode exits to Normal"
    );
    assert_eq!(
        f.cursor(),
        12,
        "vim 9.1: the cursor stays on the visual cursor end (start of the 'ddd' row, offset 12)"
    );
}

// ---------------------------------------------------------------------------
// Finding 9 (P2): the `g;` changelist walk is inverted and the first g; falsely reports E662
//
// Repro A: buffer "abcd\n", (0,1), x (change @1), then A typing " z" (change @3), gg, g ;.
//   vim 9.1 (oracle w4: col=4 → offset 3): the first g; lands on the most recent change (where A
//   appended). Engine: lands on the oldest change (offset 1).
// Repro B: buffer "abcd\n", (0,1), x, $, g ;. vim (oracle w6: col=2, no error text): the single g;
//   lands silently on the change. Engine: first reports "E662: At start of changelist" with a bell,
//   then throws the cursor at changes[0] (which happens to be offset 1 as well).
#[test]
fn changelist_walk_starts_at_oldest_and_falsely_reports_E662() {
    let mut f = Fixture::at("abcd\n", 0, 1);
    f.feed(["x"]);
    f.feed(["A"]);
    f.type_text(" z");
    f.feed(["<Esc>", "g", "g", "g", ";"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: the first g; lands on the most recent change (the A append point, offset 3)"
    );

    let mut g = Fixture::at("abcd\n", 0, 1);
    g.feed(["x", "$", "g", ";"]);
    assert_eq!(g.cursor(), 1, "vim 9.1: g; lands on the change position (offset 1)");
    assert!(
        g.host.statuses.is_empty(),
        "vim 9.1: the first g; succeeds silently; the engine falsely reported {:?}",
        g.host.statuses
    );
    assert_eq!(g.host.bells, 0, "vim 9.1: a successful g; does not ring");
}

// ---------------------------------------------------------------------------
// Finding 10 (P3): `^/gi stop on the last inserted character -- vim stops one position after it
//
// Repro: buffer "hello\n", (0,0), l i then type "XY", Esc, ` ^ (or g i and keep typing).
// vim 9.1 (oracle z3b: col=4 → offset 3 after `^; w3: typing Z with gi gives "hXYZello"):
// '^ records the position where the insert ended (after the last inserted character). The engine
// records it on the last inserted character (offset 2) → typing Z with gi gives "hXZYello".
#[test]
fn caret_mark_and_gi_stop_on_last_inserted_char() {
    let mut f = Fixture::at("hello\n", 0, 0);
    f.feed(["l", "i"]);
    f.type_text("XY");
    f.feed(["<Esc>", "`", "^"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: `^ stops after the last inserted character (offset 3)"
    );
    f.feed(["g", "i"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hXYZello\n", "vim 9.1: gi keeps typing at the `^ position");
}

// ---------------------------------------------------------------------------
// Finding 11 (P3): visual `<` does not ring when there is no indent left to remove (same family: the
// first audit round's L3 already proved that plain `<<` without indent rings in vim and the engine
// was fixed; the visual operator path missed the same feedback)
//
// Repro: buffer "foo\n  bar\n", (0,0), V <.
// vim 9.1: `<` with no indent to remove = a failure bell (the same operator semantics as L3's plain
// `<<`; this item did not go through the oracle separately and is flagged for re-check).
// Engine: silent and unchanged (bells=0).
#[test]
fn visual_indent_left_noop_is_silent() {
    let f = edit("foo\n  bar\n", 0, 0, &["V", "<"]);
    assert_eq!(f.host.bells, 1, "vim 9.1: visual < rings when there is no indent to remove");
    assert_eq!(f.text(), "foo\n  bar\n", "text unchanged (the engine already satisfies this half)");
}
