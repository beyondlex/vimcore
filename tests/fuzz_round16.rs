//! 第十六轮 fuzz：覆盖本轮修复面的路径轰炸。
//!
//! 相对 round15 的增量：
//! - autoindent 空行生命周期（did_ai）：o/O 开行、`<CR>` 拆行、行首
//!   `<BS>`、cc/S 恢复缩进、Esc 退出——剥离与计数复制的次序交互；
//! - 空文本计数重复（`3o`/`2O`/`3i`/`3A`/`3R` + Esc）与导航弃权；
//! - 失败删除族（x/X/D/J/gJ/s）的响铃路径（静默回归此处会先炸不变量
//!   之外的行为，键表保证路径被踩到）；
//! - **宿主事件注入**：随机 `set_visual_range` / `set_cursor_offset` /
//!   `type_text` / `replace_range` 在任意模式（含 insert/cmdline）投递
//!   ——旧实现里 insert/cmdline 期的拖选会把模式机撕裂（round16 #4）。
//!
//! 不变量沿用 round13-15：光标/mark/`last_visual`/live 选区全部可寻址、
//! 行数 ≥ 1、模式与指示器一致。

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

// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步（与 round13-15 同预算）
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // 本轮修复面：autoindent 空行生命周期
    "o", "O", "<CR>", "<BS>", "<Esc>", "cc", "S", "cc<Esc>", "o<CR>", "o<BS>",
    // 本轮修复面：空文本计数重复 + 带文本计数重复
    "3o", "2O", "5o", "3i", "3A", "3R", "2Rab", "3o<Esc>", "2O<Esc>",
    // 本轮修复面：失败删除/接合族
    "x", "X", "D", "C", "s", "J", "2J", "3J", "gJ", "99J",
    // 本轮修复面：:sort 装饰排序路径（语义面不变量由 parity 钉住）
    ":sort u<CR>", ":%sort i<CR>", ":2,4sort! <CR>",
    // round15 键表节选（保持既有压力面）
    ":s/a/b\\/c/<CR>", "&", ":1+99999999999999999999d<CR>", "}", "{", "d}",
    "d{", "2}", "ci\"", "ca\"", "1%", "2%", "50%", "%", "d2%", "\"_yy",
    "\"\"p", "3Rab", "\"1P", ".", "2.", "dd", "dw", "<C-h>", "i<C-h>",
    "R<C-h>", "V'a", "v`a", "'a", "`a", "mA", "t2", "2t3", ";", "dtx",
    "\"ax", "\"ayy", "<C-v>jlo", "<C-v>jlO", "VjO", "3:", "3:<CR>",
    "2/x<Esc>", "3/foo<CR>", ":1,2d 3x", ":y a3", ":%s/x//n", ":s/a\\/b/x/",
    ":setlocal ts=8", ":bne", "gv", "gvd", "@a", "@:", "p", "yy", ":5+2+1d",
    ":3,4s", "2*", "yaw", "d-", "d'a", "ma", ":%s/x/y/", ":%j", "<Left>",
    "<Right>", "/x<CR>", "?中<CR>", ":set ts=8", ":set noic", ":set scs",
    ":setlocal ts?", ":brew", ">>", "guw", "<C-r>", "yiw", ":'<,'>d", "h",
    "j", "k", "l", "w", "b", "e", "0", "$", "^", "G", "d", "c", "y", "a",
    "i", "v", "V", "<C-v>", "u", "gJ", "~", "f", "n", "N", "*", "iw", "aw",
    "cw", "gg", "<C-a>", "<C-x>", "<C-o>", "<C-i>", ":marks", ":reg",
    "<Del>", "<C-c>", "<Tab>",
];

fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

fn check_invariants(f: &Fixture, text: &str, ctx: &str) {
    assert!(!text.is_empty() || f.buf.line_count() == 1, "{ctx}: 行数 >= 1");
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
    // 模式指示器自洽（宿主状态栏的可见面）
    match f.vim.mode_indicator() {
        "" => assert!(
            !matches!(
                f.vim.mode(),
                vimcore::mode::Mode::Insert
                    | vimcore::mode::Mode::Replace
                    | vimcore::mode::Mode::Visual { .. }
                    | vimcore::mode::Mode::CommandLine { .. }
            ),
            "{ctx}: normal 模式指示器必须为空"
        ),
        other => assert!(
            !other.is_empty(),
            "{ctx}: 非 normal 模式必须有指示器 ({other})"
        ),
    }
    // last_matches 不检查：harness 全程 set_hlsearch_live_update(false)
}

#[test]
fn fuzz_round16_holds_invariants() {
    // 缓冲池围绕本轮修复面：缩进行（ai 剥离面）、空行（失败删除族）、
    // 纯空白行、多字节、可扩展的多行缓冲
    let buffers = [
        "",
        "a",
        "中",
        "    ind\n",
        "    ind\nzz\n",
        "a\n\nb\n",
        "a\n   \nb\n",
        "\n\n\n",
        "one\ntwo\nthree\ntail\n",
        "abc\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "éA→ ç\nｱｲｳ\n\tindent",
        "x\n",
        "    foo\n    bar\n",
        "L0\nL1\nL2\nL3\nL4\nL5\n",
        "say \"hi\" then \"bye\"\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x16AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            f.vim.set_hlsearch_live_update(false);
            // 一半种子带 `nnoremap j gj`（round14 先例）
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
                let roll = fuzz_xorshift(&mut state);
                // 宿主事件注入（~1/8 步）：拖选/点击在任意模式投递——
                // 旧实现在 insert/cmdline 期会撕裂模式机；IME 文本与
                // IME 替换路径任意时点投递
                match roll % 8 {
                    0 => {
                        let a = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        let b = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        f.vim.set_visual_range(&f.buf, a, b);
                    }
                    1 => {
                        let o = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        f.vim.set_cursor_offset(&f.buf, o);
                    }
                    2 => {
                        f.vim.record_typed_text("中");
                        f.type_text("中");
                    }
                    3 => {
                        let len = f.text().len();
                        let a = (fuzz_xorshift(&mut state) as usize) % (len + 1);
                        let b = a + (fuzz_xorshift(&mut state) as usize) % (len - a + 1);
                        f.ime_replace(a..b, "x");
                    }
                    _ => {}
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

/// 巨 count 空复制：`<巨值>o<Esc>` 的复制数必须被字节预算钳制并终止
/// （fix round16-1 的 "\n".repeat 走 clamped_repeat_count）。
#[test]
fn fuzz_round16_huge_empty_repeat_terminates() {
    let start = std::time::Instant::now();
    for count in [999_999_999usize, 1_000_000_000, 12345] {
        let mut f = common::Fixture::new("x\n");
        for c in count.to_string().chars() {
            f.feed([c.to_string().as_str()]);
        }
        f.feed(["o", "<Esc>"]);
        // 16MB 预算 → 最多 1600 万个换行，缓冲不会爆
        assert!(f.buf.len() < 20 * 1024 * 1024);
    }
    assert!(start.elapsed().as_secs() < 30, "must terminate quickly");
}
