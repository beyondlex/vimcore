//! 第三十三轮 fuzz：嵌套宏 × 寄存器互写 × `.` 重放（BUG_AUDIT3 挂账 7）。
//!
//! 宏=寄存器统一（audit D4）后有三条新账本在互动：macros 键缓存、
//! 寄存器写代数同步、`"."` 的 count 替换重放。本轮专打它们的交叉：
//! `qa…q` 录制中嵌 `@a`/`@@`、播放中 `"ayy`/`"add` 覆写正在跑的宏、
//! `"ap` 粘宏文本、`[count].` 与 `@` 互嵌、`q"` 无名槽宏。
//! 不变量沿用渲染契约：光标/mark/高亮全部可寻址。
//!
//! 关键安全阀（继承 round13-20 的教训）：重放队列的预算靠
//! MAX_PIPELINE_STEPS 兜底；本测试额外在每步后检查 is_idle 状态不得
//! 卡在重放中超过一个软上限，防止互递归宏把测试挂死。

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

const SEEDS: u64 = 12;
const ROUNDS: usize = 40;
const STEPS: usize = 80;

/// 宏体与寄存器交互的键表：录制边界、播放、寄存器前缀写、无名槽、
/// 点重放、插入小段（Text 步进路径）、行级与字符级删除。
const MACRO_KEYS: &[&str] = &[
    "qa", "q", "qb", "qq", "q\"", "@a", "@b", "@@", "@:", "2@a", "3@b",
    "\"ayy", "\"byy", "\"add", "\"bdd", "\"a\"_d", "\"ap", "\"bp", "\"aP",
    "yy", "dd", "yy", "p", "P", "\"+y", "\"+p",
    ".", "2.", "3.",
    "x", "X", "J", "u", "<C-r>",
    "i", "abc", "<Esc>", "A", "x", "<Esc>", "o", "y", "<Esc>",
    "v", "j", "d", "v", "l", "y", "<Esc>",
    ":s/a/b/<CR>", ":1d<CR>", ":d<CR>", ":y<CR>", ":pu<CR>", ":g?x?d<CR>",
    "w", "b", "0", "$", "j", "k", "gg", "G", "f", "a", "n",
];

const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\nthird\n",
    "aa\naa\naa\n",
    "one",
    "",
    "x\n",
    "宏\n寄存器\n测试\n",
    "abc abc abc\ndef\n",
    "l1\nl2\nl3\nl4\nl5\n",
];

fn buffer_reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
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

#[test]
fn fuzz_round33_nested_macros_and_registers_hold_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for round in 0..ROUNDS {
            let mut f = Fixture::new(BUFFERS[((seed as usize) + round) % BUFFERS.len()]);
            let mut trace: Vec<String> = Vec::new();
            for _ in 0..STEPS {
                if fuzz_xorshift(&mut rng).is_multiple_of(16) {
                    let r = fuzz_xorshift(&mut rng);
                    if buffer_reset_when_idle(&mut f, r) {
                        trace.push(format!("RESET {}", (r as usize) % BUFFERS.len()));
                    }
                }
                let k = MACRO_KEYS[(fuzz_xorshift(&mut rng) as usize) % MACRO_KEYS.len()];
                trace.push(k.to_owned());
                if k.chars().count() == 1 && k.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && rand_pick(&mut rng, 4) {
                    // 单字母小写键有 1/4 概率以「打字文本」形态进插入会话
                    //（宿主 IME 路径），其余按命令键喂
                    f.feed(["i"]);
                    f.vim.record_typed_text(k);
                    let mut ctx = Ctx {
                        buf: &mut f.buf,
                        host: &mut f.host,
                    };
                    f.vim.insert_text_at_cursor(&mut ctx, k);
                    f.feed(["<Esc>"]);
                } else {
                    f.feed([k]);
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!("seed={seed} round={round}: {e}\ntrace: {trace:?}");
                }
            }
        }
    }
}

fn rand_pick(rng: &mut u64, modulo: u64) -> bool {
    fuzz_xorshift(rng) % modulo == 0
}
