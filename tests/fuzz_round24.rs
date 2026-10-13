//! Round-24 fuzz: full-keyboard barrage over grapheme-cluster / wide-char corpora.
//!
//! Background: round24 fixed six paths where "the cursor lands in the middle of a
//! grapheme cluster" (Esc step-back, clamp_cursor, display-column clamping, `$`, `g_`,
//! the till landing point of f/t/F/T, the charwise p landing, insert BS, and the
//! zero-width skip of first_non_blank). The fixes jointly assert an invariant that fuzz
//! had never checked before:
//!
//!   **in normal mode the cursor never rests on a char of display width 0** (combining
//!   marks / VS16 / ZWJ are all cluster continuations — parking the cursor there means
//!   `x` splits the cluster).
//!
//! Corpus coverage: decomposed combining chars (é = e+U+0301), VS16 (#\u{FE0F}), the ZWJ
//! family (👨‍👩‍👧 + skin-tone modifiers), CJK and the ideographic space (U+3000), and
//! clusters mid-line / at line end / on a line of their own.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

/// Simple LCG: deterministic randomness (no external deps).
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

/// Grapheme-cluster corpus lines: clusters at mid-line / line end / line start, including
/// zero-width continuation chars.
const CLUSTER_CORPUS: &[&str] = &[
    // decomposed é at line end: landing points for Esc step-back / $ / g_
    "ab\u{0301}\n",
    "ab\u{0301}\ncd\n",
    // VS16 at line end
    "#\u{FE0F}\nnext\n",
    // ZWJ family at line end (family emoji, one cluster of 5 chars)
    "x\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\n",
    // cluster mid-line: landing points of tx/ta, crossing by h/l
    "a\u{0301}bc\n",
    "xa\u{0301}b\u{0301}c\n",
    // combining char at line start: landing points of ^ / gg / -
    "\u{0301}abc\n",
    // ideographic space: skipping by w/W, the tail of g_
    "a\u{3000}b\n",
    "ab\u{3000}\n",
    // mixed: CJK + clusters + ideographic space
    "中a\u{0301}文\u{3000}b\n",
    // skin-tone modifier (U+1F3FB is also a continuation char)
    "\u{1F44D}\u{1F3FB}x\n",
    // cluster alone on a line
    "one\na\u{0301}\nthree\n",
];

/// Key pool: covering motion/edit/char-arg/visual/Ex entry.
const KEY_POOL: &[&str] = &[
    "h", "l", "j", "k", "w", "b", "e", "0", "$", "^", "g", "_", "x", "X", "p", "P", "d", "d", "c",
    "c", "y", "y", "r", "x", "r", "<CR>", "~", "g", "u", "u", "g", "U", "U", "f", "x", "F", "x",
    "t", "x", "T", "x", ";", ",", "i", "a", "o", "<Esc>", "<BS>", "<C-w>", "<C-u>", "v", "V",
    "<C-v>", "d", "y", ">", "<", "g", "q", "q", "q", "J", "d", "p", "*", "n", "N", "1", "2", "3",
    ":", "s/x/Y/g\n", "/", "a\n", "<CR>", "u", ".", "g", "v", "d", "a", "w", "c", "i", "w",
    "d", "t", "x", "y", "t", "x",
];

/// In normal mode the cursor must rest on a char of display width ≥ 1 (a cluster start)
/// and on a char boundary.
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
            // the only legit exception: a lone combining mark whose base char was
            // deleted (it forms its own cluster and the line has no base char left to
            // attach to — so only the line-start shape is checked)
            let ls = f.buf.line_start(f.buf.offset_to_line(cur));
            assert!(
                cur == ls,
                "{context}: cursor parked on zero-width continuation char U+{:04X} (offset {cur}, line start {ls}) in {text:?}",
                c as u32
            );
        }
    }
    // the line-addressing contract is upheld along the way
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
fn fuzz_cluster_corpus_never_parks_cursor_on_zero_width_char() {
    let mut rng = Rng(0x24_24_24_24);
    for (ci, corpus) in CLUSTER_CORPUS.iter().enumerate() {
        for round in 0..40 {
            let mut f = Fixture::new(corpus);
            let context = format!("corpus#{ci} {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for step in 0..100 {
                let key = KEY_POOL[rng.below(KEY_POOL.len())];
                f.feed([key]);
                log.push(key.to_owned());
                // skip the cursor check while the prompt is open (the cmdline cursor is a
                // host-rendered insert bar with different semantics); in insert mode the
                // cursor may sit past line end / after a cluster (while typing)
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(&f, &format!("{context} step#{step} keys={log:?}"));
                }
            }
            // wrap-up: after Esc parks, the invariant must hold
            f.feed(["<Esc>", "<Esc>"]);
            if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                assert_cursor_on_visible_char(&f, &format!("{context} final keys={log:?}"));
            }
        }
    }
}

/// Targeted: visual operators and macro replay never panic on the cluster corpus;
/// selections stay addressable.
#[test]
fn fuzz_visual_and_macro_on_cluster_corpus() {
    let mut rng = Rng(0x24_C1A5_BEEF);
    for corpus in CLUSTER_CORPUS.iter() {
        for round in 0..25 {
            let mut f = Fixture::new(corpus);
            let context = format!("vis/macro {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for _ in 0..60 {
                match rng.below(10) {
                    0..=5 => {
                        let key = KEY_POOL[rng.below(KEY_POOL.len())];
                        f.feed([key]);
                        log.push(key.to_owned());
                    }
                    6 => {
                        f.feed(["v"]);
                        f.feed(["i", "w"]);
                        log.push("v iw".to_owned());
                    }
                    7 => {
                        f.feed(["q", "a", "x", "j", "q"]);
                        f.feed(["@", "a"]);
                        log.push("qaxjq @a".to_owned());
                    }
                    8 => {
                        f.feed(["g", "v"]);
                        log.push("gv".to_owned());
                    }
                    _ => {
                        f.feed(["<C-v>"]);
                        log.push("<C-v>".to_owned());
                    }
                }
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(&f, &format!("{context} keys={log:?}"));
                }
            }
        }
    }
}

/// Targeted regression: `^` on a line starting with a combining char jumps to the first
/// visible char (the zero-width skip of first_non_blank).
#[test]
fn caret_skips_leading_combining_mark() {
    // U+0301 takes 2 bytes: 'a' sits at offset 2
    let f = common::edit("\u{0301}abc\n", 0, 4, &["^"]);
    assert_eq!(f.cursor(), 2, "^ should land on the first visible char a");
    // mark right after the indent: " \u{0301}x" → lands on x (space@0, mark@1..3, x@3)
    let f = common::edit(" \u{0301}x\n", 0, 0, &["^"]);
    assert_eq!(f.cursor(), 3, "^ should skip the mark and land on x");
}

/// Targeted regression: inside a block-insert session, IME cluster text + backspace keep
/// the exit-side delta invariant.
#[test]
fn block_session_cluster_typing_and_backspace() {
    use std::iter;
    let mut f = Fixture::new("ab\ncd\nef\n");
    // block-select the first column of three lines, I enters block insert
    f.feed(["<C-v>", "2", "j", "I"]);
    f.type_text("x\u{0301}"); // cluster text
    f.feed_raw(vimcore::key::Key::named("backspace")); // deletes the whole cluster
    f.type_text("Z");
    f.feed(["<Esc>"]);
    let text = f.text();
    assert!(
        text.matches('Z').count() == 3,
        "each of the three lines gets one Z (cluster BS must not break the copy invariant): {text:?}"
    );
    let _ = iter::once('x');
}
