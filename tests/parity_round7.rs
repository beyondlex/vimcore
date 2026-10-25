//! Round-7 review regression tests.
//!
//! The workflow follows the earlier rounds: read the code and list suspicions ->
//! prove them with vim 9.1 probes (`-es` scripts) -> fix -> record here. Each
//! test notes how it was proven.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::mode::VisualKind;

// ---- 1. the active_visual lifetime ----------------------------------------

/// After `Vjd`, `:'<,'>d` must read the `'<`/`'>` marks (the newest selection)
/// and may not use the stale live offsets from before the operator. The old
/// behaviour deleted the whole buffer.
#[test]
fn cmdline_range_uses_marks_not_stale_live_visual() {
    let mut f = Fixture::new("aaa\nbbb\nccc\nddd\n");
    f.feed(["V", "j", "d"]); // delete L1-L2 -> ["ccc", "ddd"]
    assert_eq!(f.buf.slice(0..f.buf.len()), "ccc\nddd\n");
    f.feed([":", "'", "<", ",", "'", ">", "d", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ddd\n",
        ":'<,'>d should delete the line the cursor is on (ccc), not a stale live range"
    );
}

/// A visual-mode text object extension (the cross-line `vi(`) must keep the live
/// range in sync so that the `'<,'>` of `:` covers the whole object. The old
/// behaviour only replaced the collapsed line.
#[test]
fn object_extension_updates_live_range_for_cmdline() {
    let mut f = Fixture::new("let a = foo(\n    bar,\n    baz,\n);\n");
    // move the cursor onto bar of L1 (offset 17); vi( selects L1-L2 and :s
    // replaces both lines
    f.vim.cursor.offset = 17;
    f.feed(["v", "i", "("]);
    // the pattern ba hits both bar and baz: if the range collapsed onto L1 only
    // one of them would be replaced
    f.feed([":", "s", "/", "b", "a", "/", "X", "/", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "let a = foo(\n    Xr,\n    Xz,\n);\n",
        "after vi( the :s should cover every line of the object (bar and baz)"
    );
}

// ---- 2. gv restores the selection after a visual change session -----------

/// After `viwcX<Esc>`, `gv` restores the selection. vim 9.1 probe: `'<`/`'>`
/// keep the pre-edit byte range (gv reselects 5 characters) instead of an empty
/// range after collapsing/shifting.
#[test]
fn gv_restores_selection_after_visual_change() {
    let mut f = Fixture::new("alpha beta\ngamma\n");
    f.feed(["v", "i", "w"]); // select alpha (offset 0..5)
    f.feed(["c"]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "X beta\ngamma\n");
    f.feed(["g", "v"]);
    let sel = f.vim.visual_selection().expect("gv should restore the selection");
    // the 9.1 oracle matrix (audit C12): the CHARWISE gv reselection is a
    // **reversed** selection -- the anchor on the last character, the cursor at
    // the start
    assert_eq!(sel.0, 4, "the gv anchor = the last character of the original selection");
    assert_eq!(sel.2, VisualKind::Char);
    assert_eq!(sel.1, 0, "the gv cursor = the start of the original selection");
}

/// After the block insert `C-v jj I`, `gv` restores the original block. vim 9.1
/// probe: `'<`/`'>` land on the columns of the original block.
#[test]
fn gv_restores_block_after_block_insert() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["<C-v>", "j", "j", "I"]);
    f.type_text("Z");
    f.feed(["escape"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "Zab\nZcd\nZef\n");
    f.feed(["g", "v"]);
    let sel = f.vim.visual_selection().expect("gv should restore the block selection");
    assert_eq!(sel.2, VisualKind::Block);
    assert_eq!(sel.0, 0, "block start = the first cell of the original block (start of L0)");
    // block gv puts the cursor back at the start of the block's last line (the
    // cursor end follows the edit funnel to post-edit coordinates)
    assert_eq!(sel.1, 6, "block end = the last cell of the original block (start of L2, pre-edit offset)");
}

// ---- 3. with hlsearch=false, cancelling the prompt must not leak highlights

/// hlsearch=false + incsearch=true: the `/` preview highlights must be cleared
/// once Esc cancels. The old behaviour republished `last_matches` (the matches of
/// the previous pattern) verbatim, leaving permanent highlights.
#[test]
fn cancel_cmdline_respects_hlsearch_off() {
    let mut f = Fixture::new("foo\nbar\nfoo\n");
    f.vim.options_mut().hlsearch = false;
    f.feed(["/", "f", "o", "o", "\n"]);
    assert!(
        f.host.highlights.is_empty(),
        "with hlsearch=false there should be no highlights after a search"
    );
    f.feed(["/", "b"]); // the incsearch preview
    f.feed(["escape"]);
    assert!(
        f.host.highlights.is_empty(),
        "after cancelling the prompt the last_matches highlights must not come back"
    );
}

// ---- 4. Esc precedence ----------------------------------------------------

/// `g<Esc>` cancels silently (the old implementation went through the trie-miss
/// retry path and rang the bell by mistake).
#[test]
fn escape_cancels_pending_state_quietly() {
    let mut f = Fixture::new("abc\n");
    f.feed(["g"]);
    f.feed(["escape"]);
    assert_eq!(f.host.bells, 0, "g<Esc> should cancel silently");
}

/// After `3"<Esc>` the count must be cancelled along with it, so dd deletes only
/// one line.
#[test]
fn escape_at_register_prefix_cancels_count() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
    f.feed(["3", "\""]);
    f.feed(["escape"]);
    f.feed(["d", "d"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "l2\nl3\nl4\n",
        "Esc must cancel the whole pending `3\"` (the count included), so dd deletes one line"
    );
}

// ---- 5. :sort u on a single line builds no phantom undo group -------------

/// `:sort u` on a single-line range is a no-op (one line has no consecutive
/// duplicates to dedup), so it must not declare an undo group for an unchanged
/// replace. The old behaviour burned one `u` every time.
#[test]
fn sort_u_single_line_creates_no_undo_group() {
    let mut f = Fixture::new("hello\n");
    f.feed([":", "s", "o", "r", "t", " ", "u", "\n"]);
    assert_eq!(f.host.group_count, 0, "a single-line :sort u changes nothing, so no undo group");
    f.feed(["x"]);
    assert_eq!(f.host.group_count, 1, "only x builds a group");
}

// ---- 6. after visual :, '< '> keep the pre-execution range ----------------

/// Opening `:` from a visual selection and running :s must leave `'<`/`'>` holding
/// the selection as it was when the prompt opened (vim: it keeps the range that
/// was executed). The old behaviour rewrote them as "anchor..post-execution
/// cursor": :s puts the cursor on the first non-blank, the selection collapsed to
/// 1 character and the second-order semantics of gv broke.
#[test]
fn visual_colon_keeps_prompt_time_range_in_marks() {
    let mut f = Fixture::new("hello world\nsecond\n");
    f.feed(["v", "i", "w"]); // select hello (anchor 0, cursor 4)
    f.feed([":", "s", "/", "h", "e", "l", "/", "H", "E", "L", "/", "\n"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "HELlo world\nsecond\n");
    // :s leaves the cursor at the start of the line (col 0) -- were we to rewrite
    // from the post-execution cursor, '> would collapse to 1
    let lt = f.vim.marks.resolve('<', &f.buf).unwrap();
    let gt = f.vim.marks.resolve('>', &f.buf).unwrap();
    assert_eq!(lt, 0, "'< = the start of the selection at prompt time");
    // audit E3: `'>` resolves to the **last character** of the selection ('o' =
    // offset 4) and no longer exposes the exclusive-end byte; gv's internal
    // storage is still 0..5
    assert_eq!(gt, 4, "'> = the last character of the selection at prompt time, it does not collapse with the cursor after the command");
    // gv reselects the original range (0..5; 9.1 oracle matrix, audit C12: the
    // CHARWISE gv is a reversed selection -- anchor on the last character, cursor
    // at the start)
    f.feed(["g", "v"]);
    assert_eq!(
        f.vim.visual_selection().map(|(a, c, _)| (a, c)),
        Some((4, 0)),
        "gv should restore the selection of hello"
    );
}

// ---- 7. the register argument of :d ---------------------------------------

/// `:1,2d a` stores the deleted lines into `"a` (vim 9.1: `:d [x]`; the old
/// implementation ignored that argument).
#[test]
fn ex_delete_stores_into_named_register() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed([":", "1", ",", "2", "d", " ", "a", "\n"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "three\n");
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a should have content");
    assert_eq!(reg.text, "one\ntwo\n");
    assert!(matches!(
        reg.kind,
        vimcore::registers::RegisterKind::Linewise
    ));
}

/// `:2d 3` (the count counted from the last line of the range) still deletes
/// lines 2-4 per the round-6 semantics, and without a register argument it goes
/// down the existing delete-register path.
#[test]
fn ex_delete_count_semantics_unchanged() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\nfive\n");
    f.feed([":", "2", "d", " ", "3", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "one\nfive\n",
        ":2d 3 deletes 3 lines starting from line 2"
    );
    // the deleted multi-line text lands in "1 (the numbered ring)
    let reg = f.vim.registers.lock().unwrap().get('1').cloned().expect("a multi-line delete should go into \"1");
    assert_eq!(reg.text, "two\nthree\nfour\n");
}

// ---- 8. fixes caught by the adversarial fuzz ------------------------------

/// `db` at the start of a line: b crosses to the end of the previous line, and
/// exclusive + column 1 + a start before the first non-blank -> a linewise delete
/// of the **previous line** (vim 9.1 probe: ['abc','def'] -> ['def']). The old
/// implementation's `target_line - 1` underflowed a usize and panicked when
/// crossing upwards.
#[test]
fn db_at_line_start_deletes_previous_line_linewise() {
    let mut f = Fixture::new("abc\ndef\n");
    f.vim.cursor.offset = f.buf.line_start(1);
    f.feed(["d", "b"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "def\n",
        "db at the start of a line = a linewise delete of the previous line (vim probe)"
    );
}

/// When the match selected by `gn` ends with a multi-byte character, the cursor
/// must land on the **start** of the last character; `end - 1` byte arithmetic
/// would drop it inside a character (fuzz: after matching `中`, the host's
/// `offset_to_line` panicked outright).
#[test]
fn gn_cursor_lands_on_multibyte_match_start() {
    let mut f = Fixture::new("foo 中 bar\n");
    f.feed(["/", "中", "\n"]); // search for 中
    f.feed(["g", "g"]); // back to the start (the pattern is still live)
    f.feed(["g", "n"]); // select the match
    let co = f.vim.cursor_offset();
    let text = f.buf.slice(0..f.buf.len());
    assert!(
        text.is_char_boundary(co),
        "after gn the cursor {co} must sit on a character boundary (text {text:?})"
    );
    let sel = f.vim.visual_selection().expect("gn should enter visual mode");
    assert_eq!(sel.0, 4, "selection start = the start of 中");
}

// ---- harness helpers ------------------------------------------------------
// insert typing goes through common::Fixture::type_text (record_typed_text +
// insert_text_at_cursor, the same shape as place_text in the integration layer).
