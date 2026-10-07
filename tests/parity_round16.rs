//! 第十六轮回归：空文本 linewise 计数重复（`3o<Esc>`）、autoindent 空行
//! 生命周期（vim did_ai 旗标：Esc/CR/BS 三路剥离）、失败删除族的响铃、
//! 宿主拖选 API 的模式守卫、showcmd 的 char-arg 命令字母。全部期望行为
//! 经 vim 9.1 探针实证（探针脚本与输出见 NOTES 第十六轮：P/Q/R/S 系列）。

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

// ---- 空文本计数重复（探针 Q3-Q5 / R1 / R7 / Q2）----------------------------

/// PROBE R1/R7: vim 9.1 `3o<Esc>` 于缩进行 → 三行**纯空行**（ai 缩进不随
/// 空复制走），光标落末份复制行。旧引擎空文本即放弃，只开一行。
#[test]
fn empty_count_repeat_opens_lines() {
    let f = edit("    ind\n", 0, 0, &["3", "o", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n\n\n", "R1: three PLAIN empty lines");
    assert_eq!(f.line(), 3, "cursor on the last copy");

    let f = edit("x\n", 0, 0, &["2", "o", "<Esc>"]);
    assert_eq!(f.text(), "x\n\n\n", "R7: 2o<Esc> → two lines");

    let f = edit("    ind\n", 0, 0, &["3", "O", "<Esc>"]);
    assert_eq!(f.text(), "\n\n\n    ind\n", "3O<Esc> opens three above");
}

/// PROBE Q3/Q4/Q5: charwise 空会话（`3i`/`3A`/`3R` + Esc）什么都不重复。
#[test]
fn empty_charwise_repeat_is_noop() {
    let f = edit("x\n", 0, 0, &["3", "i", "<Esc>"]);
    assert_eq!(f.text(), "x\n", "Q3: 3i<Esc>");
    let f = edit("abc\n", 0, 0, &["3", "A", "<Esc>"]);
    assert_eq!(f.text(), "abc\n", "Q5: 3A<Esc>");
    let f = edit("abc\n", 0, 0, &["3", "R", "<Esc>"]);
    assert_eq!(f.text(), "abc\n", "Q4: 3R<Esc>");
}

/// 导航弃权对空会话同样生效：`2o<Down><Esc>` 不复制。
#[test]
fn navigated_empty_repeat_does_not_replicate() {
    let mut f = edit("a\nb\n", 0, 0, &["2", "o"]);
    f.feed(["<Down>", "<Esc>"]);
    assert_eq!(f.text(), "a\n\nb\n");
}

// ---- autoindent 空行生命周期（探针 S1-S8）-----------------------------------

/// PROBE S1: `o<Esc>` 留**纯空行**（不是缩进），光标落行首 (2,1)。
#[test]
fn open_line_esc_strips_autoindent() {
    let mut f = edit("    ind\n", 0, 0, &["o"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n", "S1");
    assert_eq!(f.cursor(), 8, "cursor at line start like vim (2,1)");
}

/// PROBE S2: `o<Down><Esc>` —— 未触打的行在光标离开后**保留**缩进
/// （Esc 只剥当前行）。
#[test]
fn open_line_then_down_keeps_indent() {
    let mut f = edit("    ind\nzz\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.feed(["<Down>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n    \nzz\n", "S2");
}

/// PROBE S3: `o<CR><Esc>` 两行全空 —— CR 在拆行前剥掉被拆行的陈旧缩进，
/// Esc 再剥新行的。
#[test]
fn open_line_cr_esc_leaves_both_empty() {
    let mut f = edit("    ind\n", 0, 0, &["o"]);
    f.feed(["<CR>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n\n", "S3");
}

/// PROBE S4: 打满再删光（`ofoo<BS><BS><BS><Esc>`）**保留**缩进 —— 打字
/// 解除了 did_ai。
#[test]
fn typed_then_deleted_keeps_indent() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.type_text("foo");
    f.feed(["<BS>", "<BS>", "<BS>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n    \n", "S4");
}

/// PROBE S5: `o<BS>` 一笔删掉整段缩进（不是一字符）。
#[test]
fn backspace_on_untouched_indent_deletes_it_all() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(f.text(), "    ind\n\n", "S5");
}

/// PROBE S6/S8: `O<Esc>` 与 `cc<Esc>` 同族 —— 纯空行。
#[test]
fn open_above_and_cc_strip_too() {
    let mut f = edit("    ind\n", 0, 0, &["O"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\n    ind\n", "S6");
    let f = edit("    ind\n", 0, 0, &["c", "c", "<Esc>"]);
    assert_eq!(f.text(), "\n", "S8");
}

/// 打字文本的缩进照常保留（R2：`3ofoo<Esc>` 三份都带缩进）。
#[test]
fn typed_open_line_keeps_indent_and_replicates_it() {
    let mut f = edit("    ind\n", 0, 0, &["<Esc>", ":", "s", "e", "t", " ", "a", "i", "<CR>", "3", "o"]);
    f.type_text("foo");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ind\n    foo\n    foo\n    foo\n", "R2");
    assert_eq!(f.line(), 3, "cursor on the last copy");
}

/// noai 下行为不变：`o` 本就不插缩进，Esc 无可剥。
#[test]
fn noautoindent_open_line_unchanged() {
    let f = edit(
        "    ind\n",
        0,
        0,
        &[":", "s", "e", "t", " ", "n", "o", "a", "i", "<CR>", "o", "<Esc>"],
    );
    assert_eq!(f.text(), "    ind\n\n");
}

// ---- 失败删除族响铃（vim 同款）----------------------------------------------

/// 空行 `x`、行首 `X`、空行 `D`、EOF 的 `J`：vim 响铃，旧引擎静默。
#[test]
fn failed_deletes_and_joins_bell() {
    let f = edit("a\n\nb\n", 1, 0, &["x"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1, "x on an empty line must bell");

    let f = edit("ab\n", 0, 0, &["X"]);
    assert_eq!(f.text(), "ab\n");
    assert_eq!(f.host.bells, 1, "X at column zero must bell");

    let f = edit("a\n\nb\n", 1, 0, &["D"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1, "D on an empty line must bell");

    let f = edit("a\nb\n", 1, 0, &["J"]);
    assert_eq!(f.text(), "a\nb\n");
    assert_eq!(f.host.bells, 1, "J at EOF must bell");

    let f = edit("a\nb\n", 1, 0, &["g", "J"]);
    assert_eq!(f.text(), "a\nb\n");
    assert_eq!(f.host.bells, 1, "gJ at EOF must bell");
}

/// R3 探针：count 接合触不到缓冲末时，够得着的接合照常完成。
/// `3J` 于 EOF-1：两个接缝只剩一个，vim 对失败的那次响铃；
/// `2J`（单接缝）则完全成功、不响（探针 R3 无铃）。
#[test]
fn partial_join_bells_after_doing_its_work() {
    let f = edit("a\nb\nc\n", 1, 0, &["2", "J"]);
    assert_eq!(f.text(), "a\nb c\n", "the reachable join still runs");
    assert_eq!(f.host.bells, 0, "2J has exactly one seam — no failure");

    let f = edit("a\nb\nc\n", 1, 0, &["3", "J"]);
    assert_eq!(f.text(), "a\nb c\n", "the reachable join still runs");
    assert_eq!(f.host.bells, 1, "the second seam is impossible — bell");
}

/// `s` 于空行：删除半失败响铃，但插入半照常进入。
#[test]
fn substitute_char_on_empty_line_bells_then_inserts() {
    let f = edit("a\n\nb\n", 1, 0, &["s"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert_eq!(f.host.bells, 1);
    assert!(
        f.vim.mode_indicator().contains("INSERT"),
        "s still enters insert mode"
    );
}

// ---- 宿主拖选 API 的模式守卫-------------------------------------------------

/// 拖选在 insert 会话/提示符期间被忽略：模式机不得裂成两半
/// （旧实现会把 mode 改写成 Visual 而 insert_session 仍存活）。
#[test]
fn host_visual_range_ignored_outside_normal_and_visual() {
    let mut f = Fixture::new("alpha\nbeta\n");
    f.feed_raw(Key::char('i'));
    f.vim.set_visual_range(&f.buf, 0, 5);
    assert!(
        f.vim.mode_indicator().contains("INSERT"),
        "drag during insert must not change the mode"
    );
    assert!(f.vim.visual_selection().is_none());
    // 会话依旧闭合：Esc 正常退出
    f.feed_raw(Key::escape());
    assert_eq!(f.vim.mode_indicator(), "");

    f.feed_raw(Key::char(':'));
    f.vim.set_visual_range(&f.buf, 0, 5);
    assert_eq!(
        f.vim.mode_indicator(),
        "CMDLINE",
        "drag during the prompt must not change the mode"
    );
}

/// 提示符期的点击不移动缓冲光标（Ex 范围默认地址读它）。
#[test]
fn host_click_during_cmdline_ignored() {
    let mut f = Fixture::new("alpha\nbeta\n");
    let before = f.vim.cursor_offset();
    f.feed_raw(Key::char(':'));
    f.vim.set_cursor_offset(&f.buf, 8);
    assert_eq!(
        f.vim.cursor_offset(),
        before,
        "click at the prompt must not move the buffer cursor"
    );
}

// ---- showcmd 补全 ------------------------------------------------------------

/// 等待 char-argument 期间 showcmd 显示命令字母（旧实现空白）；
/// 前缀 count 一并显示，参数到达后追加。
#[test]
fn showcmd_shows_pending_char_arg_command() {
    let mut f = Fixture::new("abc\n");
    f.feed(["3"]);
    assert_eq!(f.vim.showcmd(), "3", "count shows alone");
    f.feed(["r"]);
    assert_eq!(f.vim.showcmd(), "3r", "count then r");
    f.feed(["x"]);
    assert_eq!(f.vim.showcmd(), "", "completed command clears showcmd");

    let mut f = Fixture::new("abc\n");
    f.feed(["f"]);
    assert_eq!(f.vim.showcmd(), "f");
    let mut f = Fixture::new("abc\n");
    f.feed(["'", ]);
    assert_eq!(f.vim.showcmd(), "'");
    let mut f = Fixture::new("abc\n");
    f.feed(["@"]);
    assert_eq!(f.vim.showcmd(), "@");
}
