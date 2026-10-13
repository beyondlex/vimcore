//! Round-33 independent audit (group C): the three visual kinds v/V/C-v and their interaction area (2026-10-08).
//!
//! Each `#[test]` corresponds to one newly found issue; the assertions encode the **expected results
//! empirically verified against vim 9.1 (patches 1-1752, `-Nu NONE -N -i NONE -s` typeahead oracle)**,
//! so they must fail on the current working tree. Each test comment gives: severity, reproduction
//! (buffer / (row, col) / keys), vim expectation vs current engine behavior, and oracle evidence
//! (verbatim oracle script output). Cross-checked item by item against the known divergences in
//! NOTES.md (incl. #58 visual J count, #74 $-block gv), the 50 round-31 audit items (G/K series)
//! and the 59 round-32 audit items (E series: block o/O rectangles, `'>` landing, gv folding,
//! `` `[``/`` `] ``, visual C-o, g;); none of the entries below is already on the books.
//!
//! Probe-channel lesson (one source of false negatives in this round's sampling): when sampling with
//! `:call writefile` while still in visual state, the visual `:` seeds `'<,'>` into the command line,
//! and executing a range command drags the cursor to the range's first line — cursor sampling for
//! gv/o must Esc back to Normal before sampling.
//!
//! Falsified this round (oracle proved engine and vim agree; no need to re-check next round):
//! - `$` survives a blockstate V→C-v round trip (c1b: `<C-v>j$V<C-v>A` appends at each line's end on both sides);
//! - `5h` past the line start: vim also clamps to col 0, treats the motion as successful and does not ring the bell (c3a/c3b, same inside and outside blocks);
//! - visual `r` then `.`: vim replays the whole visual replace (`vlllrXllll.` on "abcdefgh" →
//!   "XXXXXXXX", r4); the engine's key-sequence replay is isomorphic;
//! - visual `p` with count: vim replaces the selection with register × count (c21 "aababab"); engine agrees;
//! - Esc landing on a **reversed** selection: both sides land at the selection start (= cursor end, c7b);
//! - block `o` twice restores the rectangle involution (c26 "bcdef\nb"), block `o` then d (q10), block `o`
//!   then `$A` (q11 appends at each line's own end): oracle and engine agree;
//! - the "collapse multiple lines into one" semantics of linewise/charwise visual `C`/`S` (q4/q5/q6):
//!   the effective span of the engine's `ops::apply` Change agrees with vim (only block C diverges, see C10);
//! - cursor of `Vj>` (r10 lands at first non-blank of the first line), cursor of `Vju` (c13 lands at line
//!   start), gv cursor of a forward selection (r1 lands at the last char on the '> side), the linewise cursor
//!   of the `Vj`-shaped gv (the col-0 shape fixed in E3b coincidentally agrees on both sides);
//! - linewise `Vj<` dedents normally (b1); `3g C-a` on a single-line linewise selection (c4: both sides add +3 only to the first number).

mod common;

use common::edit;

// ---------------------------------------------------------------------------
// C1 (P1) switching v/V/C-v kind does not reset the block virtual column — a stale block_cursor_col squeezes the rectangle narrow
//
// Repro A: buffer "abc\nabc\n", (0,0), keys <C-v> l v l <C-v> d.
//   vim 9.1 (oracle c1a, two re-runs agree): the C-v→v→C-v transitions keep the anchor/cursor's
//   actual columns, block = col 0..3 (single line) → d deletes all of line 1 → "\nabc\n".
//   Engine: `VisualCmd::ToggleKind` only changes the kind; `block_cursor_col` stays at 1 left over
//   from the first `l` → the block is read as col 0..2 → only "ab" is deleted → "c\nabc\n".
// Repro B: buffer "abc\ndef\n", (0,0), keys <C-v> l j v l <C-v> d.
//   vim (oracle c1a2): 3 columns deleted from each line → "\n\n".
//   Engine: stale virtual column 1 → "c\nf\n".
//   (root-cause clue: the `VisualCmd::ToggleKind` arm of `execute_visual_cmd` in src/state.rs does
//   not clean up block_cursor_col/block_anchor_vcol.)
#[test]
fn c1_toggle_kind_keeps_stale_block_virtual_column() {
    let f = edit("abc\nabc\n", 0, 0, &["<C-v>", "l", "v", "l", "<C-v>", "d"]);
    assert_eq!(f.text(), "\nabc\n", "vim 9.1: after the v→C-v transition the block is col 0..3, d deletes the whole line content");
    let f = edit("abc\ndef\n", 0, 0, &["<C-v>", "l", "j", "v", "l", "<C-v>", "d"]);
    assert_eq!(f.text(), "\n\n", "vim 9.1: col 0..3 deleted from each of the two lines");
}

// ---------------------------------------------------------------------------
// C2 (P1) the block-column clamping of j/k is permanent — after passing an empty/short line the rectangle width never comes back
//
// Repro A: buffer "abcdef\n\nabcdef\n", (0,0), keys <C-v> 3 l j k d.
//   vim 9.1 (oracle c2): curswant is a column memory; j to the empty line shows the rectangle at
//   zero width on that line, and k coming back restores col 0..4 → "abcd" deleted from each line →
//   "ef\n\nef\n".
//   Engine: the Up/Down arms of goto_motion write min(col, landing-line width) back into
//   block_cursor_col itself (src/state.rs ~1959); col becomes permanently 0 → only 'a' is deleted
//   from each line → "bcdef\n\nabcdef\n".
// Repro B (short line): buffer "abcdef\nab\nabcdef\n", (0,0), same keys.
//   vim (oracle c2c): → "ef\nab\nabcdef\n" (middle-line content preserved).
//   Engine: col clamped to 2 and never restored → "def\n\ndef\n".
//   (The "rectangle narrows while on the short line" fixed in E1c itself agrees with vim; the gap
//   is that vim only narrows the display on that line — curswant is never rewritten by j/k.)
#[test]
fn c2_jk_clamp_destroys_block_width_permanently() {
    // Oracle re-verification (2026-10-08): after `<C-v>3ljk`, k collapses the selection back to the
    // first line and `d` deletes only cols 0..4 of row 0 (the "ef\n\nef" in the draft case A was a
    // channel artifact — same origin as the sampling trap warned about in this file's header); case
    // B matches the re-verification. Semantic point: curswant is not permanently narrowed by j/k
    // (the old engine wrote min(col, row width) back to bcc, after which even row 0's width could not come back).
    let f = edit("abcdef\n\nabcdef\n", 0, 0, &["<C-v>", "3", "l", "j", "k", "d"]);
    assert_eq!(f.text(), "ef\n\nabcdef\n", "vim 9.1: k collapses the selection, block width restored to full at row 0");
    let f = edit("abcdef\nab\nabcdef\n", 0, 0, &["<C-v>", "3", "l", "j", "k", "d"]);
    assert_eq!(
        f.text(),
        "ef\nab\nabcdef\n",
        "vim 9.1: short-line content is not deleted, col 0..4 deleted at row 0"
    );
}

// ---------------------------------------------------------------------------
// C3 (P1) count formula of block-visual g C-a: vim adds +count*(i+1) per line, engine adds +count+i
//
// Repro: buffer "1\n1\n1\n", (0,0), keys <C-v> j j 2 g <C-a>.
//   vim 9.1 (oracle r11/g5): 3\n5\n7 (line i gets +2*(i+1)).
//   Engine: SequentialIncrement uses `base + i` (src/state.rs VisualCmd::SequentialIncrement) →
//   3\n4\n5. With count=1 the two formulas coincide (the K1 probe happened to only exercise count=1).
#[test]
fn c3_block_gca_count_multiplies_per_line() {
    let f = edit("1\n1\n1\n", 0, 0, &["<C-v>", "j", "j", "2", "g", "<C-a>"]);
    assert_eq!(f.text(), "3\n5\n7\n", "vim 9.1: line i gets +count*(i+1) → 3/5/7");
}

// ---------------------------------------------------------------------------
// C4 (P1) block-visual g C-a ignores the block column bounds — it increments numbers outside the block too
//
// Repro: buffer "x1\nx2\nx3\n", (0,0) (the block only covers the 'x' column of each line), keys
// <C-v> g <C-a>.
//   vim 9.1 (oracle c5): no number inside the block columns → buffer untouched.
//   Engine: SequentialIncrement walks the lines covered by the span, placing the cursor at each
//   line start before incrementing (and the span is the diagonal charwise span of anchor==cursor,
//   so it effectively only processes line 1) → line 1 becomes "x2" → "x2\nx2\nx3\n". Wrong on both
//   counts: both the affected lines and columns escape the block bounds.
#[test]
fn c4_block_gca_ignores_block_columns() {
    let f = edit("x1\nx2\nx3\n", 0, 0, &["<C-v>", "g", "<C-a>"]);
    assert_eq!(f.text(), "x1\nx2\nx3\n", "vim 9.1: g C-a leaves the buffer alone when no number lies in the block columns");
}

// ---------------------------------------------------------------------------
// C5 (P1) block-visual `>` rings the bell and refuses — vim indents highlighted lines for all three kinds
//
// Repro: buffer "ab\ncd\n", (0,0), keys <C-v> j >. (Engine defaults et/ts=4/sw=4 align with the
// oracle's `:set ts=4 sw=4 et`.)
//   vim 9.1 (oracle f1): one column of indent added to each line → "    ab\n    cd\n", cursor lands
//   at the first line's start (offset 0), no bell.
//   Engine: the `_ =>` fallback arm of apply_block_operator only rings the bell for Indent*
//   (src/state.rs ~2903) → text unchanged + bell.
//   For contrast: linewise `Vj>` agrees between engine and vim (r10, cursor at first non-blank).
#[test]
fn c5_block_gt_indents_instead_of_bell() {
    let f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", ">"]);
    assert_eq!(f.text(), "    ab\n    cd\n", "vim 9.1: block > indents the highlighted lines (et, 4 spaces)");
    assert_eq!(f.cursor(), 0, "vim 9.1: cursor lands at the first line's start");
    assert_eq!(f.host.bells, 0, "vim 9.1: a successful indent does not ring the bell");
}

// ---------------------------------------------------------------------------
// C6 (P1) block-visual I/A discards the count — vim repeats the typed text count times on each line
//
// Repro A: buffer "aa\nbb\n", (0,0), keys <C-v> j 3 I, type "-", Esc.
//   vim 9.1 (oracle c9): "---aa\n---bb\n".
//   Engine: the I/A of visual_key goes straight into begin_block_insert; insert_repeat is only
//   assembled in the EnterInsert arm of execute_command and explicitly excludes block sessions → "-aa\n-bb\n".
// Repro B (same family as A): buffer "ab\ncd\n", (0,0), <C-v> j 3 A + "-" + Esc.
//   vim (oracle d2): "a---b\nc---d\n"; engine: "a-b\nc-d\n".
#[test]
fn c6_block_insert_ignores_count_repeat() {
    let mut f = edit("aa\nbb\n", 0, 0, &["<C-v>", "j", "3", "I"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "---aa\n---bb\n", "vim 9.1: 3I types three copies of the text on each line");

    let mut f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", "3", "A"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a---b\nc---d\n", "vim 9.1: 3A repeats three copies at each line's end");
}

// ---------------------------------------------------------------------------
// C7 (P2) the count entering visual mode is discarded — vim's {count}v/V/C-v preselects count units
//
// Repro A: buffer "abcdef\n", (0,0), keys 3 v x.
//   vim 9.1 (oracle c10): 3v selects 3 characters → "def\n".
//   Engine: the EnterVisual arm ignores the count (src/state.rs ~3905) → selects 1 character → "bcdef\n".
// Repro B: buffer "aaa\nbbb\nccc\n", (0,0), 3 V x. vim (oracle c19) deletes 3 lines →
//   empty buffer; engine deletes 1 line → "bbb\nccc\n".
// Repro C: buffer "abcdef\nabcdef\n", (0,0), 3 <C-v> x. vim (oracle c20) deletes 3
//   columns of line 1 → "def\nabcdef\n"; engine deletes 1 column → "bcdef\nabcdef\n".
//   (Distinct from the audit2 falsified item "3vj's count is discarded by vim": that count precedes
//   the selection motion; here the count belongs to the v/V/C-v key itself.)
#[test]
fn c7_count_on_visual_entry_selects_count_units() {
    let f = edit("abcdef\n", 0, 0, &["3", "v", "x"]);
    assert_eq!(f.text(), "def\n", "vim 9.1: 3v preselects 3 characters");
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["3", "V", "x"]);
    assert_eq!(f.text(), "", "vim 9.1: 3V preselects 3 lines");
    let f = edit("abcdef\nabcdef\n", 0, 0, &["3", "<C-v>", "x"]);
    assert_eq!(f.text(), "def\nabcdef\n", "vim 9.1: 3C-v preselects a 3-column block");
}

// ---------------------------------------------------------------------------
// C8 (P1) block-visual X deletes whole lines — vim deletes by block columns in block mode (same as x)
//
// Repro: buffer "ab\ncd\nef\n", (0,0), keys <C-v> j X.
//   vim 9.1 (oracle c11a): 1-column block (col 0) → deletes 1 character from each of the first two
//   lines → "b\nd\nef\n".
//   Engine: X is registered as LinewiseOp(Delete) (src/tables.rs ~664; the oracle probe at the time
//   only exercised the charwise form) → both first lines vanish entirely → "ef\n".
#[test]
fn c8_block_x_deletes_block_not_lines() {
    let f = edit("ab\ncd\nef\n", 0, 0, &["<C-v>", "j", "X"]);
    assert_eq!(f.text(), "b\nd\nef\n", "vim 9.1: in block mode X deletes the block columns (here 1 column)");
}

// ---------------------------------------------------------------------------
// C9 (P1) block-visual Y yanks linewise — vim yanks the rectangular block in block mode
//
// Repro: buffer "ab\ncd\nef\n", (0,0), keys <C-v> j Y j P.
//   vim 9.1 (oracle c11b): Y gets a 1-column block register ["a","c"], cursor returns to the block
//   start (0,0); after j to line 2, P puts the block before col 0 → "ab\nacd\ncef\n".
//   Engine: Y → LinewiseOp(Yank) → linewise yank ["ab","cd"], cursor stays at the visual cursor end
//   (1,0); j P puts two whole lines linewise → "ab\ncd\nab\ncd\nef\n".
#[test]
fn c9_block_y_yanks_block_not_lines() {
    let f = edit("ab\ncd\nef\n", 0, 0, &["<C-v>", "j", "Y", "j", "P"]);
    assert_eq!(f.text(), "ab\nacd\ncef\n", "vim 9.1: block Y yanks the rectangle, P puts it back as a block");
}

// ---------------------------------------------------------------------------
// C10 (P1) block-visual C collapses lines with a linewise change — vim keeps one line per highlighted line and enters insert
//
// Repro: buffer "abcd\nefgh\nijkl\n", (0,0), keys <C-v> j C, type "-", Esc.
//   vim 9.1 (oracle q4; c12a same shape): each of the two lines becomes "-" → "-\n-\nijkl\n".
//   Engine: C/S share LinewiseOp(Change), the effective span deletes both lines' content **including
//   the newline between them** → collapses to one line "-\nijkl\n" (S's semantics; the falsified
//   result showed charwise/linewise C and S collapse identically on both sides — the gap is only block C).
#[test]
fn c10_block_c_keeps_one_line_per_highlighted_line() {
    let mut f = edit("abcd\nefgh\nijkl\n", 0, 0, &["<C-v>", "j", "C"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "-\n-\nijkl\n", "vim 9.1: block C changes per line without collapsing the line count");
}

// ---------------------------------------------------------------------------
// C11 (P2) after Esc exits visual mode the cursor is moved back to the selection start — vim stays at the visual cursor end
//
// Repro A: buffer "abcdef\n", (0,0), keys l l l v l l l <Esc>.
//   vim 9.1 (oracle e1): cursor stays at the visual cursor end, offset 5.
//   Engine: exit_visual unconditionally sets `cursor.offset = lo` (src/state.rs ~2725) → the anchor
//   column, offset 3.
// Repro B: buffer "aaa\nbbb\nccc\n", (0,0), V j <Esc>. vim (oracle e3) cursor at
//   (2,1)=offset 4; engine offset 0.
// Repro C: buffer "abcdef\nabcdef\nabcdef\n", (0,0), <C-v> j j <Esc>.
//   vim (oracle e6) cursor at (3,1)=offset 12; engine offset 0.
//   (For reversed selections both sides agree at the selection start = cursor end, falsified; the
//   gap is all forward selections.)
#[test]
fn c11_esc_keeps_cursor_at_visual_end() {
    let f = edit("abcdef\n", 0, 0, &["l", "l", "l", "v", "l", "l", "l", "<Esc>"]);
    assert_eq!(f.cursor(), 5, "vim 9.1: after Esc the cursor stays at the visual cursor end");
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["V", "j", "<Esc>"]);
    assert_eq!(f.cursor(), 4, "vim 9.1: forward linewise selection's Esc cursor is on the last line");
    let f = edit("abcdef\nabcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "j", "<Esc>"]);
    // (3,1) 1-based = start of line 3 = offset 14 (the probe draft miscounted the \n)
    assert_eq!(f.cursor(), 14, "vim 9.1: block selection's Esc cursor is at the last line's start");
}

// ---------------------------------------------------------------------------
// C12 (P2) gv restoring a reversed selection puts the cursor on the '> side — vim returns to the original cursor end
//
// Repro: buffer "abcdef\n", (0,0), keys l l l v h <Esc> g v (selection col 2..3,
// anchored at 3, cursor at 2).
//   vim 9.1 (oracle r2, sampled in Normal state after Esc): gv puts the cursor back at the original
//   visual cursor end, offset 2.
//   Engine: RestoreVisual always sets cursor = floor(hi-1) (src/state.rs ~4840) → offset 3. For
//   forward selections the two formulas coincide (r1 falsified); the root cause is that last_visual
//   does not record which end was the cursor at the time.
#[test]
fn c12_gv_backward_cursor_returns_to_original_end() {
    let f = edit("abcdef\n", 0, 0, &["l", "l", "l", "v", "h", "<Esc>", "g", "v"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: gv restoring a reversed selection lands on the original cursor end, offset 2");
}

// ---------------------------------------------------------------------------
// C13 (P2) gv restoring a linewise selection loses the cursor column — vim keeps the original column, engine clamps to first non-blank
//
// Repro: buffer "aaa\nbbbb\nccc\n", (0,0), keys V $ j <Esc> g v.
//   vim 9.1 (oracle r3): $ sets curswant to the virtual column at line end, j carries the column
//   down, and after gv the cursor returns to (2,4) (last char of "bbbb", offset 7).
//   Engine: the Line arm of RestoreVisual lands on `first_non_blank(last line)` (the "audit E3b
//   follow-up" probe cited in the comment used the col-0 Vj shape, where both landings coincide by
//   accident) → offset 4.
#[test]
fn c13_gv_linewise_restores_cursor_column() {
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "$", "j", "<Esc>", "g", "v"]);
    assert_eq!(f.cursor(), 7, "vim 9.1: gv restoring a linewise selection keeps the cursor column");
}

// ---------------------------------------------------------------------------
// C14 (P3) cursor does not move after block-visual `<` — vim's text is unchanged (quirk) but the cursor lands at the first line's first non-blank
//
// Repro: buffer "    ab\n    cd\n", (0,0), keys <C-v> j <. (et/ts=4/sw=4 aligned with the oracle.)
//   vim 9.1 (oracle f2/r9): text unchanged (block `<` not dedenting is vim's own behavior), cursor
//   lands at (1,5) = offset 4 (first non-blank of the first line), no failure feedback.
//   Engine: the fallback arm of apply_block_operator rings the bell and the cursor stays at the
//   visual end, offset 7.
//   (Text side agrees on both sides; the difference is the cursor landing and the bell.)
#[test]
fn c14_block_lt_moves_cursor_to_first_non_blank() {
    let f = edit("    ab\n    cd\n", 0, 0, &["<C-v>", "j", "<"]);
    assert_eq!(f.text(), "    ab\n    cd\n", "vim 9.1: block < does not change the text (both sides agree)");
    assert_eq!(f.cursor(), 4, "vim 9.1: cursor lands at the first line's first non-blank");
    assert_eq!(f.host.bells, 0, "vim 9.1: no failure feedback");
}

// ---------------------------------------------------------------------------
// C15 (P1) the block column model counts a TAB as one cell throughout — block selections crossing TAB lines have wrong column bounds
//
// Repro: buffer "\tab\nabcdef\n", (0,0), keys <C-v> l j d. (Engine default ts=4; oracle aligned
// with `:set ts=4`.)
//
// Correction (re-verified in audit6): the original expectation of this case, "\t\nabcd\n", was
// **wrong** — it came from a misreading of the oracle, and at the time already admitted "the TAB
// block-edge rounding rule was not fully reverse-engineered". Re-check against the authoritative
// 9.1 oracle (ts=4, one ad-hoc PTY run and one typeahead run, corroborated by `getreg`):
//   `\tab\nabcdef\n` + `<C-v>ljd` → buffer "b\nf\n",
//   `"` register = blockwise, width 5, content "\\ta\nabcde".
//   I.e. the block = display columns [0,4]: line 1 covers TAB (cols 0-3) + 'a' (col 4) → deletes "\\ta" leaving "b";
//   line 2 covers "abcde" → leaves "f". The block's left edge is the anchor's display column (the
//   TAB's **start**, 0), not "snapped to the TAB's end". The engine's TAB special-case branch is
//   removed accordingly; block columns uniformly go through `display_column_with(ts)` (and the
//   virtual-column advance of `l`/`h` uses each character's display width).
#[test]
fn c15_block_columns_count_tab_as_one_cell() {
    let f = edit("\tab\nabcdef\n", 0, 0, &["<C-v>", "l", "j", "d"]);
    assert_eq!(f.text(), "b\nf\n", "vim 9.1: block edges counted in tabstop virtual columns (block = [0,4])");
}
