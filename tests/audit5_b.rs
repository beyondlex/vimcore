//! audit5_b: independent audit of the Normal motion / operator / text object / join / indent-shift
//! domains (round-4 independent audit, 2026-10-09).
//!
//! Each `#[test]` is a **bug confirmed by vim 9.1 (local `/usr/bin/vim` = 9.1 patches 1-1752)**;
//! the assertions hold the oracle's expected values, so they should fail on the current working
//! tree (turning into regression pins once fixed).
//!
//! oracle channels:
//! * typeahead: python3 writes buffer/keys → `vim -Nu NONE -N -i NONE -n -s keys buf </dev/null`,
//!   sampled with `:call writefile(...)` (unique file name per case).
//! * bell-class: a real PTY driven by expect (`spawn vim -Nu NONE -N -n buf` + `send`), counting
//!   BEL bytes in the output — the typeahead channel cannot hear bells.
//! * behavioral-assertion sources: vim `src/testdir/` (test_normal.vim / test_textobjects.vim /
//!   test_join.vim / test_shift.vim / test_charsearch*) + local typeahead/PTY re-checks.
//!
//! Deduplicated against: NOTES.md known divergences #1-77+, the 50 round-31 audit items, the 59
//! round-32 audit items (A series TAB virtual columns, B series aw whitespace matrix/cw≡ce/object
//! count across lines), the 55 round-33 audit items (B1-B14 just fixed this domain).
//!
//! ---------------------------------------------------------------
//! Falsified this round (oracle proved engine agrees with vim 9.1; skip next round):
//! * On a single-line buffer `}` lands on the last character; `d}` starting mid-line deletes to buffer end (B13 fix in place).
//! * Pure-whitespace lines do not break paragraphs: `d}` steps over the whitespace line and deletes the whole section (the dual semantics of motion-stops-but-operator-does-not, see B-1's comment).
//! * `g_`/`2g_` land on the line's last non-blank; `_` with count 1 = this line's first non-blank (does not move lines).
//! * `99go` out of bounds lands on the last character 'c' (clamped to the buffer's last byte, not the last line's start).
//! * J cursor lands on the first line's last character ("aa\nbb" → col 3; empty second line oracle [0,1,2,0] same position); gJ cursor lands on the second line's first character (col 16).
//! * The no-space-added matrix of J over empty/whitespace/`)`-starting lines; `5J`/`10J` counting.
//! * `gJ` preserves the second line's indent, `3gJ` counting, `gJ` on the last line rings the bell.
//! * `cc` clears the indent (noai) with cursor at col 1; `3C`/`2S`/`2s`/`2dd` counting; `2X` backward delete.
//! * The `<<`/`>>` display-column model (the TAB-line matrix under noet ts=8 sw=2), `>>` skipping empty lines, `>>` cursor = first non-blank of the range's first line, `#define` lines indenting normally.
//! * `dge` from word end across words (inclusive of both ends), from mid-word stopping at word end, `ge` stuck at the start of the same word (vim does not ring either); `de` mid-word deletes only the current word ('foo, bar' → ', bar').
//! * `dw` over whitespace runs/punctuation words, `cw`/`cW`/`caw` over whitespace runs, `cw` on punctuation, `yiw` on punctuation words, `y2iw` = "foo " (that is how vim's object counting works).
//! * `10~` counting, `ß→ẞ` (U+1E9E), `guG`/`gugu`/`gUgU` linewise double application, `gU$` leaving digits unaffected.
//! * `2fX` counting, `tx` stopping on a near target, `2tx` stop point col 3, `;` failure ringing without moving, the column semantics of CJK `f最;`.
//! * `%` on an unpaired character rings; `d%` deletes the pair; `%` from a non-bracket character searching forward for a bracket.
//! * The whitespace and punctuation shapes of `d2aw`/`daw`/`daW`/`diW`/line-end `daw` (audit2 in place).
//! * `dis` basic shape, single-sentence `yis`/`yas`, mid-sentence `yis`, last-sentence `yas`.
//! * `cit` empty tag, `da<`/`di<` basic shapes.
//! * `yap` paragraph span (incl. the leading-blank-line variant); `dip` on an all-whitespace buffer = one empty line.
//! * `gqq` text side and cursor landing on the last line; `2D` line end + 1 line; `d2w` from a whitespace start across lines ('ab \n') and from a mid-word start ('ab c\n' — rule (a)'s last-line shape agrees on both sides).
//! * `100g_` with a big count clamped to the last line's last non-blank (oracle [0,2,8,0] — cursor_down only FAILs when the cursor is already on the last line; engine agrees); `g_`/`2g_`/`_` basic shapes.
//! * `~` clamps back to the cursor line's last character (0~~~ → 'Ab', engine agrees); `2~` on a multibyte line counts characters.
//! * `daw` mid-line shape and cursor landing (start of the remaining text), daw on a word-only line leaves an empty line, daw on a TAB segment deletes TAB + next word, `d2aw`/`2caw` counting, `yiw` on whitespace takes the whitespace run.
//! * `dip` on the last paragraph leaves 'aa\n\n'; the whitespace-run semantics of `diw`/`daw` (audit2 in place).
//! * `%` from a non-bracket character searching forward for an in-line bracket, cross-line pairing (% on (a\nb) lands at [0,2,2,0], engine agrees — briefly misjudged this round, the col 1→byte conversion re-check was wrong); `cG` changing from mid-line to buffer end; `guap` linewise touching only the current paragraph; `>>` on an empty buffer not indenting; `d}}` from a line-start on 'a\n\n\n' promoting to linewise and deleting only one line; `cw`/`dw` on a TAB moving only the TAB; `ge` on a punctuation word landing at the punctuation's end.
//! ---------------------------------------------------------------

mod common;

use common::{edit, Fixture};

fn unnamed(f: &Fixture) -> String {
    f.vim
        .registers
        .get('"')
        .map(|r| r.text.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- B-1 (falsified → flipped into a regression pin)
// The draft asserted `}` stops at the whitespace line's start (offset 6) — the 2026-10-09 typeahead
// re-check overturned it: on `aa\n   \nbb\n`, `}` lands at **[0,3,2,0]** (offset 8, the second
// character of the next paragraph 'bb' — nv_paragraph steps over the whitespace line and lands on
// the next paragraph's non-blank). The engine's empty-only check already agrees with this; B-1 was
// a sampling misread and is withdrawn; this test is converted into pinning the re-verified oracle
// landing (the [0,2,1,0] cited in the draft came from conflating a different buffer shape).
#[test]
fn b1_para_motion_lands_on_next_paragraph_nonblank() {
    let f = edit("aa\n   \nbb\n", 0, 0, &["}"]);
    assert_eq!(f.cursor(), 8, "oracle [0,3,2,0]: nv_paragraph lands on the next paragraph's non-blank (re-verified 2026-10-09)");
}

// ---------------------------------------------------------------- B-2 (P3)
// `b`/`B` at buffer start must ring the bell. The engine's WordBack returns "in-place success"
// rather than stuck at the origin (the b arm of motions.rs lacks the stuck branch the ge arm has)
// — silent.
// oracle (expect PTY, counting BEL bytes in the output): after printf 'one\n' send `b`/`B`/`ge`/
//   `gE`, each captures 1 BEL; the cursor never moves. Measured on the engine side: b/B bell 0,
//   ge/gE bell 1 (ge/gE already compliant; kept in the assertions as pins).
#[test]
fn b2_backward_word_motions_at_buffer_start_must_bell() {
    let cases: &[&[&str]] = &[&["b"], &["B"], &["g", "e"], &["g", "E"]];
    for keys in cases {
        let mut f = Fixture::new("one\n");
        f.feed(keys.iter().map(|k| k.to_string()).collect::<Vec<_>>());
        assert!(
            f.host.bells > 0,
            "vim: {:?} at buffer start rings the bell (PTY measured 1 BEL)",
            keys
        );
        assert_eq!(f.cursor(), 0, "vim: {:?} at buffer start does not move", keys);
    }
}

// ---------------------------------------------------------------- B-3 (P1)
// `y2b` across an empty line: the shape of exclusive rule (a) (endpoint at col 1 → endpoint moves
// to the previous line's end, turning inclusive) when "the previous line is empty". vim's empty
// line has no characters, so what gets collected into the yank is the **previous line's** newline
// ("two\n", 4 bytes); the engine also collects the empty line's own newline ("two\n\n", 5 bytes).
// oracle: printf 'one two\n\nfoo bar\n'; `3Gy2b` + strtrans(getreg('"'))
//   → "two^@" (i.e. "two\n"). The engine has one newline too many.
#[test]
fn b3_y2b_exclusive_rule_on_empty_previous_line() {
    let mut f = Fixture::new("one two\n\nfoo bar\n");
    f.feed(["3", "G", "y", "2", "b"]);
    assert_eq!(unnamed(&f), "two\n", "vim: y2b across an empty line yanks two+newline (oracle)");
    // The same-family non-empty-line shape (falsified, pinned against regression): b stops at the whitespace line's start
    let mut f = Fixture::new("one two\n  \nfoo bar\n");
    f.feed(["3", "G", "y", "b"]);
    assert_eq!(unnamed(&f), "two\n  ", "vim: b stops at the start of a non-blank whitespace line");
}

// ---------------------------------------------------------------- B-4 (P1)
// The block object does not search forward when the cursor is **before** the block. vim (:h i()):
// when the cursor is not inside a block, search forward for the first unmatched `(`. The engine's
// block_range only scans backward, so `0di)` lands nowhere.
// oracle: printf 'foo (bar (baz (quux)))\n'; `0di)` → "foo ()";
//   `02di)` ('foo (bar (baz) (quux))') → "foo (bar () (quux))";
//   `03di)` on that buffer → unchanged (no third nesting level, testdir).
#[test]
fn b4_block_object_searches_forward_when_cursor_before_block() {
    let f = edit("foo (bar (baz (quux)))\n", 0, 0, &["0", "d", "i", ")"]);
    assert_eq!(f.text(), "foo ()\n", "vim: 0di) searches forward for the block (oracle q_di)");
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "2", "d", "i", ")"]);
    assert_eq!(f.text(), "foo (bar ())\n", "vim: 02di) descends to the second level (testdir)");
    let f = edit("foo (bar (baz) (quux))\n", 0, 0, &["0", "d", "i", ")"]);
    assert_eq!(f.text(), "foo ()\n", "vim: 0di) takes the first unmatched pair (oracle q_di2)");
    let f = edit("foo (bar (baz) (quux))\n", 0, 0, &["0", "3", "d", "i", ")"]);
    assert_eq!(
        f.text(),
        "foo (bar (baz) (quux))\n",
        "vim: 03di) with no third nesting level → no-op (testdir)"
    );
}

// ---------------------------------------------------------------- B-5 (P2)
// The yank shape of the same root cause: `0yi)` forward + count.
// oracle: printf 'foo (bar (baz (quux)))\n'; `0yi)` + getreg →
//   "bar (baz (quux))"; `03yi)` → "quux" (testdir). The engine yanks empty.
#[test]
fn b5_yank_block_object_forward() {
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "y", "i", ")"]);
    assert_eq!(unnamed(&f), "bar (baz (quux))", "vim: 0yi) forward (testdir)");
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "3", "y", "i", ")"]);
    assert_eq!(unnamed(&f), "quux", "vim: 03yi) descends to the third level (testdir)");
}

// ---------------------------------------------------------------- B-6 (P2)
// `cib` on "text()" (empty pair after the cursor): vim searches forward to the empty () and
// inserts in the middle. The engine's object fails and the c is cancelled.
// oracle: printf 'text()\n'; `0cibtext<Esc>` → "text(text)" (same shape as testdir's
//   Test_inner_block_empty_paren; local typeahead re-check agrees).
#[test]
fn b6_cib_on_empty_parens_after_cursor() {
    let mut f = Fixture::new("text()\n");
    f.feed(["0", "c", "i", "b"]);
    f.type_text("text");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "text(text)\n", "vim: cib on an empty () searches forward and inserts in the middle");
}

// ---------------------------------------------------------------- B-7 (P2)
// The `<`/`>` block objects search forward the same way (Test_textobj_find_paren_forward).
// oracle: printf '#include <foo.h>\n'; `0yi<` → "foo.h"; `0ya>` → "<foo.h>". The engine yields empty for both.
#[test]
fn b7_angle_block_object_forward() {
    let mut f = Fixture::new("#include <foo.h>\n");
    f.feed(["0", "y", "i", "<"]);
    assert_eq!(unnamed(&f), "foo.h", "vim: 0yi< forward (testdir)");
    let mut f = Fixture::new("#include <foo.h>\n");
    f.feed(["0", "y", "a", ">"]);
    assert_eq!(unnamed(&f), "<foo.h>", "vim: 0ya> forward (testdir)");
}

// ---------------------------------------------------------------- B-8 (P2)
// Whitespace taking for `a'`: vim prefers the **trailing** whitespace and takes the **leading**
// whitespace when there is none trailing. The engine takes only the quoted string itself in both shapes.
// oracle: printf "some    'special'    string\n"; `0ya'` + getreg →
//   "'special'    "; "some    'special'string\n" → "    'special'"
//   (same shape as testdir's Test_textobj_quote).
#[test]
fn b8_aquote_whitespace_asymmetry() {
    let mut f = Fixture::new("some    'special'    string\n");
    f.feed(["y", "a", "'"]);
    assert_eq!(unnamed(&f), "'special'    ", "vim: trailing whitespace preferred (oracle q_aq)");
    let mut f = Fixture::new("some    'special'string\n");
    f.feed(["y", "a", "'"]);
    assert_eq!(unnamed(&f), "    'special'", "vim: no trailing → take leading (testdir)");
}

// ---------------------------------------------------------------- B-9 (P2)
// The `v`/`V`/`<C-v>` keys in the operator-pending state: vim uses them to switch the motion's
// inclusive/exclusive/linewise (:h o_v). The engine's pending phase has none of these three, so
// after `d` a `v` trie miss → bell + cancel, making the whole `dvgo` a no-op.
// oracle: printf 'one two three four\n'; `5|dvgo` (col 5 = 't') →
//   "wo three four" (v makes the arrival side inclusive, deleting including the cursor character).
#[test]
fn b9_operator_pending_v_toggles_motion_kind() {
    let f = edit("one two three four\n", 0, 4, &["d", "v", "g", "o"]);
    assert_eq!(f.text(), "wo three four\n", "vim: dvgo deletes including the cursor character (oracle t5)");
}

// ---------------------------------------------------------------- B-10 (P2)
// The count shape of `2yis`: vim's is count extension merges the **joining whitespace** between sentences.
// The engine stays inside the first sentence.
// oracle: printf 'A sentence.  A sentence?  A sentence!\n'; `2yis` +
//   getreg → "A sentence.  "; `3yis` → "A sentence.  A sentence?";
//   `2yas` → "A sentence.  A sentence?  " (same shape as testdir's Test_sentence,
//   oracle q_2yis re-check agrees).
#[test]
fn b10_sentence_inner_count_includes_joining_whitespace() {
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["2", "y", "i", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  ", "vim: 2yis (oracle q_2yis/testdir)");
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["3", "y", "i", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  A sentence?", "vim: 3yis (testdir)");
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["2", "y", "a", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  A sentence?  ", "vim: 2yas (testdir)");
}

// ---------------------------------------------------------------- B-11 (P2)
// When `das`'s cursor sits on the inter-sentence whitespace (the space after '?'): vim takes the
// **next sentence** (with its trailing whitespace). The engine took the previous sentence.
// oracle: printf a 3-line buffer; `3ggf?ldas` + getline →
//   line 3 = "Even with a question? And no sentence here" (oracle q_das).
#[test]
fn b11_das_on_inter_sentence_whitespace_targets_next_sentence() {
    let mut f = Fixture::new(
        "This is a test. With some sentences!\n\nEven with a question? And one more. And no sentence here\n",
    );
    f.feed(["3", "G", "f", "?", "l", "d", "a", "s"]);
    assert_eq!(
        f.text(),
        "This is a test. With some sentences!\n\nEven with a question? And no sentence here\n",
        "vim: das on inter-sentence whitespace takes the next sentence (oracle q_das/testdir)"
    );
}

// ---------------------------------------------------------------- B-12 (P2)
// `dg_` (g_ under an operator): inclusive, ending at the **last non-blank character**, with line-end
// whitespace preserved. The engine deleted the line-end whitespace too.
// oracle: printf '  ab  cd  \n'; `dg_` + getline/getpos →
//   line = "    " (2 leading + 2 trailing spaces), cursor col 3 (oracle r4).
#[test]
fn b12_dgunderscore_keeps_trailing_whitespace() {
    let mut f = Fixture::new("  ab  cd  \n");
    f.feed(["d", "g", "_"]);
    assert_eq!(f.text(), "    \n", "vim: dg_ keeps line-end whitespace (oracle r4)");
    assert_eq!(f.cursor(), 2, "vim: after dg_ cursor at col 3");
}

// ---------------------------------------------------------------- B-13 (P3)
// A paragraph-object count beyond the paragraphs must ring (testdir Test_paragraph:
// assert_beeps('normal 3yap')). The engine stays silent.
// oracle (expect PTY): printf '\n\n\nFirst line.\nSecond line.\n';
//   send `2G3yap` → captures 1 BEL.
#[test]
fn b13_yap_count_beyond_paragraphs_must_bell() {
    let mut f = Fixture::new("\n\n\nFirst line.\nSecond line.\n");
    f.feed(["2", "G", "3", "y", "a", "p"]);
    assert!(f.host.bells > 0, "vim: 3yap with no third paragraph rings (PTY measured 1 BEL)");
}

// ---------------------------------------------------------------- B-14 (P1)
// Tag-object parsing is fooled by a `>` inside quotes in an attribute: the engine's tag_range
// truncates the tag with a bare find('>'), so `<div attr="attr >> foo >> bar ">` gets cut at
// `<div attr="attr >`. vim's tag parsing skips quoted strings.
// oracle: printf '<div attr="attr >> foo >> bar ">Hello</div>\n';
//   `fHyit` + getreg → "Hello" (oracle t3; same shape as testdir's
//   Test_string_html_objects).
#[test]
fn b14_tag_object_skips_quoted_gt_in_attributes() {
    let mut f = Fixture::new("<div attr=\"attr >> foo >> bar \">Hello</div>\n");
    f.feed(["f", "H", "y", "i", "t"]);
    assert_eq!(unnamed(&f), "Hello", "vim: yit skips the >> inside attributes (oracle t3)");
    let mut f = Fixture::new("<div attr='attr >> foo >> bar '>Hello 123</div>\n");
    f.feed(["f", "H", "y", "i", "t"]);
    assert_eq!(unnamed(&f), "Hello 123", "vim: same for single-quoted attributes (testdir)");
}

// ---------------------------------------------------------------- B-15 (P2)
// Multiline inner block objects: vim's inner start/end take "next line's start / previous line's
// end", keeping the newlines on both sides (testdir: di[ on [' ', '[', 'one [two]', 'thre', ']']
// → [' ', '[', ']'). The engine deletes byte-adjacently, swallowing the newlines and merging lines.
// oracle: printf ' \n[\none [two]\nthre\n]\n'; `3gg0di[` → three lines;
//   printf 'x = (\n    one,\n    two,\n)\n'; `di(` → ['x = (', ')'].
#[test]
fn b15_multiline_inner_block_keeps_newlines() {
    let mut f = Fixture::new(" \n[\none [two]\nthre\n]\n");
    f.feed(["3", "G", "0", "d", "i", "["]);
    assert_eq!(f.text(), " \n[\n]\n", "vim: di[ across lines keeps the newlines (oracle u2/testdir)");
    let mut f = Fixture::new("x = (\n    one,\n    two,\n)\n");
    f.feed(["d", "i", "("]);
    assert_eq!(f.text(), "x = (\n)\n", "vim: di( deletes the indented content whole-line (oracle u3)");
}

// ---------------------------------------------------------------- B-16 (P3)
// `aw` count beyond the words must ring (testdir Test_textobj_a_word:
// assert_beeps('normal 0y3aw')). The engine stays silent.
// oracle (expect PTY): printf 'one a\n'; send `y3aw` → captures 1 BEL.
#[test]
fn b16_y3aw_beyond_last_word_must_bell() {
    let mut f = Fixture::new("one a\n");
    f.feed(["y", "3", "a", "w"]);
    assert!(f.host.bells > 0, "vim: y3aw with too few words rings (PTY measured 1 BEL)");
}

// ---------------------------------------------------------------- B-17 (on the books → #[ignore])
// testdir's full d2at sequence (oracle w1: `1gg0da<1pjd2at` leaves [' ']). The engine yields
// [" "," "] on that sequence. Established facts (2026-10-09 oracle matrix):
// * The nesting climb itself is fine: on the clean intermediate state " <div>\n<a…>xyz</a>\n    </div>\n \n",
//   object_span_count(Tag, 2) with the cursor at '<a' = the div outer span (1,54).
// * vim's 2at deletion range on that shape = [div open tag .. buffer end] (j0d2at → [" "], even the
//   " " on line 4 goes; j0dat with count=1 deletes only the a tag, j0dit keeps the structure) — the
//   trailing newline/blank line after the tag block's close tag is merged into the deletion range;
//   which interaction with exclusive rule (a) causes this has not been reverse-engineered (the
//   linewise-promotion shape does not match; candidate models see the round-34 audit's open books).
//   Needs a dedicated per-shape PTY reverse-engineering before implementing.
#[test]
#[ignore = "B-17 open book: the trailing whitespace/newline attribution of at's outer span is not reverse-engineered (oracle data in this comment)"]
fn b17_d2at_testdir_sequence() {
    let mut f = Fixture::new("<div> \n<a href=\"foobar\" class=\"foo\">xyz</a>\n    </div>\n \n");
    f.feed([
        "1", "g", "g", "0", "d", "a", "<", "1", "p", "j", "d", "2", "a", "t",
    ]);
    assert_eq!(f.text(), " \n", "vim: testdir d2at sequence leaves [' '] (oracle w1)");
}

// ---------------------------------------------------------------- B-18 (P2)
// `3cc` on the last line (count beyond the lines): vim's cc family **fails entirely** when there
// are not enough lines (does not enter insert mode, buffer unchanged).
// oracle: printf 'a\nb\n'; `G3ccX<Esc>` + getline → 'a\nb\n' (X never took effect; after 3cc fails
//   X is a normal-mode X — a no-op at col 1; oracle x4).
#[test]
fn b18_counted_cc_beyond_last_line_fails() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["G", "3", "c", "c"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a\nb\n", "vim: 3cc on the last line fails entirely (oracle x4)");
}

// ---------------------------------------------------------------- B-19 (P2)
// `dap` when the first line is empty: vim deletes the empty line together with the whole paragraph
// after it (incl. the third line), leaving [''] (text "\n").
// oracle: printf '\naa\nbb\n'; `dap` + getline → [''] (oracle y4).
#[test]
fn b19_dap_on_leading_empty_line() {
    let mut f = Fixture::new("\naa\nbb\n");
    f.feed(["d", "a", "p"]);
    // Expectation corrected to the engine's internal-text convention (2026-10-09): vim's
    // writefile(['']) = "\n" is the [write-back] shape (fixendofline appends the terminator); the
    // engine represents the same "single empty line" state as "" (line_count = 1 on both sides,
    // consistent with the existing dd-clears-buffer pins, e.g. huge_counts_saturate). oracle y4's
    // [''] and "" are the same state.
    assert_eq!(
        f.text(),
        "",
        "oracle [''] (same state under the internal-text convention): dap with a leading empty line empties the buffer leaving one empty line"
    );
}

// ---------------------------------------------------------------- B-20 (P2)
// `db` upward from the second line's start: b lands on the previous line's word start, and
// exclusive rule (a) moves the endpoint to the **previous line's end** (inclusive, i.e. deleting up
// to the line's last character) — the newline survives and the lines do not merge.
// oracle: printf 'one two\nfoo\n'; `2Gdb` + getline → ['one ', 'foo'] (oracle v10).
#[test]
fn b20_db_upward_keeps_line_break() {
    let f = edit("one two\nfoo\n", 1, 0, &["d", "b"]);
    assert_eq!(f.text(), "one \nfoo\n", "vim: db upward deletes to the previous line's end without merging lines (oracle v10)");
}
