//! 第二十九轮探针（二）：块插入会话、边界缓冲、`~`/E18 反馈、宏追加。

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

/// 探针 A：块可视 `c` 会话——打字、退格、Esc 后副本随 typing 行净变化收缩。
#[test]
fn probe_block_change_backspace_shrinks_replica() {
    let mut f = Fixture::at("abcd\nefgh\n", 0, 1);
    f.feed(["<C-v>", "j", "c"]); // 选中两行各 1 列（b/f），进入块插入
    f.type_text("XY");
    f.feed_raw(Key::backspace()); // 撤掉 Y → 净输入 X
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "aXcd\neXgh\n", "副本应与 typing 行净输入一致");
}

/// 探针 B：块可视 `c` 立即 Esc——选中列删除、行保留、无副本、无 panic。
#[test]
fn probe_block_change_empty_escape() {
    let f = {
        let mut f = Fixture::at("abcd\nefgh\n", 0, 1);
        f.feed(["<C-v>", "j", "c", "<Esc>"]);
        f
    };
    assert_eq!(f.text(), "acd\negh\n", "选中列（b/f）被删，无任何副本");
}

/// 探针 C：`$` 块删除把选中的行清空（vim：删 0..行内容尾，不删行本身）；
/// `.` 重放在空行上安全（选区覆盖同两行，无内容可删，不 panic）。
#[test]
fn probe_dot_after_dollar_block_delete() {
    let mut f = Fixture::at("aaa bbb\nccc ddd\neee fff\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "d"]);
    assert_eq!(f.text(), "\n\neee fff\n");
    f.feed(["."]);
    assert_eq!(f.text(), "\n\neee fff\n", "`.` 重放到空行上是安全无操作");
}

/// 探针 D：`~` 在空行上无操作、不 panic。
#[test]
fn probe_tilde_on_empty_line_noop() {
    let f = edit("\nfoo\n", 0, 0, &["~"]);
    assert_eq!(f.text(), "\nfoo\n");
}

/// 探针 E：insert 模式 `<C-r>` 粘贴 linewise 寄存器。
/// 寄存器文本（"one\n"）落在行尾追加点：行成为 "twoone"，寄存器自带的
/// 换行拆出新行，缓冲原有的行终止符再留一个空行——vim 同款。
#[test]
fn probe_insert_ctrl_r_linewise_register() {
    let mut f = Fixture::at("one\ntwo\n", 0, 0);
    f.feed(["y", "y"]); // "0 = "one\n"
    f.feed(["j", "$"]);
    f.feed(["a"]); // 行尾追加
    f.feed(["<C-r>", "0"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one\ntwoone\n\n");
}

/// 探针 F：`:0d` 删除第一行（vim 语义：地址 0 与第 1 行等价作用首行）。
#[test]
fn probe_ex_zero_delete() {
    let f = {
        let mut f = Fixture::at("a\nb\nc\n", 0, 0);
        f.feed([":", "0", "d", "<CR>"]);
        f
    };
    assert_eq!(f.text(), "b\nc\n");
}

/// 探针 G：`-0` 上 `<C-a>`：结果非负时去掉负号（vim 9.1 语义）。
#[test]
fn probe_increment_negative_zero_drops_sign() {
    let f = edit("-0\n", 0, 1, &["<C-a>"]);
    assert_eq!(f.text(), "1\n");
}

/// 探针 H：`qA` 追加录制后 `@@` 重放整个宏；内容含两次会话的步骤。
#[test]
fn probe_qa_append_then_atat() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["q", "a", "x", "q"]); // a = [x]
    f.feed(["j"]);
    f.feed(["q", "A", "x", "q"]); // a = [x, x]
    assert_eq!(f.vim.macro_len('a'), 2);
    f.feed(["@@", "@@"]); // 连放两次（每次 2 步）
    assert_eq!(f.text(), "b\nd\n", "@@ 重放宏 a：两次 x");
}

/// 探针 I：`g`/`gu` 后按 Esc——安静取消，不响铃。
#[test]
fn probe_pending_prefix_escape_is_quiet() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["g", "<Esc>"]);
    f.feed(["g", "u", "<Esc>"]);
    f.feed(["3", "<Esc>"]);
    assert_eq!(f.host.bells, 0, "前缀取消应安静，got {} 次响铃", f.host.bells);
    assert_eq!(f.text(), "abc\n");
    // 取消后计数不得残留到下一条命令。
    f.feed(["x"]);
    assert_eq!(f.text(), "bc\n", "Esc 取消后 x 只删 1 个字符");
}

/// 探针 J：空行上 `v$` + `d`——空行的可视选区覆盖其换行符，d 删掉空行
/// 本身（vim 的 `vd` 删空行语义），不 panic。
#[test]
fn probe_visual_dollar_on_empty_line() {
    let f = edit("a\n\nb\n", 1, 0, &["v", "$", "d"]);
    assert_eq!(f.text(), "a\nb\n", "空行选区覆盖换行符，d 删除该空行");
}

/// 探针 K：空缓冲上 `:%s/./x/`——E486，不 panic。
#[test]
fn probe_substitute_on_empty_buffer() {
    let mut f = Fixture::new("");
    f.feed([":", "%", "s", "/", ".", "/", "x", "<CR>"]);
    assert_eq!(f.text(), "");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E486")),
        "无匹配应报 E486，got {:?}",
        f.host.statuses
    );
}

/// 探针 L：`ciw` 于单字符缓冲。
#[test]
fn probe_ciw_single_char_buffer() {
    let mut f = Fixture::at("x", 0, 0);
    f.feed(["c", "i", "w"]);
    f.type_text("y");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "y");
}

/// 探针 M：`.` 重放带显式寄存器前缀的删除（`"add` → `.` 仍进 "a）。
#[test]
fn probe_dot_replays_named_register_delete() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["\"", "a", "d", "d"]);
    f.feed(["."]);
    assert_eq!(f.text(), "cc\n", "`.` 重复 \"ad 删除两行");
    // 寄存器 a 装着第二次删除的内容。
    f.feed(["G"]);
    f.feed(["\"", "a", "P"]);
    assert_eq!(f.text(), "bb\ncc\n", "\"aP 粘贴 `.` 删掉的行");
}
