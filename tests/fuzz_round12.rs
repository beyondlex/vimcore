//! Round-12 extended fuzz: covers the paths fixed this round.
//!
//! Key-table delta relative to round11:
//! - new Ex-range semantics: offset chains (`:5+2+1d`), multibyte mark names
//!   (`:'中d`, `:,'中d`), equal-address ranges (`:2,2j`), explicit count
//!   (`:1,2d 1`), special registers (`:d _`, `:d +`), ranged-substitute replay (`:3,4s`);
//! - search-jump counts (`2*`, `2#`, `3gN`, `d2gn`, `c2gN`) and whitespace objects
//!   (`yaw`, `caw`, `d2aw`);
//! - edge motions (`d-`, `d+`, `2-`, `3+`) and empty-line `$`/`g_` operators.
//!
//! Invariants carried over from round11: cursor/mark/`last_visual`/live
//! selection/`last_matches` must all stay addressable.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
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
    // New this round: Ex-range offset chains and multibyte marks (fixed paths)
    ":5+2+1d",
    ":3-1d",
    ":2+1",
    ":'中d",
    ":,'中d",
    ":2,'中y",
    ":'a,+2d",
    // New this round: equal ranges / explicit count / special registers (fixed paths)
    ":2,2j",
    ":3,3d",
    ":2,2y",
    ":1,2d 1",
    ":1,2y 1",
    ":2j 3",
    ":d _",
    ":d +",
    ":y a 2",
    ":3,4s",
    // New this round: search counts and whitespace objects (fixed paths)
    "2*",
    "2#",
    "3gN",
    "2gn",
    "d2gn",
    "c2gN",
    "yaw",
    "caw",
    "d2aw",
    "y2aw",
    // New this round: edge motions and empty-line operators (fixed paths)
    "d-",
    "d+",
    "2-",
    "3+",
    "d$",
    "dg_",
    "d2$",
    "y$",
    "c$",
    // New this round: quote objects between strings (fixed paths)
    "ci\"",
    "ca\"",
    "di\"",
    // round11 key-table excerpt (keeps regression pressure)
    "d'a",
    "ma",
    "mb",
    "'a",
    ":2,5",
    ":1,2,3d",
    ":,3d",
    ":&",
    ":s",
    ":%s/x/y/",
    ":%j",
    "R",
    "<BS>",
    "/x<CR>",
    "?a<CR>",
    "/中<CR>",
    "2i",
    "3o",
    ":set ts=8",
    ":set noic",
    ":set ic",
    ":set hls!",
    ">>",
    "guw",
    "Vgq",
    "<C-r>",
    "yiw",
    ":d",
    ":sort u",
    ":'<,'>d",
    "(",
    "{",
    "ge",
    "H",
    "+",
    "-",
    "<C-e>",
    "Y",
    "i\"",
    "a\"",
    "i'",
    "a'",
    "iW",
    "aW",
    "is",
    "as",
    "i[",
    "a[",
    "i{",
    "a{",
    "daw",
    "das",
    "dap",
    "<C-w>",
    "3D",
    "<C-v>jU",
    "<C-v>jI",
    "Vp",
    ":noh",
    "qa",
    "q",
    "@a",
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
    "G",
    "d",
    "c",
    "y",
    "p",
    "P",
    "x",
    "s",
    "D",
    "C",
    "r",
    "a",
    "i",
    "v",
    "V",
    "<C-v>",
    "u",
    "<Esc>",
    "J",
    "gJ",
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
    "dd",
    "dw",
    "yy",
    "cc",
    "cw",
    "gg",
    ".",
    "<C-a>",
    "<C-x>",
    "<C-o>",
    "<C-i>",
    "<CR>",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
    "gv",
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
fn fuzz_round12_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        // Multi-string lines: the pressure surface of quote objects between strings
        "say \"hi\" then \"bye\" ok\nx = \"a\" + \"b\";\n",
        // Inter-word whitespace + whitespace-only lines + true empty lines: pressure surface for aw/w/d$
        "foo   bar  baz\n   \n\nnext word\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "0x1f 077 0b101 -99\n42 0099",
        "a b a b a b\nmatch me and me\n",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "one two three\nfour five\n",
        "Do not quote 'this' or `that`, ok?\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0xDEAD_BEEF ^ seed.wrapping_mul(0x2722_0A95);
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
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    if fuzz_xorshift(&mut state).is_multiple_of(3) {
                        f.type_text("a\nb中");
                    } else {
                        f.type_text("t中x");
                    }
                }
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
                for (i, m) in f.vim.search.last_matches.iter().enumerate() {
                    assert_addressable(&format!("match{i}.start"), m.start, &text, &ctx);
                    assert_addressable(&format!("match{i}.end"), m.end, &text, &ctx);
                }
            }
        }
    }
}

/// Random-combination bombardment of Ex range arguments: addresses/offsets/separators/registers/
/// counts spliced arbitrarily; commands may only fail (E16/E20/E78/E486/bell), never panic.
#[test]
fn ex_range_garbage_never_panics() {
    let addresses = [
        "", ".", "$", "%", "0", "1", "5", "99", "'a", "'中", "'<", "'>", "''",
    ];
    let offsets = ["", "+1", "-1", "+2-1", "+99", "-99", "+99999999999999999999"];
    let seps = [",", ";"];
    let commands = ["", "d", "d _", "d +", "y", "y a", "j", "j!", "s", "sort", ">", "w", "q", "1"];
    let buffers = ["a\nb\nc\nd\ne\n", "中文\nabc\n", ""];
    let mut state: u64 = 0xCAFE_0001;
    for round in 0..600 {
        let mut line = String::from(":");
        let addr_a = addresses[(fuzz_xorshift(&mut state) as usize) % addresses.len()];
        line.push_str(addr_a);
        if fuzz_xorshift(&mut state).is_multiple_of(2) {
            line.push_str(seps[(fuzz_xorshift(&mut state) as usize) % seps.len()]);
            let addr_b = addresses[(fuzz_xorshift(&mut state) as usize) % addresses.len()];
            line.push_str(addr_b);
        }
        let off = offsets[(fuzz_xorshift(&mut state) as usize) % offsets.len()];
        line.push_str(off);
        if fuzz_xorshift(&mut state).is_multiple_of(3) {
            let off2 = offsets[(fuzz_xorshift(&mut state) as usize) % offsets.len()];
            line.push_str(off2);
        }
        let cmd = commands[(fuzz_xorshift(&mut state) as usize) % commands.len()];
        line.push_str(cmd);
        let mut f = Fixture::new(buffers[round % buffers.len()]);
        f.vim.set_cursor_offset(&f.buf, 2.min(f.text().len()));
        // feed as Ex: colon already in `line`, then each char, then CR
        for ch in line.chars().skip(1) {
            f.feed_raw(vimcore::key::Key::char(ch));
        }
        f.feed_raw(vimcore::key::Key::parse("<CR>"));
        let text = f.text();
        let ctx = format!("round {round} line {line:?}");
        assert_addressable("cursor", f.vim.cursor_offset(), &text, &ctx);
        assert!(f.buf.line_count() >= 1, "{ctx}: line count underflow");
    }
}
