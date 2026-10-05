//! 第二十九轮探针（四）：宏/重放 × 映射的展开键边界。

mod common;

use common::Fixture;
use vimcore::keymap::ModeClass;

fn f_with_nmap(initial: &str, lhs: &str, rhs: &str) -> Fixture {
    let mut f = Fixture::at(initial, 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Normal, lhs, rhs, true);
    f
}

/// 探针 BA：映射删除后 `.` 重放原始键（经映射再展开）。
#[test]
fn probe_dot_after_mapped_delete() {
    let mut f = f_with_nmap("a1\na2\na3\n", "x", "dd");
    f.feed(["x"]);
    assert_eq!(f.text(), "a2\na3\n");
    f.feed(["."]);
    assert_eq!(f.text(), "a3\n", "`.` 重放 x，经映射再删一行");
}

/// 探针 BB：计数重复 `2.` 对映射键逐次展开。
#[test]
fn probe_count_dot_with_mapped_key() {
    let mut f = f_with_nmap("a1\na2\na3\na4\n", "x", "dd");
    f.feed(["x"]);
    f.feed(["2", "."]);
    assert_eq!(f.text(), "a4\n", "2. 展开两次 dd");
}

/// 探针 BC：含映射键的宏用计数重放 `2@a`。
#[test]
fn probe_count_at_replay_with_mapped_key() {
    let mut f = f_with_nmap("a1\na2\na3\na4\na5\n", "x", "dd");
    f.feed(["q", "a", "x", "q"]); // 录制执行一次 dd
    assert_eq!(f.text(), "a2\na3\na4\na5\n");
    f.feed(["2", "@", "a"]);
    assert_eq!(f.text(), "a4\na5\n", "2@a 展开两次 dd");
}

/// 探针 BD：映射键触发的改动是正常 undo 组。
#[test]
fn probe_undo_after_mapped_delete() {
    let mut f = f_with_nmap("a1\na2\n", "x", "dd");
    f.feed(["x"]);
    f.feed(["u"]);
    assert_eq!(f.text(), "a1\na2\n", "u 撤销映射展开的 dd");
}

/// 探针 BE：insert 映射 `z` → `<Esc>`：退出插入，`.` 边界干净。
#[test]
fn probe_insert_mapping_escape_flow() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Insert, "z", "<Esc>", true);
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["z"]); // 展开为 Esc
    assert_eq!(f.vim.mode_indicator(), "");
    assert_eq!(f.text(), "Xab\ncd\n");
    // last_change 只含本次插入，不因映射展开错乱。
    f.feed(["G"]);
    f.feed(["."]);
    // G 落末行行首非空白，`.` 在那里重放插入。
    assert_eq!(f.text(), "Xab\nXcd\n", "`.` 重放插入 X");
}

/// 探针 BF：映射展开期间的 bell 不误报——`nnoremap <F2> dd` 用 Named 键。
#[test]
fn probe_named_key_mapping_in_macro() {
    let mut f = Fixture::at("r1\nr2\nr3\n", 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Normal, "<F2>", "dd", true);
    f.feed(["q", "a", "<F2>", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "Named 键同样存原始键");
    assert_eq!(f.text(), "r2\nr3\n");
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "r3\n");
}

/// 探针 BG：redo-register `"1P .` 与映射无冲突（展开不进 redo 路径）。
#[test]
fn probe_redo_register_unaffected_by_mappings() {
    let mut f = f_with_nmap("one\ntwo\nthree\n", "y", "yy"); // y 不再是 yank 前缀!
    f.feed(["d", "d"]);
    f.feed(["d", "d"]);
    f.feed(["\"", "1", "P"]);
    f.feed(["."]);
    assert_eq!(f.text(), "one\ntwo\nthree\n", "redo-register 连续恢复两行");
    let _ = &mut f;
}

/// 探针 BH：嵌套展开（noremap 指向 remap）录制深度有界且记录最外层。
#[test]
fn probe_nested_expansion_bounded() {
    let mut f = Fixture::at("xy\n", 0, 0);
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "a", "b");
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Normal, "b", "x", true);
    f.feed(["a"]); // a → b（remap）→ x（noremap，字面删除）
    assert_eq!(f.text(), "y\n", "嵌套展开到 noremap 层停止");
}

/// 探针 BI：普通（非映射）路径的录制在修复后行为不变。
#[test]
fn probe_plain_typing_recording_unchanged() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["d", "d"]);
    f.feed(["j"]);
    f.feed(["."]);
    assert_eq!(f.text(), "", "普通 dd 的 . 重放不受影响");
}
