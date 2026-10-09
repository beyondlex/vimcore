//! 第三十六轮 fuzz：模板会话（可视/插入/宏/算子待决）× 更怪缓冲
//! （独立审计第五轮伴随赛道）。
//!
//! 与 round35（Ex 随机组装为主）互补，本文件的生成器偏置：
//! - **模板会话**占 40%：可视三态会话、插入控制键会话、宏录制→重放→
//!   重录、算子×计数×motion/对象组合，一次性灌 3-8 键；
//! - 单键表加入确认循环键（y/n/q/l）、`q` 录制、`g?`、`ga`、`ZZ` 等
//!   round35 未覆盖的角落（仍无 `!` 过滤、无 `:q`/`:w`/`:e` 会话级命令）；
//! - 缓冲池换血：ZWJ emoji、组合字符行首、孤立 CR、星面字符、行尾
//!   空白密排、TAB+宽字符混排。
//!
//! 安全阀不变：`!` 过滤算子不进键表（不得触发 shell），`<`/`>` 只以
//! 单键出现。不变量沿用渲染契约：光标/mark/高亮可寻址、line_count ≥ 1。

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

const SEEDS: u64 = 40;
const ROUNDS: usize = 40;
const STEPS: usize = 100;

/// 单键表（round36 增补：确认循环、录制、冷门 g 族）。
const KEYS: &[&str] = &[
    "d", "c", "y", ">", "<", "=", "~", "gu", "gU", "g?", "2", "3", "0", "J",
    "w", "e", "b", "ge", "j", "k", "h", "l", "^", "$", "g_", "gg", "G", "|",
    "f", "t", "F", "T", ";", ",", "%", "n", "N", "*", "#", "gd", "gD", "go",
    "gj", "gk", "gn", "gN", "iw", "aw", "i\"", "a\"", "i(", "a(", "i)", "a)",
    "i{", "a{", "i[", "a[", "ip", "ap", "is", "as", "it", "at", "x", "X", "s",
    "S", "D", "C", "gJ", "dd", "yy", "cc", "p", "P", "gp", "gP", "o", "O",
    "r", "u", "<C-r>", ".", "ma", "mb", "'a", "'b", "`a", "`<", "'>", "'[",
    "']", "'{", "'}", "'^", "'.", "g;", "g,", "z<CR>", "z.", "zz", "v", "V",
    "<C-v>", "gv", "\"ayy", "\"add", "\"ap", "\"aP", "\"0p", "\"2p", "\"_dd",
    "\"Ay", "\"+yiw", "/", "?", "y", "n", "q", "l", "ga", "ZZ", "g&", "U",
    // 插入会话散键（idle 时多半落 normal 侧或响铃，无妨）
    "i", "a", "A", "I", "R", "<Esc>", "<BS>", "<Del>", "<Tab>", "<CR>",
    "<C-w>", "<C-u>", "<C-t>", "<C-d>", "<C-y>", "<C-e>", "<C-r>a", "<C-o>",
    "<Up>", "<Down>", "<Left>", "<Right>", "<Home>", "<End>", "<PageUp>",
    "<PageDown>",
];

/// Ex 整句（生成器之外的高价值形态）。
const EX_LINES: &[&str] = &[
    ":1,3d<CR>", ":3,1d<CR>", ":$-1,$m0<CR>", ":1t$<CR>", ":2,4y a<CR>",
    ":g/\\v</d<CR>", ":v/./d<CR>", ":%s/a/b/g<CR>", ":1,2j 3<CR>",
    ":sort i<CR>", ":retab 2<CR>", ":ce 20<CR>", ":ri 4<CR>", ":le 2<CR>",
    ":delm a b<CR>", ":'a,'bd<CR>", ":.,+2s/./-&/g<CR>", ":5<CR>",
    ":normal <CR>", ":g/x/normal <CR>", ":g/^$/j<CR>", ":s/a/b/c<CR>",
    ":1,3m'a<CR>", ":2,5co0<CR>", ":> <CR>", ":< <CR>", ":3>2<CR>",
    ":marks<CR>", ":1,2s/\\d/#/g<CR>", ":g/b/t$<CR>", ":0<CR>", ":$<CR>",
    ":set ts?<CR>", ":set sw=2<CR>", ":.y a<CR>", ":\"ap<CR>",
];

/// 可视会话的进入键。
const VISUAL_ENTER: &[&str] = &["v", "V", "<C-v>"];

/// 可视会话的走位与算子。
const VISUAL_MOVES: &[&str] =
    &["j", "k", "h", "l", "w", "e", "b", "$", "0", "}", "{", "gg", "G", "f\"", "iw", "ap"];

const VISUAL_OPS: &[&str] = &[
    "d", "c", "y", "~", "u", "U", "J", "gJ", "p", "P", ">", "<", "gu", "gU",
    "I", "A", "x", "s", "r", "o", "O", "gv",
];

/// 插入会话的载荷键。
const INSERT_KEYS: &[&str] = &[
    "x", "中", "ｱ", "<Tab>", "<CR>", "<C-w>", "<C-u>", "<C-t>", "<C-d>",
    "<C-y>", "<C-e>", "<C-r>a", "<C-r>\"", "<C-o>x", "<C-v>u4e2d",
    "<C-v>65", "<BS>", "<Del>", "<Up>", "<Down>", "<Left>", "<Right>",
];

/// 算子待决会话的算子与对象。
const OP_PEND_OPS: &[&str] = &["d", "c", "y", "gu", "gU", "g?", ">", "<", "="];

const OP_PEND_TARGETS: &[&str] = &[
    "w", "e", "b", "iw", "aw", "i(", "a(", "i{", "a{", "i\"", "a\"", "ip",
    "ap", "is", "as", "it", "at", "j", "k", "$", "0", "gg", "G", "gn", "%",
];

/// 宏会话的寄存器名。
const MACRO_REGS: &[&str] = &["a", "b", "z"];

const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\nthird\n",
    "\t\ttabbed\tline\n\tmixed\t中英\nend\n",
    "中文缓冲区\n第二行日本語\n第三行\n",
    "👨\u{200D}👩\u{200D}👧 family\n🀄\t🀄🀄\nmahjong\n",
    "comb\u{0301}ining\n\u{0301}leading marks\nok\n",
    "one",
    "",
    "x\n",
    "\n\n\n\n",
    "crlf\r\nline\r\n",
    "trailing   \n   leading\nall spaces    \n",
    "𝕏𝕪 astral 𝄞 clef\nmixed 𝕏x\n",
    "supercalifragilisticexpialidocious\na\nb\n",
    "aa\taa\taa\n  indented\n\t\tdeep\n",
    "quote\"text\"here\n'quoted'\nback\\slash\n",
    "<tag attr=\"v\">html</tag>\n<p>nested <b>x</b></p>\n",
    "()[]{}\n)(][}{\nunclosed ( {\n",
    "1\n22\n333\n4444\n55555\n",
    "a b  c   d\n multiple  spaces \n",
];

fn reset_when_idle(f: &mut Fixture, roll: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    let text = BUFFERS[(roll as usize) % BUFFERS.len()];
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
            return Err(format!("highlight {hl:?} unaddressable (len={len}) in {text:?}"));
        }
    }
    Ok(())
}

fn panic_msg(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "opaque panic".to_owned())
}

/// 守护一段会话：panic 时带上完整 trace 报 P0。
fn guarded(f: &mut Fixture, trace: &[String], label: &str, body: impl FnOnce(&mut Fixture)) {
    let guard =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(f)));
    if let Err(p) = guard {
        panic!("PANIC {label}\ntrace: {trace:?}\n{}", panic_msg(&*p));
    }
}

#[test]
fn fuzz_round36_template_sessions_hold_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        fn pick<'a>(rng: &mut u64, table: &[&'a str]) -> &'a str {
            table[(fuzz_xorshift(rng) as usize) % table.len()]
        }
        for round in 0..ROUNDS {
            let mut f =
                Fixture::new(BUFFERS[((seed as usize) * 7 + round * 3) % BUFFERS.len()]);
            let mut trace: Vec<String> = Vec::new();
            for _ in 0..STEPS {
                if fuzz_xorshift(&mut rng).is_multiple_of(10) {
                    let r = fuzz_xorshift(&mut rng);
                    if reset_when_idle(&mut f, r) {
                        trace.push(format!("RESET {}", (r as usize) % BUFFERS.len()));
                    }
                }
                let roll = fuzz_xorshift(&mut rng) % 10;
                if roll < 4 {
                    // 模板会话：一次灌 3-8 键
                    let kind = fuzz_xorshift(&mut rng) % 4;
                    let mut keys: Vec<String> = Vec::new();
                    let label;
                    match kind {
                        0 => {
                            // 可视会话
                            label = "visual".to_owned();
                            keys.push(pick(&mut rng, VISUAL_ENTER).to_owned());
                            for _ in 0..1 + fuzz_xorshift(&mut rng) % 3 {
                                keys.push(pick(&mut rng, VISUAL_MOVES).to_owned());
                            }
                            keys.push(pick(&mut rng, VISUAL_OPS).to_owned());
                            if fuzz_xorshift(&mut rng).is_multiple_of(3) {
                                keys.push("<Esc>".to_owned());
                            }
                        }
                        1 => {
                            // 插入会话
                            label = "insert".to_owned();
                            keys.push(pick(&mut rng, &["i", "a", "A", "I", "o", "O", "R"]).to_owned());
                            for _ in 0..2 + fuzz_xorshift(&mut rng) % 4 {
                                keys.push(pick(&mut rng, INSERT_KEYS).to_owned());
                            }
                            keys.push("<Esc>".to_owned());
                        }
                        2 => {
                            // 宏会话：录制 → 重放 → 追加重录
                            label = "macro".to_owned();
                            let reg = pick(&mut rng, MACRO_REGS);
                            keys.push("q".to_owned());
                            keys.push(reg.to_owned());
                            for _ in 0..2 + fuzz_xorshift(&mut rng) % 3 {
                                keys.push(pick(&mut rng, KEYS).to_owned());
                            }
                            keys.push("q".to_owned());
                            keys.push("@".to_owned());
                            keys.push(reg.to_owned());
                            if fuzz_xorshift(&mut rng).is_multiple_of(2) {
                                keys.push("@@".to_owned());
                            }
                        }
                        _ => {
                            // 算子待决组合
                            label = "op-pend".to_owned();
                            keys.push(pick(&mut rng, OP_PEND_OPS).to_owned());
                            if fuzz_xorshift(&mut rng).is_multiple_of(2) {
                                keys.push(pick(&mut rng, &["2", "3"]).to_owned());
                            }
                            keys.push(pick(&mut rng, OP_PEND_TARGETS).to_owned());
                            if fuzz_xorshift(&mut rng).is_multiple_of(3) {
                                keys.push("<Esc>".to_owned());
                            }
                        }
                    }
                    trace.push(format!("{label}::{keys:?}"));
                    guarded(&mut f, &trace, &label, |f| {
                        f.feed(keys.iter().map(|s| s.as_str()));
                        let _ = f.text();
                        let _ = f.cursor();
                        let _ = f.line();
                    });
                } else if roll == 4 {
                    // IME 提交路径
                    let s = ["中", "中文x", "x漢字", "ｱｲｳ", "🀄xx"][(fuzz_xorshift(&mut rng) % 5) as usize];
                    trace.push(format!("IME({s})"));
                    guarded(&mut f, &trace, "ime", |f| {
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
                } else if roll == 5 {
                    // Ex 整句
                    let line = pick(&mut rng, EX_LINES);
                    trace.push((*line).to_owned());
                    let keys: Vec<String> = if line.starts_with(':') {
                        line.chars().map(|c| c.to_string()).collect()
                    } else {
                        vec![line.to_owned()]
                    };
                    guarded(&mut f, &trace, line, |f| {
                        f.feed(keys.iter().map(|s| s.as_str()));
                        let _ = f.text();
                        let _ = f.cursor();
                    });
                } else {
                    let k = pick(&mut rng, KEYS);
                    trace.push(k.to_owned());
                    guarded(&mut f, &trace, k, |f| {
                        f.feed([k]);
                        let _ = f.text();
                        let _ = f.cursor();
                        let _ = f.line();
                    });
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!("seed={seed} round={round}: {e}\ntrace: {trace:?}");
                }
            }
        }
    }
}
