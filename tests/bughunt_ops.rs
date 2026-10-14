//! Bug-hunt probes for the operator/motion/text-object/search/register domain
//! (`src/ops.rs`, `src/motions.rs`, `src/objects.rs`, `src/search.rs`,
//! `src/registers.rs`, `src/word.rs`).
//!
//! Every expectation below was sampled from `/usr/bin/vim` 9.1.1752 through a
//! raw typeahead script (`-Nu NONE -N -i NONE -n -s keys.in`, cursor placed
//! with `:call cursor(l,c)` so indented lines land at the TRUE column, buffer
//! written with `:w!`, state dumped with `:redir` + `:registers`/`getpos`).
//! Oracle artifacts live in /tmp/vimprobe (case ids quoted per test).
//! Cases that fail here are REPRODUCED DIVERGENCES — do not "fix" the
//! assertion without re-running the oracle.

mod common;

use common::Fixture;
use vimcore::registers::RegisterKind;

fn unnamed(f: &Fixture) -> (String, RegisterKind) {
    let r = f
        .vim
        .registers
        .get('"')
        .expect("unnamed register should exist");
    (r.text.clone(), r.kind)
}

/// Assert buffer + cursor, with the oracle bytes described in the doc comment.
fn expect_buf(f: &Fixture, text: &str, cursor: usize) {
    assert_eq!(f.text(), text, "buffer");
    assert_eq!(f.cursor(), cursor, "cursor offset");
}

// ---------------------------------------------------------------------------
// w / W spans and the exclusive operator region (oracle cases c*, d*, m*, n*, r*)
// ---------------------------------------------------------------------------

/// `w` from a leading-blank position skips the whole blank run and stops on the
/// first non-blank. Source: vim, case c1 ("  hello world", cursor 0,0 -> lnum 1
/// col 3, i.e. offset 2).
#[test]
fn bare_w_from_indent_lands_on_first_nonblank() {
    let mut f = Fixture::at("  hello world\n", 0, 0);
    f.feed(["w"]);
    assert_eq!(f.cursor(), 2);
}

/// `dw` starting on the blank run of an indented line deletes ONLY the blanks
/// (the motion lands at the word start). Source: vim, case c2
/// ("  hello world", 0,0 `dw` -> "hello world\n", reg charwise "  ", pos 1,1).
#[test]
fn dw_from_indent_deletes_the_blank_run() {
    let mut f = Fixture::at("  hello world\n", 0, 0);
    f.feed(["d", "w"]);
    expect_buf(&f, "hello world\n", 0);
    assert_eq!(unnamed(&f), ("  ".to_owned(), RegisterKind::Charwise));
}

/// `dw` on a one-word line from col 0: the motion lands on the NUL past EOL
/// (operator-pending `w` stops at "last char + 1"), `adjust_cursor()` makes the
/// region inclusive at the last char — the NEWLINE SURVIVES as an empty line.
/// Source: vim, case d1 ("aaa\nbbb\n", 0,0 `dw` -> "\nbbb\n", reg c "aaa").
#[test]
fn dw_on_last_word_does_not_eat_the_newline() {
    let mut f = Fixture::at("aaa\nbbb\n", 0, 0);
    f.feed(["d", "w"]);
    expect_buf(&f, "\nbbb\n", 0);
    assert_eq!(unnamed(&f), ("aaa".to_owned(), RegisterKind::Charwise));
}

/// `dw` from the very last char of a line deletes that char only (same NUL
/// landing clamped to inclusive). Source: vim, case d2 ("aaa\nbbb\n", 0,2 `dw`
/// -> "aa\nbbb\n", reg c "a", cursor offset 1).
#[test]
fn dw_from_last_char_of_line_deletes_one_char() {
    let mut f = Fixture::at("aaa\nbbb\n", 0, 2);
    f.feed(["d", "w"]);
    expect_buf(&f, "aa\nbbb\n", 1);
    assert_eq!(unnamed(&f), ("a".to_owned(), RegisterKind::Charwise));
}

/// `d2w` from col 0 of an indented line whose landing is col 1 of the next
/// line: the col-0 exclusive landing is backed up to the previous EOL
/// (NL excluded) and the residue is NOT whitespace, so it stays charwise.
/// Source: vim, case c3 ("  ind1\n  ind2\n", 0,0 `d2w` -> "\n  ind2\n",
/// reg c "  ind1").
#[test]
fn d2w_indent_landing_col0_stays_charwise() {
    let mut f = Fixture::at("  ind1\n  ind2\n", 0, 0);
    f.feed(["d", "2", "w"]);
    assert_eq!(unnamed(&f), ("  ind1".to_owned(), RegisterKind::Charwise));
    expect_buf(&f, "\n  ind2\n", 0);
}

/// `d2w` on one-word-per-word lines runs to EOF inside a single fwd_word
/// iteration and vim promotes the spanning delete to LINWISE (cpo-z rule).
/// Source: vim, case d3 ("aaa\nbbb\n", 0,0 `d2w` -> empty buffer,
/// reg l "aaa\nbbb\n").
#[test]
fn d2w_over_two_words_deletes_linewise() {
    let mut f = Fixture::at("aaa\nbbb\n", 0, 0);
    f.feed(["d", "2", "w"]);
    assert!(f.text().is_empty() || f.text() == "\n", "buffer {:?}", f.text());
    assert_eq!(
        unnamed(&f),
        ("aaa\nbbb\n".to_owned(), RegisterKind::Linewise)
    );
}

/// Same shape with a third surviving line. Source: vim, case n3
/// ("aaa\nbbb\nccc\n", 0,0 `d2w` -> "ccc\n", reg l "aaa\nbbb\n").
#[test]
fn d2w_three_lines_keeps_the_tail() {
    let mut f = Fixture::at("aaa\nbbb\nccc\n", 0, 0);
    f.feed(["d", "2", "w"]);
    expect_buf(&f, "ccc\n", 0);
    assert_eq!(
        unnamed(&f),
        ("aaa\nbbb\n".to_owned(), RegisterKind::Linewise)
    );
}

/// `d2w` from 0,0 on "xx\naa\nbbb\n": 2w lands past EOL of line 2, delete
/// becomes LINWISE covering lines 1-2. Source: vim, case n5 (out "bbb\n",
/// reg l "xx\naa\n").
#[test]
fn d2w_two_short_words_deletes_two_lines() {
    let mut f = Fixture::at("xx\naa\nbbb\n", 0, 0);
    f.feed(["d", "2", "w"]);
    expect_buf(&f, "bbb\n", 0);
    assert_eq!(
        unnamed(&f),
        ("xx\naa\n".to_owned(), RegisterKind::Linewise)
    );
}

/// `d3w` covers all three lines linewise -> empty buffer. Source: vim, case d5
/// ("aaa\nbbb\nccc\n", 0,0 `d3w` -> empty, reg l "aaa\nbbb\nccc\n").
#[test]
fn d3w_deletes_the_whole_buffer_linewise() {
    let mut f = Fixture::at("aaa\nbbb\nccc\n", 0, 0);
    f.feed(["d", "3", "w"]);
    assert!(f.text().is_empty() || f.text() == "\n", "buffer {:?}", f.text());
    assert_eq!(
        unnamed(&f),
        ("aaa\nbbb\nccc\n".to_owned(), RegisterKind::Linewise)
    );
}

/// `daw` from col 0 of an indented line takes the blanks + word but NOT the
/// newline (charwise). Source: vim, cases c4/c5 ("  ind1\n  ind2\n", 0,0
/// `daw` -> "\n  ind2\n" reg c "  ind1"; `yaw` leaves the buffer unchanged
/// with reg c "  ind1").
#[test]
fn aw_from_indent_covers_blanks_plus_word_without_newline() {
    let mut f = Fixture::at("  ind1\n  ind2\n", 0, 0);
    f.feed(["y", "a", "w"]);
    expect_buf(&f, "  ind1\n  ind2\n", 0);
    assert_eq!(unnamed(&f), ("  ind1".to_owned(), RegisterKind::Charwise));

    let mut g = Fixture::at("  ind1\n  ind2\n", 0, 0);
    g.feed(["d", "a", "w"]);
    expect_buf(&g, "\n  ind2\n", 0);
    assert_eq!(unnamed(&g), ("  ind1".to_owned(), RegisterKind::Charwise));
}

/// `daw` on the sole word of a one-line buffer deletes blanks+word charwise,
/// leaving an empty line. Source: vim, case c6 ("  aaa\n", 0,0 -> "\n",
/// reg c "  aaa").
#[test]
fn daw_on_sole_indented_word_leaves_empty_line() {
    let mut f = Fixture::at("  aaa\n", 0, 0);
    f.feed(["d", "a", "w"]);
    expect_buf(&f, "\n", 0);
    assert_eq!(unnamed(&f), ("  aaa".to_owned(), RegisterKind::Charwise));
}

/// `yaw` at the trailing blanks of a line captures blanks + NL + next word,
/// WITHOUT the word's trailing newline. Source: vim, case v17
/// ("foo bar  \nbaz\n", 0,7 `yaw` -> reg c "  \nbaz").
#[test]
fn yaw_from_trailing_blanks_stops_at_word_end() {
    let mut f = Fixture::at("foo bar  \nbaz\n", 0, 7);
    f.feed(["y", "a", "w"]);
    assert_eq!(unnamed(&f), ("  \nbaz".to_owned(), RegisterKind::Charwise));
}

/// Same, mid-line: `yaw` from inside the gap captures gap + NL + next word
/// only. Source: vim, cases y5/w3 ("a   \nb\n", 0,1 `yaw` -> reg c "   \nb").
#[test]
fn yaw_from_gap_captures_gap_nl_word() {
    let mut f = Fixture::at("a   \nb\n", 0, 1);
    f.feed(["y", "a", "w"]);
    assert_eq!(
        unnamed(&f),
        ("   \nb".to_owned(), RegisterKind::Charwise)
    );
}

/// `daw` of the same gap span deletes gap+NL+word charwise, leaving "a\n".
/// Source: vim, case y5 ("a   \nb\n", 0,1 `daw` -> "a\n", cursor 0).
#[test]
fn daw_from_gap_leaves_the_first_word_and_nl() {
    let mut g = Fixture::at("a   \nb\n", 0, 1);
    g.feed(["d", "a", "w"]);
    expect_buf(&g, "a\n", 0);
    assert_eq!(
        unnamed(&g),
        ("   \nb".to_owned(), RegisterKind::Charwise)
    );
}

/// `yaw` on an EMPTY line: charwise NL + next word (no trailing NL).
/// Source: vim, case c8 ("foo\n\nbar\n", line2 col0 `yaw` -> reg c "\nbar").
#[test]
fn yaw_on_empty_line_is_charwise_without_trailing_nl() {
    let mut f = Fixture::at("foo\n\nbar\n", 1, 0);
    f.feed(["y", "a", "w"]);
    assert_eq!(unnamed(&f), ("\nbar".to_owned(), RegisterKind::Charwise));
}

/// `daw` on an EMPTY line deletes empty line + following line LINWISE.
/// Source: vim, case c9 ("foo\n\nbar\n", line2 `daw` -> "foo\n", reg l
/// "\nbar\n", cursor offset 0).
#[test]
fn daw_on_empty_line_is_linewise_pair() {
    let mut f = Fixture::at("foo\n\nbar\n", 1, 0);
    f.feed(["d", "a", "w"]);
    assert_eq!(
        unnamed(&f),
        ("\nbar\n".to_owned(), RegisterKind::Linewise)
    );
    assert_eq!(f.text(), "foo\n");
    assert_eq!(f.cursor(), 0);
}

/// `d2w` from an empty line: deletes "\nbar\n" (the two lines, linewise
/// residue). Source: vim, case b1 ("foo\n\nbar\n", line2 col0 `d2w` ->
/// "foo\n", reg l "\nbar\n").
#[test]
fn d2w_from_empty_line_deletes_next_line() {
    let mut f = Fixture::at("foo\n\nbar\n", 1, 0);
    f.feed(["d", "2", "w"]);
    expect_buf(&f, "foo\n", 0);
    assert_eq!(
        unnamed(&f),
        ("\nbar\n".to_owned(), RegisterKind::Linewise)
    );
}

/// `diw` on an empty line fails quietly (buffer untouched).
/// Source: vim, case b3 ("foo\n\nbar\n", line2 `diw` -> unchanged).
#[test]
fn diw_on_empty_line_is_a_noop() {
    let mut f = Fixture::at("foo\n\nbar\n", 1, 0);
    f.feed(["d", "i", "w"]);
    expect_buf(&f, "foo\n\nbar\n", 4);
}

/// `d2aw` from a word crossing to the next line consumes the word, the NL,
/// and the next word's blanks+start. Source: vim, cases r3/r4
/// ("aa bb\ncc dd\n", 0,3 `d2aw` -> "aa dd\n", reg c "bb\ncc ";
///  "ab cd\nef gh\n", 0,3 `d2aw` -> "ab gh\n", reg c "cd\nef ").
#[test]
fn d2aw_crosses_the_line_boundary_once() {
    let mut f = Fixture::at("aa bb\ncc dd\n", 0, 3);
    f.feed(["d", "2", "a", "w"]);
    expect_buf(&f, "aa dd\n", 3);
    assert_eq!(
        unnamed(&f),
        ("bb\ncc ".to_owned(), RegisterKind::Charwise)
    );

    let mut g = Fixture::at("ab cd\nef gh\n", 0, 3);
    g.feed(["d", "2", "a", "w"]);
    expect_buf(&g, "ab gh\n", 3);
    assert_eq!(
        unnamed(&g),
        ("cd\nef ".to_owned(), RegisterKind::Charwise)
    );
}

/// `d2w` mid-line whose second w reaches the final line's EOL: deletes
/// "two\nthree" charwise leaving "one \n". Source: vim, case r1
/// ("one two\nthree\n", 0,4 `d2w` -> "one \n", reg c "two\nthree", cursor 3).
#[test]
fn d2w_to_buffer_end_is_charwise_two_newline_three() {
    let mut f = Fixture::at("one two\nthree\n", 0, 4);
    f.feed(["d", "2", "w"]);
    expect_buf(&f, "one \n", 3);
    assert_eq!(
        unnamed(&f),
        ("two\nthree".to_owned(), RegisterKind::Charwise)
    );
}

// ---------------------------------------------------------------------------
// sentences (objects.rs)
// ---------------------------------------------------------------------------

/// `dis` with the cursor on the single-space gap between two sentences
/// deletes ONLY the gap. Source: vim, case t43 ("Aa. Bb.\n", 0,3 `dis` ->
/// "Aa.Bb.\n", cursor offset 3).
#[test]
fn dis_on_gap_deletes_just_the_gap() {
    let mut f = Fixture::at("Aa. Bb.\n", 0, 3);
    f.feed(["d", "i", "s"]);
    expect_buf(&f, "Aa.Bb.\n", 3);
}

/// `das` with the cursor on the gap takes gap + following sentence.
/// Source: vim, case t42 ("Aa. Bb.\n", 0,3 `das` -> "Aa.\n", cursor offset 2).
#[test]
fn das_on_gap_takes_gap_and_next_sentence() {
    let mut f = Fixture::at("Aa. Bb.\n", 0, 3);
    f.feed(["d", "a", "s"]);
    expect_buf(&f, "Aa.\n", 2);
}

// ---------------------------------------------------------------------------
// blocks / paragraphs (objects.rs)
// ---------------------------------------------------------------------------

/// `2di)` when the cursor is OUTSIDE both blocks and the count runs past the
/// last block: vim beeps and changes NOTHING (it does not "skip to the second
/// sibling" the way a naive forward scan would).
/// Source: vim, case u44 ("foo (bar) (baz)", 0,0 `2di)` -> unchanged).
#[test]
fn di_block_count_beyond_outer_blocks_is_a_noop() {
    let mut f = Fixture::at("foo (bar) (baz)\n", 0, 0);
    f.feed(["2", "d", "i", ")"]);
    expect_buf(&f, "foo (bar) (baz)\n", 0);
}

/// `da{` from inside the block removes the whole 3-line block (empty buffer).
/// Source: vim, cases u34/b4 ("{\na\n}\n", line2 `da{` -> empty file).
#[test]
fn da_brace_from_inside_empties_the_buffer() {
    let mut f = Fixture::at("{\na\n}\n", 1, 0);
    f.feed(["d", "a", "{"]);
    assert!(f.text().is_empty() || f.text() == "\n", "buffer {:?}", f.text());
}

/// `dip` where the entire buffer is one paragraph empties it.
/// Source: vim, cases u35 ("hello\n" `dip` -> empty) and b12
/// ("a\nb\nc\n", line2 `dip` -> empty, reg l "a\nb\nc\n").
#[test]
fn dip_on_sole_paragraph_empties_the_buffer() {
    let mut f = Fixture::at("hello\n", 0, 0);
    f.feed(["d", "i", "p"]);
    assert!(f.text().is_empty() || f.text() == "\n", "buffer {:?}", f.text());

    let mut g = Fixture::at("a\nb\nc\n", 1, 0);
    g.feed(["d", "i", "p"]);
    assert!(g.text().is_empty() || g.text() == "\n", "buffer {:?}", g.text());
    assert_eq!(
        unnamed(&g),
        ("a\nb\nc\n".to_owned(), RegisterKind::Linewise)
    );
}

// ---------------------------------------------------------------------------
// yank cursor landing + registers (ops.rs / registers.rs)
// ---------------------------------------------------------------------------

/// `y0` MOVES THE CURSOR to the motion landing (col 0) — then `p` pastes
/// after offset 0. Source: vim, case u60/v60 ("hello world\n", 0,6 `y0p` ->
/// "hhello ello world\n", reg c "hello ").
#[test]
fn yank_moves_cursor_to_the_motion_landing() {
    let mut f = Fixture::at("hello world\n", 0, 6);
    f.feed(["y", "0"]);
    assert_eq!(f.cursor(), 0, "y0 cursor");
    assert_eq!(unnamed(&f), ("hello ".to_owned(), RegisterKind::Charwise));
    f.feed(["p"]);
    assert_eq!(f.text(), "hhello ello world\n");
}

/// `"_dd` deletes to the black hole: text goes, unnamed is NOT touched
/// (nothing was in it yet, so `p` must paste nothing).
/// Source: vim, case t52 ("a\nb\n", `"_dd p` -> "b\n").
#[test]
fn blackhole_delete_does_not_fill_unnamed() {
    let mut f = Fixture::at("a\nb\n", 0, 0);
    f.feed(["\"", "_", "d", "d", "p"]);
    expect_buf(&f, "b\n", 0);
}

/// `"Byy` then `"bp` pastes register b below. Source: vim, case t54
/// ("xxx\n", `"B yy` `"b p` -> "xxx\nxxx\n").
#[test]
fn named_yank_then_paste_duplicates_the_line() {
    let mut f = Fixture::at("xxx\n", 0, 0);
    f.feed(["\"", "B", "y", "y", "\"", "b", "p"]);
    assert_eq!(f.text(), "xxx\nxxx\n");
    let r = f
        .vim
        .registers
        .get('b')
        .expect("register b should exist");
    assert_eq!(r.text, "xxx\n".to_owned());
    assert_eq!(r.kind, RegisterKind::Linewise);
}

/// Blockwise `I` on a single real line with the block extended past EOF:
/// insert applies to the one line only. Source: vim, case u63 ("x\n",
/// `<C-v>jIY<Esc>` -> "Yx\n").
#[test]
fn block_insert_past_buffer_end_only_touches_the_last_real_line() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("Y");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Yx\n");
}

// ---------------------------------------------------------------------------
// panic smoke: pathological key feeds on edge positions must not unwind
// ---------------------------------------------------------------------------

#[test]
fn edge_feeds_do_not_panic() {
    // (buffer, line, col, keys) torture list — surviving without a panic is
    // the whole assertion.
    let cases: Vec<(&str, usize, usize, Vec<&str>)> = vec![
        ("", 0, 0, vec!["d", "2", "w", "d", "a", "w", "y", "i", "w", "d", "i", "s", "2", "d", "i", ")"]),
        ("\n", 0, 0, vec!["d", "a", "w", "y", "a", "w", "d", "2", "w", "d", "i", "p"]),
        ("a", 0, 0, vec!["d", "2", "w", "d", "3", "w", "d", "a", "w", "y", "a", "w", "d", "0", "d", "$"]),
        (" ", 0, 0, vec!["d", "a", "w", "d", "i", "w", "d", "2", "w", "c", "a", "w"]),
        ("x\ny\nz", 2, 1, vec!["d", "2", "w", "d", "a", "w", "y", "2", "w", "d", "i", "}"]),
        ("(((", 0, 0, vec!["2", "d", "i", "(", "d", "a", ")", "d", "i", "%"]),
        ("a.b\n \nc.d", 1, 0, vec!["d", "i", "s", "d", "a", "s", "d", "2", "a", "w"]),
        ("foo\n\n", 1, 0, vec!["d", "2", "a", "w", "y", "2", "e", "d", "2", "e"]),
    ];
    for (buf, line, col, keys) in cases {
        let mut f = Fixture::at(buf, line, col);
        f.feed(keys);
        let _ = f.text();
        let _ = f.cursor();
    }
}
