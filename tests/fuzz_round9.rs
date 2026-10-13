//! Round-9 extended fuzz: deterministic multi-seed bombardment covering the paths fixed this round.
//!
//! Delta relative to `fuzz_round8`:
//! - key table completes this round's changed paths: insert `<C-w>` (line-head join /
//!   block-session blocking), `3D`/`3C` (line counts), `r<CR>` (line-split landing),
//!   `<Tab>` (display-column alignment), `:substitute/x/y/` (full spelling + missing
//!   replacement), `:2j 3` (join with count), `:bfirst`/`:blast`;
//! - buffer pool adds CJK and 4 equal-length lines (the shapes of Tab alignment and
//!   D/C count saturation).
//!
//! Invariants carried over from round8: cursor/all marks/`last_visual`/live
//! selection/`gv`-rebuilt-selection offsets all stay addressable (char boundary or
//! buffer end).

mod common;

use vimcore::key::parse_key_sequence;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

const KEYS: &[&str] = &[
    // Paths fixed this round
    "<C-w>",
    "3D",
    "3C",
    "99D",
    "r<CR>",
    "<Tab>",
    ":substitute/x/y/",
    ":s/x",
    ":2j 3",
    ":bfirst",
    ":blast",
    "Vgq",
    "gqq",
    // Existing coverage (round8 key-table excerpt, keeps regression pressure)
    "gq}",
    "v$gq",
    "<C-v>jU",
    "Vp",
    "vp",
    "&",
    ":s/(/x/",
    ":2",
    ":noh",
    "qa",
    "q",
    "@a",
    ":",
    "g;",
    "g,",
    "h",
    "j",
    "k",
    "l",
    "w",
    "b",
    "e",
    "0",
    "$",
    "^",
    "g",
    "G",
    "d",
    "c",
    "y",
    "p",
    "P",
    "x",
    "X",
    "s",
    "S",
    "D",
    "C",
    "r",
    "a",
    "i",
    "o",
    "O",
    "v",
    "V",
    "<C-v>",
    "u",
    "<Esc>",
    "J",
    "gJ",
    ">",
    "<",
    "gu",
    "gU",
    "g~",
    "~",
    "f",
    "t",
    ";",
    "%",
    "n",
    "N",
    "*",
    "#",
    "iw",
    "aw",
    "i(",
    "a(",
    "it",
    "ip",
    "ap",
    "dd",
    "dw",
    "yy",
    "cc",
    "cw",
    "gg",
    "zz",
    "m",
    "`",
    "'",
    ".",
    "<C-a>",
    "<C-x>",
    "<C-r>",
    "<C-o>",
    "<C-i>",
    "1",
    "3",
    "/",
    "?",
    "<CR>",
    ":d a",
    ":1,2j",
    ":j!",
    ":%d",
    ":$d",
    ":sort iu",
    ":sort!",
    ":s/x/y/",
    ":s//y/",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
    "gv",
    "gn",
    "gN",
    "R",
    "gi",
    "gI",
    "\"a",
    "\"1p",
    "\"_dd",
];

/// Verify an offset is addressable in `text` (a char boundary or the end).
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} not addressable in {text:?}"
    );
}

#[test]
fn fuzz_round9_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
        "aaaa\nbbbb\ncccc\ndddd",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "aBc\ndEf\nGhI\n",
        "0x1f 077 0b101 -99\n42 0099",
        "one two three\nfour five\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0x9E37_79C9 ^ seed.wrapping_mul(0x85EB_CA6B);
        for round in 0..100 {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            for step in 0..160 {
                let k = KEYS[(fuzz_xorshift(&mut state) as usize) % KEYS.len()];
                for key in parse_key_sequence(k) {
                    f.feed_raw(key);
                }
                // IME typing path: randomly injected in insert/replace mode (incl. multiline text)
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    f.type_text("t中x");
                }
                // Host click: random landing point (deliberately incl. non-char-boundaries — the entry must floor)
                if fuzz_xorshift(&mut state).is_multiple_of(11) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
                }
                let ctx = format!("seed {seed} round {round} step {step} key {k}");
                let text = f.text();
                assert_addressable("cursor", f.vim.cursor_offset(), &text, &ctx);
                for (name, off) in f.vim.marks.items() {
                    assert_addressable(&format!("mark {name}"), off, &text, &ctx);
                }
                if let Some((lo, hi, _)) = f.vim.marks.last_visual {
                    assert_addressable("last_visual.lo", lo, &text, &ctx);
                    assert_addressable("last_visual.hi", hi, &text, &ctx);
                }
                if let Some((a, c, _)) = f.vim.visual_selection() {
                    assert_addressable("visual.anchor", a, &text, &ctx);
                    assert_addressable("visual.cursor", c, &text, &ctx);
                }
            }
        }
    }
}
