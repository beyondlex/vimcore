//! 第三十五轮 fuzz：Ex 命令随机组装 × 不寻常缓冲（独立审计第四轮伴随赛道）。
//!
//! 与 audit5 智能体的语义审计互补：本文件只抓**不变量违规与 panic**
//! （P0 级），语义对错交给 oracle 探针。新维度：
//! - Ex 行由「地址段 × 命令段」随机组装（倒序区间、偏移流、搜索地址、
//!   mark 地址、`g`/`v` 嵌套 `:s`、`retab`/`sort`/`ce`/`ri`/`le` 全上）；
//! - 缓冲池加入 TAB 密排、CJK、组合字符、^M、noeol、空行阵、单字符；
//! - 算子 × 计数 × motion × 文本对象自由组合（2c3ap 一类）。
//!
//! 安全阀：键表**不含 `!` 过滤算子**（`!!` 会把后续随机字符交给
//! `sh -c` 执行，fuzz 不得触发真实 shell）；不含 `:q`/`:w`/`:e` 一类
//! 会话级命令。不变量沿用渲染契约：光标/mark/高亮全部可寻址、
//! line_count ≥ 1。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

const SEEDS: u64 = 48;
const ROUNDS: usize = 48;
const STEPS: usize = 90;

/// Normal/可视/插入键表：算子×计数×对象自由拼、寄存器前缀、marks、
/// z/g 族、搜索、插入控制键。
const KEYS: &[&str] = &[
    // 算子与计数
    "d", "c", "y", ">", "<", "=", "~", "gu", "gU", "g??", "2", "3", "7", "0",
    // motion
    "w", "e", "b", "ge", "j", "k", "h", "l", "0", "^", "$", "g_", "gg", "G",
    "|", "f", "t", "F", "T", ";", ",", "%", "n", "N", "*", "#", "gd", "gD",
    "go", "gm", "gj", "gk", "gn", "gN",
    // 文本对象（跟在算子后随机命中）
    "iw", "aw", "i\"", "a\"", "i'", "a'", "i(", "a(", "i)", "a)", "i{",
    "a{", "i[", "a[", "ip", "ap", "is", "as", "it", "at",
    // 行操作
    "x", "X", "s", "S", "D", "C", "J", "gJ", "dd", "yy", "cc", "yy", "p",
    "P", "gp", "gP", "o", "O", "r", "u", "<C-r>", ".",
    // marks 与跳转
    "ma", "mb", "mA", "'a", "'b", "`a", "`<", "'>", "'[", "']", "'{", "'}",
    "'^", "'.", "'0", "g;", "g,",
    // z 族（视口语义，单缓冲应全部无害）
    "z<CR>", "z.", "z-", "zt", "zz", "zb",
    // 可视
    "v", "V", "<C-v>", "o", "O", "gv", "iw", "<Esc>",
    // 寄存器前缀
    "\"ayy", "\"add", "\"ap", "\"aP", "\"+y", "\"+p", "\"0p", "\"-p",
    "\"_d", "\"2p", "\"Ay",
    // 搜索与替换提示符
    "/", "?", ":s/a/b/<CR>", ":s//x/<CR>", ":&<CR>", ":~<CR>",
    // 插入会话
    "i", "a", "A", "I", "R", "<Esc>", "<BS>", "<Del>", "<Tab>", "<CR>",
    "<C-w>", "<C-u>", "<C-t>", "<C-d>", "<C-y>", "<C-e>", "<C-r>a",
    "<C-o>", "<C-v>x41",
    // Ex：完整命令（生成器之外的高价值整句）
    ":1,3d<CR>", ":3,1d<CR>", ":$-1,$m0<CR>", ":1t$<CR>", ":2,4y<CR>",
    ":g/\\v</d<CR>", ":v/./d<CR>", ":%s/a/b/g<CR>", ":1,2j 3<CR>",
    ":sort i<CR>", ":retab 2<CR>", ":ce 20<CR>", ":ri 4<CR>", ":le<CR>",
    ":delm a b<CR>", ":'a,'bd<CR>", ":.,+2s/./-&/g<CR>", ":5<CR>",
    ":normal <CR>",
];

/// Ex 行生成器的地址段。
const EX_HEADS: &[&str] = &[
    "", "%", ".", "$", "1", "2", "3", "0", "5", ".,+2", ".,-1", "1,3", "3,1",
    "'a,'b", "'<,'>", "/ab/", "?ab?", "/ab/,/cd/", "+1", "-1", "$-1", ".,$",
    ".,+99", "'{,'}",
];

/// Ex 行生成器的命令段。
const EX_CMDS: &[&str] = &[
    "d", "d 2", "y", "y 3", "t.", "t$", "m$", "m0", "m.", "j", "j 2", "co 2",
    "s/a/b/", "s/a/b/g", "s//b/", "s/a/", "s/[0-9]/#/g", "ce", "ce 4", "ri",
    "le", "retab", "retab 3", "retab!", "sort", "sort!", "nu", "#", "=",
    "delm a", "marks", "marks ab", "g/a/d", "g!/a/d", "v/a/d",
    "g/a/s/a/b/g", "&", "&&", "~", "p", "1", "+3",
];

const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\nthird\n",
    "\t\ttabbed\tline\n\tmixed\t中英\nend\n",
    "中文缓冲区\n第二行日本語\n第三行\n",
    "comb\u{0301}ining\n\u{200D}zwj line\nok\n",
    "one",
    "",
    "x\n",
    "\n\n\n\n",
    "digits 123 456\n3abc\ndef2\n",
    "crlf\r\nline\r\n",
    "single very long line here with lots of words to walk through\nshort\n",
    "aa\taa\taa\n  indented\n\t\tdeep\n",
    "quote\"text\"here\n'quoted'\nback\\slash\n",
    "<tag>html</tag>\n<p>nested <b>x</b></p>\n",
    "()[]{}\n)(][}{\nunclosed ( {\n",
];

fn reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    let text = BUFFERS[(seed as usize) % BUFFERS.len()];
    *f.buf.0.borrow_mut() = text.to_owned();
    f.vim.cursor.offset = 0;
    f.vim.cursor.desired_col = None;
    let mut ctx = Ctx {
        buf: &mut f.buf,
        host: &mut f.host,
    };
    f.vim.refresh_highlights(&mut ctx);
    true
}

fn invariants_hold(f: &Fixture) -> Result<(), String> {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    if buf.line_count() == 0 {
        return Err("line_count == 0".into());
    }
    let cur = f.vim.cursor.offset;
    if cur > len || !text.is_char_boundary(cur) {
        return Err(format!("cursor {cur} unaddressable (len={len}) in {text:?}"));
    }
    for (name, off) in f.vim.marks.items() {
        let o = off.min(len);
        let o = (0..=o).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        if buf.offset_to_line(o) >= buf.line_count() {
            return Err(format!("mark {name} at {o} past last line (len={len})"));
        }
    }
    for hl in f.host.highlights.iter().chain(f.host.current_highlight.iter()) {
        if hl.end > len || !text.is_char_boundary(hl.start) || !text.is_char_boundary(hl.end) {
            return Err(format!(
                "highlight {hl:?} unaddressable (len={len}) in {text:?}"
            ));
        }
    }
    Ok(())
}

fn report_panic(
    guard: &Result<(), std::boxed::Box<dyn std::any::Any + std::marker::Send>>,
    seed: u64,
    round: usize,
    key: &str,
    trace: &[String],
) {
    if let Err(p) = guard {
        let msg = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "opaque panic".to_owned());
        panic!("PANIC seed={seed} round={round} key={key}: {msg}\ntrace: {trace:?}");
    }
}

#[test]
fn fuzz_round35_ex_assembly_and_weird_buffers_hold_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for round in 0..ROUNDS {
            let mut f = Fixture::new(BUFFERS[((seed as usize) * 3 + round) % BUFFERS.len()]);
            let mut trace: Vec<String> = Vec::new();
            for _ in 0..STEPS {
                if fuzz_xorshift(&mut rng).is_multiple_of(12) {
                    let r = fuzz_xorshift(&mut rng);
                    if reset_when_idle(&mut f, r) {
                        trace.push(format!("RESET {}", (r as usize) % BUFFERS.len()));
                    }
                }
                let roll = fuzz_xorshift(&mut rng) % 10;
                if roll < 2 {
                    // 随机组装一条 Ex 行：地址段 × 命令段
                    let head = EX_HEADS[(fuzz_xorshift(&mut rng) as usize) % EX_HEADS.len()];
                    let cmd = EX_CMDS[(fuzz_xorshift(&mut rng) as usize) % EX_CMDS.len()];
                    let line = format!("{head}{cmd}");
                    trace.push(format!(":{line}"));
                    let mut keys: Vec<String> = vec![":".to_owned()];
                    keys.extend(line.chars().map(|c| c.to_string()));
                    keys.push("<CR>".to_owned());
                    let guard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        f.feed(keys);
                        let _ = f.text();
                        let _ = f.cursor();
                    }));
                    report_panic(&guard, seed, round, &format!(":{line}"), &trace);
                } else if roll == 2 {
                    // IME 提交路径：插入会话中整段提交 CJK
                    let s = ["中", "中文x", "x漢字", "ｱｲｳ"][(fuzz_xorshift(&mut rng) % 4) as usize];
                    trace.push(format!("IME({s})"));
                    let guard = std::panic::AssertUnwindSafe(|| {
                        f.feed(["i"]);
                        f.vim.record_typed_text(s);
                        let mut ctx = Ctx {
                            buf: &mut f.buf,
                            host: &mut f.host,
                        };
                        f.vim.insert_text_at_cursor(&mut ctx, s);
                        f.feed(["<Esc>"]);
                        let _ = f.text();
                        let _ = f.cursor();
                    });
                    let g = std::panic::catch_unwind(guard);
                    report_panic(&g, seed, round, &format!("IME({s})"), &trace);
                } else {
                    let k = KEYS[(fuzz_xorshift(&mut rng) as usize) % KEYS.len()];
                    trace.push(k.to_owned());
                    let guard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        f.feed([k]);
                        // 采样动作（harness 切片）也必须在轨道内，否则
                        // panic 时丢 trace
                        let _ = f.text();
                        let _ = f.cursor();
                        let _ = f.line();
                    }));
                    if let Err(p) = guard {
                        let msg = p
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "opaque panic".to_owned());
                        panic!(
                            "PANIC seed={seed} round={round} key={k}: {msg}\ntrace: {trace:?}"
                        );
                    }
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!("seed={seed} round={round}: {e}\ntrace: {trace:?}");
                }
            }
        }
    }
}
