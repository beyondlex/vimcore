//! audit2_a: independent audit (2026-10-07) of normal-mode motions / counts / text objects.
//!
//! Each test corresponds to a newly discovered finding and asserts the **expected
//! result of vim 9.1 (empirically verified via `-Nu NONE -N -s` typeahead)**, so it
//! should fail on the current working tree. Each oracle probe was re-checked with
//! `printf … > /tmp/*.txt; vim -Nu NONE -N -i NONE -s …; cat output`; the evidence is
//! written in each test's comments (1-based col already converted back to 0-based
//! byte offsets).

mod common;

use common::edit;
use vimcore::buffer::VimBuffer;

// ---------------------------------------------------------------- 1. `|` and tab

/// **Finding 1 (P1)**: `{count}|` treats TAB as 1 display cell when counting
/// columns; vim expands it by 'ts'=8.
/// oracle: on `ab<Tab>c`, `5|` → col=3 (stops on the TAB, vcol 5 falls inside the
/// tab's cells 3..10); the engine lands on offset 3 ('c', column 4).
#[test]
fn pipe_column_ignores_tabstop() {
    // oracle sampling used ts=8; the engine defaults to ts=4 (deliberate NOTES
    // divergence), so pin it first
    let f = edit("ab\tc", 0, 0, &[":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>", "5", "|"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: 5| stops on the TAB (col 3, 1-based)");
}

// ---------------------------------------------------------------- 2. `d2aw` on an empty line

/// **Finding 2 (P1)**: the repeated scan for a text-object count breaks its chain at
/// an empty line (the probe falls back to the last word of the current line and
/// breaks once the range makes no progress). vim treats the empty line itself as a
/// word, so 2aw passes through it.
/// oracle: cursor on foo, `foo␊␊bar␊` with `d2aw` → buffer completely empty (n=1,
/// l1 empty); the engine leaves only `\n\nbar\n`.
#[test]
fn d2aw_count_stops_at_empty_line() {
    let f = edit("foo\n\nbar\n", 0, 0, &["d", "2", "a", "w"]);
    assert_eq!(
        f.buf.line_count(),
        1,
        "vim 9.1: d2aw crosses the empty line and deletes foo and bar, leaving one empty line"
    );
    assert_eq!(f.buf.line_content(0), "");
}

// ------------------------------------- 3. `daw` at trailing whitespace + empty line + next word

/// **Finding 3 (P1)**: when the cursor sits on trailing whitespace, `aw`'s
/// blank-run+next-word search skips over the empty line and swallows the word below;
/// vim stops at the empty line (itself a word) and deletes only `   \n`.
/// oracle: cursor on the last space, `a   ␊␊b␊` with `daw` → `a␊b␊` (n=2);
/// the engine yields `a` (b is swallowed too, losing the line structure).
#[test]
fn daw_after_trailing_ws_eats_next_word_across_empty_line() {
    let f = edit("a   \n\nb\n", 0, 3, &["d", "a", "w"]);
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: delete only '   \\n', keep the empty line and b");
}

// ------------------------------------ 4. `daw` on a whitespace-only line + empty line + next word

/// **Finding 4 (P1)**: when the cursor is on a whitespace-only line (including a
/// tab-only line), it likewise swallows the word after the empty line.
/// oracle: cursor on the whitespace line, `foo␊   ␊␊bar␊` with `daw` → `foo␊bar␊`
/// (n=2, deleting `   \n\n`); the tab version `foo␊\t␊␊bar␊` → `foo␊bar␊`.
/// The engine yields `foo␊` (bar swallowed).
#[test]
fn daw_on_ws_only_line_eats_next_word_across_empty_line() {
    let f = edit("foo\n   \n\nbar\n", 1, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "foo\nbar\n", "vim 9.1: delete '   \\n\\n', keep bar");
}

// --------------------------------------- 5. `daw` on whitespace with no next word (toward EOF)

/// **Finding 5a (P1)**: when the cursor is on trailing whitespace with no word after
/// it (straight to EOF / end of the empty line), vim's `daw` is a **no-op** (it cannot
/// find the "next word", so the object fails); the engine's blank_run_plus_next_word
/// keeps scanning treating `\n` as ordinary blank and deletes the blanks together with
/// the newline.
/// oracle: cursor on the space, `a   ␊` with `daw` → buffer unchanged (n=1,
/// l1="a   "); the engine yields `a` (even the newline is gone).
#[test]
fn daw_trailing_ws_without_next_word_is_noop() {
    let f = edit("a   \n", 0, 3, &["d", "a", "w"]);
    assert_eq!(f.text(), "a   \n", "vim 9.1: daw does nothing when there is no next word");
}

/// **Finding 5b (P1)**: the same applies to a whitespace-only **last line** (no
/// trailing newline). oracle: cursor on the whitespace line, `foo␊   ` with `daw` →
/// unchanged (n=2, l1=foo l2="   "); the engine yields `foo␊` (the three spaces deleted).
#[test]
fn daw_ws_only_last_line_without_next_word_is_noop() {
    let f = edit("foo\n   ", 1, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "foo\n   ", "vim 9.1: daw does nothing when there is no next word");
}

// ---------------------------------------------------------------- 6. `cw` and U+3000

/// **Finding 6 (P1)**: `cw`'s "excludes trailing whitespace" special case only
/// strips `' '`/`'\t'`; vim's cw is equivalent to ce — **no** trailing whitespace is
/// touched. The engine treats the ideographic space U+3000 as post-word blank and
/// deletes it.
/// oracle: `a　b␊` (U+3000) with `cwX<Esc>` → `X　b`; the engine yields `Xb`.
#[test]
fn cw_keeps_wide_space_vim_ce_semantics() {
    let mut f = edit("a\u{3000}b\n", 0, 0, &["c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X\u{3000}b\n", "vim 9.1: cw=ce, keep U+3000");
}

// ---------------------------------------------------------------- 7. tag case

/// **Finding 7a (P1)**: `it`/`at` match open/close tag names case-sensitively
/// (`rposition` compares by name equality); vim 9.1 is case-insensitive for HTML tags.
/// oracle: cursor on x, `<P>x</p>␊` with `dit` → `<P></p>`; the engine leaves it
/// unchanged.
#[test]
fn tag_object_matches_open_tag_case_insensitively() {
    let f = edit("<P>x</p>\n", 0, 3, &["d", "i", "t"]);
    assert_eq!(f.text(), "<P></p>\n", "vim 9.1: P/p count as the same name, x deleted");
}

/// **Finding 7b (P1)**: the reverse direction likewise (lowercase open, uppercase
/// close). oracle: `<p>x</P>␊` with `dit` → `<p></P>`; the engine leaves it unchanged.
#[test]
fn tag_object_matches_close_tag_case_insensitively() {
    let f = edit("<p>x</P>\n", 0, 3, &["d", "i", "t"]);
    assert_eq!(f.text(), "<p></P>\n", "vim 9.1: p/P count as the same name, x deleted");
}

// ---------------------------------------------------------------- 8. section motions

/// **Finding 8 (P2)**: `]]`/`[[` (and the `][`/`[]`/`]m`/`[m` family) are entirely
/// unbound — no rows in tables.rs; pressing them rings the bell and stays in place.
/// vim 9.1 has them built in: `]]` goes to the next `{`/`}` in column 1, `[[` to the
/// previous `{` in column 1 (a `}` in column 1 does not block `[[`).
/// oracle: on `foo␊{␊bar␊}␊baz␊`, `]]` → line 2 (the `{` line); `G[[` → line 2.
#[test]
fn section_motions_brace_column_unbound() {
    let f = edit("foo\n{\nbar\n}\nbaz\n", 0, 0, &["]", "]"]);
    assert_eq!(f.line(), 1, "vim 9.1: ]] lands on the line with an opening brace in column 1");
    let f = edit("foo\n{\nbar\n}\nbaz\n", 4, 0, &["[", "["]);
    assert_eq!(f.line(), 1, "vim 9.1: [[ lands on the line with an opening brace in column 1");
}

// ------------------------------------------- 9. `d2aw` count chain breaks at the line break

/// **Finding 9a (P1)**: when the first object is the last word on the line, the
/// repeated scan falls back from span.end (the newline position) to the last word of
/// the current line and breaks once the range makes no progress — the count silently
/// fails. vim's second aw crosses the newline and takes the word on the next line.
/// oracle: cursor on the space before cd, `ab cd␊ef gh␊` with `d2aw` → `abgh` (n=1,
/// ` cd\nef ` deleted); the engine yields `ab␊ef gh␊` (only ` cd` deleted).
#[test]
fn d2aw_count_stops_at_line_break() {
    let f = edit("ab cd\nef gh\n", 0, 2, &["d", "2", "a", "w"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: the two lines merge into one line abgh");
    assert_eq!(f.buf.line_content(0), "abgh");
}

/// **Finding 9b (P1)**: punctuation-word variant. oracle: cursor on `!`, `ab!␊cd␊`
/// with `2daw` → `ab` (n=1, `!\ncd` deleted); the engine yields `ab␊cd␊` (only `!`
/// deleted).
#[test]
fn d2aw_punct_line_final_stops_at_line_break() {
    let f = edit("ab!\ncd\n", 0, 2, &["2", "d", "a", "w"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: only one line ab left");
    assert_eq!(f.buf.line_content(0), "ab");
}

// ---------------------------------------------- 10. `2cw` across an empty line

/// **Finding 10 (P1)**: when `2cw`'s landing position is line 2 column 1 (after the
/// empty line), the span is clamped back to the end of the current line by the next
/// line's "column-1 landing" rule, so after cw strips trailing whitespace only the
/// cursor word remains; vim's 2cw changes all the way to the **end** of the second
/// word (deleting the whole stretch `b \n\ncd`).
/// oracle: cursor on b, `ab ␊␊cd␊` with `2cwX<Esc>` → `aX` (n=1); the engine yields
/// `aX ␊␊cd␊`.
#[test]
fn c2w_across_empty_line_reaches_last_word() {
    let mut f = edit("ab \n\ncd\n", 0, 1, &["2", "c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: the change swallows the empty line and cd");
    assert_eq!(f.buf.line_content(0), "aX");
}

// --------------------------------------------------- 11. `j` and column preservation with tab

/// **Finding 11 (P1)**: column preservation for `j`/`k` (and `<C-d>`/`<C-u>`/`<C-f>`/
/// `<C-b>`) uses unicode-width, counting TAB as 1 cell; vim preserves the **virtual
/// column** (expanded with 'ts'=8).
/// oracle: on `a<Tab>b␊0123456789x␊`, `w` (col 3, virtcol 9) then `j` → line 2 col 9
/// ('8'); the engine lands on '2' (col 3).
#[test]
fn j_preserves_virtcol_with_tabstop() {
    // Same as above: pin the oracle's ts=8 premise first
    let f = edit("a\tb\n0123456789x\n", 0, 2, &[
        ":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>", "j",
    ]);
    assert_eq!(f.cursor(), 12, "vim 9.1: j keeps virtcol 9 → column 9, the '8'");
}
