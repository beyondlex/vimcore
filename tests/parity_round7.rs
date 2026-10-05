//! 第七轮检视回归测试。
//!
//! 流程沿用前几轮：读码列可疑点 → vim 9.1 探针实证（`-es` 脚本）→
//! 修复 → 这里入册。每条测试标注实证方式。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::mode::VisualKind;

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
    let sel = f.vim.visual_selection().expect("gv 应恢复选区");
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
    let sel = f.vim.visual_selection().expect("gv 应恢复块选区");
    assert_eq!(sel.2, VisualKind::Block);
    assert_eq!(sel.0, 0, "块起点 = 原块首（L0 行首）");
    assert_eq!(sel.1, 6, "块终点 = 原块末（L2 行首，编辑前偏移）");
}

// ---- 3. hlsearch=false 时取消提示符不得泄漏高亮 -----------------------------

/// hlsearch=false + incsearch=true：`/` 预览高亮在 Esc 取消后必须清空。
/// 旧行为把 `last_matches`（上一个模式的匹配）原样发布，留下永久高亮。
#[test]
fn cancel_cmdline_respects_hlsearch_off() {
    let mut f = Fixture::new("foo\nbar\nfoo\n");
    f.vim.options_mut().hlsearch = false;
    f.feed(["/", "f", "o", "o", "\n"]);
    assert!(
        f.host.highlights.is_empty(),
        "hlsearch=false 搜索后不应有高亮"
    );
    f.feed(["/", "b"]); // incsearch 预览
    f.feed(["escape"]);
    assert!(
        f.host.highlights.is_empty(),
        "取消提示符后不得恢复 last_matches 高亮"
    );
}

// ---- 4. Esc 优先级 ---------------------------------------------------------

/// `g<Esc>` 静默取消（旧实现走 trie-miss 重试路径误响铃）。
#[test]
fn escape_cancels_pending_state_quietly() {
    let mut f = Fixture::new("abc\n");
    f.feed(["g"]);
    f.feed(["escape"]);
    assert_eq!(f.host.bells, 0, "g<Esc> 应静默取消");
}

/// `3"<Esc>` 后 count 必须一起取消，dd 只删一行。
#[test]
fn escape_at_register_prefix_cancels_count() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
    f.feed(["3", "\""]);
    f.feed(["escape"]);
    f.feed(["d", "d"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "l2\nl3\nl4\n",
        "Esc 必须取消 `3\"` 的整个 pending（含 count），dd 只删一行"
    );
}

// ---- 5. :sort u 单行不建幻影 undo 组 ---------------------------------------

/// `:sort u` 在单行范围上无操作（单行没有可去重的连续重复），
/// 不得为无变化的 replace 宣告 undo 组。旧行为每次空耗一个 `u`。
#[test]
fn sort_u_single_line_creates_no_undo_group() {
    let mut f = Fixture::new("hello\n");
    f.feed([":", "s", "o", "r", "t", " ", "u", "\n"]);
    assert_eq!(f.host.group_count, 0, "单行 :sort u 无变化，不得建 undo 组");
    f.feed(["x"]);
    assert_eq!(f.host.group_count, 1, "只有 x 建组");
}

// ---- 6. visual : 执行后 '< '> 保留执行前范围 --------------------------------

/// visual 选区打开 `:` 执行 :s 后，`'<`/`'>` 必须保留提示符打开时的
/// 选区（vim：保留执行的 range）。旧行为按「锚点..执行后光标」重写：
/// :s 把光标放到行首非空白，选区塌缩成 1 字符，gv 二次语义被破坏。
#[test]
fn visual_colon_keeps_prompt_time_range_in_marks() {
    let mut f = Fixture::new("hello world\nsecond\n");
    f.feed(["v", "i", "w"]); // 选 hello（anchor 0，cursor 4）
    f.feed([":", "s", "/", "h", "e", "l", "/", "H", "E", "L", "/", "\n"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "HELlo world\nsecond\n");
    // :s 把光标留在行首（col 0）——若按执行后光标重写，'> 会塌缩到 1
    let lt = f.vim.marks.resolve('<', &f.buf).unwrap();
    let gt = f.vim.marks.resolve('>', &f.buf).unwrap();
    assert_eq!(lt, 0, "'< = 提示符时选区起点");
    assert_eq!(gt, 5, "'> = 提示符时选区末字符之后，不随命令后的光标塌缩");
    // gv 复选原选区（0..5，cursor 停在末字符 o=4）
    f.feed(["g", "v"]);
    assert_eq!(
        f.vim.visual_selection().map(|(a, c, _)| (a, c)),
        Some((0, 4)),
        "gv 应恢复 hello 的选区"
    );
}

// ---- 7. :d 的寄存器参数 ----------------------------------------------------

/// `:1,2d a` 把删除的行存进 `"a`（vim 9.1：`:d [x]`，旧实现忽略该参数）。
#[test]
fn ex_delete_stores_into_named_register() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed([":", "1", ",", "2", "d", " ", "a", "\n"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "three\n");
    let reg = f.vim.registers.get('a').expect("寄存器 a 应有内容");
    assert_eq!(reg.text, "one\ntwo\n");
    assert!(matches!(
        reg.kind,
        vimcore::registers::RegisterKind::Linewise
    ));
}

/// `:2d 3`（count 从范围末行起算）仍按第六轮语义删 2-4 行，且不带
/// 寄存器参数时走既有删除寄存器路径。
#[test]
fn ex_delete_count_semantics_unchanged() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\nfive\n");
    f.feed([":", "2", "d", " ", "3", "\n"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "one\nfive\n",
        ":2d 3 从第 2 行起删 3 行"
    );
    // 被删的多行文本落在 "1（编号环）
    let reg = f.vim.registers.get('1').expect("多行删除应进 \"1");
    assert_eq!(reg.text, "two\nthree\nfour\n");
}

// ---- 8. 对抗性 fuzz 抓取的修复 ---------------------------------------------

/// `db` 在行首：b 跨到上一行行尾，exclusive + 列 1 + 起点在首个非空白
/// 之前 → linewise 删除**上一行**（vim 9.1 探针：['abc','def'] → ['def']）。
/// 旧实现 `target_line - 1` 在向上跨行时 usize 下溢 panic。
#[test]
fn db_at_line_start_deletes_previous_line_linewise() {
    let mut f = Fixture::new("abc\ndef\n");
    f.vim.cursor.offset = f.buf.line_start(1);
    f.feed(["d", "b"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "def\n",
        "db 在行首 = linewise 删上一行（vim 探针）"
    );
}

/// `gn` 选中的匹配以多字节字符结尾时，光标必须落在末字符**起点**，
/// 不能用 `end - 1` 字节算术落进字符中间（fuzz：匹配 `中` 后宿主
/// `offset_to_line` 直接 panic）。
#[test]
fn gn_cursor_lands_on_multibyte_match_start() {
    let mut f = Fixture::new("foo 中 bar\n");
    f.feed(["/", "中", "\n"]); // 搜索 中
    f.feed(["g", "g"]); // 回到开头（pattern 仍在）
    f.feed(["g", "n"]); // 选中匹配
    let co = f.vim.cursor_offset();
    let text = f.buf.slice(0..f.buf.len());
    assert!(
        text.is_char_boundary(co),
        "gn 后光标 {co} 必须在字符边界（text {text:?}）"
    );
    let sel = f.vim.visual_selection().expect("gn 应进入 visual");
    assert_eq!(sel.0, 4, "选区起点 = 中 的起点");
}

// ---- harness 辅助 -----------------------------------------------------------
// insert 打字走 common::Fixture::type_text（record_typed_text +
// insert_text_at_cursor，与集成层 place_text 同款）。
