//! Round-21 (added during the round-22 review window) fuzz: the `:s` replacement-string
//! escape surface.
//!
//! Background: round22 fixed the literal pass-through of `\r` (newline) and `\n` (NUL)
//! in replacement strings — previously they were written into the buffer verbatim; after
//! the fix a substitution can **change the line count** and can introduce NUL bytes.
//! This is a state space newly reached by ex_substitute (line-number drift, the
//! `last_sub_line` split landing point, marks/highlight adjustment under multi-line
//! replace); this round barrages with a hostile replacement alphabet and asserts:
//!   1. engine invariants (cursor/highlight/marks addressable — continuation of the
//!      round20 contract);
//!   2. **line-addressing contract**: after a substitution every line's
//!      `line_start/line_end/offset_to_line` must be self-consistent (the line-split
//!      implementation joins the replacement text into the original range — one wrong
//!      line and it crashes);
//!   3. NUL bytes (vim's semantics for `\n`) break no addressing math.
//!
//! Plus a targeted CJK-side search-motion probe (the multibyte shape of round21's new
//! feature).

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;

fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn assert_addressable(f: &Fixture, context: &str) {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= len && text.is_char_boundary(cur),
        "{context}: cursor {cur} unaddressable (len={len}) in {text:?}"
    );
    // line-addressing contract: every line's start/end is a char boundary and the ranges are ordered
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= len && text.is_char_boundary(s) && text.is_char_boundary(e),
            "{context}: line {line} range {s}..{e} broken (len={len}) in {text:?}"
        );
        assert_eq!(
            buf.offset_to_line(s),
            line,
            "{context}: line {line} start {s} resolves to line {}",
            buf.offset_to_line(s)
        );
    }
    // render contract: highlight ranges addressable (round20 contract extended to the
    // line-splitting replacement surface)
    for hl in f
        .host
        .highlights
        .iter()
        .chain(f.host.current_highlight.iter())
    {
        assert!(
            hl.end <= len && text.is_char_boundary(hl.start) && text.is_char_boundary(hl.end),
            "{context}: highlight {hl:?} unaddressable in {text:?}"
        );
    }
}

/// Hostile replacement-string alphabet: escape pairs (\r \n \\ \/) interleaved with bare
/// metacharacters.
const REPL_ALPHABET: &[&str] = &[
    "X", "\\r", "\\n", "\\\\", "\\/", "$0", "$1", "&", "\\x", "\\\\", "\\r\\n", "\\\\',", "中",
    "\\r中", "\\n\\",
];

const PATTERN_ALPHABET: &[&str] = &["a", "中", "$", "^", "a*", "[ab]", "(a|b)", "\\\\r", "."];

const TARGETS: &[&str] = &[
    "aaa\nbbb\n",
    "中文中\n文\n",
    "a\n",
    "a",
    "aba aba\n\n中a文\n",
    "aaa",
];

#[test]
fn fuzz_substitute_replacement_escapes_hold_line_addressing() {
    for (i, target) in TARGETS.iter().enumerate() {
        for (pi, pat) in PATTERN_ALPHABET.iter().enumerate() {
            for (ri, rep) in REPL_ALPHABET.iter().enumerate() {
                let mut f = Fixture::new(target);
                let context = format!("target#{i} s/{pat}/{rep}/ (p{pi} r{ri})");
                feed_ex(&mut f, &format!("%s/{pat}/{rep}/g"));
                assert_addressable(&f, &context);
                // after line splitting run n/N + highlight re-publish once more — the
                // line-number-cache mismatch surface
                f.feed(["n", "N"]);
                assert_addressable(&f, &context);
                // marks addressable (the adjust_replace surface after split-line edits)
                f.feed(["g", "g"]);
                f.feed(["m", "a"]);
                feed_ex(&mut f, "%s/a/b/g");
                assert_addressable(&f, &context);
            }
        }
    }
}

/// Targeted: after `%s/a/\r/` the cursor must land on the **post-split** last replaced
/// line (vim 9.1: "aaa\nbbb" → lands on the "aa" line; g-shape → lands on line 4).
#[test]
fn substitute_line_split_cursor_lands_post_split() {
    let mut f = Fixture::new("aaa\nbbb\n");
    feed_ex(&mut f, "%s/a/\\r/");
    assert_eq!(f.text(), "\naa\nbbb\n");
    assert_eq!(f.line(), 1, "cursor on the post-split 'aa' line");

    let mut g = Fixture::new("aaa\nbbb\n");
    feed_ex(&mut g, "%s/a/\\r/g");
    assert_eq!(g.text(), "\n\n\n\nbbb\n");
    assert_eq!(g.line(), 3, "cursor on the 4th line (last split slot)");

    // middle-line replacement: line counts above stay unchanged, splitting happens only
    // inside the replaced line; the cursor lands on the last line of the post-split
    // replaced region (vim 9.1 probe: line(".") = 4)
    let mut h = Fixture::new("keep1\naba\nkeep2\n");
    feed_ex(&mut h, "2s/a/\\r/g");
    assert_eq!(h.text(), "keep1\n\nb\n\nkeep2\n");
    assert_eq!(h.line(), 3);
}

/// NUL bytes (the `\n` replacement) break no addressing math and match vim's byte shape.
#[test]
fn substitute_nul_replacement_keeps_addressing() {
    let mut f = Fixture::new("foo bar");
    feed_ex(&mut f, "s/o/X\\nY/");
    assert_eq!(f.text(), "fX\u{0}Yo bar");
    // cursor/marks/highlights stay addressable on text containing NUL
    f.feed(["g", "g", "m", "a"]);
    f.feed(["/", "Y"]);
    f.feed_raw(Key::enter());
    assert_addressable(&f, "nul-replacement");
    // vim 9.1 byte probe: fX 00 59 6f 20 62 61 72
    assert!(f.text().contains('\u{0}'));
}

/// Targeted: search-motion on CJK matches — spans and selection extension must both land
/// on char boundaries. (round21's semantics are vim-verified; this pins the multibyte
/// shape.)
#[test]
fn search_motion_multibyte_spans_stay_on_char_boundaries() {
    // operator surface: d/文<CR> deletes up to the byte start of 「文」
    let mut f = Fixture::new("中中 文字\n尾\n");
    f.feed(["d", "/"]);
    for c in "文".chars() {
        f.feed_raw(Key::char(c));
    }
    f.feed_raw(Key::enter());
    let text = f.text();
    assert!(
        text.is_char_boundary(f.vim.cursor.offset),
        "cursor mid-char after d/文: {} in {text:?}",
        f.vim.cursor.offset
    );
    // vim semantics: d/pat deletes [cursor, match_start) — "中中 " gets deleted
    assert_eq!(text, "文字\n尾\n");

    // visual surface: v?中<CR> extends upwards, selection endpoints addressable
    let mut g = Fixture::new("上\n中文下\n");
    g.feed(["j"]);
    g.feed(["l"]);
    g.feed(["v", "?"]);
    for c in "中".chars() {
        g.feed_raw(Key::char(c));
    }
    g.feed_raw(Key::enter());
    let text2 = g.text();
    let cur = g.vim.cursor.offset;
    assert!(
        text2.is_char_boundary(cur),
        "v?中 cursor {cur} mid-char in {text2:?}"
    );
    assert!(matches!(g.vim.mode(), vimcore::mode::Mode::Visual { .. }));
}

/// `&` replay of a splitting substitution: the stored form is the raw pre-escape line, so
/// replay must split again (but not double-split).
#[test]
fn ampersand_repeats_splitting_substitute_once() {
    // & ignores the stored range (vim 9.1 probe: :1s + move + & acts on the current
    // line), and replays exactly once (already-split lines stay put)
    let mut f = Fixture::new("aba\naba\n");
    feed_ex(&mut f, "1s/a/\\r/");
    assert_eq!(f.text(), "\nba\naba\n");
    f.feed(["j", "j"]); // to line 3 (aba)
    f.feed(["&"]);
    assert_eq!(f.text(), "\nba\n\nba\n");
    // bare `:s` replay: the typed range takes effect (the stored "1" is stripped) — the
    // old engine stored the range and replayed it per line, `&` re-printed the absolute
    // line number, and a bare :s after a range-carrying store hit E492 outright.
    // :2s = 1-based line 2 (0-based 1, "ba") → split at the first a
    let mut g = Fixture::new("aba\naba\n");
    feed_ex(&mut g, "1s/a/\\r/");
    feed_ex(&mut g, "2s");
    assert_eq!(g.text(), "\nb\n\naba\n");
}
