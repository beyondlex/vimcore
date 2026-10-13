//! Round-4 independent audit (search / display column / hostile host data domains).
//!
//! Each `#[test]` corresponds to a newly discovered finding; the assertions encode
//! the expected values empirically verified against vim 9.1 (the `-Nu NONE -N -s`
//! typeahead oracle), so they **must fail** on the current engine.
//! The comment at the top of each test gives: severity, repro, vim expectation,
//! engine current state.

mod common;

use common::Fixture;

// ---------------------------------------------------------------------------
// F1 (P2) `gd`/`gD` unbound, and `gd` silently arms the delete operator
//
// Repro: buffer "int x;\nuse x here\n", cursor (line 1, col 4) (the x of "use x"),
//       keys g d x.
// vim 9.1 (oracle: gd lands at (1,5), i.e. the x of "int x;"; then x deletes it): text
//       "int ;\nuse x here\n", cursor offset 4.
// Engine: `gd` trie miss → drops g, re-feeds d → the Delete operator is silently armed;
//       the following x is an illegal motion, so it rings the bell and gives up. Text
//       unchanged, cursor stays in place (11).
#[test]
fn f1_gd_unbound_arms_delete_operator() {
    let mut f = Fixture::at("int x;\nuse x here\n", 1, 4);
    f.feed(["g", "d", "x"]);
    assert_eq!(f.cursor(), 4, "gd must jump to the declaration (line0 col4)");
    assert_eq!(f.text(), "int ;\nuse x here\n", "x then deletes the declared 'x'");
}

// ---------------------------------------------------------------------------
// F2 (P2) `g*`/`g#` missing, degrading to whole-word `*`/`#` (plus one spurious bell)
//
// Repro A: buffer "foo bar\nfoofoo\nfoo\n", (0,0), g *.
//   vim: g* = substring search without \<\> boundaries → lands at line2 col1
//   (start of foofoo, offset 8).
//   Engine: after the trie-miss bell it replays `*` as a whole-word search →
//   lands at line3 col0 (offset 15).
// Repro B: buffer "foofoo\nfoo bar\n", (1,0), g #.
//   vim: substring search backwards (upwards) → the second "foo" inside line1, offset 3.
//   Engine: whole-word `\bfoo\b` finds nothing upwards → wraps around and stops on itself
//   (offset 7).
#[test]
fn f2_gstar_ghash_missing_degrade_to_whole_word() {
    let mut f = Fixture::at("foo bar\nfoofoo\nfoo\n", 0, 0);
    f.feed(["g", "*"]);
    assert_eq!(f.cursor(), 8, "g* searches the substring 'foo' (no word bounds)");
    assert_eq!(f.host.bells, 0, "g* is a real command, not a trie miss");

    let mut g = Fixture::at("foofoo\nfoo bar\n", 1, 0);
    g.feed(["g", "#"]);
    assert_eq!(g.cursor(), 3, "g# finds the earlier in-word 'foo'");
}

// ---------------------------------------------------------------------------
// F3 (P2) `n`/`N` do not re-apply the search offset (and the first jump of `/pat/e`
//         anchors the wrong match)
//
// Repro: buffer "foo x\nfoo y\n", (0,0): / f o o / e <CR> → n → n.
// vim 9.1 (oracle, sampled step by step): the first jump lands at (1,3)=offset 2 (the end
//   of the word of the match the cursor sits on), the first n lands at (2,3)=offset 8, and
//   the second n wraps back to (1,3)=offset 2 — every n re-applies the `e` anchor
//   (:h search-offset).
// Engine: the first jump skips the match at the cursor and lands at offset 8; afterwards
//   each n only lands at the **start** of the match (0), losing the offset entirely. The
//   doc comment of search.offset claims "reapplied by n/N like vim", yet the SearchNext
//   arm of motions.rs never calls apply_search_offset.
#[test]
fn f3_n_does_not_reapply_search_offset() {
    let mut f = Fixture::at("foo x\nfoo y\n", 0, 0);
    f.feed(["/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(f.cursor(), 2, "first /foo/e anchors the cursor match's end");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 8, "n re-applies the e offset to the next match");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 2, "wrapping n re-applies the e offset again");
}

// ---------------------------------------------------------------------------
// F4 (P2) operator/visual search prompts discard the search offset entirely
//         (d/v + /pat/e)
//
// Repro A: buffer "x foo y\n", (0,0): d / f o o / e <CR>.
//   vim (oracle): deletes through the end of the matched word → " y".
//   Engine: cmdline.rs only applies apply_search_offset when motion==None → the operator
//   span stops at the match start → "foo y\n" (under-deletes "foo").
// Repro B: same buffer: v / f o o / e <CR>.
//   vim: the selection extends to the end of the match (cursor offset 4).
//   Engine: search_motion_visual jumps to the match start (cursor offset 2).
#[test]
fn f4_operator_visual_search_ignore_offsets() {
    let mut f = Fixture::at("x foo y\n", 0, 0);
    f.feed(["d", "/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(f.text(), " y\n", "d/pat/e deletes through the match end");

    let mut v = Fixture::at("x foo y\n", 0, 0);
    v.feed(["v", "/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(v.cursor(), 4, "v/pat/e extends the selection to the match end");
}

// ---------------------------------------------------------------------------
// F5 (P1) s/b/e offset tails with a digit treat the digit as a "line shift"; vim means
//         a "character shift"
//
// Repro: buffer "aaaaaaaaaa\nbbbbbbbbbb\nccc foo\ndddddddddd\neeeeeeeeee\n",
//       (0,0), line-start offsets L1=0 L2=11 L3=22 L4=30 L5=41, match "foo" at (3,5..8).
// vim 9.1 (oracle, verified one by one): the [±N] after s/b/e is a **character** shift —
//   /foo/b2  → (3,7)  = offset 28 (start + 2 chars)
//   /foo/e2  → (4,2)  = offset 31 (word end + 2 chars, crossing lines)
//   /foo/s-2 → (3,3)  = offset 24 (start - 2 chars)
// Engine: parse_search_offset treats N as line_shift and pins the column at the anchor —
//   b2 → 45, e2 → 47, s-2 → 4, landing on entirely wrong lines.
#[test]
fn f5_offset_sbe_count_is_chars_not_lines() {
    let buf = "aaaaaaaaaa\nbbbbbbbbbb\nccc foo\ndddddddddd\neeeeeeeeee\n";

    let mut f = Fixture::at(buf, 0, 0);
    f.feed(["/", "f", "o", "o", "/", "b", "2", "<CR>"]);
    assert_eq!(f.cursor(), 28, "/foo/b2 = start + 2 chars (line3 col7)");

    let mut g = Fixture::at(buf, 0, 0);
    g.feed(["/", "f", "o", "o", "/", "e", "2", "<CR>"]);
    assert_eq!(g.cursor(), 31, "/foo/e2 = end + 2 chars (line4 col2)");

    let mut h = Fixture::at(buf, 0, 0);
    h.feed(["/", "f", "o", "o", "/", "s", "-", "2", "<CR>"]);
    assert_eq!(h.cursor(), 24, "/foo/s-2 = start - 2 chars (line3 col3)");
}

// ---------------------------------------------------------------------------
// F6 (P2) pure line-shift offsets ([N]/+N/-N) keep the match column; vim lands in
//         column 1
//
// Repro: buffer "x foo y\n    ind\nzzz\n", (0,0): / f o o / + 1 <CR>.
// vim 9.1 (oracle): match line 1 + 1 line → line2 **col 1** (the first space of "    ind";
//   column 1, neither first-non-blank) → offset 8.
// Engine: apply_search_offset keeps the match-start column (display col 2) → offset 10
//   ('i').
#[test]
fn f6_line_offset_lands_column1() {
    let mut f = Fixture::at("x foo y\n    ind\nzzz\n", 0, 0);
    f.feed(["/", "f", "o", "o", "/", "+", "1", "<CR>"]);
    assert_eq!(f.cursor(), 8, "/pat/+1 lands in column 1 of the shifted line");
}

// ---------------------------------------------------------------------------
// F7 (P1) the display-column model counts TAB as 1 cell; vim expands it by 'tabstop'
//         (`|`, j/k, C-d/C-u and search-offset column preservation are all affected)
//
// Repro: buffer "a\tb\nefg\n", (0,0): 4 | x.
// vim 9.1 (oracle): the tab spans virtual columns 2..9; `4|` parks on the tab, and `x`
//   deletes the whole tab → "ab\nefg\n", cursor offset 1 ('b').
// Engine: char_display_width('\t')=1 → `4|` clamps to the last char 'b', and x deletes
//   'b' → "a\t\nefg\n" (wrong char deleted, tab survives).
#[test]
fn f7_tab_counts_one_cell_in_column_math() {
    let mut f = Fixture::at("a\tb\nefg\n", 0, 0);
    f.feed(["4", "|", "x"]);
    assert_eq!(f.text(), "ab\nefg\n", "4| parks on the tab; x deletes the whole tab");
    assert_eq!(f.cursor(), 1, "cursor rests on 'b' after deleting the tab");
}

// ---------------------------------------------------------------------------
// F8 (P3) the backwards-direction incsearch preview marks the "nearest forward" match
//         as current
//
// Repro: buffer "foo\nbar\nfoo bar\n", cursor (2,5) (offset 13): ? o (no Enter).
// vim: the incsearch preview marks the match Enter would jump to — nearest backwards =
//   the second 'o' on line3 (offset 10..11; the oracle proves it via the `?o<CR>` landing
//   at (3,3)).
// Engine: publish_incsearch ignores the direction and always takes the first match after
//   the cursor with wrapping → current_highlight = 1..2 (the o on line1).
#[test]
fn f8_incsearch_backwards_preview_marks_forward_match() {
    let mut f = Fixture::at("foo\nbar\nfoo bar\n", 2, 5);
    f.feed(["?", "o"]);
    assert_eq!(
        f.host.current_highlight,
        Some(10..11),
        "?-incsearch preview marks the match Enter would land on"
    );
}

// ---------------------------------------------------------------------------
// F9 (P2) Ex range addresses do not accept offset tails (:/foo/+1)
//
// Repro: buffer "foo\nbar\nbaz foo\n", (0,0): : / f o o / + 1 <CR>.
// vim 9.1 (oracle): address = the first foo match line after the cursor line (line3) + 1
//   → clamped to line3 col1 → offset 8, with no error at all.
// Engine: the /pat/ branch of parse_range only strips the trailing terminator, so
//   "foo/+1" is taken as the pattern → E16: Invalid range, cursor unmoved (offset 0).
#[test]
fn f9_ex_range_address_rejects_offset_tail() {
    let mut f = Fixture::at("foo\nbar\nbaz foo\n", 0, 0);
    f.feed([":", "/", "f", "o", "o", "/", "+", "1", "<CR>"]);
    assert_eq!(f.cursor(), 8, ":/foo/+1 moves to the offset address line");
    assert!(
        f.host.statuses.is_empty(),
        "address movement is silent; engine says {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------------------
// F10 (P1) linewise put into a fully emptied buffer drops the final newline / P-shape
//          loses a line
//
// Repro A: buffer "abc\n", (0,0): d d p.
//   vim (oracle): lines = ["", "abc"] keeping the final EOL → text "\nabc\n".
//   Engine: the trailing \n of the register is consumed as a line separator → "\nabc"
//   (final EOL lost, so writing the file truncates one byte).
// Repro B: buffer "abc\n", (0,0): d d P.
//   vim (oracle): lines = ["abc", ""] → text "abc\n\n".
//   Engine: inserts out "abc\n" (1 line, one empty line fewer than vim).
#[test]
fn f10_linewise_put_into_emptied_buffer_loses_final_newline() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["d", "d", "p"]);
    assert_eq!(f.text(), "\nabc\n", "ddp on a single-line buffer keeps the EOL");

    let mut g = Fixture::at("abc\n", 0, 0);
    g.feed(["d", "d", "P"]);
    assert_eq!(g.text(), "abc\n\n", "ddP keeps the emptied line below");
}

// ---------------------------------------------------------------------------
// F11 (P2) `[count]<C-f>` (and the family C-b/C-d/C-u) ignores the count
//
// Repro: buffer of 200 lines "line000\n".."line199\n" (8 bytes each), host viewport
//       (0,9) (equivalent to a 10-line vim window under `:set lines=12`), (0,0): 2 <C-f>.
// vim 9.1 (oracle, lines=12): 1<C-f> lands at line idx 9; **2<C-f> lands at idx 18**
//   (w0=19, each page multiplied by the count) → offset 18*8 = 144.
// Engine: the step of PageDown in motions.rs is always visible, the count never
//   participates → idx 9 (offset 72), indistinguishable from 1<C-f>.
#[test]
fn f11_page_scroll_ignores_count() {
    let mut buf = String::new();
    for i in 0..200 {
        buf.push_str(&format!("line{i:03}\n"));
    }
    let mut f = Fixture::at(&buf, 0, 0);
    f.host.viewport = (0, 9);
    f.feed(["2", "<C-f>"]);
    assert_eq!(f.cursor(), 18 * 8, "2<C-f> scrolls two pages (line idx 18)");
}
