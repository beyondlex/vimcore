//! Round-8 review regression tests.
//!
//! Same workflow as the earlier rounds: read the code and list the suspects -> verify each with a
//! vim 9.1 probe (`-es` script) -> fix -> register it here. Every test states how it was verified.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

// ---- 1. gq formats from the line start (no duplication of the pre-cursor text) --

/// vim 9.1 probe: with the cursor on 'bb', `gq}` reformats the first whole line from column 0
/// (['aaaa bb cc ddd','ee ff'] → ['aaaa bb cc ddd ee ff']).
/// The old behavior started the replacement range at span.start (mid-line) while the reformatted
/// text held the whole line → the "aaaa " before the cursor was duplicated
/// ("aaaa aaaa bb cc ddd ee ff").
#[test]
fn gq_formats_from_line_start_with_midline_cursor() {
    let mut f = Fixture::at("aaaa bb cc ddd\nee ff\n\ngg hh ii jj kk ll\n", 0, 5);
    f.feed(["g", "q", "}"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa bb cc ddd ee ff\n\ngg hh ii jj kk ll\n",
        "gq formats from the line start and must not duplicate the text before the cursor"
    );
}

/// The same fix covers the charwise visual span start: `v$gq` reformats from the line start under a
/// narrow textwidth (vim 9.1 probe: a line that fits stays as is; the point is that the text before
/// the cursor must not be repeated).
#[test]
fn gq_formats_from_line_start_with_visual_span() {
    let mut f = Fixture::at("aaaa bb cc ddd\nee ff\n", 0, 5);
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["v", "$", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa bb cc\nddd\nee ff\n",
        "the gq of v$ reformats the whole line from its start (the old behavior duplicated the aaaa before the cursor)"
    );
}

// ---- 2. non-mutating Ex commands stay out of the `.` replay --------------------

/// vim 9.1 probe: after `:2` (a pure move) plus `x`, `gg` then `.` replays only `x`
/// ("one"→"ne"). The old behavior left `:2<CR>` in the recording, turning `.` into
/// "jump to line 2 then delete" -- the replay landed on a completely wrong line.
#[test]
fn dot_ignores_nonmutating_ex_commands() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed([":", "2", "\n"]); // a move with no edit
    f.feed(["x"]); // last_change = x ("two" → "wo")
    f.feed(["g", "g"]);
    f.feed(["."]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ne\nwo\nthree\n",
        ". replays only x, never the earlier :2 move"
    );
}

/// An E492 unknown command does not pollute the recording either (under the old behavior the next
/// change after `:foo` carried it along).
#[test]
fn dot_ignores_failed_ex_commands() {
    let mut f = Fixture::new("ab\n");
    f.feed([":", "f", "o", "o", "\n"]); // E492
    f.feed(["x"]);
    f.feed(["."]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n", "two x in a row empty the line");
}

/// After `:noh` (a query/clear-type command) `.` still replays the change itself.
#[test]
fn dot_after_noh_repeats_the_change_only() {
    let mut f = Fixture::new("abc\n");
    f.feed(["x"]);
    f.feed([":", "n", "o", "h", "\n"]);
    f.feed(["."]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "c\n");
}

// ---- 3. case operators on a blockwise visual selection (u/U/~) ------------------

/// vim 9.1 probe: `<C-v>jllU` uppercases the block columns and parks the cursor at the block start.
/// The old behavior rang and did nothing (apply_block_operator knew only Delete/Yank/Change).
#[test]
fn block_visual_uppercase_maps_rows() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "U"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ABC\nDEF\nghi\n",
        "blockwise U covers two rows and three columns (ll after j makes the block 3 wide)"
    );
    assert_eq!(f.line(), 0, "the cursor lands on the block's start row");
}

#[test]
fn block_visual_toggle_and_lowercase() {
    // the block of <C-v>j~ is 1 wide (the character under the cursor itself): it toggles only the
    // first character of each row
    let mut f = Fixture::at("aBc\ndEf\n", 0, 0);
    f.feed(["<C-v>", "j", "~"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "ABc\nDEf\n", "blockwise ~ covers only the block columns");

    let mut f = Fixture::at("ABC\nDEF\n", 0, 1);
    f.feed(["<C-v>", "j", "u"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "AbC\nDeF\n", "blockwise u lowercases only the block columns");
}

/// Blockwise U on multibyte rows: block columns resolve by display column (CJK is 2 cells wide) and
/// the replacement goes through edit_replace, so the in-line byte offset does not drift.
#[test]
fn block_visual_case_on_multibyte_rows() {
    let mut f = Fixture::at("中文ab\n日本cd\n", 0, 6); // byte 6 = 'a' (display column 4)
    f.feed(["<C-v>", "j", "U"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "中文Ab\n日本Cd\n",
        "blockwise U covers the 'a'/'c' column of both rows"
    );
}

// ---- 4. Vp linewise paste leaves the surviving indented lines alone ------------

/// vim 9.1 probe: register "XY" (linewise), V over ['abc'] then p -- the surviving indented line
/// ' ghi' stays intact (['XY',' ghi','tail']). The old behavior inserted at the cursor parked by
/// delete_span (first_non_blank, mid-line), yielding " XY" and losing the indent of 'ghi'.
#[test]
fn linewise_put_replace_pastes_at_line_start() {
    let mut f = Fixture::new("abc\n ghi\ntail\nXY\n");
    f.feed(["G", "\"", "a", "y", "y"]); // "a{XY}
    f.feed(["d", "d"]); // remove the fixture line
    assert_eq!(f.buf.slice(0..f.buf.len()), "abc\n ghi\ntail\n");
    f.feed(["g", "g"]);
    f.feed(["V", "p"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "XY\n ghi\ntail\n",
        "the linewise paste of Vp lands at the start of the selection's first line; the surviving indented line is intact"
    );
}

/// Same shape for a multi-line selection: the pasted lines replace all selected lines wholesale.
#[test]
fn linewise_put_replace_multi_line_selection() {
    let mut f = Fixture::new("aaa\nbbb\nccc\nXY\n");
    f.feed(["G", "\"", "a", "y", "y", "d", "d"]);
    f.feed(["g", "g"]);
    f.feed(["V", "j", "p"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "XY\nccc\n");
}

// ---- 5. an :s with an invalid pattern does not become the "last substitute" -----

/// `:s/(/x/` (pattern compile fails) must not displace the last usable `:s` -- `&` should replay the
/// most recent command that compiled (the old behavior recorded last_substitute before build()).
#[test]
fn amp_skips_substitute_with_invalid_pattern() {
    let mut f = Fixture::new("foo\n");
    f.feed([":", "s", "/", "o", "/", "0", "/", "\n"]); // legal, establishes the last substitute first
    f.feed([":", "s", "/", "(", "/", "x", "/", "\n"]); // invalid pattern, rings the bell
    f.feed(["&"]); // should replay o->0
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "f00\n",
        "& replays the most recent :s that compiled; an invalid pattern must not take its place"
    );
}

// ---- 6. cross-line lockdown of block-insert sessions (caught by fuzz round8) ---

/// In a block session a backspace at the line start merged the previous line and scrambled the copy
/// offset (fuzz: inserting past the end of the buffer). After the fix it refuses with a bell, and at
/// exit the copy still lands on the right rows.
#[test]
fn block_session_rejects_backspace_line_join() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["j"]); // cursor row = line 1 (the typing row; only a BS at the line start triggers a join)
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X"); // line1 = "Xcd", block.text = "X"
    // <Left> + <Del> remove the typed X from the buffer: vim 9.1 evidence (block-insert probe) is
    // that the session's copies vanish with the typed text and are not reproduced at exit -- round
    // 8's expectation "X is still copied" contradicts vim, corrected here in this round (13) per the probe
    f.feed(["<Left>", "<Del>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "ab\ncd\nef\n");
    // BS at the line start: the old behavior joined ab/cd and scrambled the copy offset; now refused
    f.feed(["<BS>"]);
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ab\ncd\nef\n",
        "the typed text was deleted and the session diverged -- vim probe: no row is reproduced"
    );
}

/// round13: BS in a block session deletes the last typed character, and only at the end of the typed
/// text (vim 9.1 evidence: `ab<BS>` → "a" plus the copy "a"; with `ab<Left><BS>` the BS is a no-op
/// and never eats into the line content).
#[test]
fn block_session_bs_only_deletes_typed_tail() {
    // BS at the end: deletes b and the copy shortens to "a"
    let mut f = Fixture::new("cd\nef\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("ab");
    f.feed(["<BS>", "escape"]);
    assert_eq!(f.text(), "acd\naef\n", "the trailing BS deletes a typed char and shortens the copy in step");

    // BS after Left: a no-op (no line content deleted), the copy stays "ab"
    let mut f2 = Fixture::new("cd\nef\n");
    f2.feed(["<C-v>", "j", "I"]);
    f2.type_text("ab");
    f2.feed(["<Left>", "<BS>", "escape"]);
    assert_eq!(f2.text(), "abcd\nabef\n", "a mid-typing BS is a no-op and the copy keeps the full typed text");
}

/// In a block session a <Del> that removes a newline would likewise join lines -- refused.
#[test]
fn block_session_rejects_delete_newline() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed(["<C-v>", "j", "I"]); // typing row = cursor row line 1, copy target = line 0
    f.type_text("X"); // line1 = "Xcd", block.text = "X"
                      // <Del> at the end of the typing row: the old behavior deleted the \n and joined
    f.feed(["<End>", "<Del>"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ab\nXcd\n",
        "the cross-line <Del> is refused"
    );
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "Xab\nXcd\n",
        "the copy is unaffected by <Del> and still lands on the remaining block rows"
    );
}

/// In a block session IME text containing a newline would split the typing row -- it is refused
/// wholesale (same lock as <CR>), and refused text must not reach the copy made at exit
/// (the record/insert contract).
#[test]
fn block_session_rejects_newline_in_typed_text() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("x\ny"); // contains a newline: refused wholesale
    assert_eq!(f.buf.slice(0..f.buf.len()), "ab\ncd\n", "text containing \\n is refused");
    f.type_text("z"); // after the refusal, ordinary typing still takes effect
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "zab\nzcd\n",
        "only text that really landed in the buffer takes part in the copy"
    );
}

/// A host click inside a block session must not move the typing point (fuzz: the click threw the
/// cursor onto another row and the copy offset was written past the end of the buffer).
#[test]
fn block_session_ignores_host_click() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    // the host clicks the end of the buffer (line 2): it must be ignored while the session runs
    let len = f.buf.len();
    f.vim.set_cursor_offset(&f.buf, len);
    f.type_text("Y");
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "XYab\nXYcd\nef\n",
        "the click does not move the typing point and the copy lands whole on the block rows"
    );
}

// ---- 7. experience extras (blockwise O / :marks specials / the :bN alias) ------

/// [Round-14 correction] The probe10 this test froze was a misreading. Three independent vim 9.1
/// probes agree: blockwise `O` keeps the cursor's **row** and moves it to the other **column** corner
/// of the block (`:h v_O` "the cursor moves to the other corner in the same line"), the anchor
/// mirrors onto the other column while the rectangle is unchanged; a single-row block is a real
/// horizontal corner swap rather than a no-op; and `O` in charwise/linewise visual equals `o`
/// (no bell). The three probes: anchor(0,0)+cursor(1,2) → cursor(1,0);
/// anchor(1,1)+cursor(2,2) (this geometry) → cursor(2,1); single row (0,2) → (0,0).
#[test]
#[allow(non_snake_case)] // O is a key name
fn block_visual_O_moves_to_other_row_same_col() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\ndddd\n");
    f.feed(["2", "g", "g", "l"]); // line1 col1 (0-based)
    f.feed(["<C-v>", "j", "l"]); // block lines1-2 cols1-2, cursor (2,2)
    f.feed(["O"]);
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(f.buf.offset_to_line(cursor), 2, "O keeps the cursor's row");
    assert_eq!(
        vimcore::buffer::display_column(&f.buf, cursor),
        1,
        "the cursor moves to the other column corner of the block"
    );
    assert_eq!(f.buf.offset_to_line(anchor), 1, "the anchor's row is unchanged");
    assert_eq!(
        vimcore::buffer::display_column(&f.buf, anchor),
        2,
        "the anchor mirrors onto the other column (rectangle unchanged)"
    );

    // O on a single-row block: a real horizontal corner swap (vim probe (0,2) → (0,0))
    let mut f = Fixture::new("aaaa\nbbbb\n");
    f.feed(["<C-v>", "l"]);
    f.feed(["O"]);
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(f.buf.offset_to_line(cursor), 0);
    assert_eq!(vimcore::buffer::display_column(&f.buf, cursor), 0, "O swaps corners on a single-row block");
    assert_eq!(vimcore::buffer::display_column(&f.buf, anchor), 1);
}

/// [Round-14 correction] `O` in charwise/linewise visual equals `o` (vim 9.1 probes:
/// `vllO` → cursor (0,0), `VjO` → (0,0), no bell).
#[test]
#[allow(non_snake_case)] // O is a key name
fn char_visual_O_mirrors_ends_like_o() {
    let mut f = Fixture::new("abcdef\n");
    f.feed(["v", "l", "l", "l", "O"]);
    assert_eq!(f.host.bells, 0, "O in charwise visual does not ring the bell");
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(cursor, 0, "the cursor mirrors to the opposite end");
    assert_eq!(anchor, 3);
}

/// `:marks` now also lists `.` (last change) and `^` (where insert mode was left).
#[test]
fn marks_listing_shows_change_and_insert_specials() {
    let mut f = Fixture::new("alpha\nbeta\n");
    f.feed(["x"]); // the change → `.
    f.feed(["i"]); // the insert session → `^
    f.type_text("Z");
    f.feed(["escape"]);
    f.feed([":", "m", "a", "r", "k", "s", "\n"]);
    // the vim 9.1 line format is ` %c %6ld %4d %s` (mark.c), so the special marks start with " ."/" ^"
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with(" .")),
        ":marks should list `.` ({:?})",
        f.host.statuses
    );
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with(" ^")),
        ":marks should list `^` ({:?})",
        f.host.statuses
    );
}

/// `:bN` is vim's alias for `:bprev` (the old behavior gave E492).
#[test]
#[allow(non_snake_case)] // bN is a command name
fn bN_aliases_bprev() {
    let mut f = Fixture::new("text\n");
    f.feed([":", "b", "N", "\n"]);
    assert_eq!(f.host.bells, 0, ":bN should hit bprev instead of ringing with E492");
}
