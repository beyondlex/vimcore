//! 第十五轮 fuzz：覆盖本轮修复面的路径轰炸。
//!
//! 相对 round14 的增量键表：
//! - substitute replacement 转义（`\/`、`\\`、未知转义）+ `&`/裸 `:s`
//!   重放共享 split_escaped 重建；
//! - 越界 Ex 地址（巨值 + 偏移链 E1247、负偏移合法路径）；
//! - `}`/`{`/`)`/`(` 段落/句子 motion 在空白行缓冲上的新边界；
//! - 空行接缝的 `J`（免空格条件扩面）；
//! - `ci"` 光标越末对引号的第一对回退；
//! - `N%` ceil 公式 + `1%` 文件百分比改写（GoToFilePercent）；
//! - `"_yy`（unnamed 不再被污染）、`""` 前缀；
//! - `3R` count-repeat（退出覆盖路径）；
//! - redo-register（`"1P` 后的 `.` 递增）；
//! - `<C-h>` 规范 ctrl 形态（提示符/插入）与 visual `'a`/`` `a ``。
//!
//! 不变量沿用 round13/14：光标/mark/`last_visual`/live 选区/`last_matches`
//! 全部可寻址；巨 count 必须在有限时间内终止（预算即隐式断言）。

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

// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步（与 round13/14 同预算）
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // 本轮修复面：substitute 转义与重放
    ":s/a/b\\/c/",
    ":s/a/b\\\\c/<CR>",
    ":s/x/q\\z/<CR>",
    "&",
    ":s<CR>",
    ":%s/o/0\\/1/<CR>",
    ":&<CR>",
    // 本轮修复面：越界地址与偏移链
    ":1+99999999999999999999d",
    ":5-99999999999999999999d<CR>",
    ":2+3-1d<CR>",
    ":.+99d<CR>",
    ":1000000d<CR>",
    ":0d<CR>",
    ":-1d<CR>",
    // 本轮修复面：段落/句子边界 + 空行 J
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
    // 本轮修复面：quote 对象回退
    "ci\"",
    "ca\"",
    "di\"",
    "yi\"",
    // 本轮修复面：N% / 1%
    "1%",
    "2%",
    "50%",
    "30%",
    "101%",
    "100%",
    "%",
    "d2%",
    "y50%",
    // 本轮修复面：黑洞 yank / Replace repeat / redo-register
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
    // 本轮修复面：C-h 形态 / visual 标记跳转
    "<C-h>",
    "i<C-h>",
    "R<C-h>",
    "V'a",
    "v`a",
    "'a",
    "`a",
    "mA",
    // round14 键表节选（保持既有压力面）
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
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

fn addressable(off: usize, text: &str) -> bool {
    off <= text.len() && (off == text.len() || text.is_char_boundary(off))
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
    // last_matches 不检查：harness 全程 set_hlsearch_live_update(false)，
    // 匹配缓存允许在两次重扫之间过期（generation 机制保证 n/N 消费前
    // 重扫——round13/14 同款约束）
}

#[test]
fn fuzz_round15_holds_invariants() {
    // 缓冲池围绕本轮修复面设计：空白行段落边界、空行接缝、多行引号、
    // 巨偏移地址可作用的行数、多字节
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
            // 一半种子带 `nnoremap j gj`（round14 先例）：续键守卫与映射
            // 展开重放持续受轰
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

/// 巨 count 随机轰炸：`e`/`}`/`)`/`b`/`%` 与随机 count 组合必须在
/// 预算时间内终止（旧引擎 999999999e 单发即 10⁹ 次全词扫描）。
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
