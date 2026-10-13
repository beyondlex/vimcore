//! Round-8 extended fuzz: deterministic multi-seed bombardment covering the paths fixed this round and new operators.
//!
//! Delta relative to `fuzz_round7`:
//! - key table completes this round's changed paths: `gq}`/`gw`/block-selection `U`/`u`/`~`,
//!   `Vp`/`vp`, `&`, illegal `:s`, macros recording+replaying Ex commands (`qa:xq@a`), bare
//!   line number `:2`, `g;`/`g,`;
//! - two invariants added: both ends of the live visual selection are addressable; the
//!   selection rebuilt by `gv` is addressable;
//! - host clicks injected (`set_cursor_offset` random landing points, incl. non-boundary offsets).

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
    "gq}", "gqq", "gww", "gqgq", "v$gq", "<C-v>jU", "<C-v>ju", "<C-v>j~", "Vp", "vp", "VP", "&",
    ":s/(/x/", ":2", ":noh", "qa", "q", "@a", ":", "g;", "g,",
    // Existing coverage (round7 key-table excerpt, keeps regression pressure)
    "h", "j", "k", "l", "w", "b", "e", "0", "$", "^", "g", "G", "d", "c", "y", "p", "P", "x", "X",
    "s", "S", "D", "C", "r", "a", "i", "o", "O", "v", "V", "<C-v>", "u", "<Esc>", "J", "gJ", ">",
    "<", "gu", "gU", "g~", "~", "f", "t", "F", "T", ";", "%", "n", "N", "*", "#", "iw", "aw",
    "i\"", "a\"", "i(", "a(", "it", "ip", "ap", "dd", "dw", "yy", "cc", "cw", "gg", "zz", "m", "`",
    "'", ".", "<C-a>", "<C-x>", "<C-r>", "<C-o>", "<C-i>", "1", "3", "/", "?", "<CR>", ":", ":d a",
    ":1,2j", ":j!", ":%d", ":$d", ":sort iu", ":sort!", ":s/x/y/", ":s//y/", ":marks", ":reg",
    "<Del>", "<C-c>", "gv", "gn", "gN", "R", "gi", "gI", "\"a", "\"1p", "\"_dd",
];

/// Verify an offset is addressable in `text` (a char boundary or the end).
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} not addressable in {text:?}"
    );
}

#[test]
fn fuzz_round8_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
        "x\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "aaaa bb cc ddd\nee ff\n\ngg hh ii jj kk ll\n",
        "abc\n ghi\ntail\n",
        "aBc\ndEf\nGhI\n",
        "你好, world 123 -45\n#tag\"",
        "a;b|c,d<e>f?g",
        "\n\n\n",
        "0x1f 077 0b101 -99\n42 0099",
        "one two three\nfour five\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0xD00D_8BAD ^ seed.wrapping_mul(0x9E37_79B9);
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
                // Both ends of the live selection and the gv-rebuilt selection are addressable
                if let Some((a, c, _)) = f.vim.visual_selection() {
                    assert_addressable("visual.anchor", a, &text, &ctx);
                    assert_addressable("visual.cursor", c, &text, &ctx);
                }
            }
        }
    }
}
