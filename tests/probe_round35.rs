//! Companion probes of the fourth audit round (regressions for the bugs the
//! round-35 fuzz caught, 2026-10-09).
//!
//! These assertions state the expected results of the vim 9.1 oracle and should
//! fail before the fix.
//! Oracle channel: `vim -Nu NONE -N -i NONE -n -s/-es` (sampled 2026-10-09).

mod common;

use common::Fixture;

// ---- R35-1 (P0): `vl<Esc>:marks` panics on a CJK buffer -------------------
// `list_marks` renders the `>` row with `hi.saturating_sub(1)`, a raw minus of 1
// byte that can land inside a multi-byte character, so the host's
// `offset_to_line` slice panics.
//
// oracle: on `中文缓冲`, after `vll<Esc>` the `:marks` output shows the `>` row
// normally as ` >      1    6 中文缓冲` (row format ` %c %6ld %4d %s`; the round-35
// fuzz only hit the panic surface; see R35-2 for the col semantics).
#[test]
fn r35_1_marks_gt_row_on_cjk_selection_does_not_panic() {
    let mut f = Fixture::new("中文缓冲\n");
    f.feed(["v", "l", "l", "<Esc>"]);
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("marks".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    // the `>` row sits on the last character of the selection (缓), col = the
    // 0-based byte column 6
    let gt = f
        .host
        .statuses
        .iter()
        .find(|s| s.starts_with(" >"))
        .cloned();
    let gt = gt.unwrap_or_else(|| format!("no > row in {:?}", f.host.statuses));
    assert!(
        gt.ends_with("6 中文缓冲"),
        "oracle: the `>` row has col 6 (the 0-based byte column of the last selection char 缓): {gt:?}"
    );
}

// ---- R35-2 (P2): the col column of `:marks` is a 0-based byte column, not
// display column + 1
//
// oracle (redir channel, 2026-10-09): on `ab\tcd`, after `fc` `md` the `:marks d`
// output is ` d      1    3 ab^Icd`. d's 0-based **byte** column is 3 (its display
// column is 9 with ts=8); the line content is listed verbatim (indent not
// trimmed) and TAB becomes `^I`.
#[test]
fn r35_2_marks_col_is_zero_based_byte_column() {
    let mut f = Fixture::new("ab\tcd\nsecond\n");
    f.feed(["f", "c", "m", "d"]);
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("marks d".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(
        f.host.statuses.last().map(String::as_str),
        Some(" d      1    3 ab^Icd"),
        "oracle: col is the 0-based byte column 3, the line is not trimmed and TAB becomes ^I: {:?}",
        f.host.statuses
    );
}

// ---- R35-3 (P1): after `:retab` the cursor stays on the same character -----
//
// Old behaviour: after the whole-paragraph `edit_replace` the cursor kept its raw
// byte offset -- once a whitespace run grew, the cursor pointed at a different
// character or even fell inside a multi-byte character (round-35 fuzz: IME Chinese
// plus `:1,3retab` put the cursor inside 文).
//
// oracle (2026-10-09): on `a\t中文x`, after `:set et` and `0wl` (cursor on 文)
// then `:1retab` -> the line becomes `a       中文x` (ts=8, the TAB expanded to 7
// columns) and getpos('.') = [0,1,12,0] = the new byte position of 文.
#[test]
fn r35_3_retab_keeps_cursor_on_same_char() {
    let mut f = Fixture::new("a\t中文x\nsecond\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>"]);
    f.feed(["0", "w", "l"]); // the cursor lands on 文
    assert_eq!(f.cursor(), 5, "precondition: 0wl lands on 文 (a=0, TAB=1, 中=2..5, 文=5..8)");
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("1retab".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(
        f.text(),
        "a       中文x\nsecond\n",
        "with ts=8 the TAB (column 1, width 7) expands to 7 spaces"
    );
    assert_eq!(f.cursor(), 11, "oracle [0,1,12,0]: the cursor stays on 文 at its new byte position 11");
}

// ---- R35-4 (P1): after `:ce`/`:ri`/`:le` the cursor stays on the same char --
//
// oracle (2026-10-09): on `  hello 中文`, after `0ll` (cursor on h) and `:ce 20`
// -> the line becomes `     hello 中文` and getpos('.') = [0,1,6,0] = the new byte
// position of h.
#[test]
fn r35_4_align_keeps_cursor_on_same_char() {
    let mut f = Fixture::new("  hello 中文\nsecond\n");
    f.feed(["0", "l", "l"]); // the cursor is on h
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("ce 20".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(f.text(), "     hello 中文\nsecond\n", "centring pads to 5 columns");
    assert_eq!(f.cursor(), 5, "oracle [0,1,6,0]: the cursor stays on h at its new byte position 5");
}

// ---- R35-6 (P1): `r<C-E>` skips each position with no neighbour char instead
// of giving up wholesale
//
// oracle (keys-file channel, 2026-10-09): on `abcdef` + `abX`, `0ll2r<C-E>`
// -> the line becomes `abXdef` ('c' ← 'X', and 'd' has no character in the
// neighbouring line so it is skipped), getpos('.') = [0,1,4,0] = the last handled
// position 'd'. The engine's old implementation returned false wholesale (text
// unchanged + bell).
#[test]
fn r35_6_r_ctrl_e_skips_missing_neighbor_chars() {
    let mut f = Fixture::new("abcdef\nabX\n");
    f.feed(["0", "l", "l", "2", "r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "abXdef\nabX\n", "oracle: 'c' ← 'X', 'd' skipped and left as is");
    assert_eq!(f.cursor(), 3, "oracle [0,1,4,0]: the cursor lands on the last handled position 'd'");
}

// ---- R35-7 (P1): when `r<C-E>` replaces with a shorter char the cursor lands
// on the replacement
//
// round-35 fuzz seed=26: the cursor was on ｳ (3 bytes) and r<C-E> took 'e'
// (1 byte) from the neighbouring line; the old implementation used `end -
// last_len` with the stale pre-edit end, so the cursor walked backwards into the
// following 漢 (byte 9, unaddressable).
#[test]
fn r35_7_r_ctrl_e_cursor_on_shorter_replacement() {
    let mut f = Fixture::new("\nｱｲｳ漢ｱｲｳ字\none\n");
    f.feed(["j", "l", "l"]); // the cursor reaches ｳ of the second line (bytes 7..10)
    assert_eq!(f.cursor(), 7);
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "\nｱｲe漢ｱｲｳ字\none\n", "ｳ ← the 'e' at col 2 of the neighbouring line");
    assert_eq!(f.cursor(), 7, "the cursor lands on the replacement char 'e' (byte 7), not inside 漢");
    assert!(f.text().is_char_boundary(f.cursor()));
}

// ---- R35-8 (P3): `r<C-E>` with no character for any position = a no-op (no
// bell)
//
// In vim's source nv_replace, a NUL position is handled with
// `++curwin->w_cursor.col` (skip it) and there is no E message (the old
// implementation of the fuzz-hit `r<C-e>` path called note_bell).
#[test]
fn r35_8_r_ctrl_e_all_skipped_is_quiet_noop() {
    let mut f = Fixture::new("abｱcd\nx\n");
    f.feed(["0", "l", "l"]); // the cursor is on ｱ (col 2); col 2 of the neighbouring "x" line has no character
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "abｱcd\nx\n", "everything skipped = no edit");
    assert_eq!(f.cursor(), 2, "the cursor does not move");
    assert_eq!(f.host.bells, 0, "no bell (vim emits no E message)");
}

// ---- R35-5 (P1): on a line after the range, the cursor shifts by the byte
// delta after a whole-paragraph rewrite
// When the cursor is on a line after the edited range it likewise must not keep a
// raw offset (the content moved by delta bytes overall). The oracle shape matches
// the "marks adjustment" semantics of R35-3/4: the cursor stays on the same
// character.
#[test]
fn r35_5_range_rewrite_shifts_cursor_after_range() {
    let mut f = Fixture::new("中文\n\nab\tcd\n");
    f.feed(["G", "0", "f", "d"]); // the cursor is on the d of line 3
    let base = f.cursor();
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("%retab".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    // line 1 has no whitespace change; on line 3 the `\t` (column 2, ts=4 so width
    // 2) expands to 2 spaces, and since the cursor line is after lines 1..3 the
    // byte delta is +1, so d moves back with it
    assert_eq!(
        f.text(),
        "中文\n\nab  cd\n",
        "with ts=4 the TAB of line 3 (column 2, width 2) expands to 2 spaces"
    );
    assert_eq!(
        f.cursor(),
        base + 1,
        "a change on a line before the range shifts the post-range cursor by +1 byte overall, so d stays under the cursor"
    );
}
