//! 第三十轮探针（一）：替换模式 R、Ex 范围边界、非法正则、多字节低频路径。
//!
//! 本轮探针走「意外操作 × 不寻常数据」路线：所有断言先按 vim 9.1 语义
//! 预期书写，跑挂的逐条上 vim 实证（或证伪为已知分歧），再决定修/记。

mod common;

use common::{edit, Fixture};

// ---------------------------------------------------------------- R 替换模式

/// 探针 A：`R` 越过行尾后继续打字——vim 是追加（无字符可覆盖）。
#[test]
fn r_mode_appends_past_eol() {
    let mut f = edit("ab\ncd\n", 0, 0, &["R"]);
    f.type_text("XYZW"); // 覆盖 ab，越过行尾后追加
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "XYZW\ncd\n");
}

/// 探针 B：`R` 中途 BS 一路退回复原。vim 9.1 typeahead 实证：
/// `RXY<BS><BS>Q<Esc>` → `Qbcdef`（BS 复原被覆盖字符，Q 再覆盖 a）。
#[test]
fn r_mode_backspace_restores() {
    let mut f = edit("abcdef\n", 0, 0, &["R"]);
    f.type_text("XY");
    f.feed(["<BS>", "<BS>"]);
    f.type_text("Q");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Qbcdef\n");
}

/// 探针 C：`R` 覆盖多字节字符——「中」3 字节，单字节覆盖后不得撕裂。
#[test]
fn r_mode_overwrites_multibyte_cleanly() {
    let mut f = edit("中文\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\n", "ab 覆盖整个「中」（3 字节）");
}

/// 探针 D：`3Rab<Esc>` 的重复覆盖语义（vim：ababab 覆盖 6 字符）。
#[test]
fn r_mode_count_repeat_overwrites() {
    let mut f = edit("abcdefghij\n", 0, 0, &["3", "R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababghij\n");
}

/// 探针 E：`R` 后直接 `<Esc>`（零输入）——不得有任何改动。
#[test]
fn r_mode_immediate_escape_is_noop() {
    let mut f = edit("ab\ncd\n", 0, 0, &["R", "<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ab\ncd\n", "空 R 会话也不该留下可撤销组");
}

/// 探针 F：替换会话里 `<C-w>`（删词）——vim 9.1 typeahead 实证：C-w 删掉
/// 打的词并**还原被覆盖的字符**（`RXX<C-w>` → `one two`，光标落打字起点；
/// `RXX<C-w>X` → `Xne two`）。
#[test]
fn r_mode_ctrl_w_restores_overwritten() {
    let mut f = edit("one two\n", 0, 0, &["R"]);
    f.type_text("XX");
    f.feed(["<C-w>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one two\n", "C-w 还原被覆盖的 on");
    // 还原后光标落打字起点，继续打字覆盖原首字符
    let mut f = edit("one two\n", 0, 0, &["R"]);
    f.type_text("XX");
    f.feed(["<C-w>"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Xne two\n");
}

// ---------------------------------------------------------------- Ex 范围边界

/// 探针 G：反向范围 `:5,1d`——vim 交互式询问交换、脚本模式拒绝（9.1
/// 实证：不删任何行）。引擎修复后报 E16 且缓冲不动。
#[test]
fn ex_reversed_range_delete_reports_e16() {
    let f = edit("l1\nl2\nl3\nl4\nl5\n", 0, 0, &[":", "5", ",", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl3\nl4\nl5\n", "反向范围不删行");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "应报 E16，实际：{:?}",
        f.host.statuses
    );
}

/// 探针 H：`:0d`——vim 9.1 实证删第一行（地址 0 合法，落到首行）。
#[test]
fn ex_zero_address_delete() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "0", "d", "<CR>"]);
    assert_eq!(f.text(), "l2\nl3\n", "0 指向首行，删第一行");
}

/// 探针 I：`:2,2d` 单行范围。
#[test]
fn ex_same_address_delete() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "2", ",", "2", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl3\n");
}

/// 探针 J：范围超出末行 `:99d`——vim 报 E16: Invalid range。
#[test]
fn ex_out_of_range_address() {
    let f = edit("l1\nl2\n", 0, 0, &[":", "9", "9", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\n", "越界地址不删行");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "应报 E16，实际：{:?}",
        f.host.statuses
    );
}

/// 探针 K：`:.,+2d` 相对范围——+2 表示「再往下 2 行」，共删 3 行。
#[test]
fn ex_relative_range_delete() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\n", 1, 0);
    f.feed([":", ".", ",", "+", "2", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\n");
}

// ---------------------------------------------------------------- 非法正则

/// 探针 L：未闭合的字符类 `/[abc` ——不得 panic，应响铃报错。
#[test]
fn search_invalid_regex_no_panic() {
    let mut f = edit("abc [abc\n", 0, 0, &["/"]);
    f.type_text("[abc");
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "abc [abc\n");
    // E383/E54 类错误信息或响铃，二有其一
    assert!(
        f.host.bells > 0 || !f.host.statuses.is_empty(),
        "非法正则需要反馈"
    );
}

/// 探针 M：`:s/[abc/X` 同样不得 panic。
#[test]
fn substitute_invalid_regex_no_panic() {
    let mut f = edit("abc\n", 0, 0, &[":", "%", "s", "/"]);
    f.type_text("[abc/X");
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "abc\n", "替换失败，缓冲不动");
}

/// 探针 N：替换文本里的 `$1` 是本引擎的正则分组语法（NOTES 分歧 #1，
/// vim 用 `\1`）。无分组时 `$1y` 展开为空（regex crate 按 $ 后最长名字
/// 找捕获组，找不到整体丢弃）；字面 `$` 用 `$$` 转义。
#[test]
fn substitute_dollar_expansion_dialect() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed([":", "%", "s", "/", "a", "b", "c", "/", "X", "$", "1", "y", "/", "<CR>"]);
    assert_eq!(f.text(), "X\n", "无分组 1：$1y 按整名查找捕获组失败 → 整体为空");
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed([":", "%", "s", "/", "a", "b", "c", "/", "X", "$", "$", "1", "y", "/", "<CR>"]);
    assert_eq!(f.text(), "X$1y\n", "$$ 转义出字面 $");
}

// ---------------------------------------------------------------- C-w/C-u 输入范围（round 30 修复）

/// 探针 T1：insert `C-u` 只删本会话输入的字符，已有文本存活（vim 9.1
/// typeahead 实证 P4：`hello world` 列 2 插入 `ab` + C-u → `hello world`）。
#[test]
fn insert_ctrl_u_scoped_to_typed_text() {
    let mut f = edit("hello world\n", 0, 2, &["i"]);
    f.type_text("ab");
    f.feed(["<C-u>", "<Esc>"]);
    assert_eq!(f.text(), "hello world\n", "C-u 不吃插入起点之前的文本");
}

/// 探针 T2：insert `C-w` 同样钳制在打字起点（P5：词中插入 X + C-w 只删 X）；
/// 第二次 C-w 光标已在打字起点，是无操作（D2），不走进已有词。
#[test]
fn insert_ctrl_w_scoped_to_typed_text() {
    let mut f = edit("abcd\n", 0, 2, &["i"]);
    f.type_text("X");
    f.feed(["<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "abcd\n");
    let mut f = edit("abcd\n", 0, 2, &["i"]);
    f.type_text("X");
    f.feed(["<C-w>", "<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "abcd\n", "第二次 C-w 不吃 ab");
}

/// 探针 T3：跨行会话的 C-u 按行作用（只删当前行的输入）。
#[test]
fn insert_ctrl_u_per_line() {
    let mut f = edit("aa\nbb\n", 1, 0, &["i"]);
    f.type_text("cd");
    f.feed(["<C-u>", "<Esc>"]);
    assert_eq!(f.text(), "aa\nbb\n", "第二行的 C-u 不动第一行");
}

/// 探针 T4：Replace 会话的 C-u 全量还原（vim 9.1 实证：
/// `Rab cd<C-u>Z` → `Zbcdefg`——还原 abcde 后 Z 覆盖 a）。
#[test]
fn replace_ctrl_u_restores_overwritten() {
    let mut f = edit("abcdefg\n", 0, 0, &["R"]);
    f.type_text("ab cd");
    f.feed(["<C-u>"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Zbcdefg\n");
}

/// 探针 T5：C-w 的词边界在 Replace 模式下同样生效且还原跨度内字符
/// （vim 9.1 实证：`Rab cd<C-w>Z` → `ab Zefg`——只删词 cd，还原 d、e）。
#[test]
fn replace_ctrl_w_word_scoped_restore() {
    let mut f = edit("abcdefg\n", 0, 0, &["R"]);
    f.type_text("ab cd");
    f.feed(["<C-w>"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab Zefg\n");
}

/// 探针 T6：多字节还原——覆盖「中」后 C-w 还原整个字符，不撕裂字节。
#[test]
fn replace_ctrl_w_restores_multibyte() {
    let mut f = edit("中文x\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "中文x\n");
}

// ---------------------------------------------------------------- :& 与 :~ 重放

/// 探针 V1：`:&` 不携带上次 `:s` 的旗标（vim 9.1 实证：
/// `:1,1s/a/B/g` 后 `:2,2&` → 第 2 行只换首个匹配）。
#[test]
fn ex_ampersand_repeats_without_flags() {
    let f = edit(
        "xaxax\nxaxax\nxaxax\n",
        0,
        0,
        &[
            ":", "1", ",", "1", "s", "/", "a", "/", "B", "/", "g", "<CR>",
            ":", "2", ",", "2", "&", "<CR>",
        ],
    );
    assert_eq!(f.text(), "xBxBx\nxBxax\nxaxax\n");
}

/// 探针 V2：`:~` 用**当前搜索模式** + 原替换串，同样不带旗标（vim 9.1
/// 实证：`/x` 重指后 `:3,3~` 把 x 换成 Q——首条 `:s` 的替换串）。
#[test]
fn ex_tilde_uses_last_search_pattern() {
    let f = edit(
        "axa\nxyx\nxax\n",
        0,
        0,
        &[
            ":", "1", ",", "1", "s", "/", "a", "/", "Q", "/", "<CR>",
            "/", "x", "<CR>",
            ":", "3", ",", "3", "~", "<CR>",
        ],
    );
    assert_eq!(f.text(), "Qxa\nxyx\nQax\n");
}

/// 探针 V3：无上次替换时 `:&`/`:~` 报 E33 并响铃（与裸 `:s` 同款）。
#[test]
fn ex_ampersand_tilde_without_previous_report_e33() {
    let mut f = Fixture::new("abc\n");
    f.feed([":", "&", "<CR>"]);
    assert_eq!(f.text(), "abc\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E33")));
    assert!(f.host.bells > 0);
    let mut f = Fixture::new("abc\n");
    f.feed([":", "~", "<CR>"]);
    assert!(f.host.statuses.iter().any(|s| s.contains("E33")));
}

/// 探针 V4：搜索模式含分隔符时 `:~` 转义存活（`/a\/b` 后 `:~` 重建的
/// `s/a\/b/Q/` 不碎裂——模式里的 `/` 必须重新转义）。
#[test]
fn ex_tilde_escapes_separator_in_pattern() {
    let f = edit(
        "a/b x\n",
        0,
        0,
        &[
            ":", "s", "/", "x", "/", "Q", "/", "<CR>", // a/b Q（模式 x）
            "/", "a", "\\", "/", "b", "<CR>", // 搜索模式 "a/b"
            ":", "~", "<CR>", // s/a\/b/Q/ → "Q Q"
        ],
    );
    assert_eq!(f.text(), "Q Q\n");
}

// ---------------------------------------------------------------- 多字节低频路径

/// 探针 O：`3r中` —— count × 多字节替换。
#[test]
fn r_cmd_count_multibyte() {
    let f = edit("abcdef\n", 0, 0, &["3", "r", "中"]);
    assert_eq!(f.text(), "中中中def\n");
}

/// 探针 P：`~` 在行尾（最后一个字符上）不跨行。
#[test]
fn tilde_at_line_end_stays() {
    let f = edit("ab\ncd\n", 0, 1, &["~"]);
    assert_eq!(f.text(), "aB\ncd\n");
    assert_eq!(f.line(), 0, "不得跨行");
}

/// 探针 J2：`J` 连接多字节行——vim 9.1 实证 CJK 行尾同样插一个空格
/// （J 只在「行尾已有空白/空行」时省略空格，与字符宽度无关）。
#[test]
fn j_multibyte_inserts_separator_space() {
    let f = edit("中文\n第二行\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "中文 第二行\n");
}

/// 探针 Q：`x` 在宽字符（占两列的 CJK）上一次删整个字符。
#[test]
fn x_on_wide_char() {
    let f = edit("中a\n", 0, 0, &["x"]);
    assert_eq!(f.text(), "a\n");
}

/// 探针 R2：`f中` 查找多字节字符目标。
#[test]
fn f_to_multibyte_target() {
    let f = edit("a中b\n", 0, 0, &["f", "中"]);
    assert_eq!(f.cursor(), "a".len(), "f 落在中字的字节起点");
}

/// 探针 S：空缓冲上的各种「无意义」操作不得 panic 或产生脏状态。
#[test]
fn empty_buffer_hostile_ops() {
    let mut f = Fixture::new("");
    for keys in [
        vec!["d", "d"],
        vec!["y", "y"],
        vec!["c", "c"],
        vec!["g", "v"],
        vec!["p"],
        vec!["P"],
        vec!["J"],
        vec!["~"],
        vec!["D"],
        vec!["C"],
        vec!["S"],
        vec!["X"],
        vec!["<C-a>"],
        vec!["g", ";"],
        vec!["g", ","],
        vec!["'", "'"],
        vec!["`", "`"],
    ] {
        f.feed(keys);
    }
    assert_eq!(f.text(), "", "空缓冲不得被开出内容");
    f.feed(["i"]);
    f.type_text("ok");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ok", "后续正常编辑不受污染");
}
