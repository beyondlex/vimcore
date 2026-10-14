//! audit5_c: registers / put / visual v,V / block editing / marks & jumps /
//! undo-redo / insert-mode control keys — independent audit (2026-10-09, domain
//! C fourth independent round).
//!
//! Every `#[test]` asserts the **expected result of the vim 9.1 oracle** and
//! should fail on the current work tree (it becomes a regression once fixed).
//! Oracle channels:
//! * default `vim -Nu NONE -N -i NONE -n -s <keys> <buf>` (typeahead, the keys
//!   file ends with `:call writefile(...)` + `:wq`; registers sampled via
//!   getreg/getregtype, cursor via getpos).
//! * control keys written as raw bytes (\x12=C-r, \x16=C-v, \x19=C-y).
//!
//! Sampling traps experienced this round (beware when re-verifying):
//! * in a `-es -S` script, `setline` and the following `normal` merge into
//!   **the same undo block** (one u returns to an empty buffer) — undo
//!   granularity / undo-cursor conclusions must go through typeahead or PTY;
//!   this round the whole undo matrix was resampled for that reason (the
//!   script channel's "undo lands at (1,1)" readings are all artifacts).
//! * in a `-s` keys file, tokens like `<C-y>` are **not** control keys but six
//!   literal characters.
//! * writefile's List argument errors with E730 when a nested List leaks in —
//!   getline(1,"$") must not be mixed into a string list.
//!
//! ## Falsified (oracle proved the engine agrees with vim; no re-check needed)
//! - falsified: undo/redo **cursor** (typeahead channel) — `3|x u` → (1,3),
//!   `$p u` → (1,11), `2G3|x u` → (2,3), `dd u` → (2,1): the engine's
//!   "snapshot when the group opens" model matches vim's uh_cursor point by
//!   point.
//! - falsified: `i<C-r>"` putting a linewise register (noai shape) — after
//!   `:2y`, `ggI<C-r>"xyz<Esc>` → ['def','xyzabc','def']; the engine inserts
//!   the text verbatim (splitting lines on `\n`) consistently.
//! - falsified: `Vp` (linewise visual × linewise register in both multi-line
//!   and single-line shapes + charwise register shape) — audit G5's line
//!   replacement model matches vim.
//! - falsified: blockwise visual `p` × charwise register (each row puts the
//!   full text, oracle ['One','T','Tee','One']) — block_put_replace's `_` arm
//!   matches.
//! - falsified: `Vj3>` (visual indent count = shift per round) matches.
//! - falsified: `R<C-r>-<C-r>-` (Replace puts a register: overwrite then
//!   append) — oracle ['foo','foofoo']; the engine's Replace arm matches.
//! - falsified: `i<C-y>`/`<C-e>` TAB alignment and short-line shapes (byte
//!   columns take the neighbor line's chars) match.
//! - falsified: `qA` append recording (oracle @a='Ix'+'Ay<Esc>') — the engine's
//!   seed merge matches.
//! - falsified: `S` writes `"1`+unnamed, `s`/`x` write only `"-`, block `p`
//!   swap, `2gp` charwise cursor, `v_u`/`v_U`/`v_~`, the search-motion body of
//!   `dn`.
//! - falsified: after visual `p`, `gv` reselects **the pasted text** — the
//!   engine's PutReplace gv chain already matches nv_put_opt's b_visual
//!   rewrite (oracle `wviwpgvy` yanks back the pasted text; the first probe
//!   spun through vacuously because the setup register held the same value,
//!   and re-verifying with `"a` showed the engine really gives 'XY'. That
//!   candidate, originally numbered C-14, was removed from this file).
//! - probe-methodology note: visual-paste probes must paste **different**
//!   text (putting 'def' back onto 'def' leaves the text unchanged and cannot
//!   be told apart from a no-op, producing a vacuous pass).
//!
//! ## Unimplemented, not bugs (listed separately, no probes)
//! - `U` (undo-line two-level undo), `g-`/`g+`/`:earlier`/`:later`,
//!   `:undojoin`, `{count}undo` absolute numbering — the engine only rings,
//!   loudly absent.
//! - `:k{mark}` setting a mark — E492 (domain A's spelling table).
//! - `` m{A-Z} `` file marks jumping across buffers — inexpressible in the
//!   single-buffer model.
//! - `:lockmarks`, the `undotree()`/`getmarklist()` family.

mod common;

use common::Fixture;
use vimcore::key::Key;
use vimcore::registers::RegisterKind;

// ---------------------------------------------------------------- C-1 (P2)
// use_reg_one (`%` × named registers): when `%`, `(`/`)`, `` ` ``/`'`, `/`,
// `?`, `n`, `N`, `{`, `}` do a delete, even a charwise single-line small delete
// must **additionally** write the numbered ring (vim src/ops.c op_delete:
// `motion_type == MLINE || line_count > 1 || use_reg_one` →
// shift_delete_registers + write y_regs[1]; use_reg_one is set by
// nv_percent/nv_brace/nv_findpar/nv_cursormark/normal_search). The engine's
// store_delete has no use_reg_one concept.
//
// Repro: `"ad%` on `(12)3`.
// vim 9.1: @1='(12)', @- unset.
// Engine: writes only @a.
// oracle: printf '(12)3\n' > b_c1; keys b'\x1b"ad%:call writefile(
//   [string(getreg("1")), string(getreg("-"))], "out")\r:wq\r'
//   → ['(12)'] [''].
// Source: vim src/testdir/test_registers.vim Test_register_one (`%` arm).
#[test]
fn regone_named_delete_via_percent_writes_register_one() {
    let mut f = Fixture::new("(12)3\n");
    f.feed(["\"", "a", "d", "%"]);
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: \"ad% writes both \"a and the numbered ring \"1 (use_reg_one)"
    );
    assert_eq!(
        f.vim.registers.get('-'),
        None,
        "vim 9.1: a use_reg_one delete with a named register does not write \"-"
    );
}

// ---------------------------------------------------------------- C-2 (P2)
// The unnamed shape of C-1: `d%` writes the small delete into **both** `"1`
// and `"-` (op_delete's shift block and small-delete block both fire, in order).
//
// oracle: `d%` on `(12)3` → @1='(12)', @-='(12)', @"='(12)'.
#[test]
fn regone_unnamed_delete_via_percent_writes_both_one_and_small() {
    let mut f = Fixture::new("(12)3\n");
    f.feed(["d", "%"]);
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: d% writes the numbered ring \"1"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: d% also writes the small-delete register \"- (both the shift \
         block and the small-delete block fire)"
    );
}

// ---------------------------------------------------------------- C-3 (P2)
// use_reg_one for search motions: `d/pat<CR>` writes `"1`+`"-` (normal_search
// sets use_reg_one). `dn`, `d?pat<CR>`, `d{`, `` d`x ``, `d'a` are the same family.
//
// oracle: `0d/abc<CR>` on `abc abc` → @1='abc ', @-='abc ', buffer 'abc'.
#[test]
fn regone_search_delete_writes_register_one() {
    let mut f = Fixture::new("abc abc\n");
    f.feed(["0", "d", "/"]);
    f.feed(["a", "b", "c", "<CR>"]);
    assert_eq!(f.text(), "abc\n", "shape self-check: d/pat deletes up to the match");
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("abc ".to_owned()),
        "vim 9.1: d/pat writes \"1 via use_reg_one"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("abc ".to_owned()),
        "vim 9.1: d/pat also writes \"-"
    );
}

// ---------------------------------------------------------------- C-4 (P2)
// use_reg_one's brace family (single-line charwise shape): `d}` from inside a
// word to the paragraph end stays charwise (the exclusive col-1 promotion only
// fires at an endpoint's line start), but use_reg_one makes it write
// `"1`+`"-`.
//
// oracle: `d}` on `one two\n\nzzz` with cursor (1,4) (the t of 'two') →
//     @1='two', @-='two', buffer ['one ', '', 'zzz'].
#[test]
fn regone_brace_delete_writes_register_one() {
    let mut f = Fixture::at("one two\n\nzzz\n", 0, 4);
    f.feed(["d", "}"]);
    assert_eq!(f.text(), "one \n\nzzz\n", "shape self-check: d}} only deletes to the paragraph end");
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("two".to_owned()),
        "vim 9.1: single-line d}} writes \"1 via use_reg_one"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("two".to_owned()),
        "vim 9.1: single-line d}} also writes \"-"
    );
}

// ---------------------------------------------------------------- C-5 (P2)
// Put books `'[`/`']` (do_put starts with `b_op_start = b_op_end = cursor`,
// then each branch rewrites them): `` `] `` jumps to the last character of the
// last pasted text. The engine's put_ex / blockwise-visual PutReplace neither
// writes this mark pair — `` `] `` stays on the stale yank-era booking.
//
// Repro: `yy` `j` `p` on `abc\ndef` (pastes 'abc' as the third line).
// vim 9.1: '[=(3,1), ']=(3,3).
// Engine: '[='] stuck on the yank's old booking (byte 0..2).
// oracle: printf 'abc\ndef\n' > b_c5; keys b'\x1byyjp:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,3,1,0]] [[0,3,3,0]].
#[test]
fn put_sets_change_marks() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed(["y", "y", "j", "p"]);
    assert_eq!(f.text(), "abc\ndef\nabc\n", "shape self-check");
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        10,
        "vim 9.1: after a put, `] lands on the last char of the pasted text ((3,3) = byte 10)"
    );
    f.feed(["`", "["]);
    assert_eq!(
        f.cursor(),
        8,
        "vim 9.1: after a put, `[ lands on the first pasted char ((3,1) = byte 8)"
    );
}

// ---------------------------------------------------------------- C-6 (P2)
// Undo/redo book `'[`/`']` (u_undoredo sets b_op_start/end to the first/last
// **lines** of the undone change, column 0). Same origin as test_undo.vim
// Test_undo_mark. The engine's history_step never touches marks — after undo,
// `]` still sits at the deletion point.
//
// Repro: `3|x` `u` on `abcdef` (then `<C-r>`).
// vim 9.1: after undo '[=']=(1,1); after redo likewise (1,1).
// oracle: printf 'abcdef\n' > b_c6; keys b'\x1b3|xu:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r\x12:call
//   writefile([string(getpos("'[")), string(getpos("']"))], "out2")\r:wq\r'
//   → out/out2 both [[0,1,1,0]]×2.
#[test]
fn undo_and_redo_set_change_marks() {
    let mut f = Fixture::new("abcdef\n");
    f.feed(["3", "|", "x"]);
    f.feed(["u"]);
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        0,
        "vim 9.1: after undo, '] lands at the first line of the undone change ((1,1))"
    );
    f.feed_raw(Key::ctrl_char('r')); // <C-r> redo
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        0,
        "vim 9.1: after redo, '] likewise lands at the first line's start (oracle um4)"
    );
}

// ---------------------------------------------------------------- C-7 (P2)
// The insert session books `'[`/`']`: '[ = first inserted char, '] = one past
// the inserted text (booked by ins_esc; `:h '[`). o/O/a/A/R share this exit
// (the o shape is in C-17). The engine's exit_insert only writes `^` — after a
// pure insert, `` `[ ``/`` `] `` are unset (the jump rings).
//
// Repro: `0lliXYZ<Esc>` on `one two`.
// vim 9.1: '[=(1,3) ('X'), ']=(1,6) (one past 'Z').
// oracle: printf 'one two\n' > b_c7; keys b'\x1b0lliXYZ\x1b:call
//   writefile([string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,1,3,0]][[0,1,6,0]].
#[test]
fn insert_session_sets_change_marks() {
    let mut f = Fixture::new("one two\n");
    f.feed(["0", "l", "l", "i"]);
    f.type_text("XYZ");
    f.feed(["<Esc>"]);
    let bells_before = f.host.bells;
    f.feed(["`", "["]);
    assert_eq!(f.host.bells, bells_before, "`` `[ `` must not ring (mark is set)");
    assert_eq!(
        f.cursor(),
        2,
        "vim 9.1: after the insert, `[ lands on the first inserted char 'X' (byte 2)"
    );
    f.feed(["`", "]"]);
    // assertion corrected (2026-10-09 typeahead re-verification): `] lands on
    // the mark itself (byte 5 = one past ins_esc's b_op_end value); nv_brackets
    // has no dec at all — the draft's "clamps back to
    // 'Z'" was an inference from the `:h ']
        // wording, oracle getpos(".") = [0,1,6,0].
    assert_eq!(f.cursor(), 5, "oracle [0,1,6,0]: `] lands on the mark itself (byte 5)");
}

// ---------------------------------------------------------------- C-8 (P2)
// The real meaning of gp's line-wise cursor "after the new text": it lands
// **one line after** the pasted block (do_put MLINE arm `PUT_CURSEND:
// w_cursor.lnum = lnum + 1`), not on the last pasted line. audit C5's old
// conclusion "gp lands on the last pasted line" was a clamping coincidence of a
// buffer-end probe (at the end, lnum+1 clamps back to the block tail and both
// models coincide); mid-buffer blocks expose the difference.
//
// Repro: `yy` `2gg1gp` (and `2gp`) on `a1\na2\na3`.
// vim 9.1: 1gp cursor (4,1) (one line after the pasted block); 2gp → (5,1).
// Engine: lands on the last pasted line ((3,1)/(4,1)).
// oracle: printf 'a1\na2\na3\n' > b_c8; keys b'\x1byy2gg1gp:call
//   writefile([string(getpos("."))], "out")\r:wq\r' → [0,4,1,0];
//   2gp → [0,5,1,0].
#[test]
fn gp_linewise_cursor_lands_after_the_pasted_block() {
    let mut f = Fixture::new("a1\na2\na3\n");
    f.feed(["y", "y", "2", "g", "g", "1", "g", "p"]);
    assert_eq!(
        f.line(),
        3,
        "vim 9.1: the 1gp line-wise cursor = one line after the pasted block (line 4, 0-based 3)"
    );
    let mut f2 = Fixture::new("a1\na2\na3\n");
    f2.feed(["y", "y", "2", "g", "g", "2", "g", "p"]);
    assert_eq!(
        f2.line(),
        4,
        "vim 9.1: the 2gp line-wise cursor = one line after both pasted blocks (line 5, 0-based 4)"
    );
}

// ---------------------------------------------------------------- C-9 (P2)
// p/P cursor for a multiline charwise register: lands on the **first pasted
// char** (do_put's MCHAR multiline arm puts the cursor back at b_op_start;
// only single-line charwise lands on the last char). The engine's put_ex
// always stops at prev_grapheme(at+len) = the last char.
//
// Building the register (the engine fixture has no :let): `gg v j y` on
// 'b/c/hello' (lines b, c, hello) yields charwise 'b\nc' (oracle:
// getregtype='v', getreg strtrans='b^@c').
// vim 9.1: `G$p` → buffer ['b','c','hellob','c'], cursor (3,6)=byte 9 ('b', the
//     first pasted char); `G0P` → ['b','c','b','chello'], cursor (3,1)=byte 4.
// Engine: cursor lands on the last char (byte 11 / byte 6).
// oracle: printf 'b\nc\nhello\n' > b_c9; keys b'\x1bggvjyGG$p:call
//   writefile([string(getpos(".")) . string(getline(1,"$"))], "out")\r:wq\r'
//   → [0,3,6,0]['b','c','hellob','c'].
#[test]
fn multiline_charwise_put_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // charwise 'b\nc'
    assert_eq!(
        f.vim.registers.get('"').map(|r| (r.text.clone(), r.kind)),
        Some(("b\nc".to_owned(), RegisterKind::Charwise)),
        "shape self-check: a cross-line visual yank yields a charwise multiline register"
    );
    f.feed(["G", "$", "p"]);
    assert_eq!(
        f.text(),
        "b\nc\nhellob\nc\n",
        "shape self-check: p pastes multiline"
    );
    assert_eq!(
        f.cursor(),
        9,
        "vim 9.1: multiline charwise p cursor = the first pasted char (the 'b' of 'hellob')"
    );
}

// ---------------------------------------------------------------- C-10 (P2)
// The `P` shape of C-9: oracle `G0P` → buffer ['b','c','b','chello'], cursor
// (3,1) = byte 4 = the first pasted char.
#[test]
fn multiline_charwise_put_before_cursor_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // charwise 'b\nc'
    f.feed(["G", "0", "P"]);
    assert_eq!(
        f.text(),
        "b\nc\nb\nchello\n",
        "shape self-check: P pastes before the line start (the pasted text carries its own newline, splitting into two lines)"
    );
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: multiline charwise P cursor = the first pasted char ((3,1) = byte 4)"
    );
}

// ---------------------------------------------------------------- C-11 (P1)
// `I` in v/V (non-blockwise) mode: inserts once at the **selection start** line
// and exits visual (I/A in non-blockwise visual degrade to plain commands
// landing on a selection corner; per-line repetition is blockwise-mode only).
// The engine's I/A is intercepted only in the Block branch; under v/V it falls
// to a Visual trie Miss → rings and stays in visual mode, where subsequent text
// keys extend the selection and the literal text is lost.
//
// Repro: `VjI-<Esc>` on ['aa','bb','cc'].
// vim 9.1: ['-aa','bb','cc'].
// oracle: printf 'aa\nbb\ncc\n' > b_c11; keys b'\x1bVjI-\x1b:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['-aa','bb','cc'].
#[test]
fn visual_line_i_inserts_at_selection_start_only() {
    let mut f = Fixture::new("aa\nbb\ncc\n");
    f.feed(["V", "j", "I"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "-aa\nbb\ncc\n",
        "vim 9.1: V+I inserts once at the selection start line and exits visual"
    );
}

// ---------------------------------------------------------------- C-12 (P1)
// `A` in v/V (non-blockwise) mode: appends once at the line end of the
// **selection end** line and exits visual.
//
// Repro: `VjA-<Esc>` on ['aa','bb'].
// vim 9.1: ['aa','-bb'].
// oracle: printf 'aa\nbb\n' > b_c12; keys b'\x1bVjA-\x1b:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['aa','-bb'].
#[test]
fn visual_line_a_appends_at_selection_end_only() {
    let mut f = Fixture::new("aa\nbb\n");
    f.feed(["V", "j", "A"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "aa\n-bb\n",
        "vim 9.1: V+A appends once at the line end of the selection's end line and exits visual"
    );
}

// ---------------------------------------------------------------- C-13 (P2)
// Visual `P` does not swap registers (nv_put_opt: `keep_registers = (cmdchar
// == 'P')`, the selection delete goes through the blackhole): the replaced
// text is **discarded** and the paste source register is kept intact. Only `p`
// swaps. The engine's PutReplace treats p/P identically (delete_span(None)) —
// on the P side the replaced text entered unnamed and `"-`.
//
// Repro: on `abc def`, `wvey` yanks 'def' (unnamed='def'), then `0` `viwP` —
// pasting 'def' over 'abc' (probe-methodology note: must paste **different**
// text; pasting 'def' onto 'def' leaves the text unchanged and cannot be told
// apart from a no-op — the draft spun through vacuously here).
// vim 9.1: buffer 'def def', unnamed still 'def', `"-` unset.
// Engine: unnamed='abc', `"-`='abc'.
// oracle: printf 'abc def\n' > b_c13; keys b'\x1bwvey0viwP:call
//   writefile([string(getreg("\\"")), string(getreg("-"))], "out")\r:wq\r'
//   → 'def' ''.
#[test]
fn visual_capital_p_keeps_the_put_register() {
    let mut f = Fixture::new("abc def\n");
    f.feed(["w", "v", "e", "y"]); // unnamed = 'def'
    f.feed(["0", "v", "i", "w", "P"]); // paste 'def' over 'abc'
    assert_eq!(f.text(), "def def\n", "shape self-check: P pastes the yanked 'def'");
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("def".to_owned()),
        "vim 9.1: visual P does not swap — unnamed keeps the paste source"
    );
    assert_eq!(
        f.vim.registers.get('-'),
        None,
        "vim 9.1: visual P's replaced text goes to the blackhole; \"- is not set"
    );
}

// ---------------------------------------------------------------- C-14 (P2)
// Blockwise visual `P` likewise writes no registers (nv_put_opt's
// keep_registers routes the selection delete through `oap->regname = '_'`):
// the deleted block is discarded and unnamed keeps the paste source. The
// engine's block_put_replace treats p/P identically (store_block_rows(None)).
//
// Repro: `gg yiw` on `XY\nabcdef\nghijkl`, then `j 0 <C-v> j l P`.
// vim 9.1: ['XY','XYcdef','XYijkl'], unnamed still 'XY'.
// Engine: unnamed becomes the deleted block 'ab'+'gh'.
// oracle: printf 'XY\nabcdef\nghijkl\n' > b_c15; keys b'\x1bggyiwj0\x16jlP:
//   call writefile([string(getreg("\\"")) . string(getline(1,"$"))],
//   "out")\r:wq\r' → 'XY' ['XY','XYcdef','XYijkl'].
#[test]
fn block_visual_capital_p_keeps_registers() {
    let mut f = Fixture::new("XY\nabcdef\nghijkl\n");
    f.feed(["g", "g", "y", "i", "w"]); // unnamed = 'XY'
    f.feed(["j", "0"]); // to the second line's start (block begins at 'abcdef')
    f.feed_raw(Key::ctrl_char('v')); // <C-v>
    f.feed(["j", "l", "P"]);
    assert_eq!(
        f.text(),
        "XY\nXYcdef\nXYijkl\n",
        "shape self-check: block P pastes 'XY' over two rows (the first line, outside the block, stays as-is)"
    );
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("XY".to_owned()),
        "vim 9.1: blockwise visual P does not swap — unnamed keeps the paste source"
    );
}

// ---------------------------------------------------------------- C-15 (P2)
// A new change invalidates the redo branch (vim's undo is a tree: a new change
// after undo cuts the redo branch and `<C-r>` only rings). Neither the harness
// HostView nor the engine contract has any path for "a new edit invalidates
// the redo stack" — after `u`, editing then `<C-r>` resurrects the undone state.
//
// The shape uses an `A!` session (the typeahead channel merges adjacent bare
// changes like xx into one undo **block — only after u is there a sync point;
// once the insert session closes the block is separate, so this shape is
// faithful): on `one\n`, `A!<Esc>` ('one!'), `u` ('one'), `0x` ('ne'), `<C-r>`.
// vim 9.1: rings, buffer still 'ne\n'; engine+harness: resurrects 'one!'.
//
// oracle: printf 'one\n' > b_c16; keys b'\x1bA!\x1bu0x\x12:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['ne'].
#[test]
fn redo_is_invalidated_by_a_new_change() {
    let mut f = Fixture::new("one\n");
    f.feed(["A"]);
    f.type_text("!");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one!\n", "shape self-check");
    f.feed(["u"]);
    assert_eq!(f.text(), "one\n", "shape self-check: u returns to 'one'");
    f.feed(["0", "x"]);
    assert_eq!(f.text(), "ne\n", "shape self-check: the new change gives 'ne'");
    let bells_before = f.host.bells;
    f.feed_raw(Key::ctrl_char('r')); // <C-r>
    assert_eq!(
        f.text(),
        "ne\n",
        "vim 9.1: a new change after undo cuts the redo branch; <C-r> is a no-op"
    );
    assert!(
        f.host.bells > bells_before,
        "vim 9.1: <C-r> rings when there is nothing to redo"
    );
}

// ---------------------------------------------------------------- C-16 (P2)
// After `c`, `']` = one past the inserted text (b_op_end grows with the typing
// and is booked at ins_esc). The engine's apply() nails '] to the last byte of
// the to-be-deleted span before the deletion — after the insert it is only
// shifted by the funnel, landing **inside** the inserted text (off by one byte).
//
// Repro: `cwXYZ<Esc>` on `ab cd`.
// vim 9.1: ']=(1,4) (one past 'Z', byte 3) — `` `] `` clamps back to 'Z' (byte 2).
// Engine: '] lands at byte 4 (' ').
// oracle: printf 'ab cd\n' > b_c17; keys b'\x1bcwXYZ\x1b:call writefile(
//   [string(getpos("']"))], "out")\r:wq\r' → [0,1,4,0].
#[test]
fn change_marks_end_tracks_the_inserted_text() {
    let mut f = Fixture::new("ab cd\n");
    f.feed(["c", "w"]);
    f.type_text("XYZ");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "XYZ cd\n", "shape self-check");
    f.feed(["`", "]"]);
    // assertion corrected (same re-verification as C-7): `] lands on the mark
    // itself at byte 3 (one past 'Z'); no backward clamp
    assert_eq!(f.cursor(), 3, "oracle: `] lands on the ']' mark itself (byte 3)");
}

// ---------------------------------------------------------------- C-17 (P2)
// `o` (open-line insert) also books `'[`/`']`: '[ = start of the new line,
// '] = one past the last inserted char. The engine leaves both unset, same as C-7.
//
// oracle: printf 'x\n' > b_c18; keys b'\x1bGoaaaa\x1b:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,2,1,0]][[0,2,5,0]].
#[test]
fn open_line_insert_sets_change_marks() {
    let mut f = Fixture::new("x\n");
    f.feed(["G", "o"]);
    f.type_text("aaaa");
    f.feed(["<Esc>"]);
    f.feed(["`", "["]);
    assert_eq!(f.cursor(), 2, "vim 9.1: after o, '[ = the new line's start ((2,1) = byte 2)");
    let bells_before = f.host.bells;
    f.feed(["`", "]"]);
    assert_eq!(
        f.host.bells,
        bells_before,
        "vim 9.1: ']=(2,5) is set (the engine's unset mark would ring)"
    );
    // '] byte 6 = one past the line end; the jump clamps back to the last char 'a' (byte 5)
    assert_eq!(f.cursor(), 5, "vim 9.1: `] clamps back to the last inserted char 'a'");
}

// ---------------------------------------------------------------- C-18 (P2)
// `"=<CR>` (empty expression) reuses the previous expression (get_expr_register
// returns the cached expression line): consecutive puts reuse the same
// evaluation result. The engine's `=` prompt falls to an empty result on empty
// input and the second paste loses the text.
//
// Repro: `"=6*7<CR>p` then `"=<CR>P` on `zz`.
// vim 9.1: 'z4422z'.
// oracle: printf 'zz\n' > b_c19; keys b'\x1b"=6*7\rp"=\rP:call writefile(
//   [string(getline(1,"$"))], "out")\r:wq\r' → ['z4422z'].
#[test]
fn expression_register_empty_prompt_reuses_last_expression() {
    let mut f = Fixture::new("zz\n");
    f.feed(["\"", "="]);
    f.type_text("6*7");
    f.feed(["<CR>"]);
    f.feed(["p"]);
    assert_eq!(f.text(), "z42z\n", "shape self-check: the first evaluation pastes '42'");
    f.feed(["\"", "="]);
    f.type_text(""); // empty input = reuse the previous expression
    f.feed(["<CR>"]);
    f.feed(["P"]);
    assert_eq!(
        f.text(),
        "z4422z\n",
        "vim 9.1: \"=<CR> reuses the previous expression; P pastes '42'"
    );
}

// ---------------------------------------------------------------- C-19 (P2)
// Undo restores the visual selection (u_undoredo swaps uh_visual / the current
// b_visual): after `dd` folded '</'>, undo brings back **the pre-change**
// selection pair and gv reselects the original span on the restored text. The
// engine's last_visual is only shifted one-way by the funnel; undo does not
// restore it.
//
// Repro: on `abcd`, `vlly`, `dd`, `u`, `gvy`.
// vim 9.1: @='abcd'.
// oracle: printf 'abcd\n' > b_c20; keys b'\x1bvllyddugvy:call writefile(
//   [string(getreg("\\""))], "out")\r:wq\r' → 'abcd'.
#[test]
fn undo_restores_the_visual_selection_for_gv() {
    let mut f = Fixture::new("abcd\n");
    f.feed(["v", "l", "l", "y"]);
    f.feed(["d", "d"]);
    f.feed(["u"]);
    f.feed(["g", "v", "y"]);
    // expectation corrected (2026-10-09 typeahead re-verification): vll on
    // "abcd" selects "abc" (3 chars), so gv's reselect-yank is also 'abc' — the
    // draft's 'abcd' was a typo (report text did not match the writefile
    // output). The engine agrees; this probe turns into a regression pin.
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("abc".to_owned()),
        "oracle 'abc': after dd+u, gv reselects the original span on the restored text"
    );
}

