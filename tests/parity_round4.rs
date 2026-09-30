//! Round-4 review regressions. Every vim-semantics case here was probed
//! against vim 9.1 (headless `-es` scripts) before the engine change; the
//! probes' outputs are quoted in the test docs.

mod common;

use common::{edit, Fixture};

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
    assert_eq!(
        f.text(),
        "AAAA\n\nBBBB\n",
        "dw@0 deletes the run, not the line"
    );
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
    assert_eq!(
        f.text(),
        "  Z \nfoo\n",
        "i after ^ inserts before the last space"
    );
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

// ---- no phantom undo groups --------------------------------------------------

/// No-op commands must not open a host undo group: vim's `u` skips them (vim
/// 9.1 probe — `x` on an empty line / `J` at EOF / `i<Esc>` leave the undo
/// count untouched). The engine used to announce a group speculatively at
/// command start, so a wasted snapshot ate one real `u` press.
#[test]
fn noop_commands_open_no_undo_group() {
    // x on an empty line
    let mut f = Fixture::new("aaa\n\nbbb\n");
    f.feed(["j", "x"]);
    assert_eq!(f.host.group_count, 0, "x on an empty line opens no group");

    // J at the last line
    let mut f = Fixture::new("aaa\nbbb\n");
    f.feed(["G", "J"]);
    assert_eq!(f.host.group_count, 0, "J at EOF opens no group");

    // an empty insert session
    let mut f = Fixture::new("abc\n");
    f.feed(["i", "<Esc>"]);
    assert_eq!(f.host.group_count, 0, "i<Esc> opens no group");

    // p with an empty register
    let mut f = Fixture::new("abc\n");
    f.feed(["p"]);
    assert_eq!(
        f.host.group_count, 0,
        "p with an empty register opens no group"
    );
}

/// The wasted-undo symptom: after a no-op, ONE `u` must revert the previous
/// real change (not just consume a phantom snapshot of identical text).
#[test]
fn noop_then_undo_reverts_the_real_change_once() {
    let mut f = Fixture::new("one\n\ntwo\n");
    f.feed(["x"]); // real change: "ne\n\ntwo\n"
    f.feed(["j", "x"]); // x on the empty line: a no-op
    f.feed(["u"]);
    // with a phantom group the first u would restore identical text and the
    // deletion would need a second press
    assert_eq!(f.text(), "one\n\ntwo\n", "one u reverts the deletion");
}

/// A cancelled char-argument (`r<Esc>`) opens no group either.
#[test]
fn cancelled_replace_opens_no_undo_group() {
    let mut f = Fixture::new("abc\n");
    f.feed(["r", "<Esc>"]);
    assert_eq!(f.host.group_count, 0);
}

// ---- block insert sessions ---------------------------------------------------

/// Vertical motions are locked out mid-block-insert: the row replication
/// assumes all typing landed on the session's typing row (a page motion used
/// to point it at another row and drift the replica offsets into
/// mid-character positions — caught by the key fuzz).
#[test]
fn block_insert_locks_vertical_motions() {
    // cursor ends on the BOTTOM row of the block (`jj`), so the typing row
    // is line 2 and the replicas go to the rows above
    let mut f = Fixture::at("foo\nbar\nbaz\n", 0, 0);
    f.feed(["<C-v>", "j", "j", "I"]);
    assert!(f.vim.mode() == vimcore::Mode::Insert);
    f.feed(["up", "down", "pageup"]);
    assert!(f.host.bells > 0, "vertical moves during block insert bell");
    f.type_text("X");
    f.feed(["<Esc>"]);
    // typing still replicated onto every selected row, nothing corrupted
    assert_eq!(f.text(), "Xfoo\nXbar\nXbaz\n");
}

/// Backspace during a block insert changes the typing row's length; the
/// replica offsets must follow the row's exact byte delta, not the typed
/// text's length.
#[test]
fn block_insert_backspace_keeps_replica_offsets_valid() {
    let mut f = Fixture::at("ab\ncd\nef\n", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("xy");
    f.feed(["<BS>"]);
    f.type_text("z");
    f.feed(["<Esc>"]);
    // "y" was undone, so the replica text is "xz" — exactly what the rows get
    assert_eq!(f.text(), "xzab\nxzcd\nef\n", "BS shrank the replica text");
}

// ---- registers ----------------------------------------------------------------

/// A failed/empty charwise span stores nothing: vim 9.1 probe `ci(` on `()`
/// leaves the unnamed register untouched. The old engine stored the empty
/// string into `"-`/unnamed on every empty-span delete.
#[test]
fn empty_span_delete_keeps_previous_register() {
    let mut f = Fixture::new("keepme\na()b\n");
    f.feed(["g", "y", "y"]); // hmm — use a real yank below instead
    let mut f = Fixture::new("keepme\na()b\n");
    f.feed(["y", "y"]); // unnamed = "keepme\n"
    f.feed(["j"]);
    f.feed(["f", "(", "c", "i", "("]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "keepme\na(Z)b\n");
    let reg = f.vim.registers.get('"').expect("unnamed set");
    assert_eq!(reg.text, "keepme\n", "empty ci( kept the previous yank");
}

/// `p` with an unset register rings the bell (vim E353) and — for the VISUAL
/// variant — leaves the selection intact instead of deleting it.
#[test]
fn put_with_empty_register_bells_and_keeps_selection() {
    let mut f = Fixture::new("abc\n");
    f.feed(["p"]);
    assert!(f.host.bells > 0, "normal p bells");
    assert_eq!(f.text(), "abc\n");

    let mut f = Fixture::new("abc\n");
    f.feed(["v", "l", "p"]);
    assert!(f.host.bells > 0, "visual p bells");
    assert_eq!(f.text(), "abc\n", "visual p keeps the selection");
}

// ---- Ex commands ----------------------------------------------------------------

/// `:{range}d [count]` extends the range downward, like `:y`'s count.
#[test]
fn ex_delete_supports_count_argument() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\n");
    f.feed([":", "2", "d", " ", "2", "<CR>"]);
    assert_eq!(f.text(), "one\nfour\n", ":2d 2 deletes lines 2-3");
}

// ---- C-a/C-x radix handling (vim nrformats=bin,octal,hex) ---------------------

/// All expected values are vim 9.1 probe outputs: `007`+2 → `011` (octal),
/// `0099`+2 → `101` (8/9 force decimal), `0x1f`+2 → `0x21`, `0XAB`+1 →
/// `0XAC`, `0b101`+2 → `0b111`, `077`+3 → `0102`. The old engine treated
/// everything as decimal and REWROTE `0x1f` into `2x1f` (the digit run `1`
/// incremented, the prefix mangled).
#[test]
fn increment_decrement_radix_formats() {
    for (line, keys, want) in [
        ("x007", "2<C-a>", "x011"),     // octal
        ("x0099", "2<C-a>", "x101"),    // 9 forces decimal, zeros dropped
        ("v0x1f", "2<C-a>", "v0x21"),   // hex, cursor before the prefix
        ("0XAB", "<C-a>", "0XAC"),      // uppercase preserved
        ("v0b101", "2<C-a>", "v0b111"), // binary
        ("n 077", "3<C-a>", "n 0102"),  // octal carry
        ("-5", "<C-a>", "-4"),
        ("-5", "<C-x>", "-6"),
        ("x-5y", "<C-x>", "x-6y"), // cursor on the minus
        ("0x10", "<C-x>", "0x0f"), // hex underflow zero-pads to width
        ("010", "<C-x>", "007"),   // octal keeps the marker and width
        ("a1b2", "<C-a>", "a2b2"), // first number after the cursor
    ] {
        let mut f = Fixture::new(line);
        for key in vimcore::key::parse_key_sequence(keys) {
            f.feed_raw(key);
        }
        assert_eq!(f.text(), want, "{keys} on {line:?}");
    }
}

/// The cursor ends on the last digit of the rewritten number.
#[test]
fn increment_cursor_lands_on_last_digit() {
    let mut f = Fixture::at("ab99cd", 0, 2); // on the first '9'
    f.feed_raw(vimcore::key::Key::ctrl_char('a'));
    assert_eq!(f.text(), "ab100cd");
    assert_eq!(f.vim.cursor_offset(), 4, "on the last digit of 100");
}

/// A single-word line: `w` leaves the cursor ON the last character (never on
/// the phantom end), so a following `x` deletes it — vim probe: `wx` on
/// "abc" leaves "ab".
#[test]
fn word_motion_lands_on_last_char_at_buffer_end() {
    let f = edit("abc", 0, 0, &["w"]);
    assert_eq!(f.vim.cursor_offset(), 2, "w parks on 'c'");
    let f = edit("abc", 0, 0, &["w", "x"]);
    assert_eq!(f.text(), "ab", "x deletes the char under the cursor");
}

// ---- gn / gN: select the next match -------------------------------------------

/// `gn` selects the match containing the cursor (else the next one) as a
/// charwise visual selection; `dgn` changes exactly the match (vim probes:
/// `dgn` on "abfoo cd foocd" with pattern `foo` leaves "ab cd foocd";
/// `cgn` + `.` rewrites successive matches).
#[test]
fn gn_selects_the_match_for_operators_and_visual() {
    // dgn deletes exactly the match
    let mut f = Fixture::new("abfoo cd foocd\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed(["d", "g", "n"]);
    assert_eq!(f.text(), "ab cd foocd\n", "dgn removes just the match");

    // count: the match containing the cursor is #1, so 2gn/d2gn takes the
    // NEXT one (vim's 2gn ≙ 2n then select)
    let mut f = Fixture::new("a foo b foo c\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed(["d", "2", "g", "n"]);
    assert_eq!(f.text(), "a foo b  c\n", "2gn targets the second match");

    // plain gn enters visual mode over the match. `/` from col 0 skips the
    // match AT the cursor (vim same), landing on the second foo; gn then
    // selects the one under the cursor.
    let mut f = Fixture::new("foo bar foo\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed(["g", "n"]);
    assert_eq!(
        f.vim.mode(),
        vimcore::Mode::Visual {
            kind: vimcore::mode::VisualKind::Char
        }
    );
    f.feed(["d"]);
    assert_eq!(f.text(), "foo bar \n", "the selection was the match");

    // cgn + `.`: the classic change-every-match workflow
    let mut f = Fixture::new("a b a b a b\n");
    f.feed(["/", "b", "<CR>"]);
    f.feed(["c", "g", "n"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a X a b a b\n");
    f.feed(["."]);
    assert_eq!(f.text(), "a X a X a b\n");
    f.feed(["."]);
    assert_eq!(f.text(), "a X a X a X\n");
}

/// `gN` selects backward: with the cursor at the end of "aa b aa b" the
/// selection covers the SECOND `aa`.
#[test]
fn gn_backward_selects_the_previous_match() {
    let mut f = Fixture::new("aa b aa b\n");
    f.feed(["/", "a", "a", "<CR>"]);
    f.feed(["G", "$", "g", "N", "d"]);
    assert_eq!(f.text(), "aa b  b\n", "gN picked the trailing aa");
}

/// No pattern at all: vim opens a search prompt; the engine reports E35
/// (same as `n`) — recorded as a divergence.
#[test]
fn gn_without_pattern_reports_e35() {
    let mut f = Fixture::new("abc\n");
    f.feed(["g", "n"]);
    assert_eq!(
        f.vim.mode(),
        vimcore::Mode::Normal,
        "no selection without a pattern"
    );
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E35")),
        "E35 reported, got {:?}",
        f.host.statuses
    );
}
