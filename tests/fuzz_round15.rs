//! Round-15 fuzz: key-stream barrage over the paths touched by this round's fixes.
//!
//! Key-table increments over round14:
//! - substitute replacement escapes (`\/`, `\\`, unknown escapes) + `&` / bare `:s` replay
//!   sharing the split_escaped reconstruction;
//! - out-of-range Ex addresses (huge values + offset chains E1247, legal negative offsets);
//! - `}`/`{`/`)`/`(` paragraph/sentence motions on whitespace-only buffers: new boundaries;
//! - `J` at empty-line seams (the widened no-space condition);
//! - `ci"` falling back to the first quote pair when the cursor is past the last pair;
//! - the `N%` ceil formula + `1%` file percentage rewrite (GoToFilePercent);
//! - `"_yy` (unnamed no longer polluted), the `""` prefix;
//! - `3R` count-repeat (the exit-overwrite path);
//! - redo-register (the `.` increment after `"1P`);
//! - the canonical `<C-h>` ctrl form (prompt/insert) and visual `'a`/`` `a ``.
//!
//! The invariants carry over from round13/14: cursor/mark/`last_visual`/live selection/
//! `last_matches` are all addressable; huge counts must terminate within a bounded time (the
//! budget is the implicit assertion).

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 60 rounds × 120 steps ≈ 115k steps (same budget as round13/14)
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // This round's fix surface: substitute escapes and replay
    ":s/a/b\\/c/",
    ":s/a/b\\\\c/<CR>",
    ":s/x/q\\z/<CR>",
    "&",
    ":s<CR>",
    ":%s/o/0\\/1/<CR>",
    ":&<CR>",
    // This round's fix surface: out-of-range addresses and offset chains
    ":1+99999999999999999999d",
    ":5-99999999999999999999d<CR>",
    ":2+3-1d<CR>",
    ":.+99d<CR>",
    ":1000000d<CR>",
    ":0d<CR>",
    ":-1d<CR>",
    // This round's fix surface: paragraph/sentence boundaries + J on empty lines
    "}",
    "{",
    "d}",
    "d{",
    "y}",
    "2}",
    "d2}",
    ")",
    "(",
    "d)",
    "J",
    "2J",
    "3J",
    "gJ",
    // This round's fix surface: quote object fallback
    "ci\"",
    "ca\"",
    "di\"",
    "yi\"",
    // This round's fix surface: N% / 1%
    "1%",
    "2%",
    "50%",
    "30%",
    "101%",
    "100%",
    "%",
    "d2%",
    "y50%",
    // This round's fix surface: blackhole yank / Replace repeat / redo-register
    "\"_yy",
    "\"_yiw",
    "\"\"p",
    "3R",
    "3Rab",
    "2Rxy",
    "R",
    "\"1P",
    "\"2P",
    ".",
    "2.",
    "dd",
    "dw",
    // This round's fix surface: the C-h form / visual mark jumps
    "<C-h>",
    "i<C-h>",
    "R<C-h>",
    "V'a",
    "v`a",
    "'a",
    "`a",
    "mA",
    // Excerpt of the round14 key table (keeping the existing pressure surface)
    "t2",
    "t2;",
    "2t3",
    ";",
    "dtx",
    "\"ax",
    "\"ayy",
    "<C-v>jlo",
    "<C-v>jlO",
    "VjO",
    "3:",
    "3:<CR>",
    "2/x<Esc>",
    "3/foo<CR>",
    ":1,2d 3x",
    ":y a3",
    ":%s/x//n",
    ":s/a\\/b/x/",
    ":setlocal ts=8",
    ":bne",
    "gv",
    "gvd",
    "@a",
    "@:",
    "x",
    "p",
    "yy",
    ":5+2+1d",
    ":3,4s",
    "2*",
    "yaw",
    "d-",
    "ci\"",
    "d'a",
    "ma",
    ":%s/x/y/",
    ":%j",
    "<BS>",
    "<Left>",
    "<Right>",
    "/x<CR>",
    "?中<CR>",
    "2i",
    "3o",
    ":set ts=8",
    ":set noic",
    ":set scs",
    ":setlocal ts?",
    ":brew",
    ":brewi",
    ">>",
    "guw",
    "<C-r>",
    "yiw",
    ":sort u",
    ":'<,'>d",
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
    "gJ",
    "~",
    "f",
    "n",
    "N",
    "*",
    "iw",
    "aw",
    "cw",
    "gg",
    "<C-a>",
    "<C-x>",
    "<C-o>",
    "<C-i>",
    "<CR>",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
];

fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} is not addressable in {text:?}"
    );
}

fn check_invariants(f: &Fixture, text: &str, ctx: &str) {
    assert_addressable("cursor", f.vim.cursor_offset(), text, ctx);
    for (name, off) in f.vim.marks.items() {
        assert_addressable(&format!("mark {name}"), off, text, ctx);
    }
    if let Some((lo, hi, _)) = f.vim.marks.last_visual {
        assert_addressable("last_visual.lo", lo, text, ctx);
        assert_addressable("last_visual.hi", hi, text, ctx);
    }
    if let Some((a, c, _)) = f.vim.visual_selection() {
        let ctx = format!("{ctx} mode {:?}", f.vim.mode());
        assert_addressable("visual.anchor", a, text, &ctx);
        assert_addressable("visual.cursor", c, text, &ctx);
    }
    // last_matches is not checked: the harness calls set_hlsearch_live_update(false) throughout,
    // so the match cache may expire between two rescans (the generation mechanism guarantees a
    // rescan before n/N consumes it — the same constraint as round13/14)
}

#[test]
fn fuzz_round15_holds_invariants() {
    // The buffer pool is designed around this round's fix surface: paragraph boundaries of
    // whitespace-only lines, empty-line seams, multi-line quotes, a line count the huge offset
    // addresses can act on, multi-byte text
    let buffers = [
        "",
        "a",
        "中",
        "aaa\n   \nbbb\n",
        "\ndef\n",
        "def\n\n",
        "say \"hi\" then \"bye\" end\n",
        "abcdefghij\n",
        "one\ntwo\nthree\ntail\n",
        "a/b tail\nold here\n",
        "para one\n\npara two\n   \ntail\n",
        "x y z\nmatch me and me\nfoo !\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "éA→ ç\nｱｲｳ\n\tindent",
        "\n\n\n",
        "L0\nL1\nL2\nL3\nL4\nL5\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x15AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            // Half of the seeds carry `nnoremap j gj` (the round14 precedent): the pending-key
            // guard and the mapping-expansion replay stay under barrage
            if seed % 2 == 0 {
                f.vim.keymaps.map_str_noremap(
                    vimcore::keymap::ModeClass::Normal,
                    "j",
                    "gj",
                    true,
                );
            }
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            f.vim.set_hlsearch_live_update(false);
            for step in 0..STEPS {
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                    if seed % 2 == 0 {
                        f.vim.keymaps.map_str_noremap(
                            vimcore::keymap::ModeClass::Normal,
                            "j",
                            "gj",
                            true,
                        );
                    }
                }
                let idx = (fuzz_xorshift(&mut state) as usize) % KEYS.len();
                let seq = vimcore::key::parse_key_sequence(KEYS[idx]);
                for key in seq {
                    let mut ctx = Ctx {
                        buf: &mut f.buf,
                        host: &mut f.host,
                    };
                    let _ = f.vim.handle_key(&mut ctx, key);
                }
                check_invariants(&f, &f.text(), &format!("seed{seed} r{round} s{step} k{}", KEYS[idx]));
            }
        }
    }
}

/// Random barrage with huge counts: `e`/`}`/`)`/`b`/`%` combined with a random count must
/// terminate within the time budget (the old engine needed 10^9 whole-word scans for a single
/// 999999999e).
#[test]
fn fuzz_round15_huge_counts_terminate() {
    let buffers = ["foo\n", "foo   bar  baz\n   \n\nnext\n", "abc\ndef\n"];
    let mut state: u64 = 0xBEE5_2026;
    let start = std::time::Instant::now();
    for _ in 0..300 {
        let initial = buffers[(fuzz_xorshift(&mut state) as usize) % buffers.len()];
        let mut f = common::Fixture::new(initial);
        let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
        f.vim.set_cursor_offset(&f.buf, cur);
        let count = (fuzz_xorshift(&mut state) % 1_000_000_000) + 1;
        let count_str = count.to_string();
        let motion = ["e", "}", ")", "b", "%", "1%"][(fuzz_xorshift(&mut state) as usize) % 6];
        let mut keys: Vec<String> = count_str.chars().map(|c| c.to_string()).collect();
        keys.push(motion.to_string());
        f.feed(keys);
        let text = f.text();
        let off = f.vim.cursor_offset();
        assert!(off <= text.len());
    }
    assert!(
        start.elapsed().as_secs() < 10,
        "huge-count motions must terminate (took {:?})",
        start.elapsed()
    );
}
