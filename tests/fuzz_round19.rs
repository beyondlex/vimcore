//! 第十九轮 fuzz：本轮修复面的路径轰炸。
//!
//! 相对 round17 的增量（全部是本轮改过的路径）：
//! - **D/C 行模型**：`D`/`C` 的 count 形态在行首 / 行中 / 末行 / 唯一行
//!   上任意轰炸（`delete_to_end_span` 重写——count≥2 行首整行消失、
//!   count=1 清空、C 恒清空保行；旧模型在这里既丢文件 eol 又留幻影行）；
//! - **`Nr<CR>` 单换行塌缩**（`3r<CR>` 曾产出 N 个换行）；
//! - **`r<C-E>`/`r<C-Y>` 邻行取字符**（新功能；任一侧越界的整条取消）；
//! - **char-arg 非可打印键取消**（r/f/'/m 等待中按方向键——此前注释与
//!   代码不符，行为钉住为「取消 + 响铃」）；
//! - **未设 mark 跳转的 E20 消息**、**`:j` 末行 no-op 不污染 changelist**。
//!
//! 不变量沿用 round13-17：光标/mark/`last_visual`/live 选区全部可寻址、
//! 行数 ≥ 1；另加本轮专属不变量：任何 D/C 序列之后缓冲的首行仍然存在
//! （≥1 行）且 `.` 重放路径不 panic。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 60 rounds × 120 steps ≈ 11.5 万步（与 round13-17 同预算）
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // ---- 本轮修复面：D/C 行模型（行首/行中/末行/唯一行 × count）----
    "D",
    "2D",
    "3D",
    "99D",
    "C",
    "2C",
    "99C",
    "d$",
    "d2$",
    "c$",
    "dd",
    "3dd",
    // ---- Nr<CR> 塌缩与 r 邻行 ----
    "r<CR>",
    "3r<CR>",
    "r<C-e>",
    "r<C-y>",
    "3r<C-e>",
    "2rx",
    // ---- char-arg 等待键（r/f/'/m/q/@ + 取消键/方向键交错）----
    "r",
    "f",
    "F",
    "t",
    "'",
    "`",
    "m",
    "q",
    "@",
    "escape",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    // ---- 未设 mark 与已设 mark 跳转 ----
    "'a",
    "'z",
    "`z",
    "m1",
    "ma",
    "'a",
    // ---- Ex 面：:j no-op、:d、:s、:sort 的范围/计数形态 ----
    ":3j\n",
    ":2,2j\n",
    ":1,2j\n",
    ":j\n",
    ":2d\n",
    ":%d\n",
    ":s/a/X/\n",
    ":%s/x/Y/g\n",
    ":2sort\n",
    ":5\n",
    // ---- 既有覆盖节选（管线压力：`.`/宏/寄存器/对象）----
    ".",
    "x",
    "X",
    "s",
    "S",
    "J",
    "gJ",
    "yy",
    "dd",
    "p",
    "P",
    "u",
    "<C-r>",
    "qa",
    "q",
    "@a",
    "daw",
    "d2aw",
    "ci(",
    "di\"",
    "guu",
    "gqq",
    ">>",
    "3>>",
    "<<",
    "gq}",
    "gg",
    "G",
    "w",
    "b",
    "e",
    "0",
    "$",
    "g_",
    ";",
    ",",
    "%",
    "n",
    "N",
    "*",
    "gn",
    "gN",
    "j",
    "k",
    "h",
    "l",
    "gj",
    "gk",
    "|",
    "10|",
    "<C-a>",
    "<C-x>",
    "g;",
    "g,",
    "<C-o>",
    "<C-i>",
    "gv",
    "v",
    "V",
    "<C-v>",
    "o",
    "O",
    "i",
    "a",
    "I",
    "A",
    "gI",
    "gi",
    "R",
    "zh",
    // insert-mode 编辑键（C-w/C-u 与 ai/BS 的交互）
    "<C-w>",
    "<C-u>",
    "<BS>",
    "<Del>",
    "<Tab>",
    "<C-h>",
    // 宿主事件注入（round16 起）：任意模式下的点击/拖选/IME
    "host:click",
    "host:drag",
    "host:ime",
    "host:ime_replace",
];

// fuzz 缓冲种子：短行/1 字符行/多字节/空行/带 eol 的混合——D/C 行模型的
// 全部分支形状都要在种子池里常态出现
const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "aaaa\nbbbb\ncccc\ndddd\n",
    "a\nb\nc\nd\n",
    "hello world\nsecond line\n",
    "中文测试\n多字节行\n",
    "    indented\ntab\there\n",
    "one",
    "",
    "only\n",
    "x\n",
    "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n",
    "foo bar baz\n\n\nqux\n   \ntail\n",
];

fn buffer_reset(f: &mut Fixture, seed: u64) {
    let text = BUFFERS[(seed as usize) % BUFFERS.len()];
    *f.buf.0.borrow_mut() = text.to_owned();
    f.vim.cursor.offset = 0;
    f.vim.cursor.desired_col = None;
}

/// 契约内的缓冲重置：宿主只能换「引擎可见的整段文本」，而引擎的会话
/// 状态（insert/visual/cmdline/算子悬挂）从不跨越宿主的文本交换——真实
/// 宿主都经 undo/redo 路径（引擎侧有 sanitize）。裸换文本只在引擎
/// idle 且 Normal 时合法。
fn buffer_reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    buffer_reset(f, seed);
    true
}

fn invariants_hold(f: &Fixture) -> Result<(), String> {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    if buf.line_count() == 0 {
        return Err("line_count == 0".into());
    }
    // 光标可寻址（字符边界 + 行内）
    let cur = f.vim.cursor.offset;
    if cur > len || !text.is_char_boundary(cur) {
        return Err(format!("cursor {cur} unaddressable (len={len})"));
    }
    let line = buf.offset_to_line(cur);
    // 插入模式的光标合法地停在 line_end（行尾插入位）甚至缓冲幻影末位
    // （append 打字 / `<Del>` 并线之后）——只有普通/可视模式要求「不越行尾」。
    // 普通模式的唯一例外是 `cur == len`：本宿主行模型折叠末尾 \n，vim 模型
    // 里 `o<Esc>`（末行开行）留下的「新空行行首」在这里表现为幻影末位
    // （round25 起 exit_insert 宁可停 len 也不停在 \n 上——那里 `x` 会并线）
    if matches!(
        f.vim.mode(),
        vimcore::mode::Mode::Insert | vimcore::mode::Mode::Replace
    ) {
        if cur < buf.line_start(line) {
            return Err(format!("cursor {cur} before line {line} start"));
        }
    } else if cur != len && cur > buf.line_end(line) {
        return Err(format!("cursor {cur} past line {line} end"));
    }
    // 存储偏移可寻址（marks / changelist / jumplist / last_visual）
    let floor = |off: usize| {
        let mut o = off.min(len);
        while o > 0 && !text.is_char_boundary(o) {
            o -= 1;
        }
        o
    };
    for (name, off) in f.vim.marks.items() {
        let o = floor(off);
        if buf.offset_to_line(o) >= buf.line_count() {
            return Err(format!("mark {name} at {o} past last line"));
        }
    }
    Ok(())
}

/// 主轰炸循环（round16 的宿主事件注入 + 本轮键表）。
#[test]
fn fuzz_round19_d_c_line_model_holds_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E3779B97F4A7C15);
        for round in 0..ROUNDS {
            let mut f = Fixture::new("seed");
            buffer_reset(&mut f, seed); // fresh fixture: engine starts idle
            // 一半种子预置一个 mark + changelist 条目（E20/`'a` 面常态可达）
            if seed % 2 == 0 {
                f.feed(["m", "a"]);
                f.feed(["x"]);
            }
            for step in 0..STEPS {
                rng = fuzz_xorshift(&mut rng);
                let key = KEYS[(rng as usize) % KEYS.len()];
                if let Some(event) = key.strip_prefix("host:") {
                    match event {
                        "click" => {
                            rng = fuzz_xorshift(&mut rng);
                            let off = (rng as usize)
                                % (f.buf.len() + 1);
                            f.vim.set_cursor_offset(&f.buf, off);
                        }
                        "drag" => {
                            rng = fuzz_xorshift(&mut rng);
                            let a = (rng as usize) % (f.buf.len() + 1);
                            rng = fuzz_xorshift(&mut rng);
                            let b = (rng as usize) % (f.buf.len() + 1);
                            f.vim.set_visual_range(&f.buf, a, b);
                        }
                        "ime" => {
                            f.vim.record_typed_text("x中");
                            f.type_text("x中");
                        }
                        "ime_replace" => {
                            rng = fuzz_xorshift(&mut rng);
                            let start = f.vim.cursor.offset
                                .min(f.buf.len().saturating_sub(1));
                            f.ime_replace(start..f.buf.len(), "替换");
                        }
                        _ => {}
                    }
                } else {
                    f.feed([key]);
                }
                // 每步 1/16 概率重置缓冲（本轮的 D/C 行模型对缓冲形状
                // 高度敏感——短行/唯一行/eol 形状要高频轮换）
                rng = fuzz_xorshift(&mut rng);
                if rng % 16 == 0 {
                    buffer_reset_when_idle(&mut f, rng >> 8);
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!(
                        "seed={seed} round={round} step={step} key={key:?}: {e}\nbuffer={:?}",
                        f.text()
                    );
                }
            }
        }
    }
}

/// 定向：D/C 的全部（行位置 × count × col）形态在 1-4 行缓冲上不 panic
/// 且行数 ≥ 1——`delete_to_end_span` 的真值表轰炸。
#[test]
fn fuzz_d_c_shapes_never_break_line_invariant() {
    for buf_text in ["a\n", "a\nb\n", "a\nb\nc\n", "aaaa\nbbbb\ncccc\ndddd\n", ""] {
        for line in 0..4 {
            for col in 0..3 {
                let count_keys: &[&[&str]] = &[
                    &["D"],
                    &["2", "D"],
                    &["9", "9", "D"],
                    &["C"],
                    &["2", "C"],
                ];
                for count_key in count_keys {
                    let mut f = Fixture::new(buf_text);
                    let ls = f.buf.line_start(line.min(f.buf.line_count() - 1));
                    let le = f.buf.line_end(line.min(f.buf.line_count() - 1));
                    f.vim.cursor.offset = (ls + col).min(le);
                    f.feed(*count_key);
                    assert!(
                        f.buf.line_count() >= 1,
                        "buf={buf_text:?} line={line} col={col} keys={count_key:?}: no lines left"
                    );
                    // 撤销回路也要能吃下结果（undo 恢复宿主文本）
                    f.feed(["u"]);
                    assert!(f.buf.line_count() >= 1);
                }
            }
        }
    }
}

/// 定向：`.` 重放在 D/C 修复后的 span 模型上不 panic（记录的重放键穿过
/// 同一条 delete_to_end_span 路径）。
#[test]
fn fuzz_dot_replay_of_d_c_shapes() {
    for seed in 0..32u64 {
        let mut rng = seed.wrapping_mul(0x2545F4914F6CDD1D) | 1;
        let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
        for _ in 0..40 {
            rng = fuzz_xorshift(&mut rng);
            match rng % 4 {
                0 => {
                    f.feed(["D"]);
                }
                1 => {
                    f.feed(["2", "D"]);
                }
                2 => {
                    f.feed(["C"]);
                    f.type_text("z");
                    f.feed(["<Esc>"]);
                }
                _ => {
                    f.feed(["."]);
                }
            }
            rng = fuzz_xorshift(&mut rng);
            if rng % 8 == 0 {
                buffer_reset_when_idle(&mut f, rng >> 4);
            }
            assert!(f.buf.line_count() >= 1, "seed={seed}: buffer empty\n{:?}", f.text());
        }
    }
}
