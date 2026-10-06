//! 第三十轮探针（二）：`.` 与 Ex 边界、宏/q1/m1、搜索偏移、可视与 undo。
//!
//! 方法论：所有 vim 预期先用本机 vim 9.1 `-s` typeahead 实证（无方向键、
//! Esc:wq 收尾——见 probe_round30.rs 头注的通道教训）。

mod common;

use common::{edit, Fixture};

// ---------------------------------------------------------------- `.` 与 Ex

/// 探针 W1：`.` 不重复 `:s`（vim 9.1 实证：`:1,1s/a/B/` 后 `j.` 第 2 行
/// 不变——`.` 只重放普通模式改动，重放替换是 `&`/`:&` 的职责）。
#[test]
fn dot_does_not_repeat_substitute() {
    let f = edit(
        "aaa\naaa\naaa\n",
        0,
        0,
        &[":", "1", ",", "1", "s", "/", "a", "/", "B", "/", "<CR>", "j", ".", "<CR>"],
    );
    // `j` 移动，`.` 于第 2 行无普通改动可重放（:s 不入 last_change）
    assert_eq!(f.text(), "Baa\naaa\naaa\n");
}

// ---------------------------------------------------------------- 宏与寄存器

/// 探针 W2：`q1` 数字寄存器录制（vim 文档 q{0-9a-zA-Z"}；9.1 探针：
/// 录制同时执行按键，重放 `@1` 才是判据——x 插入两次）。
#[test]
fn digit_register_macro_roundtrip() {
    let mut f = edit("abc\n", 0, 0, &["q", "1", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q", "@", "1"]);
    assert_eq!(f.text(), "xxabc\n", "录制执行一次 x（IME 路径），@1 重放再一次");
}

/// 探针 W3：`q/` 非法寄存器响铃且不启动录制（vim 同款；后续 / 开搜索）。
#[test]
fn q_invalid_register_bells_and_stays_idle() {
    let mut f = edit("abc\n", 0, 0, &["q", "/"]);
    assert!(f.host.bells > 0, "q/ 应响铃");
    // / 照常打开搜索提示符（不处于待寄存器状态），b 正常进提示符
    f.feed(["/", "b", "<CR>"]);
    assert_eq!(f.cursor(), "a".len(), "/b 正常搜索落 b");
}

/// 探针 W4：`m1` 非法 mark 名——vim 9.1 静默无操作（文件不变），引擎
/// 同样不落账（marks.set 只收字母与 ^ .）。
#[test]
fn m_digit_mark_is_rejected() {
    let f = edit("abc\n", 0, 0, &["m", "1"]);
    assert_eq!(f.text(), "abc\n");
    // '1 跳转报 E20（未设 mark）
    let mut f = edit("abc\n", 0, 0, &["m", "1", "'", "1"]);
    assert_eq!(f.cursor(), 0, "m1 后 '1 不得有落点");
}

// ---------------------------------------------------------------- 搜索与高亮

/// 探针 W5：搜索跳过光标处匹配（vim：搜索从光标后开始）+ 空 `/` 回车
/// 重查上次模式并从当前位置向后（可回绕）。引擎实测序：`/foo<CR>` 落 8
/// （跳过 0 处当前匹配）、`n` 回绕 0、空 `/` + CR 从 0 再落 8。
#[test]
fn empty_search_enter_reuses_last_pattern() {
    let mut f = edit("foo bar foo\n", 0, 0, &["/", "f", "o", "o", "<CR>"]);
    assert_eq!(f.cursor(), 8, "搜索跳过光标处的匹配");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 0, "n 回绕到第一个匹配");
    // 空 / + CR：重查上次模式，方向随当前提示符
    f.feed(["/", "<CR>"]);
    assert_eq!(f.cursor(), 8, "重查从当前位置向后");
}

// ---------------------------------------------------------------- 可视与 undo

/// 探针 W6：`o<Esc>`（空开行）后 `u` 撤销开行（vim：一行消失）。
#[test]
fn empty_open_line_undo() {
    let f = edit("ab\n", 0, 0, &["o", "<Esc>", "u"]);
    assert_eq!(f.text(), "ab\n", "u 撤掉空行");
}

/// 探针 W7：可视行选 `Vj>` 缩进两行，撤销一次恢复（组语义）。
#[test]
fn visual_line_indent_undo_group() {
    let f = edit("a\nb\nc\n", 0, 0, &["V", "j", ">", "u"]);
    assert_eq!(f.text(), "a\nb\nc\n", "一次 u 恢复两行缩进");
}

/// 探针 W8：`cc` 于空行插入，`.` 重放位置正确（vim：`.` 在重放处执行）。
#[test]
fn cc_on_empty_line_then_dot() {
    let mut f = edit("\nab\n", 0, 0, &["c", "c"]);
    f.type_text("X");
    f.feed(["<Esc>", "j", "."]);
    // 第 2 行 "ab" 被 cc 清空后插入 X；`.` 重放在第 2 行（原 ab 行）
    assert_eq!(f.text(), "X\nX\n");
}

/// 探针 W9：单个字符缓冲上的 `diw`——vim 删掉该字符留空行。
#[test]
fn diw_on_single_char_line() {
    let f = edit("x\n", 0, 0, &["d", "i", "w"]);
    assert_eq!(f.text(), "\n");
}

/// 探针 W10：`ci(` 无配对括号——响铃不进入插入。
#[test]
fn ci_on_unbalanced_parens_bells() {
    let f = edit("abc\n", 0, 0, &["c", "i", "("]);
    let bells = f.host.bells;
    assert!(bells > 0, "无配对括号应响铃");
    assert!(
        !matches!(f.vim.mode(), vimcore::mode::Mode::Insert),
        "不得进入插入模式"
    );
}

// ---------------------------------------------------------------- Ex 杂项边界

/// 探针 W11：`:%d` 清空后缓冲仍有一行（空行不变量），光标 0。
#[test]
fn percent_delete_leaves_single_empty_line() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "%", "d", "<CR>"]);
    assert_eq!(f.text(), "");
    assert_eq!(f.cursor(), 0);
}

/// 探针 W12：`:1,2d a` 寄存器落账 + `"ap` 粘回。p 粘在光标行**之后**
/// （删完光标落 l3 行），复原序为 l3/l1/l2。
#[test]
fn ex_delete_into_named_register() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "1", ",", "2", "d", " ", "a", "<CR>", "\"", "a", "p"]);
    assert_eq!(f.text(), "l3\nl1\nl2\n");
}

/// 探针 W13：`:set ts?` 查询反馈走 status_message。
#[test]
fn set_query_reports_value() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=4")),
        "实际：{:?}",
        f.host.statuses
    );
}
