//! 第二十五轮 fuzz：行边界 × 字素簇（GB4 硬边界语料）全按键轰炸。
//!
//! 背景：round25 修复了三类「字素簇越过 `\n`」的路径——`w` 的空白跳过
//! 不吸收延续字符、`prev/next_grapheme_offset` 把行首/行尾的独立 mark 与
//! ZWJ 胶合进 `\n` 的簇、insert `<Del>` 只删基字符。这些修复共同声明了
//! 两条此前未被同时检查的不变量：
//!
//!   1. **round24 不变量**：normal 光标不落在宽度 0 的字符上（行首独立
//!      簇形态除外）；
//!   2. **本轮新增**：normal 光标不落在 `\n` 上——除非该行是空行（空行
//!      的「行首」就是 `\n` 的位置，无字符可停）。旧 GB4 bug 的表现正是
//!      clamp_cursor 把光标钳到**上一行**的换行符位置，`x` 一按就并线。
//!
//! 语料覆盖 round24 没有的形态：独立 mark 独占行/行尾、ZWJ 在行尾与行首、
//! 空格后跟 mark（`w` 的修复面）、`\n` 紧邻簇的各种排布。

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

/// 行边界 × 簇语料：独立延续字符紧贴 `\n` 的各种排布（GB4 修复面）。
const BOUNDARY_CORPUS: &[&str] = &[
    // 行尾独立 mark（修复前 $/clamp 会越界到上一行）
    "ab\n\u{0301}",
    "ab\n\u{0301}\ncd\n",
    // 行首独立 mark（^/w/与上一行的边界交互）
    "\u{0301}abc\n",
    "ab\n\u{0301}cd",
    // 行尾/行首 ZWJ（ZWJ 分支的 GB4 修复面）
    "ab\u{200D}\ncd",
    "ab\n\u{200D}cd\n",
    "x\u{200D}\n",
    // 空格后跟 mark（w 空白跳过修复面）
    "a \u{0301}bc",
    "  \u{0301}abc\nde\u{200D}\nfg\n",
    // 空行紧邻簇
    "\n\u{0301}\nabc\n",
    "one\n\n\u{0301}\n\nthree\n",
    // 全角空白 + 独立 mark 混排
    "a\u{3000}\u{0301}b\n\u{0301}\u{3000}c\n",
    // 肤色修饰符在行尾
    "\u{1F44D}\u{1F3FB}\n\u{1F44D}\nx\n",
    // VS16 行尾 + 下一行簇开头
    "#\u{FE0F}\n\u{0301}xy\n",
];

/// 按键池：移动/编辑/字符参数/可视/Ex 入口（沿 round24 的形态）。
const KEY_POOL: &[&str] = &[
    "h", "l", "j", "k", "w", "b", "e", "0", "$", "^", "g", "_", "x", "X", "p", "P", "d", "d", "c",
    "c", "y", "y", "r", "x", "r", "<CR>", "~", "g", "u", "u", "g", "U", "U", "f", "x", "F", "x",
    "t", "x", "T", "x", ";", ",", "i", "a", "o", "<Esc>", "<BS>", "<C-w>", "<C-u>", "v", "V",
    "<C-v>", "d", "y", ">", "<", "g", "q", "q", "q", "J", "d", "p", "*", "n", "N", "1", "2", "3",
    ":", "s/x/Y/g\n", "/", "a\n", "<CR>", "u", ".", "g", "v", "d", "a", "w", "c", "i", "w",
    "d", "t", "x", "y", "t", "x", "G", "g", "g", "%", "D", "C", "S", "Y", "z", "z", "m", "a",
    "'", "a", "`", "a", "g", "n", "g", "N", "d", "g", "n", "<C-a>", "<C-x>", "g", "J", "g",
    "I", "R", "x", "<Del>", "g", "e", "g", "E", "W", "B", "E", "{", "}", "(", ")", "|",
];

/// normal 光标必须落在「宽度 ≥ 1 的字符」上（簇起点），且是字符边界；
/// 且永不落在 `\n` 上——空行的行首（= `\n` 位置）是唯一例外。
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
            // 唯一合法例外：独立 mark 自成簇且位于行首
            let ls = f.buf.line_start(f.buf.offset_to_line(cur));
            assert!(
                cur == ls,
                "{context}: 光标落在宽度 0 的延续字符 U+{:04X} 上 (offset {cur}, 行起 {ls}) in {text:?}",
                c as u32
            );
        }
        if c == '\n' {
            // 本轮新增：光标不得停在 \n 上，除非是空行的行首（无字符可停）
            let (ls, le) = {
                let line = f.buf.offset_to_line(cur);
                (f.buf.line_start(line), f.buf.line_end(line))
            };
            assert!(
                cur == ls && ls == le,
                "{context}: 光标落在 \\n 上 (offset {cur}, 行起 {ls}, 行尾 {le}) in {text:?} —— GB4 修复面回归"
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
fn fuzz_boundary_corpus_never_parks_on_newline_or_mark() {
    let mut rng = Rng(0x25_25_25_25);
    for (ci, corpus) in BOUNDARY_CORPUS.iter().enumerate() {
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
                    assert_cursor_on_visible_char(
                        &f,
                        &format!("{context} step#{step} keys={log:?}"),
                    );
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

/// 定向：GB4 修复面上的算子/对象/宏交错不 panic、缓冲保持可寻址。
#[test]
fn fuzz_boundary_corpus_operators_and_macros() {
    let mut rng = Rng(0x25_B04D_C0DE);
    for corpus in BOUNDARY_CORPUS.iter() {
        for round in 0..25 {
            let mut f = Fixture::new(corpus);
            let context = format!("op/macro {corpus:?} round#{round}");
            let mut log: Vec<String> = Vec::new();
            for _ in 0..60 {
                match rng.below(12) {
                    0..=6 => {
                        let key = KEY_POOL[rng.below(KEY_POOL.len())];
                        f.feed([key]);
                        log.push(key.to_owned());
                    }
                    7 => {
                        f.feed(["v", "i", "w"]);
                        log.push("v iw".to_owned());
                    }
                    8 => {
                        f.feed(["y", "y", "p"]);
                        log.push("yyp".to_owned());
                    }
                    9 => {
                        f.feed(["q", "a", "x", "q", "@", "a"]);
                        log.push("qaxq@a".to_owned());
                    }
                    10 => {
                        // IME 风格文本落地（insert 模式内的簇文本）
                        if matches!(f.vim.mode(), vimcore::mode::Mode::Insert) {
                            f.type_text("\u{0301}");
                            log.push("type mark".to_owned());
                        }
                    }
                    _ => {
                        f.feed(["g", "b", "d"]); // 无 gb 命令：哑铃路径
                        log.push("gbd".to_owned());
                    }
                }
                if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                    assert_cursor_on_visible_char(
                        &f,
                        &format!("{context} keys={log:?}"),
                    );
                }
            }
            f.feed(["<Esc>", "<Esc>"]);
            if matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
                assert_cursor_on_visible_char(&f, &format!("{context} final keys={log:?}"));
            }
        }
    }
}
