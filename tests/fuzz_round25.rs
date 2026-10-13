//! Round-25 fuzz: full key-stream barrage over the line-boundary x grapheme-cluster corpus
//! (the GB4 hard-boundary corpus).
//!
//! Background: round25 fixed three classes of paths where "a grapheme cluster crossed `\n`" —
//! `w`'s whitespace skip not absorbing continuation chars, `prev/next_grapheme_offset` gluing
//! a lone mark or ZWJ at the line start/end into the `\n`'s cluster, and insert `<Del>` deleting
//! only the base char. Together these fixes assert two invariants that had never been checked
//! simultaneously before:
//!
//!   1. **the round24 invariant**: a normal cursor does not land on a zero-width char (except
//!      the line-start lone-cluster form);
//!   2. **new this round**: a normal cursor does not land on `\n` — unless the line is empty
//!      (for an empty line the "line start" IS the `\n` position, with no char to park on). The
//!      old GB4 bug's symptom was exactly clamp_cursor parking the cursor on the **previous**
//!      line's newline position, so pressing `x` once joined the lines.
//!
//! The corpus covers forms round24 lacked: a lone mark alone on a line / at line end, ZWJ at
//! line end and line start, a space followed by a mark (`w`'s fixed surface), and the various
//! arrangements of clusters adjacent to `\n`.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

/// Simple LCG: deterministic randomness (no external dependencies).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Line-boundary x cluster corpus: lone continuation chars adjacent to `\n` in various
/// arrangements (the GB4 fix surface).
const BOUNDARY_CORPUS: &[&str] = &[
    // Lone mark at line end (before the fix, $/clamp crossed into the previous line)
    "ab\n\u{0301}",
    "ab\n\u{0301}\ncd\n",
    // Lone mark at line start (interaction of ^/w with the previous line's boundary)
    "\u{0301}abc\n",
    "ab\n\u{0301}cd",
    // ZWJ at line end / line start (the GB4 fix surface of the ZWJ branch)
    "ab\u{200D}\ncd",
    "ab\n\u{200D}cd\n",
    "x\u{200D}\n",
    // Space followed by a mark (the fix surface of w's whitespace skip)
    "a \u{0301}bc",
    "  \u{0301}abc\nde\u{200D}\nfg\n",
    // Clusters adjacent to empty lines
    "\n\u{0301}\nabc\n",
    "one\n\n\u{0301}\n\nthree\n",
    // Fullwidth space + lone mark mixed
    "a\u{3000}\u{0301}b\n\u{0301}\u{3000}c\n",
    // Skin-tone modifier at line end
    "\u{1F44D}\u{1F3FB}\n\u{1F44D}\nx\n",
    // VS16 at line end + start of the next line's cluster
    "#\u{FE0F}\n\u{0301}xy\n",
];

/// Key pool: motions/editing/count args/visual/Ex entry (following round24's shape).
const KEY_POOL: &[&str] = &[
    "h", "l", "j", "k", "w", "b", "e", "0", "$", "^", "g", "_", "x", "X", "p", "P", "d", "d", "c",
    "c", "y", "y", "r", "x", "r", "<CR>", "~", "g", "u", "u", "g", "U", "U", "f", "x", "F", "x",
    "t", "x", "T", "x", ";", ",", "i", "a", "o", "<Esc>", "<BS>", "<C-w>", "<C-u>", "v", "V",
    "<C-v>", "d", "y", ">", "<", "g", "q", "q", "q", "J", "d", "p", "*", "n", "N", "1", "2", "3",
    ":", "s/x/Y/g\n", "/", "a\n", "<CR>", "u", ".", "g", "v", "d", "a", "w", "c", "i", "w",
    "d", "t", "x", "y", "t", "x", "G", "g", "g", "%", "D", "C", "S", "Y", "z", "z", "m", "a",
    "'", "a", "`", "a", "g", "n", "g", "N", "d", "g", "n", "<C-a>", "<C-x>", "g", "J", "g",
    "I", "R", "x", "<Del>", "g", "e", "g", "E", "W", "B", "E", "{", "}", "(", ")", "|",
];

/// A normal-mode cursor must land on a char of display width >= 1 (a cluster start) and on a
/// char boundary; and it must never land on `\n` — the only exception is the line start of an
/// empty line (= the `\n` position, with no char to park on).
fn assert_cursor_on_visible_char(f: &Fixture, context: &str) {
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= text.len() && text.is_char_boundary(cur),
        "{context}: cursor {cur} is not a char boundary (len={}) in {text:?}",
        text.len()
    );
    if let Some(c) = text[cur..].chars().next() {
        let width = vimcore::buffer::char_display_width(c);
        if width == 0 {
            // The only legal exception: a lone mark forming its own cluster at the line start
            let ls = f.buf.line_start(f.buf.offset_to_line(cur));
            assert!(
                cur == ls,
                "{context}: cursor lands on a zero-width continuation char U+{:04X} (offset {cur}, line start {ls}) in {text:?}",
                c as u32
            );
        }
        if c == '\n' {
            // New this round: the cursor must not sit on \n unless it is the start of an empty
            // line (no char to park on)
            let (ls, le) = {
                let line = f.buf.offset_to_line(cur);
                (f.buf.line_start(line), f.buf.line_end(line))
            };
            assert!(
                cur == ls && ls == le,
                "{context}: cursor lands on \\n (offset {cur}, line start {ls}, line end {le}) in {text:?} — GB4 fix-surface regression"
            );
        }
    }
    // The line-addressing contract is maintained along the way
    let buf = &f.buf;
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= text.len(),
            "{context}: line {line} range {s}..{e} out of bounds (len={})",
            text.len()
        );
    }
}

#[test]
fn fuzz_boundary_corpus_never_parks_on_newline_or_mark() {
    let mut rng = Rng(0x25_25_25_25);
    for (ci, corpus) in BOUNDARY_CORPUS.iter().enumerate() {
        for round in 0..60 {
            let mut f = Fixture::new(corpus);
            let context = format!("corpus#{ci} {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for step in 0..150 {
                let key = KEY_POOL[rng.below(KEY_POOL.len())];
                f.feed([key]);
                log.push(key.to_owned());
                // Skip the cursor check while a prompt is open (the cmdline cursor is the
                // host-rendered input bar, with different semantics); in insert mode the
                // cursor may sit at a line end / after a cluster (while typing)
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(
                        &f,
                        &format!("{context} step#{step} keys={log:?}"),
                    );
                }
            }
            // Wrap-up: after Esc returns to normal, the invariants must hold
            f.feed(["<Esc>", "<Esc>"]);
            if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                assert_cursor_on_visible_char(&f, &format!("{context} final keys={log:?}"));
            }
        }
    }
}

/// Targeted: operators/objects/macros interleaved on the GB4 fix surface must not panic and the
/// buffer must stay addressable.
#[test]
fn fuzz_boundary_corpus_operators_and_macros() {
    let mut rng = Rng(0x25_B04D_C0DE);
    for corpus in BOUNDARY_CORPUS.iter() {
        for round in 0..30 {
            let mut f = Fixture::new(corpus);
            let context = format!("op/macro {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for _ in 0..60 {
                match rng.below(12) {
                    0..=6 => {
                        let key = KEY_POOL[rng.below(KEY_POOL.len())];
                        f.feed([key]);
                        log.push(key.to_owned());
                    }
                    7 => {
                        f.feed(["v", "i", "w"]);
                        log.push("v iw".to_owned());
                    }
                    8 => {
                        f.feed(["y", "y", "p"]);
                        log.push("yyp".to_owned());
                    }
                    9 => {
                        f.feed(["q", "a", "x", "q", "@", "a"]);
                        log.push("qaxq@a".to_owned());
                    }
                    10 => {
                        // IME-style text landing (cluster text inside insert mode)
                        if matches!(f.vim.mode(), vimcore::mode::Mode::Insert) {
                            f.type_text("\u{0301}");
                            log.push("type mark".to_owned());
                        }
                    }
                    _ => {
                        f.feed(["g", "b", "d"]); // no gb command exists: the junk-key path
                        log.push("gbd".to_owned());
                    }
                }
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(
                        &f,
                        &format!("{context} keys={log:?}"),
                    );
                }
            }
            f.feed(["<Esc>", "<Esc>"]);
            if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                assert_cursor_on_visible_char(&f, &format!("{context} final keys={log:?}"));
            }
        }
    }
}
