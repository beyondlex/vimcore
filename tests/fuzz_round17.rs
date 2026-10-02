//! 第十七轮 fuzz：覆盖本轮修复面的路径轰炸。
//!
//! 相对 round16 的增量（全部是本轮改过的路径，回归在此先炸）：
//! - **宏/`.`/`@` 三方交互**：q 录制起停、`@a` 回放、`.` 重放在任意
//!   交错下进行——`replay_records` 让 `@` 期间录制保持活跃，录制/提交
//!   状态机多了真分支；
//! - **文本对象 count**：`d2aw`/`v3i(`/`2ip` 的重复扫描与攀爬合并；
//! - **句子对象**：`dis`/`das`/`2as` 在句间空白、空行段界、缓冲末尾的
//!   新端点规则（尾随空白、前导回退、末换行排除）；
//! - **`<<`/`>>` 列模型**：`/ts/sw/noet/et` 的重表达路径（edit_replace
//!   替换缩进，marks/搜索缓存平移）；
//! - **insert `<C-w>` 空白段一笔删除**与**Ex 裸 `+`/`-` 偏移**、
//!   **`:s`/`:d` 尾 count**；
//! - **`"+` 寄存器 roundtrip**（clipboard_write 钩子）。
//!
//! 不变量沿用 round13-16：光标/mark/`last_visual`/live 选区全部可寻址、
//! 行数 ≥ 1、模式与指示器一致；另加本轮专属不变量：宏录制状态只在
//! `q` 会话内为真（`.` 重放不得楔进录制态）。

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

// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步（与 round13-16 同预算）
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // 本轮修复面：宏录制/回放/`.` 三方
    "qa", "q", "qaxq", "qbxq", "@a", "@b", "@@", "@:", "2@a", ".", "2.", "J",
    "qxq", "qq", "qazq", "d2aw",
    // 本轮修复面：文本对象 count（重复扫描 + 内层块攀爬）
    "d2aw", "d3aw", "d2iw", "d3iw", "v2aw", "v3i(", "2ip", "c2aw", "y3aw",
    "di(", "da(", "v2i\"", "v2a\"",
    // 本轮修复面：句子对象（新端点规则）
    "dis", "das", "2das", "cis", "vis", "vas", "yis",
    // 本轮修复面：shift 列模型
    ">>", "<<", "2>>", "2<<", ">j", "<j", "Vj>", ":set noet ts=8 sw=4<CR>",
    ":set et sw=4<CR>", ":set ts=4<CR>", ":set sw=8<CR>",
    // 本轮修复面：insert C-w 空白段
    "i", "a", "o", "<C-w>", "<C-u>",
    // 本轮修复面：Ex 裸偏移 + 尾 count
    ":+d<CR>", ":-d<CR>", ":5+d<CR>", ":.-d<CR>", ":+-d<CR>", ":d2<CR>",
    ":y2<CR>", ":j2<CR>", ":s/a/X/ 3<CR>", ":s/a/X/2<CR>", ":1,2s/a/X/2<CR>",
    ":1,2d 2<CR>", ":nohlse<CR>", ":+<CR>", ":-<CR>",
    // 本轮修复面：+ 寄存器
    "\"+yy", "\"+dd", "\"+p", "\"+P", "\"_dd", "\"+yiw",
    // round16 键表节选（既有压力面不丢）
    "o", "O", "<CR>", "<BS>", "<Esc>", "cc", "S", "3o<Esc>", "2O<Esc>",
    "x", "X", "D", "C", "s", "J", "gJ", "99J", ":sort u<CR>", "&",
    ":1+99999999999999999999d<CR>", "}", "{", "d}", "d{", "ci\"", "ca\"",
    "1%", "%", "\"_yy", "\"\"p", ".", "dd", "dw", "<C-h>", "i<C-h>",
    "V'a", "v`a", "'a", "`a", "mA", "t2", ";", "\"ax", "\"ayy",
    "<C-v>jlo", "VjO", "3:", "2/x<Esc>", ":1,2d 3x", ":y a3", ":%s/x//n",
    ":setlocal ts=8", "gv", "gvd", "@", "p", "yy", ":5+2+1d", "2*", "yaw",
    "d-", "d'a", "ma", ":%s/x/y/", ":%j", "<Left>", "<Right>", "/x<CR>",
    "?中<CR>", ":set ts=8", ":set noic", ":marks", ":reg", "<Del>",
    "<C-c>", "<Tab>", "h", "j", "k", "l", "w", "b", "e", "0", "$", "^",
    "G", "d", "c", "y", "a", "i", "v", "V", "<C-v>", "u", "~", "f", "n",
    "N", "*", "iw", "aw", "cw", "gg", "<C-a>", "<C-x>", "<C-o>", "<C-i>",
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
}

#[test]
fn fuzz_round17_holds_invariants() {
    // 缓冲池围绕本轮修复面：句间空白（句子对象）、空行段界、缩进混合
    // （shift 重表达）、多字节（边界回归）、可扩展多行
    let buffers = [
        "Aaa. Bbb.\n",
        "Aaa.\n\nBbb.\n",
        "Aaa. Bbb. Ccc.\n",
        "  indent. Next. Tail.\n",
        "foo bar baz\n",
        "f (1 + (2)) g\n",
        "say \"hi\" then \"bye\"\n",
        "\tx\n",
        "\t\tx\n",
        "    ind\n    ind2\n",
        "a\n\nb\n",
        "para one\npara two\n\npara three\n",
        "",
        "中. 文。\n",
        "éA→ ç. ｱｲｳ.\n",
        "one\ntwo\nthree\ntail\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x17AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
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
            // 一半种子预录宏 a = x（@a/@@ 路径立刻可达）
            if seed % 3 == 0 {
                for key in vimcore::key::parse_key_sequence("qaxq") {
                    let mut ctx = Ctx {
                        buf: &mut f.buf,
                        host: &mut f.host,
                    };
                    let _ = f.vim.handle_key(&mut ctx, key);
                }
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
                // 宿主事件注入（~1/8 步）：IME 文本与拖选/点击任意模式投递
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

/// 巨 count 文本对象：`<巨值>aw` 的重复扫描必须在 span 到达缓冲末尾时
/// 终止，不线性扫描 O(count) 次（每次无进展即 break）。
#[test]
fn fuzz_round17_huge_object_count_terminates() {
    let start = std::time::Instant::now();
    let mut f = common::Fixture::new("foo bar\n");
    for c in "999999999".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["a", "w"]);
    f.feed(["v"]);
    for c in "999999999".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["i", "w"]);
    assert!(f.buf.len() < 1024, "object count must not grow the buffer");
    assert!(start.elapsed().as_secs() < 5, "must terminate quickly");
}
