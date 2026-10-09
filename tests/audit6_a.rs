//! audit6_a：域 A（Ex 命令 / 命令行编辑 / Ex 地址与范围 / 搜索 / `:s` / `:set` / `:g`）
//! 第五轮独立审计（2026-10-09），智能体 A。27 个新 bug 探针（红）+ 7 个证伪/回归钉（绿）。
//!
//! 方法：上轮（audit5_a）没扫过的角落——`:m`/`:t`/`:>`/`:=`/`:p`/`:l`/`:nu`/
//! `:k`/`:@` 缺失面、`'[`/`']` 在 `:y`/`:d`/`:pu` 的落账、`:d`/`:y`/`:pu` 的
//! 行数报告（misc1.c msgmore / "N lines yanked"）、`:s` 报告阈值（do_sub_msg
//! 的 sub_nsubs > p_report）、`:le {indent}` 的 TAB 折叠（audit4 挂账项）、
//! 命令行光标编辑（c_CTRL-B）、命令行 `<C-k>` 二合字母、`::`/`:5:` 的第二个
//! 冒号、`:y`/`:d`/`:j` 的 count=0（E939）、`:reg`/`:marks` 表头。
//!
//! 每个期望都用本机 `/usr/bin/vim`（9.1 patches 1-1752）typeahead 通道实证
//! （/tmp/audit6_o 的采样，键序与输出写在每条注释里；o 前缀 = writefile 输出
//! 文件名）。引擎默认分歧（ignorecase=on、ts=4、tw=78）在涉选项的探针里先
//! `:set` 钉平。全部 #[ignore]：修复前红着，修复后转回归。
//!
//! ## 已证伪（oracle 已证引擎与 vim 9.1 一致，下轮免重查）
//! - `:?a?+1y b`（反向搜索地址 + 偏移 + 命令）：oracle obs b=b —— 引擎一致。
//! - `:0d`：oracle od0 = [b,c] 无报错（地址 0 对行命令合法，删第一行）——
//!   引擎一致。
//! - `:2,4sort` 后光标落范围首行（oracle osc p=2 1）——引擎一致。
//! - `:%s/zz//n` 零匹配报 E486（oracle onf）——引擎一致。
//! - `<C-r>/` 粘上次搜索进命令行（oracle ocr = E492: foo）——引擎一致。
//! - `:dl` 族 = :delete + l/p/# 打印旗标（change.txt ":dl delete and list"；
//!   oracle odl4：`:1,3dl` 报 "3 fewer lines" 并打印 `d$`，寄存器 l 空）——
//!   audit5 A-7 的方向正确；打印的【行】与报告形状有独立 bug（E-A-27）。
//! - `:2y 7` 的数字实参 = COUNT 而非寄存器 7（ex_docmd.c:2373 的
//!   `!((argt & EX_COUNT) && VIM_ISDIGIT)` 规则；oracle oy7a = "3 lines
//!   yanked"，yank 范围与引擎 count 路径一致）——audit5 A-18 的钳制形状成立。
//!
//! ## 相关域备注（B/C 域与主会话参考）
//! - normal 模式 `3dd`/`3yy` 在 vim 也有 msgmore 报告（"3 fewer lines" /
//!   "3 lines yanked"）；引擎同样静默（临时探针实测 statuses=[]）。根因同
//!   E-A-17/E-A-19，修复报告层时应一并覆盖。
//! - tests/engine.rs（"2 substitutions on 1 line"）与
//!   tests/review_regressions.rs（"2 substitutions on 2 lines"）钉的 2-替换
//!   报告与 oracle 冲突（E939 见 E-A-16 的注释），修复时需更正测试。

mod common;

use common::Fixture;
use vimcore::VimBuffer;

/// 把一条 Ex 命令行逐字符送进提示符（`f.feed` 的 item 是"一个键"）。
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

/// 光标行（0 基）。
fn line_of(f: &Fixture) -> usize {
    f.line()
}

// ------------------------------------------------------------- E-A-1（P2）
// `:m`/`:move` 整族缺失：行搬移命令落 E492，文本不动。同族行命令
// （:d/:y/:j/:sort/:pu/:s）引擎已实现，缺 :m 是缺兄弟形态。
//
// oracle（om，typeahead）：buf [a,b,c,d]，keys `:2m0<CR>…`：
//   文本 [b,a,c,d]，无报错，光标 p=1 1（ex_cmds.c ex_copymove/do_move）。
#[test]
#[ignore]
fn move_command_missing() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2m0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2m0` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "b\nc\na\nd\n",
        "vim 9.1: `:2m0` 把行 2 搬到文件顶（oracle om）"
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(0),
        "vim 9.1: `:m` 后光标落被搬移行行首（oracle om p=1 1）"
    );
}

// ------------------------------------------------------------- E-A-2（P2）
// `:m` 的落点语义（修复 E-A-1 时的实现契约）：光标落在被搬移行的首个
// 非空白（do_move 尾部 begline）。
//
// oracle（om2）：buf ['  x','a','b']，`:1m2<CR>`：
//   文本 [a,'  x',b]，光标 p=2 3（行 2 col 3 = 搬移行的 'x'）。
#[test]
#[ignore]
fn move_cursor_lands_on_moved_line_first_non_blank() {
    let mut f = Fixture::new("  x\na\nb\n");
    ex(&mut f, "1m2");
    assert_eq!(f.text(), "a\n  x\nb\n", "oracle om2");
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1) + 2,
        "vim 9.1: 光标 = 搬移行首非空白（oracle om2 p=2 3）"
    );
}

// ------------------------------------------------------------- E-A-3（P2）
// `:t`/`:co`/`:copy` 整族缺失：`:2t0` 复制行 2 到文件顶，引擎 E492。
//
// oracle（ot）：buf [a,b,c,d]，`:2t0<CR>`：
//   [b,a,b,c,d]，光标 p=1 1（新副本行行首）。
#[test]
#[ignore]
fn copy_command_missing() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2t0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2t0` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(f.text(), "b\na\nb\nc\nd\n", "oracle ot");
    assert_eq!(
        f.cursor(),
        f.buf.line_start(0),
        "vim 9.1: `:t` 光标落副本行行首（oracle ot p=1 1）"
    );
}

// ------------------------------------------------------------- E-A-4（P2）
// Ex 缩进命令 `:>`/`:<<`/`:{range}>{count}` 缺失（`:h :>`），引擎 E492。
// normal `>>`/`<<` 已实现，Ex 侧是同族缺失。
//
// oracle（osh）：buf [a,b,c]，`:set sw=4 ts=4 et`，`:2,3><CR>`：
//   [a,'    b','    c']，光标 p=3 5 = 范围【末地址】行的首非空白
//   （Ex 命令的通用泊位规则：line2 + begline）。
#[test]
#[ignore]
fn ex_shift_right_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "set sw=4");
    ex(&mut f, "set ts=4");
    ex(&mut f, "set et");
    ex(&mut f, "2,3>");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2,3>` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "a\n    b\n    c\n",
        "oracle osh：行 2-3 各右移一个 shiftwidth"
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2) + 4,
        "vim 9.1: 光标落范围末行首非空白（oracle osh p=3 5）"
    );
}

// ------------------------------------------------------------- E-A-5（P3）
// `:=` 缺失：打印【范围的末地址】行号（ex_docmd.c ex_equal；无 range =
// 末行行号，不是光标行），引擎 E492。
//
// oracle（oeq/oeq2）：buf [a,b,c]：
//   `:=` → "3"（末行行号）；`:2=` → "2"；`:2,3=` → "3"。
#[test]
#[ignore]
fn equals_prints_range_last_line() {
    let mut f = Fixture::at("a\nb\nc\n", 1, 0);
    ex(&mut f, "=");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:=` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["3".to_owned()],
        "vim 9.1: 裸 `:=` 打印末行行号 3（oracle oeq），不是光标行 2"
    );
    ex(&mut f, "2=");
    assert_eq!(
        f.host.statuses.last(),
        Some(&"2".to_owned()),
        "vim 9.1: `:2=` 打印地址行号（oracle oeq2 m1=2）"
    );
}

// ------------------------------------------------------------- E-A-6（P3）
// `:p`/`:print` 缺失：把范围的行打到消息区，引擎 E492。
//
// oracle（op）：buf [a,b,c]，`let m = execute('1,2p')` → "a\nb"。
#[test]
#[ignore]
fn print_command_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "1,2p");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2p` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["a".to_owned(), "b".to_owned()],
        "vim 9.1: `:1,2p` 逐行打印（oracle op）"
    );
}

// ------------------------------------------------------------- E-A-7（P3）
// `:l`/`:list` 缺失：list 式打印（TAB 转 ^I、行尾 $），引擎 E492。
//
// oracle（ol）：buf ["a","\tb","c"]，`execute('1,2l')` → "a$\n^Ib$"。
#[test]
#[ignore]
fn list_command_missing() {
    let mut f = Fixture::new("a\n\tb\nc\n");
    ex(&mut f, "1,2l");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2l` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["a$".to_owned(), "^Ib$".to_owned()],
        "vim 9.1: `:1,2l` 打印 a$ 与 ^Ib$（oracle ol）"
    );
}

// ------------------------------------------------------------- E-A-8（P3）
// `:nu`/`:#`/`:number` 缺失：带行号前缀打印，引擎 E492。格式 =
// numberwidth-1 宽右对齐行号 + 空格 + 行文（默认 nw=4 → "  1 a"）。
//
// oracle（onu）：buf [a,b]，`execute('1,2nu')` → "  1 a\n  2 b"。
#[test]
#[ignore]
fn number_command_missing() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "1,2nu");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2nu` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["  1 a".to_owned(), "  2 b".to_owned()],
        "vim 9.1: `:nu` 行号前缀格式（oracle onu）"
    );
}

// ------------------------------------------------------------- E-A-9（P3）
// `:k{mark}` / `:ma[rk] {mark}`（设置 mark 的 Ex 命令）缺失：`:3ka` 后
// `` `a `` 应跳行 3，引擎 `:3ka` E492、mark 未设。
//
// oracle（ok）：buf [a,b,c]，keys `:3ka<CR>` `` `a `` → p=3 1。
#[test]
#[ignore]
fn k_mark_command_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "3ka");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:3ka` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    f.feed(["`", "a"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: `:3ka` 设 mark a，`` `a `` 跳行 3 列 1（oracle ok p=3 1）"
    );
}

// ------------------------------------------------------------- E-A-10（P3）
// `:@{reg}` 把寄存器内容当 Ex 命令行执行（`:h :@`），引擎 E492。
//
// oracle（oat）：buf [a,b,c]，`@a="1,2d\r"` 后 `:@a` → 缓冲 [c]，无报错。
#[test]
#[ignore]
fn at_register_executes_ex() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.vim
        .registers
        .store_yank(Some('a'), "1,2d\n".into(), vimcore::registers::RegisterKind::Linewise);
    ex(&mut f, "@a");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:@a` 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "c\n",
        "vim 9.1: `:@a` 执行寄存器 a 里的 '1,2d'（oracle oat）"
    );
}

// ------------------------------------------------------------- E-A-11（P2）
// 命令行光标编辑整族缺失：`<C-b>`/`<C-e>`/`<Left>`/`<Right>`/`<Home>`/
// `<End>`（c_CTRL-B/c_CTRL-E/c_<Left>/c_<Right>）移动命令行光标并允许从
// 中间编辑；引擎的命令行缓冲只有 append/pop 模型，这些键被静默吞掉、新键
// 一律接到行尾。
//
// oracle（ocb，typeahead）：keys `:set ts=4` `\x02` `x` `<CR>`：
//   命令行 'xset ts=4' → E492: Not an editor command: xset ts=4。
// 引擎：`<C-b>` 被吞，'x' 接尾 → 'set ts=4x' → E521: Number required
//   after =: ts=4x。
#[test]
#[ignore]
fn cmdline_ctrl_b_moves_cursor_to_start() {
    let mut f = Fixture::new("ab\n");
    f.feed([
        ":", "s", "e", "t", " ", "t", "s", "=", "4", "<C-b>", "x", "<CR>",
    ]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains("xset ts=4")),
        "vim 9.1: <C-b> 后输入的 x 落在行首（oracle ocb：E492: xset ts=4）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-12（P3）
// 命令行 `<C-k>` 二合字母（c_CTRL-K）：`:<C-k>ss` 把 'ß' 打进命令行。insert
// 模式的 <C-k> 已实现（audit H3），命令行侧是缺兄弟形态。
//
// oracle（ock）：keys `:` `\x0b` `ss` `<CR>` → E492 报文里是 'ß'。
// 引擎：C-k 被吞 → E492: Not an editor command: ss。
#[test]
#[ignore]
fn cmdline_ctrl_k_digraph() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "<C-k>", "s", "s", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains('ß')),
        "vim 9.1: <C-k>ss 在命令行产生 ß（oracle ock 的 E492 报文含 ß）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-13（P3）
// `:y` 不落 `'[`/`']`：op_yank 把 b_op_start/b_op_end 设到被 yank 文本首尾
// （Ex 的 :y 与普通 yy 同一条 op_yank 路径）。引擎 ex_yank_lines 走
// yank_span，marks 不动（audit5 C-5 只补了 normal 粘贴侧）。
//
// oracle（oy）：buf [a,b,c,d]，`:2,3y<CR>` 后：
//   `` `[ `` → p=2 1；`` `] `` → q=3 1。
#[test]
#[ignore]
fn ex_yank_sets_marks() {
    let mut f = Fixture::at("a\nb\nc\nd\n", 0, 0);
    ex(&mut f, "2,3y");
    f.feed(["`", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "vim 9.1: `:2,3y` 后 `[ = yank 首行行首（oracle oy p=2 1）"
    );
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: `:2,3y` 后 ] = yank 末行行首（oracle oy q=3 1）"
    );
}

// ------------------------------------------------------------- 证伪钉（绿）
// `:d` 的 `'[`/`']`：引擎两个 mark 都能解析到删除点（oracle od：`'[`=p=2 1、
// `']`=q=2 1，引擎探针已过）——audit5 C-5/C-6 的落账链覆盖了 :d，钉住防回归。
#[test]
#[ignore]
fn ex_delete_marks_bracket() {
    let mut f = Fixture::at("a\nb\nc\nd\n", 0, 0);
    ex(&mut f, "2,3d");
    f.feed(["'", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "oracle od：`'[` = 删除点行 2 列 1（引擎已过，回归钉）"
    );
    f.feed(["'", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "oracle od：`']` = 删除点行 2 列 1"
    );
}

// ------------------------------------------------------------- E-A-14（P3）
// `:pu` 不落 `'[`/`']`：do_put 两个 mark 都设。normal 侧 p/P 引擎已落账
// （audit5 C-5），Ex 侧 ex_put 没有。
//
// oracle（opu）：buf [a,b]，`yy` 后 `:pu<CR>`：
//   `'[` → p=3 1；`']` → q=3 1（linewise 粘贴的 ']' 也在粘贴行 col 1）。
#[test]
#[ignore]
fn ex_put_sets_marks() {
    let mut f = Fixture::at("a\nb\n", 0, 0);
    f.feed(["y", "y"]);
    ex(&mut f, "pu");
    f.feed(["'", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: `:pu` 后 '[ = 粘贴首行（oracle opu p=3 1）"
    );
    f.feed(["'", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: `:pu` 后 '] 同在粘贴行（oracle opu q=3 1）"
    );
}

// ------------------------------------------------------------- E-A-15（P3）
// `:le {indent}` 的缩进列不按 'expandtab'/'tabstop' 折叠成 TAB（audit4 挂账
// 项）：noet+ts=4 下 `:le 8` 折成两个 TAB（set_indent 规则），引擎一律
// 8 个空格。
//
// oracle（ole_out）：buf ["中文"]，`:set noet ts=4` + `:le 8` + `:w!`：
//   od = \t \t 中 文 \n。
#[test]
#[ignore]
fn left_indent_folds_to_tabs_under_noexpandtab() {
    let mut f = Fixture::new("中文\n");
    ex(&mut f, "set noet");
    ex(&mut f, "set ts=4");
    ex(&mut f, "le 8");
    assert_eq!(
        f.text(),
        "\t\t中文\n",
        "vim 9.1: noet+ts=4 下 `:le 8` 前缀折成两个 TAB（oracle ole_out）"
    );
}

// ------------------------------------------------------------- E-A-16（P3）
// `:s` 完成报告阈值错位：vim 只在替换数 > 'report'（默认 2，即 3+）时报
// "{n} substitutions on {m} lines"（ex_cmds.c do_sub_msg：sub_nsubs >
// p_report）；引擎 total > 1 就报。2 个替换时 vim 静默、引擎报
// "2 substitutions on …"。
//
// oracle（osr2/osr3/osr4）：2 个替换（"foo bar foo" %s/foo/qux/g）→ m=''；
//   3 个替换（"aaa"）→ "3 substitutions on 1 line"；[a,a,a] →
//   "3 substitutions on 3 lines"。
#[test]
#[ignore]
fn substitute_report_threshold_is_three() {
    let mut f = Fixture::new("foo bar foo\n");
    ex(&mut f, "%s/foo/qux/g");
    assert_eq!(
        f.text(),
        "qux bar qux\n",
        "替换本体两侧一致"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("substitutions")),
        "vim 9.1: 2 个替换 ≤ report 阈值，静默（oracle osr2 m=''）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
    let mut f3 = Fixture::new("aaa\n");
    ex(&mut f3, "%s/a/b/g");
    assert_eq!(
        f3.host.statuses.last(),
        Some(&"3 substitutions on 1 line".to_owned()),
        "oracle osr3：3 个替换才报告"
    );
}

// ------------------------------------------------------------- E-A-17（P3）
// `:d` 不报行数：vim 删行后经 msgmore（misc1.c，阈值 > 'report' 即 3+）
// 报 "{n} fewer lines"，n 是【净行数变化】（全删 5 行剩 1 空行 = 4）；
// 引擎 ex_delete_lines 静默。
//
// oracle（odel/odel2）：buf 5 行，`execute('1,5d')` → "4 fewer lines"；
//   `:1,2d` 静默。
#[test]
#[ignore]
fn delete_reports_fewer_lines() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    ex(&mut f, "1,5d");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("4 fewer lines")),
        "vim 9.1: `:1,5d` 报 '4 fewer lines'（净变化，oracle odel）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("5 fewer lines")),
        "净行数是 4（5 行 → 1 空行），不是删除行数 5"
    );
}

// ------------------------------------------------------------- E-A-18（P3）
// `:pu` 不报 "N more lines"：同 msgmore 通道（阈值 > 'report'），引擎静默。
//
// oracle（opu2）：`@a="x\ny\nz\n"` 后 `:pu a` → "3 more lines"。
#[test]
#[ignore]
fn put_reports_more_lines() {
    let mut f = Fixture::new("a\n");
    f.vim
        .registers
        .store_yank(Some('a'), "x\ny\nz\n".into(), vimcore::registers::RegisterKind::Linewise);
    ex(&mut f, "pu a");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("3 more lines")),
        "vim 9.1: `:pu a` 粘 3 行报 '3 more lines'（oracle opu2）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-19（P3）
// `:y` 不报 "N lines yanked"：op_yank 对 > report 行的 yank 报告（阈值 3+），
// 引擎 ex_yank_lines 静默。
//
// oracle（oy7a）：buf [a,b,c,d]，`execute('2y 7')` → "3 lines yanked"
//   （count 7 从行 2 钳到末行 = 行 2-4 共 3 行）。
#[test]
#[ignore]
fn yank_reports_lines_yanked() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2y 7");
    let got = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("b\nc\nd\n"),
        "yank 范围两侧一致（count 钳到缓冲末）"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("3 lines yanked")),
        "vim 9.1: `:2y 7` 报 '3 lines yanked'（oracle oy7a）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-20（P3）
// `::`（第二个冒号）：`:` 单独是空命令——静默无操作、光标不动（oracle
// odc：v:errmsg 空、p=1）；引擎报 E492: Not an editor command: :。
#[test]
#[ignore]
fn double_colon_is_silent_noop() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, ":");
    assert!(
        f.host.statuses.is_empty(),
        "vim 9.1: `::` 是静默空命令（oracle odc e=''）；\
         实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(line_of(&f), 0, "oracle odc：光标不动（p=1）");
}

// ------------------------------------------------------------- E-A-21（P3）
// `:5:`（地址 + 第二个冒号）：`:` 空命令 + 裸地址泊位——光标停行 5 首非空白
// （oracle oac p=5 1）；引擎对剩余的 `:` 报 E492。
#[test]
#[ignore]
fn address_then_colon_parks_like_bare_address() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "5:");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:5:` 无报错（oracle oac e=''）；实际 statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(4),
        "vim 9.1: `:5:` 光标停行 5 首非空白（oracle oac p=5 1）"
    );
}

// ------------------------------------------------------------- E-A-22（P3）
// `:y` 的 count=0：:yank 带 EX_COUNT 且无 EX_ZEROR（ex_cmds.h），count 0 走
// E939 "Positive count required: {整条命令}"（ex_docmd.c 的
// e_positive_count_required 路径），命令不执行、寄存器不动。引擎把 0 当
// "无 count" 照常 yank 进 "0。
//
// oracle（oyz）：`:2,3y 0` → E939: Positive count required: 2,3y 0，
//   缓冲 [a,b,c,d] 与 "0 原样（r0=）。
#[test]
#[ignore]
fn yank_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2,3y 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2,3y 0` 拒绝执行（oracle oyz）"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("2,3y 0")),
        "vim 9.1: E939 报文带整条命令（oracle oyz）；实际 statuses = {:?}",
        f.host.statuses
    );
    assert!(
        f.vim
            .registers
            .get('0')
            .map(|r| r.text.is_empty())
            .unwrap_or(true),
        "vim 9.1: \"0 不被写入（oracle oyz r0=）"
    );
}

// ------------------------------------------------------------- E-A-23（P3）
// `:d 0` 同 E-A-23：E939 + 不删除。
//
// oracle（odz）：`:2,3d 0` → E939: Positive count required: 2,3d 0，
//   缓冲 [a,b,c,d] 原样。
#[test]
#[ignore]
fn delete_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2,3d 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2,3d 0` 拒绝执行（oracle odz）"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("2,3d 0")),
        "oracle odz 报文；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-24（P3）
// `:j 0` 的 count=0：:join 同样 EX_COUNT 无 ZEROR → E939；引擎把 0 当无
// count 照常连接行 2-3。
//
// oracle（ojz）：`:2j 0` → E939: Positive count required: 2j 0，缓冲原样。
#[test]
#[ignore]
fn join_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2j 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2j 0` 拒绝执行（oracle ojz）"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("2j 0")),
        "oracle ojz 报文；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-25（P3）
// `:reg` 输出缺表头 "Type Name Content"（ex_register.c show_registers 的
// msg_puts_title；数据行格式引擎已对齐）。
//
// oracle（org）：yy 后 `execute('reg')` 首行 = "Type Name Content"。
#[test]
#[ignore]
fn registers_listing_has_header_line() {
    let mut f = Fixture::new("a\n");
    f.feed(["y", "y"]);
    ex(&mut f, "reg");
    assert_eq!(
        f.host.statuses.first().map(String::as_str),
        Some("Type Name Content"),
        "vim 9.1: `:reg` 首行是表头（oracle org）；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-26（P3）
// `:marks` 输出缺表头且行格式不同：vim 的表头 = "mark line  col file/text"，
// 行 = `" %c %6ld %4d %s"`（mark.c ex_marks）；引擎行 = "a  line 1  col 0  a"
// 且无表头。（col 的 0 基字节值已对齐——audit2 R35-2——这里差表头与排版。）
//
// oracle（omk）：`ma` 后 `execute('marks')` =
//   ["mark line  col file/text", " '      1    0 a", " a      1    0 a", …]。
#[test]
#[ignore]
fn marks_listing_header_and_layout() {
    let mut f = Fixture::new("a\n");
    f.feed(["m", "a"]);
    ex(&mut f, "marks");
    assert_eq!(
        f.host.statuses.first().map(String::as_str),
        Some("mark line  col file/text"),
        "vim 9.1: `:marks` 首行是表头（oracle omk）；实际 statuses = {:?}",
        f.host.statuses
    );
    let want = format!(" a {:>6} {:>4} a", 1, 0);
    assert!(
        f.host.statuses.iter().any(|s| *s == want),
        "vim 9.1: mark a 行 = {:?}（\" %c %6ld %4d %s\" 格式，oracle omk）；\
         实际 statuses = {:?}",
        want,
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-27（P3）
// `:dl` 的打印形状错两处：vim 的 l 旗标 = ex_may_print 只打印【执行后的
// 光标行】一次（ex_docmd.c ex_may_print），且删除走同一个 msgmore 报告
// （"3 fewer lines"）；引擎把被删的每一行都 list 打出来、没有报告。
//
// oracle（odl4）：buf [a,b,c,d]，`execute('1,3dl')` → m =
//   "3 fewer lines\nd$"（先报告，再打印执行后光标行的 list 形）。
#[test]
#[ignore]
fn dl_prints_surviving_cursor_line_once() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "1,3dl");
    assert_eq!(f.text(), "d\n", "删除范围 1-3 后剩 [d]（两侧一致）");
    assert_eq!(
        f.host.statuses,
        vec!["3 fewer lines".to_owned(), "d$".to_owned()],
        "vim 9.1: `:1,3dl` = 报 '3 fewer lines' + 打印执行后光标行 'd$' \
         （oracle odl4）；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- 证伪钉
// `:g`/`:v` 走 E492 是 NOTES 分歧 #12 记录过的取舍。此钉只锁形状：E492 +
// 缓冲不动；实现 :g 后改写成正向断言。
#[test]
#[ignore]
fn global_command_still_e492_recorded_divergence() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "g/b/d");
    assert_eq!(
        f.text(),
        "a\nb\nc\n",
        "分歧 #12：:g 未实现，缓冲不动"
    );
}

// ------------------------------------------------------------- 证伪钉
// `:?a?+1y b`（反向搜索地址 + 偏移折叠 + 命令）：引擎已一致（oracle
// obs：寄存器 b = "b\n"），钉住防回归。
#[test]
#[ignore]
fn backward_search_address_with_offset() {
    let mut f = Fixture::at("a\nb\nc\n", 2, 0);
    ex(&mut f, "?a?+1y b");
    let got = f.vim.registers.get('b').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("b\n"),
        "oracle obs：从行 3 反向搜 a（行 1）+1 = 行 2"
    );
}

// ------------------------------------------------------------- 证伪钉
// `:0d`：地址 0 对行命令合法（oracle od0 = [b,c] 无报错），引擎一致。
#[test]
#[ignore]
fn zero_address_delete_removes_first_line() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "0d");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "oracle od0：`:0d` 无 E16"
    );
    assert_eq!(f.text(), "b\nc\n", "oracle od0：`:0d` 删第一行");
}

// ------------------------------------------------------------- 证伪钉
// `:2,4sort` 后光标落排序范围首行（oracle osc p=2 1），引擎一致。
#[test]
#[ignore]
fn sort_cursor_parks_on_range_first_line() {
    let mut f = Fixture::at("d\nb\na\nc\nz\n", 4, 0);
    ex(&mut f, "2,4sort");
    assert_eq!(f.text(), "d\na\nb\nc\nz\n");
    assert_eq!(line_of(&f), 1, "oracle osc：光标行 2 列 1");
}

// ------------------------------------------------------------- 证伪钉
// `:%s/zz//n` 零匹配报 E486（oracle onf），n 旗标不改变失败路径，引擎一致。
#[test]
#[ignore]
fn n_flag_zero_matches_reports_e486() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "%s/zz//n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E486")),
        "oracle onf：E486: Pattern not found: zz"
    );
}

// ------------------------------------------------------------- 证伪钉
// `<C-r>/`（粘上次搜索进命令行）：oracle ocr = E492: foo，引擎一致。
#[test]
#[ignore]
fn ctrl_r_slash_pastes_last_search() {
    let mut f = Fixture::new("foo\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed([":", "<C-r>", "/", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains("foo")),
        "oracle ocr：<C-r>/ 贴出 'foo' 后 E492"
    );
}
