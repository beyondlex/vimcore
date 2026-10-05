//! 第二十九轮探针（三）：映射×宏×重放组合、Ex 边界、宿主 API 误用。

mod common;

use common::{edit, Fixture};
use vimcore::key::{parse_key_sequence, Key};
use vimcore::keymap::ModeClass;

fn f_with_map(initial: &str, lhs: &str, rhs: &str) -> Fixture {
    let mut f = Fixture::at(initial, 0, 0);
    f.vim
        .keymaps_mut()
        .map_str(ModeClass::Normal, lhs, rhs);
    f
}

/// 探针 N：映射被录进宏时记原始键，重放经管线再展开（vim 9.1 typeahead
/// 实证：`:nmap x dd` + `qaxjyyq` 的宏寄存器是 "xjyy"，不含展开键）。
#[test]
fn probe_macro_records_raw_keys_mapping_replays() {
    // :nmap x dd 后录制 qaxq：宏 = [x]；录制期间 x 已执行一次 dd。
    let mut f = f_with_map("row1\nrow2\nrow3\n", "x", "dd");
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "宏记录原始键 x");
    assert_eq!(f.text(), "row2\nrow3\n", "录制期间映射已执行");
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "row3\n", "重放时 x 经映射展开为 dd，再删一行");
}

/// 探针 O：noremap 展开不再递归映射。
#[test]
fn probe_noremap_no_recursion() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Normal, "a", "x", true);
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "b", "a");
    f.feed(["b"]); // b → a（noremap）→ x，不再展开
    assert_eq!(f.text(), "bc\n", "noremap 的展开以字面 x 执行");
}

/// 探针 P：映射 LHS 与内建前缀冲突时，内建组合仍优先（gg 不被 gX 映射卡住）。
#[test]
fn probe_builtin_beats_mapping_prefix() {
    let mut f = f_with_map("a\nb\nc\n", "gx", "dd");
    f.feed(["g", "g"]); // gg 是内建：立即跳行首，不等 gx
    assert_eq!(f.line(), 0);
    assert_eq!(f.host.bells, 0);
}

/// 探针 Q：insert 模式映射 `jk` → Esc。
#[test]
fn probe_insert_mapping_jk_escape() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Insert, "jk", "<Esc>", true);
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["j"]);
    assert_eq!(f.vim.mode_indicator(), "INSERT", "j 是映射前缀，等待中");
    f.feed(["k"]);
    assert_eq!(f.vim.mode_indicator(), "", "jk 触发 Esc 退出插入");
    assert_eq!(f.text(), "Xabc\n");
}

/// 探针 R：块插入的多行复制是一个 undo 组（一次 u 全撤）。
#[test]
fn probe_block_insert_single_undo_group() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    let groups_before = f.host.group_count;
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Xab\nXcd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ab\ncd\n", "一次 u 撤销全部行的复制");
    assert_eq!(f.host.group_count, groups_before + 1, "恰好一个 undo 组");
}

/// 探针 S：`"ap` 后 `.` 重放保留寄存器前缀。
#[test]
fn probe_dot_replays_register_put() {
    let mut f = Fixture::at("x\naa\nbb\n", 0, 0);
    f.feed(["\"", "a", "y", "y"]); // "a = "x\n"
    f.feed(["j"]);
    f.feed(["\"", "a", "p"]); // 行 2（aa）下方插入 x 行
    f.feed(["j"]);
    f.feed(["."]); // 光标行（bb）下方再插
    assert_eq!(f.text(), "x\naa\nx\nbb\nx\n");
}

/// 探针 T：`:s/a//`（空替换）删除匹配。
#[test]
fn probe_substitute_empty_replacement_deletes() {
    let f = {
        let mut f = Fixture::at("bar foo bar\n", 0, 0);
        f.feed([":", "s", "/", "b", "a", "r", "/", "/", "<CR>"]);
        f
    };
    assert_eq!(f.text(), " foo bar\n", "非 g 标志只替换每行第一个匹配");
}

/// 探针 U：`:set foo ts=8`——未知项 E518 即停，后面的 ts=8 不生效。
#[test]
fn probe_set_unknown_stops_before_later_items() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed([":", "s", "e", "t", " ", "f", "o", "o", " ", "t", "s", "=", "8", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E518")),
        "应报 E518"
    );
    f.feed([":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=4")),
        "ts=8 不得生效，got {:?}",
        f.host.statuses
    );
}

/// 探针 V：`@:` 重放上一条 Ex 命令。
#[test]
fn probe_at_colon_replays_ex() {
    let mut f = Fixture::at("zaa\nzb\nzc\n", 0, 0);
    f.feed([":", "s", "/", "z", "/", "w", "/", "<CR>"]);
    assert_eq!(f.text(), "waa\nzb\nzc\n");
    f.feed(["j"]);
    f.feed(["@", ":"]);
    assert_eq!(f.text(), "waa\nwb\nzc\n");
}

/// 探针 W：visual 中执行 `:noh`——高亮清空且退回 normal（选区关闭）。
#[test]
fn probe_visual_noh_closes_selection() {
    let mut f = Fixture::at("foo bar\n", 0, 0);
    f.feed(["/"]); 
    f.feed(["b", "a", "r", "<CR>"]);
    f.feed(["v"]); // 进入可视
    f.feed([":"]);
    for c in "noh".chars() {
        f.feed(&[c.to_string()]);
    }
    f.feed(["<CR>"]);
    assert_eq!(f.vim.mode_indicator(), "", "执行后退出可视");
    assert!(f.host.highlights.is_empty(), "noh 清空高亮");
}

/// 探针 X：宿主 API 误用——可视拖拽 anchor > cursor（向上拖）。
#[test]
fn probe_set_visual_range_drag_up() {
    let mut f = Fixture::at("abcd\nefgh\nij\n", 2, 1);
    f.vim.set_visual_range(&f.buf, 7, 0); // anchor 在后、光标在前
    let (a, c, _) = f.vim.visual_selection().unwrap();
    assert_eq!((a, c), (7, 0), "visual_selection 返回原始 anchor/cursor");
    f.feed(["d"]);
    // 选区是 (0,0)..(1,7)（含 h）：删 "abcd\nefg"，留下 "h\nij\n"。
    assert_eq!(f.text(), "h\nij\n", "向上拖出的选区整体删除（不含 anchor 行剩余）");
}

/// 探针 Y：宿主 API 误用——insert 会话中拖拽被忽略。
#[test]
fn probe_set_visual_range_ignored_in_insert() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["i"]);
    f.vim.set_visual_range(&f.buf, 0, 4);
    assert_eq!(f.vim.mode_indicator(), "INSERT", "insert 中拖拽不得改模式");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
}

/// 探针 Z：宿主 API——insert 会话外的 ime replace_range 单独成 undo 组。
#[test]
fn probe_replace_range_outside_session_closes_group() {
    let mut f = Fixture::at("hello world\n", 0, 0);
    let before = f.host.group_count;
    f.ime_replace(0..5, "goodbye");
    assert_eq!(f.text(), "goodbye world\n");
    f.feed(["x"]);
    f.feed(["u"]);
    assert_eq!(f.text(), "goodbye world\n", "u 撤销的是 x，不是 ime 替换");
    f.feed(["u"]);
    assert_eq!(f.text(), "hello world\n", "第二次 u 撤销 ime 替换");
    assert!(f.host.group_count >= before + 2, "两个独立 undo 组");
}

/// 探针 AA：`2fa` 只有一个 a 时整条失败（无部分移动）。
#[test]
fn probe_count_find_beyond_line_fails_whole() {
    let f = edit("a b a\n", 0, 0, &["2", "f", "b"]);
    assert_eq!(f.host.bells, 1, "目标不足应响铃");
    // 光标不动。
    assert_eq!(f.cursor(), 0);
}

/// 探针 AB：`dit` 取最内层标签。
#[test]
fn probe_dit_innermost_tag() {
    let f = edit("<div><p>x</p></div>", 0, 9, &["d", "i", "t"]);
    assert_eq!(f.text(), "<div><p></p></div>", "dit 只删最内层的内容");
}

/// 探针 AC：`f<CR>` 失败不覆盖 last_find（; 仍找 a）。
#[test]
fn probe_failed_f_cr_keeps_last_find() {
    let f = edit("axa\n", 0, 0, &["f", "a", "f", "<CR>", ";"]);
    // f a 落第 2 个 a(col 2)；f<CR> 失败；; 从行尾再找 a——行内没有更多，
    // 失败响铃（f 不跨行回绕），光标原地。
    assert_eq!(f.cursor(), 2, "; 重复 f a 方向的查找");
    assert_eq!(f.host.bells, 2, "f<CR> 与随后的 ; 各响铃一次");
}

/// 探针 AD：巨数重复 `999999999J` 在行尾及时收手（不挂起）。
#[test]
fn probe_huge_join_count_terminates() {
    let f = edit("a\nb\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "J"]);
    assert_eq!(f.text(), "a b\n", "J 合并到行尾为止并在接缝补空格");
    assert_eq!(f.host.bells, 1, "行数不足响铃一次");
}

/// 探针 AE：纯字符 `Key` 序列的 parse_key_sequence 与按键管线一致。
#[test]
fn probe_parse_key_sequence_angles() {
    let keys = parse_key_sequence("<Esc>ab<C-a>");
    assert_eq!(keys.len(), 4);
    assert_eq!(keys[0], Key::escape());
    assert_eq!(keys[3], Key::ctrl_char('a'));
    // 未闭合的 < 保持字面量。
    let broken = parse_key_sequence("a<b");
    assert_eq!(broken.len(), 3);
    assert_eq!(broken[1], Key::char('<'));
    assert_eq!(broken[2], Key::char('b'));
}

/// 探针 AF：VimState 通过 `Ctx` 借用检查——同一 fixture 内先 `.` 再 `u`。
#[test]
fn probe_dot_then_undo() {
    let mut f = Fixture::at("word\n", 0, 0);
    f.feed(["x"]);
    f.feed(["."]);
    assert_eq!(f.text(), "rd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ord\n", "u 撤销第二次 x");
    f.feed(["u"]);
    assert_eq!(f.text(), "word\n");
}

