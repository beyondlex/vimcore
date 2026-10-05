//! 第二十九轮探针：异常操作序列与边界数据，找回归。

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

/// 探针 1：宏录制会话结束后 `.` 不应重放 `q{reg}` 前缀。
///
/// 旧实现把 `q`/`a` 一起记进了 `recording`，`exit_insert` 提交时它们混进
/// `last_change`；`.` 重放 [q, a, …] 会重新打开一个永远不会停止的宏录制
/// 会话（status UI 显示 recording，后续按键被吞进宏）。
#[test]
fn probe_dot_after_macro_session_does_not_rearm_recording() {
    // qaxix<Esc>q：进入宏录制，i 进入插入，打 x，Esc 退出，停止录制。
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["q", "a", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q"]);
    assert_eq!(f.text(), "xab\ncd\n");
    assert_eq!(f.vim.macro_len('a'), 3, "宏应记录 [i, Text(x), Esc] 三步");
    assert!(
        f.vim.macro_recording().is_none(),
        "宏停止后不得仍在录制"
    );

    // `.` 重放插入：不得重新进入宏录制会话，宏内容保持原样。
    // （`q`/`a` 前缀在宏启动时就被 complete_char_arg 尾部的 end_command
    // 从 recording 清掉，last_change 只含 [i, Text, Esc]。）
    f.feed(["."]);
    assert_eq!(f.text(), "xxab\ncd\n", "`.` 应重复插入 x");
    assert!(
        f.vim.macro_recording().is_none(),
        "`.` 重放不得重启宏录制"
    );
    assert_eq!(f.vim.macro_len('a'), 3, "`.` 不得清空/改写宏 a");

    // `.` 之后的 x 是普通删除（光标停在重放插入的首字符上）。
    f.feed(["x"]);
    assert_eq!(f.text(), "xab\ncd\n", "`.` 之后的 x 是普通删除");
    assert_eq!(f.vim.macro_len('a'), 3, "宏 a 不应捕获 `.` 之后的按键");
}

/// 探针 1b：无编辑的宏（`qaq`）不得污染 `.` 的重放记录。
#[test]
fn probe_empty_macro_keeps_last_change() {
    let mut f = Fixture::at("foo\n", 0, 0);
    f.feed(["x"]); // last_change = [x]
    f.feed(["q", "a", "q"]); // 空宏
    f.feed(["G"]); // 移动光标
    f.feed(["."]);
    assert_eq!(f.text(), "o\n", "`.` 应重复 [x]（foo → oo → o），不被空宏覆盖");
}

/// 探针 2：空模式的 incsearch 预览。
///
/// `/foo` 后连按三次 BS 清空提示符：空正则在每个字节位置都匹配零宽，
/// 旧实现把上万个零宽匹配发布给宿主。vim 的预览回落到上一次搜索的高亮。
#[test]
fn probe_incsearch_empty_pattern_restores_previous_highlights() {
    let mut f = Fixture::at("foo bar foo\n", 0, 0);
    f.feed(["/"]); // 打开提示符
    f.feed(["f", "o", "o"]);
    let preview_count = f.host.highlights.len();
    assert!(preview_count >= 2, "incsearch 预览应显示 foo 的匹配");

    // 清空提示符：预览必须回落（不得发布零宽全量匹配）。
    f.feed(["<BS>", "<BS>", "<BS>"]);
    assert!(
        f.host.highlights.iter().all(|r| !r.is_empty()),
        "空模式预览不得发布零宽匹配，got {:?}",
        f.host.highlights
    );
    assert_eq!(
        f.host.highlights.len(),
        0,
        "此前无已接受模式时，空预览应清空高亮"
    );
}

/// 探针 3：`dgn` 在无模式时安静报 E35，不 panic。
#[test]
fn probe_dgn_without_pattern_reports_e35() {
    let f = edit("abc\n", 0, 0, &["d", "g", "n"]);
    assert_eq!(f.text(), "abc\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E35")),
        "应报 E35，got {:?}",
        f.host.statuses
    );
}

/// 探针 4：`<C-a>` 的十六进制/二进制前缀边界数据。
#[test]
fn probe_increment_radix_edges() {
    // 光标在 `0x` 的 0 上但后无十六进制数字：0x 不是合法字面量，退回十进制递增 0。
    let f = edit("0x tail\n", 0, 0, &["<C-a>"]);
    assert_eq!(f.text(), "1x tail\n");

    // 光标在 x 上：0 在光标之前，vim 报 E18 且不动。
    let f = edit("0x tail\n", 0, 1, &["<C-a>"]);
    assert_eq!(f.text(), "0x tail\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E18")),
        "光标后无数字应报 E18，got {:?}",
        f.host.statuses
    );

    // 0b 前缀后跟 2（非二进制数字）：不按二进制处理。
    let f = edit("0b2 tail\n", 0, 0, &["<C-a>"]);
    assert_eq!(f.text(), "1b2 tail\n");

    // 巨数饱和，不 panic。
    let f = edit("99999999999999999999999\n", 0, 0, &["<C-a>"]);
    assert!(!f.text().is_empty());
}

/// 探针 5：Replace 模式里敲 Enter 用换行替换光标字符（vim：拆行）。
#[test]
fn probe_replace_mode_enter_splits_line() {
    let f = {
        let mut f = Fixture::at("abc\n", 0, 1);
        f.feed(["R"]);
        f.feed_raw(Key::named("enter"));
        f
    };
    assert_eq!(f.text(), "a\nc\n", "R<CR> 应把光标字符换成换行");
}

/// 探针 6：可视块 `I` 之后立即 Esc（无输入）——不得复制、不得 panic。
#[test]
fn probe_block_insert_empty_escape() {
    let f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", "I", "<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
}

/// 探针 7：`"Ayy` 大写寄存器追加后 `:reg` 不 panic，粘贴得合并结果。
#[test]
fn probe_uppercase_append_yank() {
    let mut f = Fixture::at("one\ntwo\n", 0, 0);
    f.feed(["\"", "a", "y", "y"]); // 先把 one 装进 "a
    f.feed(["j"]);
    f.feed(["\"", "A", "y", "y"]); // "A 追加 two
    f.feed(["G"]);
    f.feed(["\"", "a", "p"]);
    assert_eq!(f.text(), "one\ntwo\none\ntwo\n", "追加应得 one+two 两行");
}

/// 探针 8：块可视 `p` 用块寄存器替换后 `gv` 恢复原块范围。
#[test]
fn probe_block_put_gv_restores_bounds() {
    let mut f = Fixture::at("abcd\nefgh\n", 0, 0);
    // 块 yank 第一列（2 行 1 列），再块 put 回去。
    f.feed(["<C-v>", "j", "y"]);
    f.feed(["<C-v>", "j", "l", "p"]);
    assert_eq!(f.text(), "acd\negh\n", "块寄存器行 [a, e] 逐行替换 [ab, ef]");
}
