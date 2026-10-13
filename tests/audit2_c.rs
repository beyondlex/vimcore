//! audit2_c: independent audit of the insert/replace internals, `.` replay and macro
//! record/replay (2026-10-07).
//!
//! One `#[test]` per finding; the assertions state **the expected results verified against vim 9.1
//! (`-Nu NONE -N -i NONE -n -s` typeahead)**, so they should fail on the current working tree. The
//! oracle evidence is written in each test comment (`printf '%b' … > keys; vim -Nu NONE -N -i NONE -n
//! -s keys buf`; buffer content sampled with `:call writefile([getline(1)…])`).
//!
//! Scope: src/insert_mode.rs plus the macro / `.` / count-insert parts of state.rs.

mod common;

use common::Fixture;
use std::matches;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;
use vimcore::mode::Mode;

// ------------------------------------------------ missing insert control keys (i_CTRL-T/D/Y/E/A/@/=)

/// **Finding 1 (P2)**: insert `<C-t>` (indent one shiftwidth) is not bound at all -- the ctrl chord
/// table knows only w/u/r and everything else falls through to Unknown and is dropped silently.
/// vim 9.1 inserts one shiftwidth at the line start (noet+sw=ts=8 → a single TAB) and leaves the
/// cursor alone.
/// oracle: on `abc`, `iab<C-t><Esc>` → getline(1) = "\tababc"; the engine gives "ababc".
#[test]
fn ctrl_t_inserts_shiftwidth_indent() {
    let mut f = Fixture::at("abc\n", 0, 0);
    // oracle environment (vim -Nu NONE): noet + sw=8 + ts=8. The engine default et+sw=4 is a
    // deliberate modern-editor set that differs from vim's defaults, so the probe must align first.
    f.feed([":", "s", "e", "t", " ", "n", "o", "e", "t", " ", "t", "s", "=", "8", " ", "s", "w", "=", "8", "<CR>"]);
    f.feed(["i"]);
    f.type_text("ab");
    f.feed_raw(Key::ctrl_char('t'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\tababc\n", "vim 9.1: i<C-t> inserts one shiftwidth at the line start");
}

/// **Finding 2 (P2)**: insert `<C-d>` (delete one indent) is likewise unbound. vim 9.1 deletes one
/// shiftwidth; `0<C-d>` (press 0 and then C-d inside insert) deletes the whole indent without leaving
/// a "0" character behind.
/// oracle: on `"        abc"`, `i<C-d>x<Esc>` → "xabc" (col=1); `i0<C-d><Esc>` → "abc" (0<C-d>
/// deletes it all). The engine does nothing in either shape (giving "x        abc" / "0        abc").
#[test]
fn ctrl_d_removes_shiftwidth_indent() {
    // same as ctrl_t: oracle environment noet + sw=8 (engine default is et+sw=4)
    let keys = [":", "s", "e", "t", " ", "n", "o", "e", "t", " ", "t", "s", "=", "8", " ", "s", "w", "=", "8", "<CR>"];
    let mut f = Fixture::at("        abc\n", 0, 0);
    f.feed(keys);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('d'));
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "xabc\n", "vim 9.1: i<C-d> deletes one shiftwidth");

    let mut f = Fixture::at("        abc\n", 0, 0);
    f.feed(keys);
    f.feed(["i"]);
    f.type_text("0");
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc\n", "vim 9.1: 0<C-d> deletes the whole indent");
}

/// **Finding 3 (P2)**: insert `<C-y>` (copy the character from the same column of the line above) is
/// unbound.
/// oracle: in `abcdef\nxy\n`, at row 2 col 1, `ix` then `<C-y>` three times → getline(2)="xxcdey"
/// (it copies c, d, e from the insert-point columns 2/3/4; once the line above is exhausted it is a
/// no-op). The engine gives "xxy".
#[test]
fn ctrl_y_copies_char_from_line_above() {
    let mut f = Fixture::at("abcdef\nxy\n", 1, 1);
    f.feed(["i"]);
    f.type_text("x");
    for _ in 0..3 {
        f.feed_raw(Key::ctrl_char('y'));
    }
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcdef\nxxcdey\n", "vim 9.1: i<C-y> copies the line above character by character");
}

/// **Finding 4 (P2)**: insert `<C-e>` (copy the character from the same column of the line below) is
/// unbound.
/// oracle: in `xyz\nab\n`, at row 1 col 0, `i` then `<C-e>` four times → getline(1)="abxyz"
/// (it copies 'a' then 'b', then a no-op once the line below is exhausted). The engine leaves the
/// buffer as it was.
#[test]
fn ctrl_e_copies_char_from_line_below() {
    let mut f = Fixture::at("xyz\nab\n", 0, 0);
    f.feed(["i"]);
    for _ in 0..4 {
        f.feed_raw(Key::ctrl_char('e'));
    }
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abxyz\nab\n", "vim 9.1: i<C-e> copies the line below character by character");
}

/// **Finding 5 (P2)**: insert `<C-a>` (reinsert the most recently inserted text) is unbound.
/// oracle: on `foo\nbar\n`, `ihello <Esc>jA<C-a>!<Esc>` → getline(2) = "barhello !"
/// (multi-line text is carried over the same way; C-a replays "hello " inside the A session).
/// The engine gives "bar!" (C-a discarded).
#[test]
fn ctrl_a_inserts_last_inserted_text() {
    let mut f = Fixture::at("foo\nbar\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hello ");
    f.feed(["<Esc>", "j", "A"]);
    f.feed_raw(Key::ctrl_char('a'));
    f.type_text("!");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hello foo\nbarhello !\n", "vim 9.1: i<C-a> replays the last inserted text");
}

/// **Finding 6 (P2)**: insert `<C-@>` (replay the last inserted text and leave insert) is unbound.
/// oracle: on `x`, `ihi<Esc>i<C-@><Esc>` → "hhiix" with mode=n (after replaying "hi" the C-@ returns
/// to Normal, so a following Z never reaches the buffer -- proven by ca12).
/// Engine: C-@ is dropped silently and the mode stays in insert ("hix").
#[test]
fn ctrl_at_inserts_last_insert_then_leaves_insert() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hi");
    f.feed(["<Esc>", "i"]);
    f.feed_raw(Key::ctrl_char('@'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hhiix\n", "vim 9.1: i<C-@> replays the last inserted text");
    assert!(
        matches!(f.vim.mode(), Mode::Normal),
        "vim 9.1: after i<C-@> insert mode should already have been left"
    );
}

/// **Finding 7 (P2)**: insert `<C-r>=` (the expression register) is not implemented -- the engine
/// looks `=` up as an ordinary register name, misses, swallows it silently, and inserts all the
/// following expression keys as text.
/// oracle: on `x`, `i<C-r>=1+1<CR><Esc>` → getline(1)="2x" (vim opens the `=` prompt, evaluates and
/// inserts the result); the engine gives "1+1\nx\n".
#[test]
fn ctrl_r_equals_expression_register() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["="]);
    f.type_text("1+1");
    f.feed(["<CR>", "<Esc>"]);
    assert_eq!(f.text(), "2x\n", "vim 9.1: i<C-r>= evaluates the expression and inserts the result");
}

// ------------------------------------------------ the accounting rules of the `".` register

/// **Finding 8 (P1)**: an empty insert session should **clear** `".` (and the same last-insert buffer
/// used by `<C-a>`/`<C-@>`), but the engine keeps the stale content.
/// oracle: after `ihi<Esc>i<Esc>`, `string(@.)` = '' (the dot2 probe; after `i<Esc>` neither `<C-a>`
/// nor `<C-@>` inserts anything -- the ba6 probe). On an empty session the engine does not clear
/// `insert_session_text`, so `i<C-r>.` pastes the stale "hi".
#[test]
fn empty_insert_session_clears_last_insert_register() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hi");
    f.feed(["<Esc>", "i", "<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    // the "hi" of the first session is still in the buffer; the assertion point is that the second
    // row no longer pastes stale text (the old engine gave "hiZZ\nhiZZ\n"; after clearing, = vim)
    assert_eq!(f.text(), "hiZZ\nZZ\n", "vim 9.1: after an empty session i<C-r>. pastes nothing");
}

/// **Finding 9 (P1)**: text deleted by `<C-w>`/`<C-u>` inside the session must be taken out of `".`,
/// but the engine's `insert_session_text` only ever grows.
/// oracle: after `ifo<C-w><Esc>`, `i<C-a><Esc>` inserts nothing (the ba1 probe; `i<C-u>` likewise per
/// rg4) -- i.e. `".` is empty. The engine's `i<C-r>.` pastes "fo".
#[test]
fn ctrl_w_and_ctrl_u_trim_last_insert_register() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("fo");
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ZZ\nZZ\n", "vim 9.1: text deleted by C-w no longer enters `\".`");
}

/// **Finding 10 (P2)**: a real backspace deletion inside the session must be recorded as a
/// **replayable backspace key**: vim's `".` holds "abc" plus the K_BS internal code (strtrans probe
/// rg1: after `iabc<BS><Esc>`, `@.` = "abc<80>kb"), so `i<C-r>.`/`<C-a>` runs the backspace **again**
/// while replaying -- the net effect is "ab" (rg2: the `ZZ` row pastes into "ZabZ").
/// The engine pastes the never-deleted "abc".
#[test]
fn backspace_in_session_replays_as_backspace_from_last_insert() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("abc");
    f.feed(["<BS>", "<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abZZ\nZabZ\n", "vim 9.1: the backspace stored in `\".` executes again on replay");
}

// ------------------------------------------------ macro record / replay

/// **Finding 11 (P2)**: `q"` (record into the unnamed register, a legal member of
/// `q{0-9a-zA-Z"}`) is refused by the engine (not alphanumeric → bell, stays idle); vim 9.1 accepts it.
/// oracle: on `ZZ`, `q"ix<Esc>q` then `@"` → "xxZZ" (the qa1b probe; `strtrans(@")` = "ix^[", qa4).
#[test]
fn q_quote_records_into_unnamed_register() {
    let mut f = Fixture::at("ZZ\n", 0, 0);
    f.feed(["q", "\"", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q", "@", "\""]);
    assert_eq!(f.text(), "xxZZ\n", "vim 9.1: q\" records into the unnamed register and @\" replays it");
}

/// **Finding 12 (P1)**: inside a linewise visual selection `@{reg}` should execute the register (the
/// first visual command of the macro applies to the active selection); the engine has no binding
/// (bell, selection left as it was).
/// oracle: on `aa\nbb\ncc\n`, `qaxq<Esc>jV@a` → the buffer becomes ["a","cc"] (the vr8 probe: the
/// selected "bb" row is deleted wholesale by the replayed `x`, cur=2,1).
#[test]
fn at_in_linewise_visual_runs_the_register() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["q", "a", "x", "q", "<Esc>", "j", "V", "@", "a"]);
    assert_eq!(
        f.text(),
        "a\ncc\n",
        "vim 9.1: @a inside V deletes the selected row with the replayed x"
    );
    assert_eq!(f.buf.line_count(), 2, "vim 9.1: the bb row is deleted as a whole line");
}

// ------------------------------------------------ `.` replay

/// **Finding 13 (P1)**: when `.` replays an insert that started with `gi`, the engine re-runs [g,i]
/// verbatim and jumps back to the `^` position; vim's `.` puts the inserted text at the **current
/// cursor** (gi's motion is not part of the change).
/// oracle: on `foo bar\nqux\n`, `giX<Esc>ww.` → ["Xfoo bar","Xqux"] (the gi2b probe). The engine
/// gives "XXfoo bar\nqux" (the X went back into `^`).
#[test]
fn dot_after_gi_inserts_at_current_cursor() {
    let mut f = Fixture::at("foo bar\nqux\n", 0, 0);
    f.feed(["g", "i"]);
    f.type_text("X");
    f.feed(["<Esc>", "w", "w", "."]);
    assert_eq!(f.text(), "Xfoo bar\nXqux\n", "vim 9.1: . inserts X at the current cursor");
}

/// **Finding 14 (P1)**: after a blockwise insert (`<C-v>jI…<Esc>`) the `.` must carry the change into
/// the redo -- vim replays the original key sequence (blockwise visual + I + text + Esc), i.e. it
/// re-inserts the text blockwise on the cursor row and the rows inside the block; the engine's
/// recording_blocked throws away the **whole** recording, so `.` replays some earlier unrelated change
/// (none here → just a bell).
/// oracle: on `aa\nbb\ncc\n`, `<C-v>jI-<Esc>w.` → ["--aa","--bb","cc"] (the b5 probe: the cursor row
/// and the block row each get one "-").
#[test]
fn dot_after_block_insert_redoes_block_insert() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("-");
    f.feed(["<Esc>", "w", "."]);
    assert_eq!(
        f.text(),
        "--aa\n--bb\ncc\n",
        "vim 9.1: . replays the blockwise insert (cursor row + block rows)"
    );
}

/// **Finding 15 (P2)**: the repeat condition of a count-driven insert is stricter than vim's -- the
/// engine requires the cursor to sit at `anchor + expanded.len()` and gives up repeating after a
/// single backspace; in vim 9.1 a backspace does **not** cancel the repeat (only arrow keys do; the
/// ar3 probe: `3ifoo<Left><Esc>` indeed does not repeat).
/// oracle: on `zz`, `3ifo<BS><Esc>` → "fffzz" (rg6: every round of "fo"+BS nets one "f" and the
/// repeat still fires). The engine's cursor drifts → the copy is skipped, giving "fzz".
#[test]
fn count_insert_repeat_survives_backspace() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("fo");
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(f.text(), "fffzz\n", "vim 9.1: a backspace does not cancel the count-insert repeat");
}
