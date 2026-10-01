//! 第十四轮 fuzz：覆盖本轮修复面的路径轰炸。
//!
//! 相对 round13 的增量键表：
//! - till 家族重复（`;`/`,`/count 与 t/T 交错）——round14 修复了
//!   「重复原地假成功」并重定义了 count 语义（fresh 数不同目标、
//!   repeat 跳过紧邻目标后按查找次数）；
//! - till operator span（`dt{c}`/`ct{c}`/`yt{c}`）——span 现延伸到目标位；
//! - `"{reg}` 前缀进 x/X/s/Del、`""` 归一化——寄存器路由面；
//! - visual `o`/`O`（块 O 是同线换列，char/line O 等同 o）；
//! - cmdline 的 count（`3:`、`3/pat`、取消路径）与 Ex 新错误面
//!   （E488 尾参、E477 bang、E16 越界、`n` 标志、转义分隔符、
//!   `:se`/`:setlocal`/`ts&`/`ts ?`）；
//! - Replace 会话的方向键 + BS 交错（位置栈）。
//!
//! 一部分种子预注册 `:nnoremap j gj`：让新的「部分命令续键不查映射」
//! 守卫（cmd_seq 非空时 FallThrough）与展开重放持续接受随机轰炸。
//!
//! 不变量沿用 round13：光标/mark/`last_visual`/live 选区/`last_matches`
//! 全部可寻址；`*` 之后 pattern 与高亮必须同步。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::parse_key_sequence;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步（与 round13 同预算）
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // round14 修复路径：till 重复与 span
    "t2",
    "t2;",
    "t2;;",
    "2t3",
    "T2",
    "T2,",
    ";",
    ",",
    "2;",
    "2,",
    "dtx",
    "dto",
    "ct2",
    "yt3",
    "dfx",
    "2fx",
    "d2t3",
    // round14 修复路径：寄存器前缀路由
    "\"ax",
    "\"aX",
    "\"as",
    "\"add",
    "\"ayy",
    "\"_dd",
    "\"\"dd",
    "\"\"p",
    "\"ap",
    "\"ayiw",
    // round14 修复路径：visual o / O
    "<C-v>jlo",
    "<C-v>jlO",
    "<C-v>jklO",
    "vllo",
    "vlO",
    "VjO",
    "Vj o",
    "o",
    "O",
    // round14 修复路径：cmdline count
    "3:",
    "2:",
    "3:<CR>",
    "3:<Esc>",
    "2/x<Esc>",
    "3/foo<CR>",
    "?x<CR>",
    "V3v:",
    // round14 修复路径：Ex 新错误面
    ":1,2d 3x",
    ":d a b",
    ":y a3",
    ":2y a 2",
    ":1,2d!",
    ":1,2y!",
    ":1000000d",
    ":2,99999d",
    ":5 +2d",
    ":0d",
    ":-1d",
    ":%s/x//n",
    ":s/a\\/b/x/",
    ":se ts?",
    ":set ts ?",
    ":set ts&",
    ":setlocal ts=8",
    ":bne",
    ":bp",
    ":ju",
    // round13 键表节选（保持回归压力）
    "<C-v>jI",
    "<C-v>jA",
    "Vjd",
    "viwd",
    "gv",
    "gvd",
    "gv:",
    "mA",
    "`A",
    "q/",
    "qa",
    "q",
    "@a",
    "@:",
    "x",
    ".",
    "p",
    "J",
    "dd",
    "dw",
    "yy",
    "2dd",
    ":5+2+1d",
    ":2,2j",
    ":d _",
    ":3,4s",
    "2*",
    "yaw",
    "d-",
    "dg_",
    "ci\"",
    "ca\"",
    "d'a",
    "ma",
    ":1,2,3d",
    ":%s/x/y/",
    ":%j",
    "R",
    "<BS>",
    "<Left>",
    "<Right>",
    "/x<CR>",
    "?中<CR>",
    "2i",
    "3o",
    ":set ts=8",
    ":set noic",
    ">>",
    "guw",
    "gUU",
    "g~~",
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
];

/// 校验一个偏移在 `text` 上可寻址(字符边界或末尾)。
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

fn addressable(off: usize, text: &str) -> bool {
    off <= text.len() && (off == text.len() || text.is_char_boundary(off))
}

/// `check_matches=false`：hlsearch_live_update 关闭时 `last_matches` 允许
/// 在两次 refresh_highlights 之间过期（generation 机制保证 n/N 消费前
/// 重扫——round13 同款约束）。
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
fn fuzz_round14_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "a2x2x2x\n",
        "a1b2c3d3e\n",
        "say \"hi\" then \"bye\" ok\n\"a\\\\\" tail\n",
        "foo   bar  baz\n   \n\nnext word\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "x3ya3za3\n2xx2yy2z2q\n",
        "a b a b a b\nmatch me and me\nfoo !\n",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "one two three\n...\n",
        "xyz\nwxyz\n",
        "old here\na/b tail\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x14AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            // 一半种子带 `nnoremap j gj`：cmd_seq 守卫 + 展开重放持续受轰
            if seed.is_multiple_of(2) {
                f.vim.keymaps.map_str_noremap(
                    vimcore::keymap::ModeClass::Normal,
                    "j",
                    "gj",
                    true,
                );
            }
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            // 大缓冲宿主模式：关逐编辑重扫（与 round13 同理，自递归宏会
            // 撑满管线预算）
            f.vim.set_hlsearch_live_update(false);
            for step in 0..STEPS {
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                    if seed.is_multiple_of(2) {
                        f.vim.keymaps.map_str_noremap(
                            vimcore::keymap::ModeClass::Normal,
                            "j",
                            "gj",
                            true,
                        );
                    }
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
                if fuzz_xorshift(&mut state).is_multiple_of(9) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
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

/// 随机键轰炸之间持续检查 round14 修复的核心语义：
/// 1. till 重复永远前进或失败——不允许「moved 但原地不动」的假成功；
/// 2. yank 之后 `g;` 不落在 yank 处（changelist 不被纯 yank 污染）。
#[test]
fn fuzz_round14_semantic_invariants() {
    let mut state: u64 = 0x00D0_0D14;
    let buffers = [
        "a2x2x2x\n",
        "a1b2c3d3e\n",
        "word\n\nnext\n",
        "aaa\nbbb\nccc\nddd\n",
    ];
    for round in 0..300u64 {
        let initial = buffers[(fuzz_xorshift(&mut state) as usize) % buffers.len()];
        let mut f = common::Fixture::new(initial);
        for step in 0..60 {
            // 1) till 重复语义：停车位之后 `;` 必须前进或失败（ bell），
            //    绝不允许 moved=true 且偏移不变
            let find_char = ['2', '3', 'x', '中']
                [(fuzz_xorshift(&mut state) as usize) % 4] as char;
            f.feed(["t"]);
            f.type_text(&find_char.to_string());
            let before = f.vim.cursor_offset();
            let bells_before = f.host.bells;
            f.feed([";"]);
            let after = f.vim.cursor_offset();
            if after == before {
                assert!(
                    f.host.bells > bells_before,
                    "round {round} step {step}: `;` 原地假成功（bells 未增）"
                );
            }
            // 2) yank 不进 changelist：记录 x 编辑位置，yiw 后 g; 必须落
            //    在编辑处而非 yank 处
            let mut f2 = common::Fixture::new("line1\nline2\nline3\n");
            f2.feed(["j", "x"]); // 真编辑在 line2
            f2.feed(["g", "g"]);
            f2.feed(["3", "G"]);
            f2.feed(["y", "i", "w"]); // yank line3
            f2.feed(["g", "g"]);
            f2.feed(["g", ";"]);
            assert_eq!(
                f2.vim.cursor_offset(),
                f2.buf.line_start(1),
                "round {round}: g; 必须落在 x 的编辑处（line2），不是 yank 处"
            );
            let _ = &mut f;
        }
    }
}
