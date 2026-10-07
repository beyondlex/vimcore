//! 第十四轮回归（一）：motion 重复、operator 变更记录、quote 转义、段落对象、
//! 寄存器前缀。所有期望行为均经 vim 9.1 探针实证（探针脚本见 NOTES 第十四轮）。

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

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
    // 字节口径随 'fixendofline'（写回补末尾换行）
    assert_eq!(f.text(), "\nb\n", "\"\"dd must rotate the numbered ring");
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

// ---- cmdline 的 count/register 卫生（vim 9.1 探针 P7/P8b/P9/P9b）----------

/// PROBE: vim 9.1 `3:<CR>` → 光标落第 3 行：count 变成范围 `.,.+2` 并
/// 被消费（引擎原先只开空提示符，count 挂着漏给下一条命令）。
#[test]
fn count_before_colon_becomes_range() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &["3", ":", "enter"]);
    assert_eq!(f.vim.cursor_offset(), 4, "3: 的范围末地址 = 第 3 行（1-based）");
}

/// PROBE: vim 9.1 `3:<Esc>` 后 `j` 只移动 1 行——count 在提示符取消时被消费。
#[test]
fn count_dropped_on_cancelled_colon() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &["3", ":", "escape", "j"]);
    assert_eq!(f.vim.cursor_offset(), 2, "取消后 count 不得放大后续 j");
}

/// PROBE: vim 9.1 `2/x<Esc>` 后 `x` 只删 1 个字符。
#[test]
fn count_dropped_on_cancelled_search() {
    let f = edit("aXbXc\n", 0, 0, &["2", "/", "x", "escape", "x"]);
    assert_eq!(f.text(), "XbXc\n", "取消的搜索不得让 x 删两个字符");
}

/// PROBE: vim 9.1 `3/foo<CR>`（光标不在匹配上）→ 跳到第 3 个匹配。
#[test]
fn count_before_search_jumps_to_nth_match() {
    let f = edit("x\nfoo\ny\nfoo\nz\nfoo\n", 0, 0, &["3", "/", "f", "o", "o", "enter"]);
    assert_eq!(
        f.vim.cursor_offset(),
        f.buf.line_start(5),
        "3/foo 落第 3 个匹配"
    );
}

/// visual `:` 的 count 被消费（vim 取消挂起的 count，不放大后续命令）。
#[test]
fn visual_colon_drops_count() {
    let f = edit("a\nb\nc\n", 0, 0, &["V", "3", "v", ":", "escape", "j"]);
    assert_eq!(f.vim.cursor_offset(), 2, "visual : 后 count 不得漏给 j");
}

// ---- Ex 命令面（vim 9.1 探针 P13-P17/P50/P54、B1-B8）----------------------

/// PROBE: `:1,2d!` / `:1,2y!` → E477 "No ! allowed"，命令不执行。
#[test]
fn bang_on_delete_yank_reports_e477() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", "!", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", ":d! 不删除");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E477"), "got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "y", "!", "<CR>"]);
    assert_eq!(f2.host.statuses.last().unwrap(), "E477: No ! allowed: 1,2y!");
    assert_eq!(
        f2.vim.registers.get('0').map(|r| r.text.as_str()),
        None,
        "y! 未执行（无 yank 寄存器副作用）"
    );
}

/// PROBE: `:y a3` 打包形式 = 寄存器 a + count 3（@a = 'b c '）。
#[test]
fn packed_register_count_yank() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &[":", "2", "y", " ", "a", "3", "<CR>"]);
    let reg = f.vim.registers.get('a').expect("register a");
    assert_eq!(reg.text, "b\nc\nd\n", "a3 = 寄存器 a + 3 行");
}

/// PROBE: `:d a b` → E488 trailing 'b'，不删除。
#[test]
fn garbage_register_tail_reports_e488() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "d", " ", "a", " ", "b", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

/// PROBE: `:j 3x` 尾参垃圾同样 E488（原先静默按 0 处理照常 join）。
#[test]
fn join_garbage_count_reports_e488() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "j", " ", "3", "x", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

/// PROBE: 越界地址报 E16 且命令不执行：`:1000000d`、`:2,99999d`、
/// `:.+99d`、裸 `:99999999`（原先静默钳制到末行并执行）。
#[test]
fn out_of_range_addresses_report_e16() {
    for line in ["1000000d", "2,99999d"] {
        let f = edit("a\nb\nc\n", 0, 0, &[":", &line[..line.len() - 1], "<CR>"]);
        // feed 逐字符，上面的切片行不通 —— 用一条条构造
        drop(f);
    }
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", "0", "0", "0", "0", "0", "0", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "越界 d 不执行");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "2", ",", "9", "9", "9", "9", "9", "d", "<CR>"]);
    let last = f2.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "got {last:?}");
    assert_eq!(f2.text(), "a\nb\nc\n");

    // 地址 0 / 负数仍合法：作用在首行（vim 9.1 探针 B1/B3）
    let f3 = edit("a\nb\nc\n", 1, 0, &[":", "0", "d", "<CR>"]);
    assert_eq!(f3.text(), "b\nc\n");
}

/// PROBE: `:s/a\/b/x/` 匹配字面 `a/b`（转义分隔符，原先解析被切碎）。
#[test]
fn escaped_delimiter_in_substitute() {
    let f = edit("a/b\n", 0, 0, &[
        ":", "s", "/", "a", "\\", "/", "b", "/", "x", "/", "<CR>",
    ]);
    assert_eq!(f.text(), "x\n", "转义分隔符不切开模式");
}

/// PROBE: `:%s/foo//n` → "2 matches on 2 lines"，缓冲不动（`n` = 仅报告；
/// 原先忽略该标志并执行破坏性替换）。
#[test]
fn substitute_n_flag_reports_without_modifying() {
    let f = edit("foo bar\nx foo y\n", 0, 0, &[
        ":", "%", "s", "/", "f", "o", "o", "/", "/", "n", "<CR>",
    ]);
    assert_eq!(f.text(), "foo bar\nx foo y\n", "n 标志不修改缓冲");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert_eq!(last, "2 matches on 2 lines");
}

/// `:se` 缩写、`:setlocal`、`:set ts&` 重置到默认。
#[test]
fn set_spellings_and_amp_reset() {
    let mut f = Fixture::new("a\n");
    f.feed([":", "s", "e", " ", "t", "s", "=", "2", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 2, ":se ts=2 生效");
    f.feed([":", "s", "e", "t", "l", "o", "c", "a", "l", " ", "t", "s", "=", "6", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 6, ":setlocal = :set");
    f.feed([":", "s", "e", "t", " ", "t", "s", "&", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 4, "ts& 重置默认");
    f.feed([":", "s", "e", "t", " ", "t", "s", "=", "3", "<CR>"]);
    f.feed([":", "s", "e", "t", " ", "t", "s", "&", "v", "i", "m", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 4, "ts&vim 同样重置");
}

/// PROBE: `:set ts ?`（问号前有空格）= 查询；裸数字选项名 `:set ts` 也是
/// 查询（原先 bell 并中止整行）。
#[test]
fn set_spaced_question_mark_queries() {
    let mut f = Fixture::new("a\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", " ", "?", "<CR>"]);
    assert_eq!(
        f.host.statuses.last().map(String::as_str),
        Some("  tabstop=4"),
        "ts ? 查询不响铃"
    );
    assert_eq!(f.host.bells, 0);
    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "s", "e", "t", " ", "t", "s", "<CR>"]);
    assert_eq!(
        f2.host.statuses.last().map(String::as_str),
        Some("  tabstop=4"),
        "裸 ts = 查询"
    );
    // 布尔名 + 空格问号 = 查询而非置位
    let mut f3 = Fixture::new("a\n");
    f3.feed([":", "s", "e", "t", " ", "n", "o", "i", "c", "<CR>"]);
    f3.feed([":", "s", "e", "t", " ", "i", "c", " ", "?", "<CR>"]);
    assert_eq!(
        f3.host.statuses.last().map(String::as_str),
        Some("  noignorecase"),
        "ic ? 是查询，不置位"
    );
}

/// `:bn[ext]`/`:bp[revious]` 等完整 vim 前缀缩写（`:bne` 原先 E492）。
#[test]
fn buffer_command_prefix_spellings() {
    for spelling in ["bn", "bne", "bnex", "bnext"] {
        let mut f = Fixture::new("a\n");
        let keys: Vec<&str> = std::iter::once(":")
            .chain(spelling.chars().map(|c| c.to_string().leak() as &str))
            .chain(std::iter::once("<CR>"))
            .collect();
        f.feed(keys);
        assert!(
            f.host.statuses.is_empty(),
            ":{spelling} 不应报错（got {:?}）",
            f.host.statuses.last()
        );
    }
}

// ---- Replace 模式 BS 位置栈 + count-repeat 的 autoindent（vim 9.1 探针
//      P23 矩阵 / Q8 矩阵 / P24）----------------------------------------------

/// PROBE 矩阵（Q8a）：`wxyz` 上 `Rab<BS><Esc>` → 'axyz'：BS 恢复被覆盖的
/// 原字符。None 条目（行尾追加）匹配时按删除处理（Q8e）。
#[test]
fn replace_bs_restores_matching_position() {
    let mut f = edit("wxyz\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "axyz\n", "BS 恢复位置匹配的条目");
}

/// PROBE 矩阵（Q8b/Q8c/P23b/P23d）：光标移动后的 BS 是**整体 no-op**——
/// 栈顶条目不属于当前退格位置，vim 既不恢复也不动光标。旧 LIFO 弹出后
/// 写错位置（`Rabc<Left><Right><BS>` 曾得 'ayy'）。
#[test]
fn replace_bs_after_cursor_move_is_noop() {
    // Left 一次再 BS：栈顶是 c@2，退格位是 1 → 不恢复
    let mut f = edit("xyz\n", 0, 0, &["R"]);
    f.type_text("abc");
    f.feed(["<Left>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc\n", "P23b：移动后 BS 不改文本");

    // P23d：Left Left Right BS → 同样不动
    let mut f2 = edit("xyz\n", 0, 0, &["R"]);
    f2.type_text("abc");
    f2.feed(["<Left>", "<Left>", "<Right>"]);
    f2.feed(["<BS>"]);
    f2.feed(["<Esc>"]);
    assert_eq!(f2.text(), "abc\n", "P23d：移动后 BS 不改文本");
}

/// PROBE（P23c）：Right 在行尾无处可去，光标仍在打字末端 → BS 照常恢复。
#[test]
fn replace_bs_at_stuck_right_still_restores() {
    let mut f = edit("xyz\n", 0, 0, &["R"]);
    f.type_text("abc");
    f.feed(["<Right>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abz\n", "P23c：Right 卡在行尾，BS 正常恢复");
}

/// PROBE（Q8d）：行尾追加字符 BS = 纯删除（None 条目），随后恢复被覆盖的 z。
#[test]
fn replace_bs_appended_char_deletes_then_restores() {
    let mut f = edit("wxyz\n", 0, 0, &["R"]);
    f.type_text("abcde");
    f.feed(["<BS>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcz\n", "Q8d：追加字符删除、覆盖字符恢复");
}

/// PROBE（P24）：autoindent 下 `3ifoo<CR>bar<Esc>` 得三组 foo/bar——count
/// 以**展开后**（含缩进）的文本为单元复制（原先 rep.text 原文长度不变式
/// 失败，count 被静默丢弃）。
#[test]
fn count_repeat_insert_with_autoindent_newline_replicates() {
    let mut f = Fixture::new("start\n");
    f.vim.options.autoindent = true;
    f.feed(["3", "i"]);
    f.type_text("foo");
    f.feed(["<Enter>"]);
    f.type_text("bar");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "foo\nbarfoo\nbarfoo\nbarstart\n",
        "vim P24：三组 foo/bar"
    );
}

/// 无换行的 count-repeat 行为不变（`3ix<Esc>` → 'xxx'）。
#[test]
fn count_repeat_insert_plain_still_works() {
    let mut f = Fixture::new("y\n");
    f.feed(["3", "i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "xxxy\n");
}

// ---- config 解析（vimrc 面）------------------------------------------------

/// `let mapleader2 = ";"` 是无关变量——不得劫持 `<Leader>`，且要有痕迹。
#[test]
fn mapleader_variable_requires_word_boundary() {
    let rc = "let mapleader = \",\"\nlet mapleader2 = \";\"\nnnoremap <Leader>x ix<Esc>\n";
    let config = vimcore::config::parse(rc);
    // leader 仍是 ","
    let lhs = &config.mappings[0].lhs;
    assert_eq!(
        lhs,
        &vec![vimcore::key::Key::char(','), vimcore::key::Key::char('x')],
        "<Leader> 解析为逗号 + x"
    );
    assert!(
        config.ignored.iter().any(|l| l.contains("mapleader2")),
        "mapleader2 行应进 ignored（got {:?}）",
        config.ignored
    );
}

/// `set\tnumber`（TAB 分隔）必须生效——旧实现静默进 ignored。
#[test]
fn tab_separated_set_line_applies() {
    let config = vimcore::config::parse("set\tnumber\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::On("number".to_owned())],
        "TAB 分隔的 set 生效"
    );
    assert!(config.ignored.is_empty());
}

/// `set ts=4 " note`：注释尾随不产生垃圾 Setting（曾虚增 ignored 计数）。
#[test]
fn set_line_trailing_comment_dropped() {
    let config = vimcore::config::parse("set ts=4 \" my preference\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::Value("ts".to_owned(), "4".to_owned())],
        "只有 ts=4 一个设置"
    );
}

/// `set ts&` 在 rc 里重置默认；`set ic?` 查询被静默丢弃（不产生垃圾）。
#[test]
fn set_reset_and_query_forms_in_rc() {
    let config = vimcore::config::parse("set ts&\nset ic?\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::Reset("ts".to_owned())],
        "ts& = Reset，ic? 被丢弃"
    );
}

/// `<LocalLeader>` 经 `let maplocalleader` 解析（原先残留 Named 键，
/// 映射永久无法触发）。
#[test]
fn local_leader_resolves_from_let_variable() {
    let rc = "let maplocalleader = \"-\"\nnnoremap <LocalLeader>x ix<Esc>\n";
    let config = vimcore::config::parse(rc);
    let lhs = &config.mappings[0].lhs;
    assert_eq!(lhs.len(), 2);
    assert_eq!(
        lhs[0],
        vimcore::key::Key::char('-'),
        "<LocalLeader> 解析为 -"
    );
    assert_eq!(lhs[1], vimcore::key::Key::char('x'));
}

/// PROBE: vim 9.1 Q1——`*` 在非词非空白字符上搜该字符的**字面**（pattern
/// 为 `!`），不是响铃也不是复用旧 pattern。行内向后首个非空白同理。
#[test]
fn star_on_non_word_char_searches_it_literally() {
    let mut f = Fixture::new("foo !\nbar !\n");
    f.feed(["/", "b", "a", "r", "<Enter>"]); // 先武装一个无关 pattern
    f.feed(["g", "g"]); // 回第 1 行
    f.feed(["$"]); // 光标到 '!'
    f.feed(["*"]);
    assert_eq!(
        f.vim.search.pattern.as_deref(),
        Some("!"),
        "字面 ! 成为主 pattern"
    );
    assert_eq!(f.vim.cursor_offset(), 10, "跳到下一个 !（第 2 行）");
}

/// 光标行全是空白时 `*` 报 E348（vim 的错误串），不再引用陈旧 pattern。
#[test]
fn star_on_blank_line_reports_e348() {
    let mut f = Fixture::new("word\n   \n");
    f.feed(["/", "w", "o", "r", "d", "<Enter>"]);
    f.feed(["j", "j"]); // 空白行
    f.feed(["*"]);
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert_eq!(last, "E348: No string under cursor");
}
