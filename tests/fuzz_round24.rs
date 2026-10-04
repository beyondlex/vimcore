//! 第二十四轮 fuzz：字素簇/宽字符语料上的全按键轰炸。
//!
//! 背景：round24 修复了六处「光标落在字素簇中间」的路径（Esc 步退、
//! clamp_cursor、display-column 钳制、`$`、`g_`、f/t/F/T 的 till 停点、
//! charwise p 落点、insert BS、first_non_blank 的零宽跳过）。修复共同
//! 声明了一条此前从未被 fuzz 检查的不变量：
//!
//!   **normal 模式下光标永远不落在宽度为 0 的字符上**（组合字符/VS16/
//!   ZWJ 都是簇的延续，光标停那里 `x` 就会把簇拆开）。
//!
//! 语料覆盖：分解式组合字符（é = e+U+0301）、VS16（#\u{FE0F}）、ZWJ 家族
//! （👨‍👩‍👧 + 肤色修饰）、CJK 与全角空白（U+3000）、簇在行中/行尾/独占行的
//! 各种布局。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

/// 简单 LCG：确定性随机（无外部依赖）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// 字素簇语料行：簇位于行中/行尾/行首，含宽度 0 的延续字符。
const CLUSTER_CORPUS: &[&str] = &[
    // 分解式 é 在行尾：Esc 步退 / $ / g_ 的落点
    "ab\u{0301}\n",
    "ab\u{0301}\ncd\n",
    // VS16 行尾
    "#\u{FE0F}\nnext\n",
    // ZWJ 家族行尾（家庭 emoji，单簇 5 字符）
    "x\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\n",
    // 簇在行中：tx/ta 的停点、h/l 的跨越
    "a\u{0301}bc\n",
    "xa\u{0301}b\u{0301}c\n",
    // 行首组合字符：^ / gg / - 的落点
    "\u{0301}abc\n",
    // 全角空白：w/W 的跳过、g_ 的尾部
    "a\u{3000}b\n",
    "ab\u{3000}\n",
    // 混合：CJK + 簇 + 全角空白
    "中a\u{0301}文\u{3000}b\n",
    // 肤色修饰符（U+1F3FB 也是延续字符）
    "\u{1F44D}\u{1F3FB}x\n",
    // 簇独占一行
    "one\na\u{0301}\nthree\n",
];

/// 按键池：覆盖移动/编辑/字符参数/可视/Ex 入口。
const KEY_POOL: &[&str] = &[
    "h", "l", "j", "k", "w", "b", "e", "0", "$", "^", "g", "_", "x", "X", "p", "P", "d", "d", "c",
    "c", "y", "y", "r", "x", "r", "<CR>", "~", "g", "u", "u", "g", "U", "U", "f", "x", "F", "x",
    "t", "x", "T", "x", ";", ",", "i", "a", "o", "<Esc>", "<BS>", "<C-w>", "<C-u>", "v", "V",
    "<C-v>", "d", "y", ">", "<", "g", "q", "q", "q", "J", "d", "p", "*", "n", "N", "1", "2", "3",
    ":", "s/x/Y/g\n", "/", "a\n", "<CR>", "u", ".", "g", "v", "d", "a", "w", "c", "i", "w",
    "d", "t", "x", "y", "t", "x",
];

/// normal 模式光标必须落在「宽度 ≥ 1 的字符」上（簇起点），且是字符边界。
fn assert_cursor_on_visible_char(f: &Fixture, context: &str) {
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= text.len() && text.is_char_boundary(cur),
        "{context}: cursor {cur} 不是字符边界 (len={}) in {text:?}",
        text.len()
    );
    if let Some(c) = text[cur..].chars().next() {
        let width = vimcore::buffer::char_display_width(c);
        if width == 0 {
            // 唯一合法的例外:基字符被删除后的独立 mark(自成簇,且行内
            // 没有可贴的基字符——只检查行首形态)
            let ls = f.buf.line_start(f.buf.offset_to_line(cur));
            assert!(
                cur == ls,
                "{context}: 光标落在宽度 0 的延续字符 U+{:04X} 上 (offset {cur}, 行起 {ls}) in {text:?}",
                c as u32
            );
        }
    }
    // 行寻址契约顺带维持
    let buf = &f.buf;
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= text.len(),
            "{context}: line {line} range {s}..{e} 越界 (len={})",
            text.len()
        );
    }
}

#[test]
fn fuzz_cluster_corpus_never_parks_cursor_on_zero_width_char() {
    let mut rng = Rng(0x24_24_24_24);
    for (ci, corpus) in CLUSTER_CORPUS.iter().enumerate() {
        for round in 0..40 {
            let mut f = Fixture::new(corpus);
            let context = format!("corpus#{ci} {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for step in 0..100 {
                let key = KEY_POOL[rng.below(KEY_POOL.len())];
                f.feed([key]);
                log.push(key.to_owned());
                // 提示符开着时跳过光标检查（cmdline 光标是宿主渲染的插入条，
                // 语义不同）；insert 模式光标允许在行尾/簇后（打字中）
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(&f, &format!("{context} step#{step} keys={log:?}"));
                }
            }
            // 收尾：Esc 归位后不变量必须成立
            f.feed(["<Esc>", "<Esc>"]);
            if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                assert_cursor_on_visible_char(&f, &format!("{context} final keys={log:?}"));
            }
        }
    }
}

/// 定向：可视化算子与宏回放在簇语料上不 panic、选区可寻址。
#[test]
fn fuzz_visual_and_macro_on_cluster_corpus() {
    let mut rng = Rng(0x24_C1A5_BEEF);
    for corpus in CLUSTER_CORPUS.iter() {
        for round in 0..25 {
            let mut f = Fixture::new(corpus);
            let context = format!("vis/macro {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for _ in 0..60 {
                match rng.below(10) {
                    0..=5 => {
                        let key = KEY_POOL[rng.below(KEY_POOL.len())];
                        f.feed([key]);
                        log.push(key.to_owned());
                    }
                    6 => {
                        f.feed(["v"]);
                        f.feed(["i", "w"]);
                        log.push("v iw".to_owned());
                    }
                    7 => {
                        f.feed(["q", "a", "x", "j", "q"]);
                        f.feed(["@", "a"]);
                        log.push("qaxjq @a".to_owned());
                    }
                    8 => {
                        f.feed(["g", "v"]);
                        log.push("gv".to_owned());
                    }
                    _ => {
                        f.feed(["<C-v>"]);
                        log.push("<C-v>".to_owned());
                    }
                }
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(&f, &format!("{context} keys={log:?}"));
                }
            }
        }
    }
}

/// 定向回归：行首组合字符上 `^` 跳到首个可见字符（first_non_blank 的
/// 零宽跳过）。
#[test]
fn caret_skips_leading_combining_mark() {
    // U+0301 占 2 字节：'a' 在 offset 2
    let f = common::edit("\u{0301}abc\n", 0, 4, &["^"]);
    assert_eq!(f.cursor(), 2, "^ 应落在首个可见字符 a");
    // mark 跟在缩进后：" \u{0301}x" → 落在 x（space@0, mark@1..3, x@3）
    let f = common::edit(" \u{0301}x\n", 0, 0, &["^"]);
    assert_eq!(f.cursor(), 3, "^ 应跳过 mark 落在 x");
}

/// 定向回归：块插入会话内 IME 簇文本 + 退格，退出侧 delta 不变量保持。
#[test]
fn block_session_cluster_typing_and_backspace() {
    use std::iter;
    let mut f = Fixture::new("ab\ncd\nef\n");
    // 块选三行首列，I 进入块插入
    f.feed(["<C-v>", "2", "j", "I"]);
    f.type_text("x\u{0301}"); // 簇文本
    f.feed_raw(vimcore::key::Key::named("backspace")); // 整簇删除
    f.type_text("Z");
    f.feed(["<Esc>"]);
    let text = f.text();
    assert!(
        text.matches('Z').count() == 3,
        "三行各得一个 Z（簇 BS 不得破坏复制不变量）: {text:?}"
    );
    let _ = iter::once('x');
}
