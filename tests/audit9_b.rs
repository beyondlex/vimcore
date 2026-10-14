//! audit9_b: second round of "fresh exploration" red probes (agent B domain: block visual C-v /
//! block editing / text objects).
//!
//! Each `#[test]` corresponds to behavior re-verified on the local `/usr/bin/vim` (9.1 patches
//! 1-1752) with `vim -Nu NONE -N -i NONE -n -s <keysfile> <buf>`. All are red before the fix
//! (engine output differs from the oracle) and turn green after; the oracle evidence stays in the comments.
//!
//! Engine-vs-vim default differences (`et=on sw=4 ts=4`) are aligned with `:set` where needed — the
//! cases in this file do not depend on those defaults (except the TAB case explicitly declares
//! ts=4, which matches the engine default).
//!
//! Key notation: whitespace-separated tokens; `<t:XY>` goes through the IME text channel (the host's
//! delivery contract for printable text), the rest go through the Key channel. `+` inside `<t:...>`
//! means a space.

#![allow(non_snake_case)] // test names encode vim key sequences (C-v, g2, $A)

mod common;

use common::Fixture;

/// Run a key sequence: `<t:...>` goes through the text channel, the rest through the key channel
/// (same contract as audit6_b's `run_case`).
fn run(buf: &str, line: usize, col: usize, keys: &str) -> Fixture {
    let mut f = Fixture::at(buf, line, col);
    for tok in keys.split_whitespace() {
        if tok.starts_with("<t:") && tok.ends_with('>') {
            f.type_text(&tok[3..tok.len() - 1].replace('+', " "));
        } else {
            f.feed([tok]);
        }
    }
    f
}

// ---------------------------------------------------------------------------
// E9-1 (P1) the left edge of a block-visual `$` is the **anchor column**, not column 0
//
// Repro: buffer "abcdef\ngh\nijklmn\n", (0,2) (cursor on 'c'), keys <C-v> j $ d.
// vim 9.1 (ts-independent): the block's left edge stays at anchor column 3 ('c'); line 1 deletes
// cols 3..6 "cdef" leaving "ab"; line 2's "gh" is too short, col 3 is out of range → nothing
// deleted; result "ab\ngh\nijklmn\n"; register `"` = blockwise width 4, content "cdef".
// The engine's old model treated the `$` block as "each row from column 0 to line end" (losing the
// anchor column), deleting down to "\n\nijklmn".
#[test]
fn e9_1_block_dollar_left_edge_is_anchor_column() {
    let f = run("abcdef\ngh\nijklmn\n", 0, 2, "<C-v> j $ d");
    assert_eq!(
        f.text(),
        "ab\ngh\nijklmn\n",
        "vim 9.1: the `$` block's left edge = anchor column; short rows are not deleted (line 1 keeps ab, line 2's gh untouched)"
    );
}

// ---------------------------------------------------------------------------
// E9-2 (P1) block-visual `S` is a **linewise change**, not a block-level change-to-EOL
//
// Repro: buffer "long line here\nabc\ndef\n", (0,0), keys <C-v> j l l S Z <Esc>.
// vim 9.1: `S` deletes the selected whole lines linewise and then enters insert → buffer "Z\ndef\n",
// register `"` = linewise "long line here\nabc\n" (nv_subst: visual `S` first sets VIsual_mode to
// 'V' and then acts as 'c'). The engine's old code routed `S` together with `C` through
// BlockChangeToEnd → "Z\nZ\ndef\n" (block-level, register blockwise).
#[test]
fn e9_2_block_S_is_linewise_change() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l S <t:Z> <Esc>");
    assert_eq!(f.text(), "Z\ndef\n", "vim 9.1: block S = whole-line linewise change");
}

// ---------------------------------------------------------------------------
// E9-3 (P1) after exiting block-visual `A` the cursor parks on the **first row's left edge**
//
// Repro: buffer "abcdef\nghijkl\nmnopqr\n", (0,1) (cursor 'b'),
// keys <C-v> j l A Z <Esc>. Block columns 2..3 ('bc'/'hi').
// vim 9.1: text "abcZdef\nghiZjkl\nmnopqr\n", cursor (1,2) = the first row's left edge at 'b'
// (op_insert sets w_cursor.col = oap->start.col; `A` also returns to the left edge).
// The engine's old formula parked on the **replica row** (the append point) + text length → (1,4).
#[test]
fn e9_3_block_A_parks_on_first_row_left_edge() {
    let f = run("abcdef\nghijkl\nmnopqr\n", 0, 1, "<C-v> j l A <t:Z> <Esc>");
    assert_eq!(f.text(), "abcZdef\nghiZjkl\nmnopqr\n");
    assert_eq!(f.cursor(), 1, "vim 9.1: after exiting `A` the cursor parks on the first row's block left edge (column 2)");
}

// ---------------------------------------------------------------------------
// E9-4 (P1) after exiting block-visual `c` the cursor parks on the **first row**
//
// Repro: buffer "long line here\nabc\ndef\n", (0,0), keys <C-v> j l l c Z <Esc>.
// vim 9.1: text "Zg line here\nZ\ndef\n", cursor (1,1) = first row, column 1 (op_change inserts on
// the **first row** and parks on the last typed character after exit). The engine's old code typed
// on the **last row**, so the cursor ended on the last row after exit (2,1).
#[test]
fn e9_4_block_c_parks_on_first_row() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l c <t:Z> <Esc>");
    assert_eq!(f.text(), "Zg line here\nZ\ndef\n");
    assert_eq!(f.line(), 0, "vim 9.1: block `c` types on the first row and parks there");
}

// ---------------------------------------------------------------------------
// E9-5 (P1) after exiting block-visual `C` the cursor parks on the **first row** (same row issue as `c`)
//
// Repro: buffer "long line here\nabc\ndef\n", (0,0), keys <C-v> j l l C Z <Esc>.
// vim 9.1: text "Z\nZ\ndef\n", cursor (1,1) = the first row. The engine's old code parked on the last row (2,1).
#[test]
fn e9_5_block_C_parks_on_first_row() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l C <t:Z> <Esc>");
    assert_eq!(f.text(), "Z\nZ\ndef\n");
    assert_eq!(f.line(), 0, "vim 9.1: block `C` types on the first row and parks there");
}

// ---------------------------------------------------------------------------
// E9-6 (P1) the virtual column of block-visual `l`/`h` advances by each **character's display width** (wide char = 2 cells)
//
// Repro: buffer "中文字\n中文字\n", (0,0), keys <C-v> j l x.
// The anchor is on '中' (display cols 0-1); `l` moves to '文' (col 2), so the block = cols [0,2] →
// covering 中+文 → deletes "中文" leaving "字", result "字\n字\n". The engine's old code had `l`
// advance the virtual column by only +1 (to col 1, still pointing at '中') → block [0,1] → deletes
// only '中' leaving "文字".
#[test]
fn e9_6_block_l_advances_by_display_width_wide_char() {
    let f = run("中文字\n中文字\n", 0, 0, "<C-v> j l x");
    assert_eq!(f.text(), "字\n字\n", "vim 9.1: for a wide char `l` advances 2 cells, deleting 中文");
}

// ---------------------------------------------------------------------------
// E9-7 (P1) block column bounds on TAB rows expand by tabstop (an anchor on a TAB does not snap to the TAB's end)
//
// Repro: buffer "\tabc\n\tdef\n", (0,0) (cursor on the TAB), keys <C-v> j l d.
// vim 9.1 (ts=4, matching the engine default): the block's left edge = the anchor's display column
// 0 (the TAB's start), the right edge = display column 4 of the cursor characters 'd'/'a' → block
// [0,4] covers TAB(0-3)+'a'(4) → deletes "\ta"/"\te" leaving "bc"/"ef". Register `"` = blockwise
// width 5, content "\ta\n\te".
// The engine's old code had a special-case branch "anchor on a TAB → snap the left edge to the
// TAB's end 4", deleting down to "\tc"/"\tf".
#[test]
fn e9_7_block_on_tab_row_uses_tabstop_columns() {
    let f = run("\tabc\n\tdef\n", 0, 0, "<C-v> j l d");
    assert_eq!(f.text(), "bc\nef\n", "vim 9.1: block left edge = the TAB's start column 0, covering TAB+'a'");
}

// ---------------------------------------------------------------------------
// E9-8 (P2) block `$` + `A`: after exit the cursor parks on the first row's anchor column
//
// Repro: buffer "abcdef\ngh\nijklmn\n", (0,2), keys <C-v> j $ A Z <Esc>.
// vim 9.1: text "abcdefZ\nghZ\nijklmn\n", cursor (1,3) = the first row's anchor column 3.
#[test]
fn e9_8_block_dollar_A_parks_on_anchor_column() {
    let f = run("abcdef\ngh\nijklmn\n", 0, 2, "<C-v> j $ A <t:Z> <Esc>");
    assert_eq!(f.text(), "abcdefZ\nghZ\nijklmn\n");
    assert_eq!(f.cursor(), 2, "vim 9.1: after a `$` block A the cursor parks on the first row's anchor column 3");
}
