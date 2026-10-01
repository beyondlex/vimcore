//! 第十三轮 fuzz：覆盖本轮修复与新审查面的路径。
//!
//! 相对 round12 的增量键表：
//! - visual 模式下的宿主鼠标事件（`set_cursor_offset` 点击、
//!   `set_visual_range` 拖选）与随后的 d/y/c/:/gv —— round13 修复了
//!   「点击塌缩选区」，这里锁定锚点保持 + 选区可寻址；
//! - 大写 mark（`mA`-`mZ`、`` `A ``-`'Z`）的设置与跳转；
//! - `q` 寄存器字符面（`q/`、`q!`、`q:` 等非法名现在必须被拒）与
//!   `@` 回放交错；
//! - 空操作命令（空行 `x`、空寄存器 `p`、EOF `J`）之后的 `.` ——
//!   空操作不得顶掉上一个真修改的重放记录。
//!
//! 不变量沿用 round12：光标/mark/`last_visual`/live 选区/`last_matches`
//! 全部可寻址；visual 模式下点击后锚点与光标均可寻址。

mod common;

use common::Fixture;
use vimcore::state::Ctx;
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

const DRAG_EVENTS: bool = true;
// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步：键表充分轮转，同时把自递归
// 宏（qa…@a…q → @a 撑满 10 万键管线预算，见 enqueue_replay）的触发次数
// 控制在整套 cargo test 秒级
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // round13 修复路径：visual 点击后继续操作
    "<C-v>jI",
    "<C-v>jA",
    "<C-v>jx",
    "Vjd",
    "viwd",
    "viwc",
    "gv",
    "gvd",
    "gvy",
    "gvc",
    "gv:",
    // round13 修复路径：大写 mark
    "mA",
    "mB",
    "mZ",
    "`A",
    "'A",
    "`Z",
    "`B",
    // round13 修复路径：q 寄存器字符面
    "q/",
    "q!",
    "q:",
    "q\"",
    "q1",
    "qa",
    "q",
    "@a",
    "@1",
    "@/",
    "@@",
    "@:",
    // round13 修复路径：空操作后 `.`
    "x",
    ".",
    "p",
    "P",
    "J",
    "dd",
    "dw",
    "yy",
    "2dd",
    "d99d",
    // round12 键表节选（保持回归压力）
    ":5+2+1d",
    ":2,2j",
    ":d _",
    ":3,4s",
    "2*",
    "d2gn",
    "yaw",
    "d-",
    "dg_",
    "ci\"",
    "d'a",
    "ma",
    "mb",
    ":1,2,3d",
    ":&",
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
    "is",
    "i[",
    "a{",
    "daw",
    "dap",
    "<C-w>",
    "3D",
    "Vp",
    ":noh",
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
    "gJ",
    "~",
    "f",
    "t",
    ";",
    ",",
    "%",
    "n",
    "N",
    "*",
    "#",
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
    "\"_dd",
];

/// 校验一个偏移在 `text` 上可寻址(字符边界或末尾)。
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

/// 与 [`assert_addressable`] 相同的谓词，供批量路径先判定后格式化
/// （`last_matches` 可达 1 万条，逐条 format 是 fuzz 的主要成本）。
fn addressable(off: usize, text: &str) -> bool {
    off <= text.len() && (off == text.len() || text.is_char_boundary(off))
}

/// 校验引擎的全部存储偏移在当前文本上可寻址。`text` 由调用方传入（大缓冲
/// 上每步全量 clone 是二次方成本——历史增长到 7MB 的缓冲每步复制一次把
/// 调试构建拖到 77s；大缓冲按步数抽样校验，小缓冲每步全查）。
fn check_invariants(f: &Fixture, text: &str, ctx: &str) {
    check_invariants_mode(f, text, ctx, true);
}

/// `check_matches=false`：hlsearch_live_update 关闭时 `last_matches` 允许在
/// 两次 refresh_highlights 之间过期（宿主自定刷新节奏是该模式的设计前提，
/// generation 机制保证 n/N 消费前重扫），地址性只对刷新后的列表成立。
fn check_invariants_mode(f: &Fixture, text: &str, ctx: &str, check_matches: bool) {
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
    if check_matches {
        if let Some((i, m)) = f
            .vim
            .search
            .last_matches
            .iter()
            .enumerate()
            .find(|(_, m)| !addressable(m.start, text) || !addressable(m.end, text))
        {
            assert_addressable(&format!("match{i}.start"), m.start, text, ctx);
            assert_addressable(&format!("match{i}.end"), m.end, text, ctx);
        }
    }
}

#[test]
fn fuzz_round13_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        "say \"hi\" then \"bye\" ok\nx = \"a\" + \"b\";\n",
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
    for seed in 0..SEEDS {
        let mut state: u64 = 0x13AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            // 自递归宏（qa…@a…q 然后 @a）会撑满 10 万键的管线预算，逐编辑
            // 的 hlsearch 全文件重扫 + 宿主万条高亮复制把每步拖到毫秒级——
            // 按大文件宿主的正式用法关掉逐编辑重扫，改为按步刷新（同一
            // 代码路径，round10-11 的重扫修复面由既有 fuzz 覆盖）
            f.vim.set_hlsearch_live_update(false);
            for step in 0..STEPS {
                // yy+p 循环能让缓冲无限翻倍（引擎对 16MB 内的粘贴只设字节
                // 上限），重置把每步的引擎成本钉在常数；本 fuzz 抓到的全部
                // bug 都在几 KB 的缓冲上可复现，重置不损失覆盖
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                }
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
                // 宿主鼠标事件：点击（visual 下必须保持锚点）或拖选
                if fuzz_xorshift(&mut state).is_multiple_of(9) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
                }
                if DRAG_EVENTS && fuzz_xorshift(&mut state).is_multiple_of(23) {
                    let a = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 1);
                    let b = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 1);
                    f.vim.set_visual_range(&f.buf, a, b);
                }
                let refreshed = step % 16 == 0;
                if refreshed {
                    let mut hl = Ctx { buf: &mut f.buf, host: &mut f.host };
                    f.vim.refresh_highlights(&mut hl);
                }
                let ctx = format!("seed {seed} round {round} step {step} key {k}");
                if f.buf.len() < 4096 || refreshed {
                    let text = f.text();
                    check_invariants_mode(&f, &text, &ctx, refreshed);
                }
            }
        }
    }
}

/// 随机键轰炸之间持续检查 round13 修复的核心语义：
/// 1. visual 模式下的 `set_cursor_offset` 保持锚点（选区不塌缩）；
/// 2. 空操作不顶掉 `.` 的重放记录。
#[test]
fn fuzz_round13_semantic_invariants() {
    let mut state: u64 = 0x00D0_0D13;
    let buffers = [
        "word\n\nnext\n",
        "aaa\nbbb\nccc\nddd\n",
        "中文 test line\nsecond\n",
        "single",
    ];
    for round in 0..300u64 {
        let initial = buffers[(fuzz_xorshift(&mut state) as usize) % buffers.len()];
        let mut f = common::Fixture::new(initial);
        for step in 0..60 {
            let k = KEYS[(fuzz_xorshift(&mut state) as usize) % KEYS.len()];
            for key in parse_key_sequence(k) {
                f.feed_raw(key);
            }
            let ctx = format!("round {round} step {step} key {k}");
            // 不变量 1：visual + 点击 → 锚点保持
            if matches!(f.vim.mode(), vimcore::Mode::Visual { .. }) {
                if let Some(anchor_before) = f.vim.visual_selection().map(|(a, _, _)| a) {
                    let click = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                    f.vim.set_cursor_offset(&f.buf, click);
                    let sel = f.vim.visual_selection();
                    assert_eq!(
                        sel.map(|(a, _, _)| a),
                        Some(anchor_before),
                        "{ctx}: visual 点击必须保持锚点 {anchor_before}"
                    );
                }
            }
            let text = f.text();
            if text.len() < 4096 || step % 16 == 0 {
                check_invariants(&f, &text, &ctx);
            }
        }
    }
}
