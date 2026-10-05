//! 第二十六号 fuzz（第二十七轮检视期内新增）：宿主历史的两条不变量。
//!
//! 引擎把 undo/redo 委托给宿主（`begin_undo_group`/`undo`/`redo`），自身
//! 只负责在每个编辑漏斗处宣布组。此前 25 个 fuzz 轮检查的都是「单步编辑
//! 后的状态合法性」，从未检查过**历史轴本身**。本轮补上两条只有把随机
//! 键序列一路 `u` 到底才能抓到的不变量：
//!
//!   1. **可逆性**：undo 到底再 redo 到底，文本必须逐字节回到 undo 前的
//!      状态——抓「宿主 redo 栈与 undo 栈不对称」类缺陷；
//!   2. **完备性**：undo 到底后文本必须等于**初始缓冲**——抓「某条编辑
//!      路径忘了走 `edit_*` 漏斗、从未进任何 undo 组」类缺陷（漏开的
//!      编辑不可撤销，两次 `u` 之间静默丢失）。
//!
//! 键表混入宿主事件（IME 组合替换、点击），这些路径同样宣布组，是
//! 「组所有权」最容易出错的接缝（replace_range 的 owns_group 规则）。
//! 语料含多字节文本；不加中途换缓冲——宿主换缓冲必然重置自己的历史栈，
//! 那是宿主的一致性责任（round20 的 reset 语义），不属于本不变量。

mod common;

use common::Fixture;
use vimcore::mode::Mode;

/// 简单 LCG：确定性随机（无外部依赖）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// 语料：ASCII / 多字节 / 空行 / 尾行无换行（历史不变量与内容无关，
/// 语料只为让编辑路径多样化）。
const BUFFERS: &[&str] = &[
    "hello world\nsecond line\nthird\n",
    "aaa bbb aaa\n\nccc\n",
    "中文行\nmultiline\ntail",
    "a\n",
    "one",
    "x y z\n1 2 3\nfoo bar\n",
];

/// 单键池（feed 的粒度是一个 Key）：移动/编辑/字符参数/可视/Ex/宏/历史。
/// `u`/`<C-r>` 也在池里——历史轴上的随机游走让 undo 栈深浅不一。
const KEY_POOL: &[&str] = &[
    "x", "d", "d", "y", "y", "p", "P", "i", "a", "o", "O", "<Esc>", "u", "<C-r>", "c", "w", "S",
    "J", "~", ">", ">", "<", "<", "D", "C", "s", "r", "x", "<C-a>", "<C-x>", "3", "x", "v", "d",
    "y", "<C-v>", "I", "A", "d", ":", "e", "s", "c", "\n", "&", ".", "q", "a", "q", "@", "a",
    "g", "J", "g", "q", "q", "f", "x", "t", "y", "m", "a", "'", "a", "`", "a", "G", "g", "g",
    "n", "N", "*", "#", "1", "2", "0", "j", "k", "h", "l", "w", "b", "e", "0", "$", "^", "z",
    "z", "<BS>", "<C-w>", "<Del>", "<C-o>", "x", "g", "u", "u", "g", "U", "U",
];

/// 回到 normal 空闲态：Esc 收掉提示符/可视/insert 与挂起的 char-arg。
/// `is_idle` 覆盖 count/寄存器/算子/char-arg/队列；模式归 normal 才算到位。
fn force_normal(f: &mut Fixture) {
    for _ in 0..6 {
        if f.vim.is_idle() && f.vim.mode() == Mode::Normal {
            return;
        }
        f.feed(["<Esc>"]);
    }
    panic!(
        "force_normal: 无法回到 normal 空闲态 (mode={:?})",
        f.vim.mode()
    );
}

/// undo 到底（以引擎的历史边缘响铃为停止信号），返回步数。
fn undo_to_bottom(f: &mut Fixture, cap: usize) -> usize {
    let mut depth = 0;
    loop {
        let before = f.host.bells;
        f.feed(["u"]);
        if f.host.bells > before {
            return depth;
        }
        depth += 1;
        assert!(depth <= cap, "u 循环 {cap} 步未到历史底 (mode={:?})", f.vim.mode());
    }
}

/// 历史不变量主体：先随机游走，再 undo 到底断言初始文本，再 redo 到底
/// 断言回到游走后的文本。
fn sweep(f: &mut Fixture, initial: &str, context: &str) {
    force_normal(f);
    let text_now = f.text();

    let depth = undo_to_bottom(f, 800);
    assert_eq!(
        f.text(),
        initial,
        "{context}: undo 到底后文本 ≠ 初始缓冲——有编辑没进任何 undo 组 (depth={depth})"
    );

    for _ in 0..depth {
        f.feed(["<C-r>"]);
    }
    force_normal(f);
    assert_eq!(
        f.text(),
        text_now,
        "{context}: undo-all/redo-all 未回到 undo 前的文本 (depth={depth})"
    );
}

/// 键序列 + 宿主事件（IME 替换 / 点击）的随机游走。
fn random_walk(f: &mut Fixture, rng: &mut Rng, steps: usize) {
    for _ in 0..steps {
        match rng.below(12) {
            // IME 组合替换：光标附近换一段文本（与按键流并行的编辑通道）
            0 => {
                let len = f.text().len();
                let a = rng.below(len + 1);
                let b = (a + rng.below(6)).min(len);
                let text = f.text();
                if text.is_char_boundary(a) && text.is_char_boundary(b) {
                    f.ime_replace(a..b, "中文x");
                }
            }
            // 宿主点击（不改文本，只动光标）
            1 => {
                let len = f.text().len();
                f.vim.set_cursor_offset(&f.buf, rng.below(len + 1));
            }
            _ => {
                let k = KEY_POOL[rng.below(KEY_POOL.len())];
                f.feed([k]);
            }
        }
    }
}

// 24 seeds × 40 rounds × (60..200 步) ≈ 13 万键（opt3 下秒级）
const SEEDS: usize = 24;
const ROUNDS: usize = 40;

#[test]
fn fuzz_history_undo_all_reaches_initial_and_redo_all_restores() {
    for seed in 0..SEEDS {
        let mut rng = Rng(0x27_00_00_01 + seed as u64);
        for round in 0..ROUNDS {
            let initial = BUFFERS[rng.below(BUFFERS.len())];
            let mut f = Fixture::new(initial);
            let steps = 60 + rng.below(140);
            random_walk(&mut f, &mut rng, steps);
            sweep(
                &mut f,
                initial,
                &format!("seed {seed} round {round}"),
            );
        }
    }
}

/// 定向语料版：多字节缓冲上的 IME 交错（floor/组所有权的接缝）。
#[test]
fn fuzz_history_multibyte_with_ime_interleave() {
    for seed in 0..8 {
        let mut rng = Rng(0x27_00_00_02 + seed as u64);
        for round in 0..20 {
            let initial = "中文行\nmultiline emoji😀\n尾行无换行";
            let mut f = Fixture::new(initial);
            for _ in 0..(40 + rng.below(80)) {
                if rng.below(4) == 0 {
                    let len = f.text().len();
                    let a = rng.below(len + 1);
                    let b = (a + rng.below(6)).min(len);
                    let text = f.text();
                    if text.is_char_boundary(a) && text.is_char_boundary(b) {
                        f.ime_replace(a..b, "组");
                    }
                } else {
                    let k = KEY_POOL[rng.below(KEY_POOL.len())];
                    f.feed([k]);
                }
            }
            sweep(
                &mut f,
                initial,
                &format!("multibyte seed {seed} round {round}"),
            );
        }
    }
}
