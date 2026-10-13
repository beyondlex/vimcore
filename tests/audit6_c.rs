//! audit6_c: registers (numbered ring / named / special) / visual operators p/P/J / undo-redo /
//! insert-mode control keys / macro record and replay — round-5 independent audit (domain C, agent C).
//!
//! Each `#[test]` asserts the **expected result of the vim 9.1 oracle**; items that fail on the
//! current working tree are candidate bugs (the oracle evidence channel is recorded in each case's
//! comment, pending re-verification by the main session).
//! Run: `cargo test --test audit6_c -- --ignored --nocapture`.
//!
//! Oracle channels:
//! * typeahead `vim -Nu NONE -N -i NONE -n -s <keys> <buf>` (control keys written as raw bytes
//!   \x12=C-r, \x16=C-v, \x19=C-y; registers sampled via getreg/getregtype, buffer read back via :w).
//! * Unique file names (leftover swap files swallow keys); engine-divergent defaults are pinned
//!   first on the oracle side with :set (this file involves et/sw, with a matching :set pin inside
//!   the probes).
//!
//! ## Falsified (code reading + probes agree with the oracle; no need to re-check next round)
//! - Consecutive small deletes **overwrite** on `"-` rather than accumulate (vim src/ops.c
//!   op_delete: small deletes go through `get_yank_register('-')` + op_yank with y_append=FALSE) —
//!   the engine's store_delete overwrite agrees.
//! - The five radix forms of `i_CTRL-V_digit` (decimal/o O/x X/u/U) agree between the engine and
//!   the insert.txt table; `0b` binary **does not exist in vim** (get_literal has no such branch).
//! - `q{0-9}` recording, `qaq` clearing the register, `@:` honoring count, basic `@@` replay,
//!   macros continuing on bell-only failure (note_bell does not abort) — engine agrees with vim.
//! - The bell at undo/redo boundaries (undo.c `beep_flush()`) and the message shape agree.
//! - The `":` register stores "the whole command line after the colon" (with the range, without
//!   the colon; ex_getln.c new_last_cmdline = ccline.cmdbuff).
//! - The `0<C-d>` check that "the 0 must have just been typed on this line" (for the shape in
//!   edit.c without this constraint see the positive case of E-C-11).

mod common;

use common::Fixture;
use vimcore::key::Key;
use vimcore::registers::{RegisterKind, UNNAMED};

// ---------------------------------------------------------------- E-C-1 (P2)
// The literal forms of i_CTRL-R (`C-r C-o{reg}`, `C-r C-r{reg}`, `C-r C-p{reg}`) are missing:
// vim's `C-r` special forms let the next key be the register name and insert it **literally**
// (edit.c ins_ctrl_r: `C-o`/`C-r` set the literally bit, `C-p` additionally sets the indent bit).
// The engine's C-r pending in insert_key only accepts printable_char() — a key with the ctrl
// modifier is silently swallowed (pending cleared, nothing inserted), and the following register-
// name character becomes ordinary typing.
//
// Repro: on "foo" do `"ayiw` (a='foo'), then `i<C-r><C-o>a<Esc>`.
// vim 9.1: 'foofoo'; engine: 'foo' (the a is dropped).
// oracle: printf 'foo\n' > b_ec1; keys
//   b'\x1b"ayiw\x16\x12\x0fa\x1b:call writefile([getline(1,"$")],"out")\r:wq\r'
//   (\x16\x12\x0f = C-v escaping? No — in the -s key file \x12=C-r and \x0f=C-o are just raw bytes)
//   → ['foofoo'].
#[test]
fn insert_ctrl_r_literal_forms_paste_the_register() {
    let mut f = Fixture::new("foo\n");
    f.feed(["\"", "a", "y", "i", "w"]); // a = 'foo'
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed_raw(Key::ctrl_char('o')); // literal form
    f.feed(["a"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "foofoo\n",
        "vim 9.1: i<C-r><C-o>a inserts register a literally (the engine eats <C-o> and 'a' lands nowhere)"
    );

    let mut f2 = Fixture::new("foo\n");
    f2.feed(["\"", "a", "y", "i", "w"]);
    f2.feed(["i"]);
    f2.feed_raw(Key::ctrl_char('r'));
    f2.feed_raw(Key::ctrl_char('r')); // C-r C-r{reg}, same family
    f2.feed(["a"]);
    f2.feed(["<Esc>"]);
    assert_eq!(
        f2.text(),
        "foofoo\n",
        "vim 9.1: i<C-r><C-r>a is also a literal form (the engine eats the key the same way)"
    );
}

// ---------------------------------------------------------------- E-C-2 (P2)
// The numbered-ring shift must translate **every slot unconditionally**: vim's
// shift_delete_registers is `y_regs[n] = y_regs[n-1]` (empty registers are shifted along too), so
// a `q{0-9}`-recorded numbered register gets **cleared** on the next big delete. The engine's
// shift_numbered_ring only copies when the from-slot exists — a "slot that does not exist" in the
// HashMap is not shifted, recorded content survives the ring shift, and `"2p` pastes ghost macro text.
//
// Repro: on "hello\nworld\n" do `q2xq` (record and run x, "2='x'), then `dd`, then `"2p`.
// vim 9.1: dd's ring shift clears "2 → `"2p` pastes nothing, buffer stays 'world'.
// Engine: "2 survives → 'wxorld'.
// oracle: printf 'hello\nworld\n' > b_ec2; keys
//   b'\x1bq2xqdd"2p:call writefile([getline(1,"$")],"out")\r:wq\r'
//   → ['world'].
#[test]
fn ring_shift_clears_recorded_numbered_registers() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed(["q", "2", "x", "q"]); // "2 = 'x' (recording also executes)
    assert_eq!(
        f.vim.registers.get('2').map(|r| r.text.clone()),
        Some("x".to_owned()),
        "shape self-check: q2 lands in the numbered register"
    );
    f.feed(["d", "d"]); // big delete → ring shift
    f.feed(["\"", "2", "p"]);
    assert_eq!(
        f.text(),
        "world\n",
        "vim 9.1: dd's ring shift overwrites \"2 with the empty \"1 — recorded macro content surviving the shift is the engine defect"
    );
}

// ---------------------------------------------------------------- E-C-3 (P2)
// The **line model** of `"A` append: vim's op_yank append block (register.c:1287-1334) joins the
// new block's lines onto the tail of the old line sequence; the **result type** is linewise when the
// new block is linewise, otherwise it keeps the **old register's** type (`curr->y_type` is only
// overwritten by a linewise new block); only when the result type is charwise are the old block's
// last line and the new block's first line **string-concatenated** (no 'J' in cpo).
// Two corollaries:
// (a) old charwise + new blockwise → merged charwise, ['fooa','c'];
// (b) old blockwise + new charwise → merge stays blockwise, ['a','c','ab'].
// The engine's append_to_named used to be "concatenate the whole text + take the new kind": (a)
// got the kind wrong as Blockwise, (b) merged lines into 'a\ncab'.
//
// oracle channel: putting the `"{reg}` prefix **before** `<C-v>` does not take effect under 9.1
// (measured: `"a<C-v>jy` leaves the register empty); you must **enter visual first, then `"ay`.
// oracle (a): printf 'foo\nab\ncd\n' > b_ec3a; keys
//   b'"ayiwj\x16j"Ay:call writefile([getregtype("a"),strtrans(getreg("a"))],"out")\r:wq\r'
//   → 'v' 'fooa^@c'.
// oracle (b): printf 'ab\ncd\nzz\n' > b_ec3b; keys
//   b'\x16j"ayviw"Ay:call writefile([getregtype("a"),strtrans(getreg("a"))],"out")\r:wq\r'
//   → "\x161" (CTRL-V width 1) 'a^@c^@ab'.
#[test]
fn uppercase_append_charwise_then_blockwise_stays_charwise() {
    let mut f = Fixture::new("foo\nab\ncd\n");
    f.feed(["\"", "a", "y", "i", "w"]); // a = 'foo' (charwise)
    f.feed(["j"]); // to line 2 (block anchor)
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["j"]); // block 2×1: 'a'/'c'
    f.feed(["\"", "A", "y"]); // append the block
    let r = f.vim.registers.get('a');
    assert_eq!(
        r.as_ref().map(|r| r.text.clone()),
        Some("fooa\nc".to_owned()),
        "shape self-check (text side)"
    );
    assert_eq!(
        r.map(|r| r.kind),
        Some(RegisterKind::Charwise),
        "vim 9.1: old charwise + new blockwise → merge stays charwise (getregtype='v'), paste does not go rectangular"
    );
}

#[test]
fn uppercase_append_blockwise_then_charwise_keeps_rows() {
    let mut f = Fixture::new("ab\ncd\nzz\n");
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["j"]); // block 2×1: 'a'/'c'
    f.feed(["\"", "a", "y"]); // a = block
    f.feed(["v", "i", "w"]); // select 'ab' (charwise)
    f.feed(["\"", "A", "y"]); // append unnamed (charwise 'ab')
    let r = f.vim.registers.get('a');
    assert_eq!(
        r.map(|r| (r.text.clone(), r.kind)),
        Some(("a\nc\nab".to_owned(), RegisterKind::Blockwise)),
        "vim 9.1: old blockwise + new charwise → three-row rectangle ['a','c','ab'] (the engine merged into two rows 'a\\ncab')"
    );
}

// ---------------------------------------------------------------- E-C-4 (P2)
// Visual `p` × a multi-line charwise register's cursor: do_put's MCHAR multi-line arm
// (register.c "put cursor on first inserted character", `curwin->w_cursor = new_cursor`) lands on
// the **first pasted character** — the same rule as audit5 C-9 for plain p, but the visual
// PutReplace arm did not sync it (stays on the last character).
//
// Repro: on "b\nc\nhello\n" do `ggvjy` (charwise 'b\nc'), then `Gviwp`.
// vim 9.1: buffer ['b','c','b','c'], cursor (3,1) = byte 4.
// Engine: byte 6 (last character).
// oracle: printf 'b\nc\nhello\n' > b_ec4; keys
//   b'\x1bggvjyGviwp:call writefile([string(getpos("."))],"out")\r:wq\r'
//   → [0,3,1,0].
#[test]
fn visual_put_multiline_charwise_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // unnamed = 'b\nc'
    f.feed(["G", "v", "i", "w", "p"]);
    assert_eq!(f.text(), "b\nc\nb\nc\n", "shape self-check");
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: visual p, multi-line charwise cursor = first pasted character ((3,1)=byte 4)"
    );
}

// ---------------------------------------------------------------- E-C-5 (P2)
// The `P` (keep_registers) shape of E-C-4: after nv_put_opt blackholes the deletion of the
// selection it still goes through the same do_put multi-line arm → cursor lands on the first pasted
// character. The engine's PutReplaceKeep charwise arm likewise stays on the last character.
//
// oracle: printf 'b\nc\nhello\n' > b_ec5; keys
//   b'\x1bggvjyGviwP:call writefile([string(getpos("."))],"out")\r:wq\r'
//   → [0,3,1,0].
#[test]
fn visual_put_keep_multiline_charwise_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]);
    f.feed(["G", "v", "i", "w", "P"]);
    assert_eq!(f.text(), "b\nc\nb\nc\n", "shape self-check");
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: visual P, multi-line charwise cursor = first pasted character"
    );
}

// ---------------------------------------------------------------- E-C-6 (P2)
// Visual `J` ignores the prefix count: v_J's line count comes from the **selection's line count**
// (do_pending_operator's `oap->line_count = end.lnum - start.lnum + 1`; the OP_JOIN arm only
// clamps the lower bound with `< 2 → 2`, cap->count0 does not participate). The engine's
// VisualCmd::Join takes `max(selection lines, count)` → `Vj3J` joins one line too many.
//
// Repro: on "a\nb\nc\nd\n" do `Vj3J`.
// vim 9.1: 'a b\nc\nd' (only the two selected lines joined, J adds one space); engine: 'a b c\nd'.
// oracle: printf 'a\nb\nc\nd\n' > b_ec6; keys
//   b'Vj3J:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → ['a b','c','d'] (`VjJ` gives the same result, the count is ignored).
#[test]
fn visual_join_ignores_the_typed_count() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["V", "j", "3", "J"]);
    assert_eq!(
        f.text(),
        "a b\nc\nd\n",
        "vim 9.1: visual J's line count = selection's line count, the count is ignored"
    );
}

// ---------------------------------------------------------------- E-C-7 (P2)
// The PUT_CURSEND cursor of gp on a block register: in do_put's MBLOCK arm
// `PUT_CURSEND → curwin->w_cursor = b_op_end; col++` — landing **one cell after the last pasted
// line's content**. The engine's put_blockwise has no leave_after parameter (put_ex returns
// directly into the block arm) and the cursor stays on the first pasted character.
//
// Repro: on "ab\ncd\n" do `<C-v>y` (1×1 block 'a'), then `2G0gp`.
// vim 9.1: buffer 'ab\ncad', cursor (2,3) (one cell after the pasted 'a' = at 'd').
// Engine: cursor on the 'a' ((2,2)).
// oracle: printf 'ab\ncd\n' > b_ec7; keys
//   b'\x1b\x16y2G0gp:call writefile([string(getpos(".")).string(getline(1,"$"))],"out")\r:wq\r'
//   → [0,2,3,0] ['ab','cad'].
#[test]
fn gp_blockwise_cursor_lands_after_the_pasted_block() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["y"]); // 1×1 block 'a'
    f.feed(["2", "G", "0", "g", "p"]);
    assert_eq!(f.text(), "ab\ncad\n", "shape self-check");
    assert_eq!(
        f.cursor(),
        5,
        "vim 9.1: block gp cursor = one cell after the last pasted content ((2,3)=byte 5)"
    );
}

// ---------------------------------------------------------------- E-C-8 (P3)
// i_CTRL-V decimal clamp above 255: get_literal's `if (cc > 255 && unicode == 0) cc = 255` —
// `C-v 300` inserts 'ÿ' (255). The engine's char::from_u32(300) yields U+012C.
//
// oracle: printf 'x\n' > b_ec8; keys b'i\x16300\x1b:call
//   writefile(getline(1,"$"),"out")\r:wq\r' → ['ÿx'] ('ÿ is **before** x, `i` inserts before the
//   cursor character).
#[test]
fn ctrl_v_decimal_value_clamps_at_255() {
    let mut f = Fixture::new("x\n");
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["3", "0", "0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "\u{ff}x\n",
        "vim 9.1: C-v 300 clamps to 255 ('ÿ', inserted before the cursor char); the engine inserts U+012C"
    );
}

// ---------------------------------------------------------------- E-C-9 (P3)
// C-v numeric value 0: get_literal's `if (cc == 0) cc = '\n'` — the comment says "NUL is stored as
// NL", i.e. it **inserts a NUL byte** (vim internally represents NUL as NL; :w writes it out as
// 0x00). Verified on 9.1: after `A<C-v>0<Esc>`, :w gives the bytes 'x' 0x00 '\n' (**not** a
// newline). The engine treats 0 as "an unfinished digit string" and cancels — buffer unchanged.
//
// oracle: printf 'x\n' > b_ec9; keys b'A\x160\x1b:call
//   writefile(getline(1,"$"),"out")\r:wq\r', od after :w → 'x' \0 \n.
#[test]
fn ctrl_v_zero_value_inserts_a_nul_byte() {
    let mut f = Fixture::new("x\n");
    f.feed(["A"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "x\0\n",
        "vim 9.1: C-v 0 has value 0 → a NUL byte (stored internally as NL); the engine cancels it as an unfinished digit string"
    );
}

// ------------------------------------------------------------ E-C-10 (P3)
// An invalid terminator key after the C-v prefix is **processed normally** (get_literal: when
// i==0, `cc = nc`; insert.txt "the 'invalid' character is dealt with in the normal way"):
// `i<C-v>xg<Esc>` inserts 'g'. The engine on an empty digit string does from_str_radix("")=0 →
// inserts NUL (the buffer gets stuffed with a control byte, data pollution) and 'g' becomes
// ordinary typing.
//
// oracle: printf '' > b_ec10; keys b'\x1bi\x16xg\x1b:call
//   writefile([strtrans(getline(1))],"out")\r:wq\r' → 'g'.
#[test]
fn ctrl_v_invalid_terminator_is_typed_normally() {
    let mut f = Fixture::new("");
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["x"]);
    f.feed(["g"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "g",
        "vim 9.1: after C-v x the invalid key 'g' is inserted normally; the engine inserts NUL and drops 'g'"
    );
}

// ---------------------------------------------------------------- E-C-11 (P3)
// `0<C-d>` removes **all** indent (edit.c's 0_CTRL-D arm deletes the indent down to 0,
// `:h i_0_CTRL-D` "delete all indent"), not just one shiftwidth step. The engine's
// insert_shift_indent typed_zero branch only deletes the '0' and then runs ordinary C-d (±sw).
//
// Repro: on "      abc" (6-space indent) do `:set sw=4` `I0<C-d><Esc>`.
// vim 9.1: 'abc' (all indent removed); engine: '  abc' (only one 4-column step removed).
// Note the `0` is **text input** and must go through type_text (the host IME channel); feed's
// printable characters do not enter the buffer — the old probe used feed, so the engine never saw the '0'.
// oracle: printf '      abc\n' > b_ec11; keys b':set sw=4\rI0\x04\x1b:call
//   writefile(getline(1),"out")\r:wq\r' → 'abc' (\x04=C-d).
#[test]
fn zero_ctrl_d_removes_all_indent() {
    let mut f = Fixture::new("      abc\n");
    f.feed([":", "s", "e", "t", " ", "s", "w", "=", "4", "<CR>"]);
    f.feed(["I"]);
    f.type_text("0");
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: 0<C-d> removes all indent; the engine removes only one shiftwidth step"
    );
}

// ---------------------------------------------------------------- E-C-12 (P3)
// `^<C-d>`: removes **all** indent and the cursor column does not move (edit.c ins_shift's
// `lastc == '0' || lastc == '^'` branch: after deleting the '^' it INDENT_SETs to 0); the '^'
// itself does not enter the buffer. The engine only recognizes '0' and treats '^' as ordinary typing.
//
// oracle: printf '      abc\n' > b_ec12; keys b':set sw=4\rI^\x04\x1b:call
//   writefile(getline(1),"out")\r:wq\r' → 'abc'.
#[test]
fn caret_ctrl_d_removes_all_indent_without_typing_caret() {
    let mut f = Fixture::new("      abc\n");
    f.feed([":", "s", "e", "t", " ", "s", "w", "=", "4", "<CR>"]);
    f.feed(["I"]);
    f.type_text("^");
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: ^<C-d> removes all indent and '^' is a command character; the engine typed a literal '^'"
    );
}

// ---------------------------------------------------------------- E-C-13 (P2)
// `@@` must count `@:` as "the previous register" (normal.c nv_at: any register name — including
// ':' — does `lastc = cap->nchar`). The engine's MacroPlay ':' arm does not update
// last_macro_played after execution, so `@@` replays an earlier macro or rings the bell.
//
// Repro: on "a\na\n" do `qai x<Esc>q` (macro a = 'ix<Esc>'), `:s/a/z/<CR>`, `@:`, `@@`. Recording
// also executes, so after recording the buffer is 'xa\na'; `:s` → 'xz\na'.
// vim 9.1: `@@` replays `@:` (nv_at's lastc takes ':') → no 'a' on line1 (E486), buffer still
// 'xz\na'. Engine: `@@` replays macro a → inserts another 'x' ('xxz\na').
// oracle: printf 'a\na\n' > b_ec13; keys
//   b'qaix\x1bq:s/a/z/\r@:@@:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → ['xz','a'].
#[test]
fn at_at_after_at_colon_repeats_the_ex_command() {
    let mut f = Fixture::new("a\na\n");
    // 'x' is **text input** (host IME channel) and must go through type_text — feed's printable
    // characters do not enter the buffer. Macro a = 'i' + Text("x") + '<Esc>'.
    f.feed(["q", "a", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q"]);
    f.feed([":", "s", "/", "a", "/", "z", "/", "<CR>"]); // line1 'xa' → 'xz'
    assert_eq!(f.text(), "xz\na\n", "shape self-check");
    f.feed(["@", ":"]); // replay :s → line1 'xz' (no 'a')
    assert_eq!(f.text(), "xz\na\n", "shape self-check 2");
    f.feed(["@", "@"]); // should replay @: (no 'a' on line1 → no-op)
    assert_eq!(
        f.text(),
        "xz\na\n",
        "vim 9.1: @@ repeats @: (nv_at's lastc takes ':'); the engine replayed macro a"
    );
}

// ---------------------------------------------------------------- E-C-14 (P3)
// The message for pasting from an empty register: do_put reports `E353: Nothing in register x`
// when y_size==0 (semsg; includes p in normal mode). The engine's put_ex only rings note_bell for
// an empty register — no E353 text in the status line.
//
// oracle: `p` on a cold register (the PTY channel shows the E353 echo; sampling v:errmsg on the
// typeahead side also works).
#[test]
fn paste_from_empty_register_reports_e353() {
    let mut f = Fixture::new("a\n");
    f.feed(["p"]); // unnamed not set
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E353")),
        "vim 9.1: p from an empty register reports E353: Nothing in register \" (the engine only rings the bell, no message)"
    );
}

// ================================================================ Falsification probes
// (the oracle already proved the engine agrees; a failure means engine regression — treat as such)

// Falsified-1: `":` stores the whole command line after the colon (including the range). oracle:
// after `:1,2d`, `":p` pastes '1,2d'. Mind the paste position: after `:1,2d` the cursor is on line 1
// 'c', `p` inserts after it → 'c1,2d' (the old probe's expectation '1,2dc' was a typo).
#[test]
fn falsified_colon_register_holds_range_without_colon() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "1", ",", "2", "d", "<CR>"]);
    f.feed(["\"", ":", "p"]);
    assert_eq!(f.text(), "c1,2d\n", "vim 9.1: \": = '1,2d' (no colon, with the range)");
}

// Falsified-2: `c`'s small delete also writes `"-` (same path in op_delete).
#[test]
fn falsified_change_writes_small_delete_register() {
    let mut f = Fixture::new("ab cd\n");
    f.feed(["c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("ab".to_owned()),
        "vim 9.1: cw's deleted text goes into \"- (cw does not include trailing whitespace)"
    );
}

// Falsified-3: consecutive small deletes **overwrite** `"-` (op_delete re-runs op_yank each time, no accumulation).
#[test]
fn falsified_consecutive_small_deletes_overwrite_dash() {
    let mut f = Fixture::new("aa bb cc\n");
    f.feed(["d", "w", "d", "w"]);
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("bb ".to_owned()),
        "vim 9.1: the second dw overwrites \"- (not concatenated)"
    );
}

// Falsified-4: basic `@@` replay (without @: interference). The macro must contain `i` to insert
// 'x' (the old probe `qax<Esc>q` actually recorded a normal `x` delete).
#[test]
fn falsified_at_at_replays_previous_macro() {
    let mut f = Fixture::new("q\n");
    f.feed(["q", "a", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q"]); // a = 'i' + Text("x") + '<Esc>'
    f.feed(["@", "a"]); // insert 'x'
    f.feed(["@", "@"]); // insert 'x' again
    assert_eq!(f.text(), "xxxq\n", "vim 9.1: @@ repeats @a");
}

// Falsified-5: `3u` / `2<C-r>` counted stepping (undo.c's count loop). `i` inserts **before** the
// cursor character, so the result is 'Xa Yb c' (the old probe wrongly wrote 'aX bY c').
#[test]
fn falsified_undo_redo_count_steps() {
    let mut f = Fixture::new("a b c\n");
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["<Esc>", "w", "i"]);
    f.type_text("Y");
    f.feed(["<Esc>", "w", "i"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    f.feed(["3", "u"]);
    assert_eq!(f.text(), "a b c\n", "3u undoes three insert sessions");
    f.feed_raw(Key::ctrl_char('r'));
    f.feed_raw(Key::ctrl_char('r'));
    assert_eq!(f.text(), "Xa Yb c\n", "2<C-r> redoes two steps");
}

// Falsified-6: the `"/` last-search-pattern register.
#[test]
fn falsified_search_register_holds_pattern() {
    let mut f = Fixture::new("foo\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed(["\"", "/", "p"]);
    assert_eq!(f.text(), "ffoooo\n", "vim 9.1: \"/ = 'foo', p pastes it out");
}

// Falsified-7: `p` does not change the register (unnamed keeps its source content).
#[test]
fn falsified_put_leaves_register_untouched() {
    let mut f = Fixture::new("xy\n");
    f.feed(["y", "i", "w"]);
    f.feed(["w", "p"]);
    f.feed(["p"]);
    assert_eq!(
        f.vim.registers.get(UNNAMED).map(|r| r.text.clone()),
        Some("xy".to_owned()),
        "vim 9.1: pasting does not write the register"
    );
}

// Falsified-8: each `:s` is its own undo group (two consecutive :s; one u undoes only the last).
// After `:%s` the cursor stays on the **last** changed line, so the range-less `:s/b/c/` changes
// line 3 → 'b\nb\nc' (the old probe's expectation 'c\nb\nb' assumed the cursor was on line 1).
#[test]
fn falsified_ex_substitutes_are_separate_undo_groups() {
    let mut f = Fixture::new("a\na\na\n");
    f.feed([":", "%", "s", "/", "a", "/", "b", "/", "<CR>"]);
    f.feed([":", "s", "/", "b", "/", "c", "/", "<CR>"]);
    assert_eq!(f.text(), "b\nb\nc\n", "shape self-check");
    f.feed(["u"]);
    assert_eq!(f.text(), "b\nb\nb\n", "vim 9.1: u undoes only the last :s");
}

// Falsified-9: `qaq` clears the register and `@a` idles with no residue (execute_command's stop
// arm does store_named_plain(reg, "")).
#[test]
fn falsified_empty_recording_clears_the_register() {
    let mut f = Fixture::new("x\n");
    f.feed(["q", "a", "i", "y"]);
    f.feed(["<Esc>", "q"]); // a = 'iy<Esc>'
    f.feed(["q", "a", "q"]); // empty recording overwrites
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "x\n", "vim 9.1: qaq clears \"a, @a is a no-op");
}
