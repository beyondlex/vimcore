//! 第三十四轮探针：BUG_AUDIT3 挂账清结的回归（2026-10-08）。
//!
//! 覆盖三组修复：
//! * 未闭合搜索地址（`:/pat`/`:?pat`）——整个行尾都是模式（挂账 5；
//!   oracle：`:?,3d` → `E486: Pattern not found: ,3d`，命令不执行；
//!   命中则落匹配行首非空白；空模式复用上次搜索，无历史报 E35）。
//! * `:reg` 的 vim 版式（挂账 6）：`  l  ""   second line^J`——两空格 +
//!   类型符 + 两空格 + 引号名 + 三空格 + 内容（linewise 尾显示 ^J）。
//! * `:undo` 到底：纯响铃、无报文（9.1 oracle v:errmsg 为空，销账）。

mod common;

use common::Fixture;

fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

#[test]
fn open_search_address_consumes_the_rest_of_the_line() {
    // `:? ,3d`：`,3d` 整体成为模式，搜索失败 E486、什么都不执行
    let mut f = Fixture::at("aaa\nbbb\nccc\n", 0, 0);
    ex(&mut f, "?,3d");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E486: Pattern not found: ,3d"),
        "vim 9.1: 未闭合 ? 的行尾全是模式"
    );
    assert_eq!(f.text(), "aaa\nbbb\nccc\n", "命令从未解析，缓冲不动");
    assert_eq!(f.vim.cursor_offset(), 0, "原地不动");

    // `:/ba` 命中：落匹配行的首个非空白；后续 `n` 沿用该模式
    let mut f = Fixture::at("aaa\nbar fizz\nccc\n", 0, 0);
    ex(&mut f, "/ba");
    assert_eq!(f.vim.cursor_offset(), 4, "落 bar 行行首（offset 4）");
    assert!(f.host.statuses.is_empty(), "命中无报文");
    f.feed(["n"]);
    assert_eq!(
        f.host.statuses.iter().any(|s| s.contains("E486")),
        false,
        "模式已激活，n 不应报 E35/E486 类错误（无更多匹配时回绕或响铃）"
    );

    // 空模式：无历史 → E35
    let mut f = Fixture::at("aaa\nbbb\n", 0, 0);
    ex(&mut f, "/");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E35: No previous regular expression"),
        "vim 9.1: :// 无历史报 E35"
    );

    // 空模式：有历史 → 复用；前向无更多匹配行时回绕到首个匹配行
    let mut f = Fixture::at("foo\nbar foo\n", 0, 0);
    ex(&mut f, "/bar");
    assert_eq!(f.vim.cursor_offset(), 4, "/bar 落 bar 行");
    ex(&mut f, "/");
    assert_eq!(
        f.vim.cursor_offset(),
        4,
        ":// 复用模式 bar；前向无更多匹配行 → 回绕到首个匹配行（同一行）"
    );
}

#[test]
fn undo_at_oldest_change_is_silent() {
    // 9.1 oracle：`:undo` 在最老变更处 v:errmsg 为空——纯响铃即正确
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["x"]);
    ex(&mut f, "undo");
    assert_eq!(f.text(), "abc\n");
    ex(&mut f, "undo");
    let noisy = f
        .host
        .statuses
        .iter()
        .any(|s| s.contains("E") && !s.contains("E492"));
    assert!(!noisy, "vim 9.1: :undo 到底无报文（{:?}）", f.host.statuses);
}

#[test]
fn reg_listing_uses_vim_layout() {
    let mut f = Fixture::at("hello world\nsecond line\n", 0, 0);
    f.feed(["y", "y", "j", "\"", "a", "d", "d"]);
    ex(&mut f, "reg");
    // `  l  ""   second line^J`（两空格 + 类型 + 两空格 + 引号名 + 三空格）
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("  l  \"\"   second line^J")),
        "vim 9.1: :reg 行版式（无名寄存器、linewise ^J 尾），实际 {:?}",
        f.host.statuses
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("  l  \"a   second line")),
        "命名寄存器 a 同版式"
    );
}

#[test]
fn redo_unsets_marks_undo_restores_again() {
    // 9.1 oracle：`ma dd u` 恢复 mark（行末字符），`<C-r>` 把它**取消**
    // （getpos("'a") = 0,0），再 `u` 又恢复。旧行为 redo 后 mark 滞留
    // 在恢复位（列漂移形状）。
    let mut f = Fixture::at("  foo\nbar\nbaz\n", 0, 2);
    f.feed(["m", "a", "d", "d", "u"]);
    assert_eq!(f.text(), "  foo\nbar\nbaz\n");
    // u 后光标在 undo 提示位（行首）；mark 本身恢复到行末字符
    assert_eq!(f.vim.marks.resolve('a', &f.buf), Some(4), "E4：mark 恢复");
    f.feed(["<C-r>"]); // C-r
    assert_eq!(f.text(), "bar\nbaz\n", "redo 重新删除");
    f.feed(["`", "a"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "redo 后 mark 'a 未设置（vim 9.1 oracle getpos = 0,0）"
    );
    f.feed(["u"]);
    f.feed(["`", "a"]);
    assert_eq!(f.cursor(), 4, "再 undo 又恢复（oracle 1,3 = 行末字符）");
}
