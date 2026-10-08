//! audit5_a：域 A（Ex 命令 / 命令行编辑 / 搜索 / `:s` / `:set` / 地址与范围）
//! 第四轮独立审计（2026-10-09）。
//!
//! 方法：移植 `~/code/github/vim/src/testdir/`（test_substitute / test_cmdline /
//! test_search / test_excmd / test_sort / test_options）的行为断言 + 攻击面清单
//! （count=0/超大、地址倒序、空模式、`~`/`&` 族、`\n` 模式、`\/`/`\?`/`\&` 地址、
//! `|` 命令分隔、命令行 `<C-r>=`/`<C-w>`、`:set` 怪值）。每个探针的期望值都用
//! 本机 `/usr/bin/vim`（9.1 patches 1-1752）typeahead 通道实证过：
//! `vim -Nu NONE -N -i NONE -n -s keys buf`，keys 以
//! `:call writefile(...)` + `:wq` 收尾；消息类经 v:errmsg / `execute()` 采样。
//! 引擎默认分歧（ignorecase=on、ts=4）已在涉选项的探针里先 `:set` 钉平。
//!
//! 每个测试断言 **vim 9.1 的期望结果**，在当前工作树上应当失败（修复后转回归）。
//!
//! ## 已证伪（oracle 已证引擎与 vim 一致，免下轮重查）
//! - 已证伪：**裸数字 count 进 `:` 的范围预填**——vim `gg4:yank<CR>` 得
//!   `"l1\nl2\nl3\nl4\n`（count 变 `.,.+3`，oracle u8）；引擎 `Char(':')` 分支
//!   预填 `.,.+(n-1)`（state.rs 3705 起）——一致，非 bug。
//! - 已证伪：**`:s//` 空 pattern 的槽位选择**——oracle p4/p5： intervening 搜索
//!   之后 `:s//Q/` 用**搜索** pattern（'ccc'→'Q aaa'），无干预时用**替换**
//!   pattern（'aaa'→'ccc Q'）——即 vim 的 RE_LAST「最近使用者」模型。引擎的
//!   单槽 search.pattern 在两条路径上的结果与之一致（`:s` 后 set_pattern）。
//! - 已证伪：**`:sort! n` 的并列稳定性**——vim 的 `!` 是对升序稳定结果整体
//!   反转（并列也反转）：[1a,1b,2] → [2,1b,1a]（oracle u17）；引擎 sort_by_key
//!   后 reverse 同形。`:sort n` 并列不动（u18: [1a,1b,2]）。
//! - 已证伪：**`:sort n` 取数规则**——`x-22` 取 -22、` 123b` 取 123、无数字行
//!   按 0 参与比较这一半与 vim 一致（vim 例外只在「无数字行整体前置」，见
//!   sort_n 探针）；test_sort.vim 期望 + numeric_sort_key 读码一致。
//! - 已证伪（读码）：`//<CR>` 空 pattern 带闭终止符——split_search_offset 对
//!   空尾返回 pattern=""，复用上次搜索，与 vim 一致；cmdline `<BS>` 用
//!   String::pop 逐字符删除，多字节安全。
//! - 已证伪：**normal `&`**——vim `:s/a/B/` + `j` + `&` 得 [Baa,Baa]（oracle
//!   u22，`&` = 当前行无旗标重放）；引擎 NormalCmd::RepeatSubstitute
//!   （state.rs:5335）同行为，探针变绿撤下。
//! - 已证伪：**`:2,3sort u` 无变化时静默**——排序后范围与原序相同则不报文
//!   （u21 两侧一致）。
//! - 已证伪：**`2/foo/yank` 之外的同族**——`:/foo/,/bar/d` 两端各有匹配、
//!   无包裹歧义时 `,` 链的可见结果与 vim 相同（分歧只在地址锚定，见 A7 探针
//!   的 yank 显形）。

mod common;

use common::Fixture;

/// 把一条 Ex 命令行逐字符送进提示符（`f.feed` 的 item 是"一个键"）。
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

// ---------------------------------------------------------------- 1（P2）
// `:sort n`：无数字行必须整体排在所有数字行之前（vim 按「有无数值」分两段，
// 各段内再排）；引擎把无数字行的键算成 0，混进了数字段的中间。
//
// 复现：['abc','5','-3','7','x']，`:%sort n`。
// vim 9.1：`abc x -3 5 7`（非数字段 abc/x 在前，数字段升序在后）。
// 引擎：numeric_sort_key('abc')=0、('x')=0 → 稳定排序得 `-3 abc x 5 7`
//     （abc/x 被 -3 压到中间）。
// oracle：cd /tmp/orc5 && printf 'abc\n5\n-3\n7\nx\n' > b1 && :sort n + writefile
//   → o1 = abc~x~-3~5~7（test_sort.vim Test_sort_large_num 的断言同形：
//   "Non-numeric lines are ordered before numerical lines"）。
#[test]
fn sort_n_orders_nonnumeric_lines_first() {
    let mut f = Fixture::new("abc\n5\n-3\n7\nx\n");
    ex(&mut f, "%sort n");
    assert_eq!(
        f.text(),
        "abc\nx\n-3\n5\n7\n",
        "vim 9.1: :sort n 无数字行整段前置，不按 0 混入数字段"
    );
}

// ---------------------------------------------------------------- 2（P3）
// `:sort u` 删重后 vim 会在消息通道报 "N fewer lines"；引擎静默。
//
// 复现：[1,1,2,2,3,3]，`:%sort u`。
// vim 9.1：缓冲 [1,2,3] + 消息 "3 fewer lines"（test_sort.vim
//     Test_sort_cmd_report 断言同文）。
// 引擎：ex_sort 去重后不发任何 status_message。
// oracle：`:let m = execute('%sort u')` → m = "3 fewer lines"，缓冲 1,2,3（u13）。
#[test]
fn sort_u_reports_lines_removed() {
    let mut f = Fixture::new("1\n1\n2\n2\n3\n3\n");
    ex(&mut f, "%sort u");
    assert_eq!(f.text(), "1\n2\n3\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("3 fewer lines")),
        "vim 9.1: :sort u 报 '3 fewer lines'；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 3（P2）
// `:s/foo/bar/0`：count 0 是错误（E939 Positive count required），命令整体
// 不执行；引擎把 0 当「无 count」照常替换——一个本应被拒绝的命令改了文本。
//
// 复现：'foo'，`:s/foo/bar/0`。
// vim 9.1：文本 'foo' 不动 + E939（test_substitute.vim Test_substitute_count：
//   assert_fails('s/foo/bar/0', 'E939:')）。
// 引擎：`Some(0)` 落进 `_ => range` 臂，继续替换 → 'bar'，无消息。
// oracle：o2 = foo~'E939: Positive count required'。
#[test]
fn substitute_count_zero_is_e939_noop() {
    let mut f = Fixture::new("foo\n");
    ex(&mut f, "s/foo/bar/0");
    assert_eq!(f.text(), "foo\n", "vim 9.1: count 0 拒绝执行，文本不动");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E939")),
        "vim 9.1: E939: Positive count required；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 4（P3）
// `:s` 的 `g` 旗标逐个出现即翻转：`:s/foo/FOO/gg` = 两个 g 抵消 = 只换每行
// 第一个（test_substitute.vim Test_substitute_gdefault 断言
//   s/foo/FOO/gg → 'FOO bar foo'）。引擎 flags.contains('g') 恒真。
//
// 复现：'foo bar foo'，`:set noic` 后 `:s/foo/FOO/gg`。
// vim 9.1：'FOO bar foo'。引擎：'FOO bar FOO'。
// oracle：o3 = FOO bar foo。
#[test]
fn doubled_g_flag_toggles_global_off() {
    let mut f = Fixture::new("foo bar foo\n");
    ex(&mut f, "set noic");
    ex(&mut f, "s/foo/FOO/gg");
    assert_eq!(
        f.text(),
        "FOO bar foo\n",
        "vim 9.1: gg 抵消，只替换每行第一个匹配"
    );
}

// ---------------------------------------------------------------- 5（P2）
// `:s` 的 `r` 旗标：空 pattern 时改用上一次【搜索】pattern（而非替换
// pattern）。引擎不认 `r`，报 E488 且整条命令不执行。
//
// 复现：['aaa','ccc aaa','ccc']；`:1s/aaa/B/`、`/ccc`、`:2s//Q/r`。
// vim 9.1：第 2 行 'ccc aaa' → 'Q aaa'（r = RE_SEARCH = 'ccc'）。
// 引擎：E488: Trailing characters: r，第 2 行原样。
// oracle：p6 = B~Q aaa~ccc~''（无 E 消息）。
#[test]
fn r_flag_reuses_last_search_pattern() {
    let mut f = Fixture::new("aaa\nccc aaa\nccc\n");
    ex(&mut f, "1s/aaa/B/");
    f.feed(["/", "c", "c", "c", "<CR>"]);
    ex(&mut f, "2s//Q/r");
    assert_eq!(
        f.text(),
        "B\nQ aaa\nccc\n",
        "vim 9.1: r 旗标让空 pattern 复用上次搜索 pattern"
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E488")),
        "vim 9.1: r 是合法旗标，不报 E488；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 6（P2）
// `:%s/\n//` 是 vim 专门特化成 join 的惯用法（do_sub 的 `\n` 快路径，
// ex_cmds.c:4165 起）；引擎逐行替换、`\n` 在行内永不匹配 → E486，命令失效。
//
// 复现：['l1','l2','l3']，`:%s/\n//`。
// vim 9.1：'l1l2l3'（test_substitute.vim Test_substitute_join 同形）。
// 引擎：E486: Pattern not found: \n，三行原样。
// oracle：u8 = l1l2l3~''。
#[test]
fn substitute_newline_pattern_joins_lines() {
    let mut f = Fixture::new("l1\nl2\nl3\n");
    ex(&mut f, "%s/\\n//");
    assert_eq!(
        f.text(),
        "l1l2l3\n",
        "vim 9.1: :%s/\\n// 把整个范围连接成一行"
    );
}

// ---------------------------------------------------------------- 7（P1）
// `2/foo/` 数字地址与搜索地址的【连写】语法 + 第二地址搜索锚定，双层全缺：
//
// a) 语法层：vim get_address 末尾 `while (*cmd == \'/\' || *cmd == \'?\')`——
//    数字后直接跟搜索地址时并入同一地址（`:h :range` 的 `N/{pattern}` 形），
//    不需要逗号。引擎的范围字符表在数字后遇到 `/` 前必须有 `,`，`2/foo/`
//    被整体当一个地址基解析 → E16: Invalid range，命令失效（scratch 实证：
//    `2/foo/y a`/`2/foo/d` 都 E16；`1,/foo/y a` 反而通过）。
// b) 锚定层（修完 a 后的下一层）：搜索从【前一个地址】起搜，不从光标起搜
//    （ex_docmd.c get_address："When search follows another address, start
//    from there"）；引擎 base_line 恒用 cursor_line。
//
// 复现：['foo 1','foo 2','foo 3']，光标行 1；`:2/foo/y a`。
// vim 9.1：地址 = 行 3；"a = "foo 3\n"（test_cmdline.vim
//     Test_lnum_and_pattern_as_range 同断言）。
// 引擎：E16，"a 从未被写（get('a') = None）。
// oracle：o5 = 'foo 3'（getreg("a")）。
#[test]
fn second_address_search_anchors_at_first_address() {
    let mut f = Fixture::at("foo 1\nfoo 2\nfoo 3\n", 0, 0);
    ex(&mut f, "2/foo/y a");
    let got = f.vim.registers.get('a').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("foo 3\n"),
        "vim 9.1: 第二地址 /foo/ 从地址 2 起搜，落在行 3"
    );
}

// ---------------------------------------------------------------- 8（P2）
// `\/`、`\?`、`\&` 地址形态缺失：`\` 不在引擎的范围字符表里，整条命令落到
// E492，什么都不执行。vim：`\/`=「上次搜索 pattern 的下一匹配行」（跟随
// 前一地址起搜），`\?` 反向，`\&` 用上次替换 pattern。
//
// 复现：['a','b','c','d']；`/b`（光标行 2）、`:2,\/d`。
// vim 9.1：从行 2 前搜包裹回行 2 自己 → 删行 2 → 'a c d'，光标行 2。
// 引擎：E492: Not an editor command: \/d，四行原样。
// oracle：u1 = a~c~d~''~2（test_cmdline.vim Test_cmdline_search_range 同族：
//   1,\/s/b/B/、\?,4s/c/C/、1,\&s/b/B/）。
#[test]
fn backslash_search_addresses_supported() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["/", "b", "<CR>"]);
    ex(&mut f, "2,\\/d");
    assert_eq!(
        f.text(),
        "a\nc\nd\n",
        "vim 9.1: `:2,\\/d` 删除 \\/ 命中的行"
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: \\/ 地址无 E492；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 9（P2）
// `:dl` 是 `:delete` + `l` 旗标（删除并打印），不是 E492 的未知命令
// （test_excmd.vim Test_ex_delete 同族：dl/dell/delel/deletl/deletel/dp…）。
// 引擎 DELETE_SPELLINGS 无 l/p/# 旗标形态 → E492，行不被删。
//
// 复现：['foo',"\tbar"]，`:dl`。
// vim 9.1：删除当前行 → 剩 '\tbar'，并打印 `^Ibar$`（q2：无 v:errmsg）。
// 引擎：E492，两行原样。
// oracle：q2 = b~c~d~''（在 a,b,c,d 上删掉当前行 foo）。
#[test]
fn dl_deletes_current_line() {
    let mut f = Fixture::new("foo\n\tbar\n");
    ex(&mut f, "dl");
    assert_eq!(f.text(), "\tbar\n", "vim 9.1: :dl 删除当前行");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: :dl 是合法命令；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 10（P2）
// `|` 命令分隔符（`:h :bar`）：`:d`/`:s`/`:y` 等「-bar 可用」命令之后的
// `|` 启动下一条命令，两条都执行。引擎把 `|…` 当尾随垃圾报 E488，第一条
// 命令也不执行——一条命令拖死整行。
//
// 复现：['a','b','c','d']，`:1,2d|j`。
// vim 9.1：删 a,b 后 join 剩余 → 一行 'c d'。
// 引擎：E488: Trailing characters: |j，四行原样。
// oracle：u19 = 'c d'（getline 剩一行）；u20（`:1s/a/B/|j`）同证。
#[test]
fn bar_runs_both_ex_commands() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "1,2d|j");
    assert_eq!(
        f.text(),
        "c d\n",
        "vim 9.1: `:1,2d|j` 删除 1-2 行后连接剩余行"
    );
}

// ---------------------------------------------------------------- 11（P3）
// 裸引号地址 `:'`（mark 名缺失）：vim 无错静默（o9：v:errmsg ''、光标不动；
// test_cmdline.vim Test_tick_mark_in_range 断言 `:'<CR>` 无失败）；引擎按
// E16: Invalid range 报错 + 响铃。
//
// oracle：cd /tmp/orc5; keys `:let v:errmsg=''|:'|writefile` → o9 = ''~1。
#[test]
fn bare_tick_address_is_silent() {
    let mut f = Fixture::new("x\ny\n");
    f.feed([":", "'", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        "vim 9.1: `:'` 无任何报文；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 12（P3）
// `:set ts=0`：vim 报 E487: Argument must be positive: ts=0 且拒绝（ts 保持
// 原值）；引擎静默接受 ts=0（后续列运算全部 .max(1) 兜底，TAB 宽 1 格）。
//
// 复现：`:set ts=6`、`:set ts=0`、`:set ts?`。
// vim 9.1：E487 + `  tabstop=6`（o10）。
// 引擎：无消息 + `  tabstop=0`。
#[test]
fn set_ts_zero_rejected_e487() {
    let mut f = Fixture::new("ab\n");
    ex(&mut f, "set ts=6");
    ex(&mut f, "set ts=0");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E487: Argument must be positive")),
        "vim 9.1: `:set ts=0` 报 E487；实际 statuses = {:?}",
        f.host.statuses
    );
    ex(&mut f, "set ts?");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=6")),
        "vim 9.1: 拒绝后 ts 保持 6；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 13（P3）
// `:set ts=2000000` 超过选项上限：vim 报 E474: Invalid argument: ts=2000000
// 且拒绝；引擎静默钳到 MAX_OPTION_VALUE(1000000)——悄悄改成另一个值。
//
// 复现：`:set ts=6`、`:set ts=2000000`、`:set ts?`。
// vim 9.1：E474 + tabstop=6（u14）。
// 引擎：无消息 + tabstop=1000000。
#[test]
fn set_ts_over_limit_rejected_e474() {
    let mut f = Fixture::new("ab\n");
    ex(&mut f, "set ts=6");
    ex(&mut f, "set ts=2000000");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E474: Invalid argument")),
        "vim 9.1: `:set ts=2000000` 报 E474；实际 statuses = {:?}",
        f.host.statuses
    );
    ex(&mut f, "set ts?");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=6")),
        "vim 9.1: 拒绝后 ts 保持 6；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 14（P3）
// `:s` 的 count 超过 2147483647：vim 报 E1510: Value too large 且不执行
// （test_substitute.vim Test_substitute_count 的 E1510 断言）；引擎按 usize
// 照单全收并真实替换。
//
// 复现：['a','b']，`:s/./b/2147483647`。
// vim 9.1：'a','b' 不动 + E1510（u9）。
// 引擎：第 1 行变 'b'，无消息。
#[test]
fn substitute_count_overflow_is_e1510() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "s/./b/2147483647");
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: count 越界拒绝执行");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E1510")),
        "vim 9.1: E1510: Value too large: 2147483647；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 15（P2）
// `:s` 不接受空格作分隔符：`:s a b` 在 vim 是 E146（Regular expressions
// can't be delimited by letters），什么都不做；引擎把空格当分隔符真实替换
// ——一次数据改写。
//
// 复现：'a b'，`:set noic`、`:s/!/Q/`（铺垫上次替换）、`:s a b`。
// vim 9.1：'a b' 不动 + E146（q4 同形：'aYb' + E146）。
// 引擎：pattern='a'、replacement='b' → 'b b'。
#[test]
fn space_delimited_substitute_is_e146() {
    let mut f = Fixture::new("a b\n");
    ex(&mut f, "set noic");
    ex(&mut f, "s/!/Q/");
    ex(&mut f, "s a b");
    assert_eq!(f.text(), "a b\n", "vim 9.1: 空格分隔符拒绝，文本不动");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E146")),
        "vim 9.1: E146: Regular expressions can't be delimited by letters；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 16（P3）
// `\` 也不能作 `:s` 分隔符：`:s\foo\bar\` vim 报 E10: \ should be followed
// by /, ? or &（do_sub 的 check_regexp_delim，q3）且不执行；引擎把 `\` 当
// 分隔符去解析 pattern（得到 foo\bar\ 的正则），落到别的错误/无操作。
//
// 复现：'foobar'，`:s\foo\bar\`。
// vim 9.1：E10 + 文本不动。引擎：非 E10 的其它消息。
#[test]
fn backslash_delimited_substitute_is_e10() {
    let mut f = Fixture::new("foobar\n");
    ex(&mut f, "s\\foo\\bar\\");
    assert_eq!(f.text(), "foobar\n", "vim 9.1: \\ 分隔符拒绝，文本不动");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E10:")),
        "vim 9.1: E10: \\ should be followed by /, ? or &；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 17（P3）
// 命令行 `<C-r>=` 打开表达式提示符求值并插入结果（c_CTRL-R_=）；求值失败
// 时报 E121/E15 且不插入任何文本。引擎把 `=` 当普通寄存器名贴陈旧值（未设
// 过则什么都不贴），随后键成了命令文本 → E492。
//
// 复现：`:`、`<C-r>`、`=`、`foo`、`<CR>`、`<CR>`。
// vim 9.1：E121: Undefined variable: foo，命令行复位为空，第二条 CR 空执行
//     （u11）。
// 引擎：`=` 无存量 → 无插入，'foo' 进命令行 → E492: Not an editor command: foo。
#[test]
fn cmdline_ctrl_r_equals_opens_expression_prompt() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "<C-r>", "=", "f", "o", "o", "<CR>", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E121") || s.contains("E15")),
        "vim 9.1: `<C-r>=foo` 求值失败报 E121/E15；实际 statuses = {:?}",
        f.host.statuses
    );
    // 断言意图修正（2026-10-09 复核）：初稿断言 statuses 里不得出现
    // "foo"——但 vim 自己的 E121 报文就引用变量名（E121: Undefined
    // variable: foo），该断言连 oracle 也过不了。意图是 `foo` 不得成为
    // 【命令文本】（旧引擎报 E492: Not an editor command: foo），改断
    // 无 E492。
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: 'foo' 不得成为命令行文本（无 E492）；实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 18（P3）
// 命令行 `<C-w>` 只删光标前一个【字母数字词】：'set ts=4' + `<C-w>` 得
// 'set ts='（'4' 被删、'=' 留下）；引擎删整个非空白段得 ''。
//
// 复现：`:`、'set ts=4'、`<C-w>`、'z'、`<CR>`。
// vim 9.1：命令行 'set ts=z' → E521: Number required after =: ts=z（u6）。
// 引擎：命令行 'z' → E492: Not an editor command: z。
#[test]
fn cmdline_ctrl_w_deletes_alnum_word_only() {
    let mut f = Fixture::new("ab\n");
    f.feed([
        ":", "s", "e", "t", " ", "t", "s", "=", "4", "<C-w>", "z", "<CR>",
    ]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E521") && s.contains("ts=z")),
        "vim 9.1: <C-w> 只删 '4'，命令行剩 'set ts=z' → E521；\
         实际 statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 19（P3）
// 超大 count 的 `:y`：vim 接受并钳到缓冲末尾（`:2y 77777777777777777777` =
// 行 2 到末行，静默；test_excmd.vim Test_address_line_overflow 同形）；引擎
// 的 parse_reg_count 用 usize 解析，20 位数字溢出 → E488 尾随垃圾，整条拒绝。
//
// 复现：['l0','l1','l2','l3','l4']，`:2y 77777777777777777777`。
// vim 9.1："= "l1\nl2\nl3\nl4\n"、无消息（u16）。
// 引擎：E488: Trailing characters: 77777777777777777777，寄存器不动。
#[test]
fn huge_yank_count_clamps_silently() {
    let mut f = Fixture::new("l0\nl1\nl2\nl3\nl4\n");
    ex(&mut f, "2y 77777777777777777777");
    let got = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("l1\nl2\nl3\nl4\n"),
        "vim 9.1: 超大 count 钳到缓冲末尾照常 yank"
    );
    assert!(
        f.host.statuses.is_empty(),
        "vim 9.1: 钳制执行无任何报文；实际 statuses = {:?}",
        f.host.statuses
    );
}

// （初稿第 20 项「normal `&` 未绑定」经引擎探针证伪：NormalCmd::RepeatSubstitute
// （state.rs:5335）已实现无旗标重放，探针变绿后撤下，证据移入已证伪清单。）

