//! 第十四轮回归（一）：motion 重复、operator 变更记录、quote 转义、段落对象、
//! 寄存器前缀。所有期望行为均经 vim 9.1 探针实证（探针脚本见 NOTES 第十四轮）。

mod common;

use common::{edit, Fixture};

/// PROBE: vim 9.1 `a1b2c3d2e` 上 `gg0t2;` → col 7（第二个 2 之前）。
/// 旧引擎 `;` 从光标重扫，立即重新命中同一个 2，原地假成功。
#[test]
fn till_repeat_semicolon_advances_to_next_target() {
    let f = edit("a1b2c3d2e\n", 0, 0, &["t", "2", ";"]);
    assert_eq!(f.vim.cursor_offset(), 6, "t2; must park before the SECOND 2");
}

/// PROBE: vim 9.1 `t2;;` — 第二个 `;` 无目标可用，光标原地不动。
#[test]
fn till_repeat_without_target_stays() {
    let f = edit("a1b2c3d2e\n", 0, 0, &["t", "2", ";", ";"]);
    assert_eq!(f.vim.cursor_offset(), 6, "exhausted repeat must not move");
}

/// PROBE: vim 9.1 `a1b2c3d3e` 上 `gg0d2t3` → `3e`（删到第二个 3 之前）。
#[test]
fn counted_till_repeat_deletes_through() {
    let f = edit("a1b2c3d3e\n", 0, 0, &["d", "2", "t", "3"]);
    assert_eq!(f.text(), "3e\n", "d2t3 stops before the second 3");
}

/// PROBE: vim 9.1 `a1b2c3d3e` 上 `gg02t3` → col 7：全新（非重复）的
/// count till 不跳过紧邻目标，count 就是查找次数。
#[test]
fn fresh_counted_till_counts_finds() {
    let f = edit("a1b2c3d3e\n", 0, 0, &["2", "t", "3"]);
    assert_eq!(f.vim.cursor_offset(), 6, "2t3 = second find, no skip");
}

/// PROBE: 紧邻目标上的全新 `t` 必须保持「成功但不动」语义（`dx` 删掉目标）。
#[test]
fn fresh_till_adjacent_target_stays_put_without_bell() {
    let mut f = Fixture::new("a2x\n");
    f.feed(["t", "2"]);
    assert_eq!(f.vim.cursor_offset(), 0);
    assert_eq!(f.host.bells, 0, "adjacent target is a SUCCESS, not a miss");
}

/// PROBE: vim 9.1 `a22` 上 `gg0t2;` → col 2（跳过一个紧邻目标后再找）。
#[test]
fn till_repeat_skips_adjacent_target_pair() {
    let f = edit("a22\n", 0, 0, &["t", "2", ";"]);
    assert_eq!(f.vim.cursor_offset(), 1);
}

/// PROBE: vim 9.1 `:changes` 在 yiw 前后条目数不变（yank 不是 change）。
/// 引擎可观测面：yank 之后 `g;` 落在真正的编辑处，而不是 yank 处。
#[test]
fn yank_does_not_enter_changelist() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed(["x"]); // the one real change, at offset 0
    f.feed(["G", "y", "i", "w"]); // yank on line 2
    f.feed(["g", "g", "g", ";"]);
    assert_eq!(
        f.vim.cursor_offset(),
        0,
        "g; must land on the x change, not the yank position"
    );
}

/// 空行上的 `d$` 是 no-op（vim 9.1），不得污染 changelist：g; 跳过它。
#[test]
fn noop_operator_does_not_enter_changelist() {
    let mut f = Fixture::new("abc\n\n");
    f.feed(["x"]); // real change at 0
    f.feed(["j", "d", "$"]); // no-op on the empty line
    f.feed(["g", "g", "g", ";"]);
    assert_eq!(f.vim.cursor_offset(), 0, "empty-span d$ must not record a change");
}

/// PROBE: vim 9.1 `"a\\"`（行内容 `"a\"`，收尾引号前是双反斜杠 = 字面
/// 反斜杠 + 真引号）上 `gg0ci"X<Esc>` → `"X"`。单反斜杠判定把收尾引号
/// 当转义引号，整行失去配对。
#[test]
fn quote_object_counts_backslash_run_parity() {
    let mut f = Fixture::new("\"a\\\\\"\n");
    f.feed(["c", "i", "\""]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "\"X\"\n", "double backslash = literal, quote is real");
}

/// 单反斜杠的转义引号仍被跳过（`"a\"b"` 配对为 "a\"b"）。
#[test]
fn quote_object_single_escape_still_skipped() {
    let mut f = Fixture::new("\"a\\\"b\"\n");
    f.feed(["c", "i", "\""]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "\"X\"\n");
}

/// PROBE: vim 9.1 [para1,"",para2] 上 `3Gdap` → ['para1']。末段无尾随
/// 空白时吞前导空行（:h ap）。
#[test]
fn ap_on_last_paragraph_takes_leading_blank() {
    let f = edit("para1\n\npara2\n", 2, 0, &["d", "a", "p"]);
    assert_eq!(f.text(), "para1\n", "separator blank goes with the paragraph");
}

/// PROBE: vim 9.1 `abc` 上 `gg0"ax` → @a='a' 且 unnamed 也是 'a'。
#[test]
fn register_prefix_routes_x_into_named_register() {
    let f = edit("abc\n", 0, 0, &["\"", "a", "x"]);
    let reg = f.vim.registers.get('a').expect("register a populated");
    assert_eq!(reg.text, "a", "\"ax must delete into register a");
}

/// PROBE: vim 9.1 `xabc` 上 `$h"aX` → @a='a'。
#[test]
fn register_prefix_routes_x_backwards_into_named_register() {
    let f = edit("xabc\n", 0, 2, &["\"", "a", "X"]);
    let reg = f.vim.registers.get('a').expect("register a populated");
    assert_eq!(reg.text, "a", "\"X must delete into register a");
}

/// PROBE: vim 9.1 `abc` 上 `gg0"asZ<Esc>` → @a='a'、行 = 'Zbc'。
#[test]
fn register_prefix_routes_substitute_into_named_register() {
    let mut f = Fixture::new("abc\n");
    f.feed(["\"", "a", "s"]);
    f.type_text("Z");
    f.feed(["escape"]);
    let reg = f.vim.registers.get('a').expect("register a populated");
    assert_eq!(reg.text, "a", "\"as must delete into register a");
    assert_eq!(f.text(), "Zbc\n");
}

/// PROBE: vim 9.1 ['a','b'] 上 `dd`、`""dd`、`"1p` → ['','b']：`""` 前缀
/// 完全等价于无前缀，编号环照常轮转。
#[test]
fn double_quote_register_prefix_behaves_like_no_prefix() {
    let f = edit("a\nb\n", 0, 0, &["d", "d", "\"\"", "d", "d", "g", "g", "\"", "1", "p"]);
    assert_eq!(f.text(), "\nb", "\"\"dd must rotate the numbered ring");
}

/// PROBE: vim 9.1 `:nnoremap j G` 后输入 `gj` → 光标落第 2 行（builtin gj
/// 直接触发，续键不查映射表）。引擎原先让映射劫持续键，G 跳到了末行。
#[test]
fn mapping_does_not_hijack_partial_builtin_continuation() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "G", true);
    f.feed(["g", "g"]);
    f.feed(["g", "j"]);
    assert_eq!(
        f.vim.cursor_offset(),
        4,
        "gj 的续键 j 必须完成 builtin，映射的 G 不得触发"
    );
}

/// `:nnoremap j gj` 后输入 `gj` 不熔断（旧引擎在展开重放里再次命中映射，
/// map_depth 熔断、响铃、gj 完全失效）。
#[test]
fn noremap_self_referential_builtin_no_runaway() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "gj", true);
    f.feed(["g", "j"]);
    assert_eq!(f.host.bells, 0, "无 runaway 熔断");
    assert_eq!(f.vim.cursor_offset(), 4, "builtin gj 触发一次");
}

/// 命令之间映射照常工作：`j→G` 映射下单独输入 `j` 仍然展开为 G。
#[test]
fn mapping_still_applies_between_commands() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "G", true);
    f.feed(["g", "g"]);
    f.feed(["j"]);
    assert_eq!(
        f.vim.cursor_offset(),
        8,
        "命令边界的 j 按映射展开为 G（跳到末行行首）"
    );
}

/// 引擎不变量：互映射自引用（`:map x y` + `:map y x`）在 maxmapdepth
/// （现与 vim 默认一致 = 1000）处熔断：响铃、清队列、不挂死。
#[test]
fn recursive_mapping_depth_guard_stops() {
    let mut f = Fixture::new("abc\n");
    f.vim.keymaps.map_str(vimcore::keymap::ModeClass::Normal, "x", "y");
    f.vim.keymaps.map_str(vimcore::keymap::ModeClass::Normal, "y", "x");
    f.feed(["x"]);
    assert!(f.host.bells > 0, "互映射必须熔断响铃");
    assert_eq!(f.text(), "abc\n", "队列被清空，缓冲不动");
}

/// 引擎不变量：流水线守卫清空队列后，剩余 no_remap 预算归零——后续
/// 按键照常走映射解析（预算泄漏曾让之后所有按键绕过映射）。
#[test]
fn guard_abort_resets_noremap_budget() {
    // 大 RHS 触发 MAX_PIPELINE_STEPS 守卫（预算 100k 键）
    let big_rhs = "x".repeat(120_000);
    let mut f = Fixture::new("abc\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "q", &big_rhs, true);
    f.feed(["q"]); // 不挂死即通过（守卫在 100k 键处清队列）

    // 守卫中止是每会话的：后续会话的映射照常解析（w→dd 删行）
    let mut f2 = Fixture::new("abc\n");
    f2.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "w", "dd", true);
    f2.feed(["w"]);
    assert_eq!(f2.text(), "", "映射 w→dd 生效（若预算泄漏则 w 走词移动、缓冲不变）");
}
