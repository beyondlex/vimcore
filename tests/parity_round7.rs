//! 第七轮检视回归测试。
//!
//! 流程沿用前几轮：读码列可疑点 → vim 9.1 探针实证（`-es` 脚本）→
//! 修复 → 这里入册。每条测试标注实证方式。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::mode::VisualKind;
use vimcore::state::VimState;


// ---- 1. active_visual 生命周期 ---------------------------------------------

/// `Vjd` 后 `:'<,'>d` 必须读 `'<`/`'>` mark（最新选区），不能用算子前
/// 的过期 live 偏移。旧行为把整个缓冲删空。
#[test]
fn cmdline_range_uses_marks_not_stale_live_visual() {
    let mut f = Fixture::new("aaa\nbbb\nccc\nddd\n");
    f.feed(["V", "j", "d"]); // 删 L1-L2 → ["ccc", "ddd"]
    assert_eq!(f.buf.slice(0..f.buf.len()), "ccc\nddd\n");
    f.feed([":", "'", "<", ",", "'", ">", "d", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ddd\n",
        ":'<,'>d 应删光标所在行（ccc），不能用过期 live 范围"
    );
}

/// visual 模式文本对象扩展（跨行 `vi(`）必须同步 live 范围，`:` 的
/// `'<,'>` 才覆盖整个对象。旧行为只替换塌缩行。
#[test]
fn object_extension_updates_live_range_for_cmdline() {
    let mut f = Fixture::new("let a = foo(\n    bar,\n    baz,\n);\n");
    // 光标到 L1 的 bar（offset 17），vi( 选 L1-L2，:s 两行都替换
    f.vim.cursor.offset = 17;
    f.feed(["v", "i", "("]);
    // 模式 ba 同时命中 bar/baz：范围若塌缩在 L1 就只换一处
    f.feed([":", "s", "/", "b", "a", "/", "X", "/", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "let a = foo(\n    Xr,\n    Xz,\n);\n",
        "vi( 后 :s 应覆盖对象的全部行（bar 与 baz）"
    );
}

// ---- 2. visual 变更会话后 gv 恢复选区 --------------------------------------

fn visual_selection_of(vim: &VimState) -> Option<(usize, usize, VisualKind)> {
    vim.visual_selection()
}

/// `viwcX<Esc>` 后 `gv` 恢复选区。vim 9.1 探针：`'<`/`'>` 保持编辑前
/// 字节范围（gv 重选 5 字符），而非塌缩/移位后的空区。
#[test]
fn gv_restores_selection_after_visual_change() {
    let mut f = Fixture::new("alpha beta\ngamma\n");
    f.feed(["v", "i", "w"]); // 选 alpha（offset 0..5）
    f.feed(["c"]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "X beta\ngamma\n");
    f.feed(["g", "v"]);
    let sel = visual_selection_of(&f.vim)
        .map(|(a, c, kind)| (a, c, kind))
        .expect("gv 应恢复选区");
    assert_eq!(sel.0, 0, "gv 起点 = 原选区起点");
    assert_eq!(sel.2, VisualKind::Char);
    // vim 探针：'> 保持原偏移（gv 重选原字节范围 0..5）
    assert_eq!(sel.1, 4, "gv 终点 = 原选区末字符（编辑前范围）");
}

/// 块插入 `C-v jj I` 后 `gv` 恢复原块。vim 9.1 探针：`'<`/`'>` 落在
/// 原块列。
#[test]
fn gv_restores_block_after_block_insert() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["<C-v>", "j", "j", "I"]);
    f.type_text("Z");
    f.feed(["escape"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "Zab\nZcd\nZef\n");
    f.feed(["g", "v"]);
    let sel = visual_selection_of(&f.vim).expect("gv 应恢复块选区");
    assert_eq!(sel.2, VisualKind::Block);
    assert_eq!(sel.0, 0, "块起点 = 原块首（L0 行首）");
    assert_eq!(sel.1, 6, "块终点 = 原块末（L2 行首，编辑前偏移）");
}

// ---- harness 辅助 -----------------------------------------------------------
// insert 打字走 common::Fixture::type_text（record_typed_text +
// insert_text_at_cursor，与集成层 place_text 同款）。
