//! audit5_c：寄存器 / 粘贴 / 可视 v,V / 块编辑 / marks 与跳转 / undo-redo /
//! 插入模式控制键 独立审计（2026-10-09，域 C 第四轮独立审计）。
//!
//! 每个 `#[test]` 断言的是 **vim 9.1 oracle 的期望结果**，在当前工作树上
//! 应当失败（修复后即回归）。oracle 通道：
//! * 默认 `vim -Nu NONE -N -i NONE -n -s <keys> <buf>`（typeahead，键文件
//!   以 `:call writefile(...)` + `:wq` 收尾；寄存器经 getreg/getregtype、
//!   光标经 getpos 采样）。
//! * 控制键写原始字节（\x12=C-r、\x16=C-v、\x19=C-y）。
//! 本轮亲历的采样坑（复验时注意）：
//! * `-es -S` 脚本里 `setline` 与后续 `normal` 并进**同一个 undo 块**
//!   （一步 u 退回空缓冲）——undo 粒度/undo 光标类结论必须走 typeahead
//!   或 PTY，本轮因此重采了整张 undo 矩阵（脚本通道的「undo 落 (1,1)」
//!   全是伪影）。
//! * `-s` 键文件里 `<C-y>` 这类记号**不是**控制键，是六个字面字符。
//! * `writefile` 的 List 参数嵌 List 报 E730——getline(1,"$") 不能混进
//!   字符串列表。
//!
//! ## 已证伪（oracle 已证引擎与 vim 一致，免下轮重查）
//! - 已证伪：undo/redo **光标**（typeahead 通道）——`3|x u` → (1,3)、
//!   `$p u` → (1,11)、`2G3|x u` → (2,3)、`dd u` → (2,1)：引擎的
//!   「组打开时快照」模型与 vim 的 uh_cursor 逐点对齐。
//! - 已证伪：`i<C-r>"` 粘 linewise 寄存器（noai 形状）——`:2y` 后
//!   `ggI<C-r>"xyz<Esc>` → ['def','xyzabc','def']，引擎按原文插入（含
//!   `\n` 拆行）一致。
//! - 已证伪：`Vp`（行可视 × linewise 寄存器多行/单行两形状 + charwise
//!   寄存器形状）——audit G5 的行替换模型与 vim 一致。
//! - 已证伪：块可视 `p` × charwise 寄存器（每行粘全量文本，oracle
//!   ['One','T','Tee','One']）——block_put_replace 的 `_` 臂一致。
//! - 已证伪：`Vj3>`（可视缩进 count = 逐轮 shift）一致。
//! - 已证伪：`R<C-r>-<C-r>-`（Replace 粘寄存器：覆盖后续接追加）——
//!   oracle ['foo','foofoo']，引擎 Replace 臂一致。
//! - 已证伪：`i<C-y>`/`<C-e>` 的 TAB 对齐与短行形状（字节列取邻行字符）
//!   一致。
//! - 已证伪：`qA` 追加录制（oracle @a='Ix'+'Ay<Esc>'）——引擎 seed 合并
//!   一致。
//! - 已证伪：`S` 写 `"1`+unnamed、`s`/`x` 只写 `"-`、块 `p` 交换、
//!   `2gp` charwise 光标、`v_u`/`v_U`/`v_~`、`dn` 的搜索 motion 本体。
//! - 已证伪：可视 `p` 后 `gv` 复选**粘贴文本**——引擎 PutReplace 的 gv
//!   链路已与 nv_put_opt 的 b_visual 重写一致（oracle `wviwpgvy` yank 回
//!   粘贴文本；初版探针因 setup 寄存器同值产生空转通过，换 `"a` 复验后
//!   引擎确实给出 'XY'。该候选原编号 C-14，已从本文件撤除）。
//! - 探针口径备注：可视粘贴类探针必须粘**不同**文本（把 'def' 粘回
//!   'def' 上文本不变，无法与 no-op 区分，会产生空转通过）。
//!
//! ## 未实现，非 bug（单列，不进探针）
//! - `U`（undo line 两级撤销）、`g-`/`g+`/`:earlier`/`:later`、
//!   `:undojoin`、`{count}undo` 绝对序号——引擎纯响铃，响亮缺失。
//! - `:k{mark}` 设 mark——E492（域 A 拼写表）。
//! - `` m{A-Z} `` 文件 mark 跨缓冲跳转——单缓冲模型无法表达。
//! - `:lockmarks`、`undotree()`/`getmarklist()` 族。

mod common;

use common::Fixture;
use vimcore::key::Key;
use vimcore::registers::RegisterKind;

// ---------------------------------------------------------------- C-1（P2）
// use_reg_one（`%` × 命名寄存器）：`%`、`(`/`)`、`` ` ``/`'`、`/`、`?`、
// `n`、`N`、`{`、`}` 做删除时，charwise 单行小删也要**额外**写编号环
// （vim src/ops.c op_delete：`motion_type == MLINE || line_count > 1 ||
// use_reg_one` → shift_delete_registers + 写 y_regs[1]；use_reg_one 由
// nv_percent/nv_brace/nv_findpar/nv_cursormark/normal_search 置位）。
// 引擎 store_delete 没有 use_reg_one 概念。
//
// 复现：`(12)3` 上 `"ad%`。
// vim 9.1：@1='(12)'、@- 未设。
// 引擎：只写 @a。
// oracle：printf '(12)3\n' > b_c1; keys b'\x1b"ad%:call writefile(
//   [string(getreg("1")), string(getreg("-"))], "out")\r:wq\r'
//   → ['(12)'] ['']。
// 来源: vim src/testdir/test_registers.vim Test_register_one（`%` 臂）。
#[test]
fn regone_named_delete_via_percent_writes_register_one() {
    let mut f = Fixture::new("(12)3\n");
    f.feed(["\"", "a", "d", "%"]);
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: \"ad% 同时写 \"a 与编号环 \"1（use_reg_one）"
    );
    assert_eq!(
        f.vim.registers.get('-'),
        None,
        "vim 9.1: 带命名寄存器的 use_reg_one 删除不写 \"-"
    );
}

// ---------------------------------------------------------------- C-2（P2）
// 同 C-1 的 unnamed 形状：`d%` 把小删同时写进 `"1` **和** `"-`
// （op_delete 的 shift 块与小删块先后都命中）。
//
// oracle：`(12)3` 上 `d%` → @1='(12)'、@-='(12)'、@"='(12)'。
#[test]
fn regone_unnamed_delete_via_percent_writes_both_one_and_small() {
    let mut f = Fixture::new("(12)3\n");
    f.feed(["d", "%"]);
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: d% 写编号环 \"1"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("(12)".to_owned()),
        "vim 9.1: d% 同时写小删寄存器 \"-（shift 块与小删块都命中）"
    );
}

// ---------------------------------------------------------------- C-3（P2）
// use_reg_one 的搜索 motion：`d/pat<CR>` 写 `"1`+`"-`（normal_search 置
// use_reg_one）。`dn`、`d?pat<CR>`、`d{`、`` d`x ``、`d'a` 同族。
//
// oracle：`abc abc` 上 `0d/abc<CR>` → @1='abc '、@-='abc '、缓冲 'abc'。
#[test]
fn regone_search_delete_writes_register_one() {
    let mut f = Fixture::new("abc abc\n");
    f.feed(["0", "d", "/"]);
    f.feed(["a", "b", "c", "<CR>"]);
    assert_eq!(f.text(), "abc\n", "形状自检：d/pat 删到匹配前");
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("abc ".to_owned()),
        "vim 9.1: d/pat 经 use_reg_one 写 \"1"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("abc ".to_owned()),
        "vim 9.1: d/pat 同时写 \"-"
    );
}

// ---------------------------------------------------------------- C-4（P2）
// use_reg_one 的大括号族（单行 charwise 形状）：`d}` 从词中删到段尾仍
// charwise（exclusive 列 1 提升只在端点行首触发），但 use_reg_one 使它
// 写 `"1`+`"-`。
//
// oracle：`one two\n\nzzz` 光标 (1,4)（'two' 的 t）上 `d}` →
//     @1='two'、@-='two'、缓冲 ['one ', '', 'zzz']。
#[test]
fn regone_brace_delete_writes_register_one() {
    let mut f = Fixture::at("one two\n\nzzz\n", 0, 4);
    f.feed(["d", "}"]);
    assert_eq!(f.text(), "one \n\nzzz\n", "形状自检：d}} 只删到段尾");
    assert_eq!(
        f.vim.registers.get('1').map(|r| r.text.clone()),
        Some("two".to_owned()),
        "vim 9.1: 单行 d}} 经 use_reg_one 写 \"1"
    );
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("two".to_owned()),
        "vim 9.1: 单行 d}} 同时写 \"-"
    );
}

// ---------------------------------------------------------------- C-5（P2）
// 粘贴落账 `'[`/`']`（do_put 顶部 `b_op_start = b_op_end = cursor`，逐
// 分支改写）：`` `] `` 跳到最后粘贴文本的末字符。引擎 put_ex / 块可视
// PutReplace 都不写这对 mark——`` `] `` 停在 yank 时代的旧账上。
//
// 复现：`abc\ndef` 上 `yy` `j` `p`（粘出第三行 'abc'）。
// vim 9.1：'[=(3,1)、']=(3,3)。
// 引擎：'[='] 停在 yank 旧账（byte 0..2）。
// oracle：printf 'abc\ndef\n' > b_c5; keys b'\x1byyjp:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,3,1,0]] [[0,3,3,0]]。
#[test]
fn put_sets_change_marks() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed(["y", "y", "j", "p"]);
    assert_eq!(f.text(), "abc\ndef\nabc\n", "形状自检");
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        10,
        "vim 9.1: 粘贴后 `] 落最后粘贴文本的末字符（(3,3) = byte 10）"
    );
    f.feed(["`", "["]);
    assert_eq!(
        f.cursor(),
        8,
        "vim 9.1: 粘贴后 `[ 落粘贴文本首字符（(3,1) = byte 8）"
    );
}

// ---------------------------------------------------------------- C-6（P2）
// undo/redo 落账 `'[`/`']`（u_undoredo 把 b_op_start/end 设为被撤销改动的
// 首末**行**、列归 0）。test_undo.vim Test_undo_mark 同源。引擎
// history_step 不碰 mark——undo 后 `]` 还停在删除点。
//
// 复现：`abcdef` 上 `3|x` `u`（再 `<C-r>`）。
// vim 9.1：undo 后 '[=']=(1,1)；redo 后同样 (1,1)。
// oracle：printf 'abcdef\n' > b_c6; keys b'\x1b3|xu:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r\x12:call
//   writefile([string(getpos("'[")), string(getpos("']"))], "out2")\r:wq\r'
//   → out/out2 均为 [[0,1,1,0]]×2。
#[test]
fn undo_and_redo_set_change_marks() {
    let mut f = Fixture::new("abcdef\n");
    f.feed(["3", "|", "x"]);
    f.feed(["u"]);
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        0,
        "vim 9.1: undo 后 '] 落被撤销改动的首行行首（(1,1)）"
    );
    f.feed_raw(Key::ctrl_char('r')); // <C-r> redo
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        0,
        "vim 9.1: redo 后 '] 同样落首行行首（oracle um4）"
    );
}

// ---------------------------------------------------------------- C-7（P2）
// 插入会话落账 `'[`/`']`：'[ = 首个插入字符、'] = 插入文本之后一格
// （ins_esc 落账；`:h '[`）。o/O/a/A/R 同一出口（o 形状见 C-17）。引擎
// exit_insert 只写 `^`——纯插入后 `` `[ ``/`` `] `` 未设（跳转响铃）。
//
// 复现：`one two` 上 `0lliXYZ<Esc>`。
// vim 9.1：'[=(1,3)（'X'）、']=(1,6)（'Z' 后一格）。
// oracle：printf 'one two\n' > b_c7; keys b'\x1b0lliXYZ\x1b:call
//   writefile([string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,1,3,0]][[0,1,6,0]]。
#[test]
fn insert_session_sets_change_marks() {
    let mut f = Fixture::new("one two\n");
    f.feed(["0", "l", "l", "i"]);
    f.type_text("XYZ");
    f.feed(["<Esc>"]);
    let bells_before = f.host.bells;
    f.feed(["`", "["]);
    assert_eq!(f.host.bells, bells_before, "`` `[ `` 不应响铃（mark 已设）");
    assert_eq!(
        f.cursor(),
        2,
        "vim 9.1: 插入后 `[ 落首个插入字符 'X'（byte 2）"
    );
    f.feed(["`", "]"]);
    // 断言修正（2026-10-09 typeahead 复验）：`] 落在 mark 本身（byte 5 =
    // ins_esc 的 b_op_end 一格后值），nv_brackets 并无 dec——初稿「钳回
    // 'Z'」是对 `:h ']
    // 措辞的推断，oracle getpos(".") = [0,1,6,0]。
    assert_eq!(f.cursor(), 5, "oracle [0,1,6,0]：`] 落 mark 本身（byte 5）");
}

// ---------------------------------------------------------------- C-8（P2）
// `gp` 行级光标「新文本之后」的真义：落在粘贴块**之后一行**（do_put MLINE
// 臂 `PUT_CURSEND: w_cursor.lnum = lnum + 1`），不是最后粘贴行。audit C5
// 的旧结论「gp 落最后粘贴行」是缓冲末端探针的钳制巧合（末端时 lnum+1 被
// 钳回块尾，两模型重合）；块在缓冲中部时差异暴露。
//
// 复现：`a1\na2\na3` 上 `yy` `2gg1gp`（及 `2gp`）。
// vim 9.1：1gp 光标 (4,1)（粘贴块后一行）；2gp → (5,1)。
// 引擎：落最后粘贴行（(3,1)/(4,1)）。
// oracle：printf 'a1\na2\na3\n' > b_c8; keys b'\x1byy2gg1gp:call
//   writefile([string(getpos("."))], "out")\r:wq\r' → [0,4,1,0]；
//   2gp → [0,5,1,0]。
#[test]
fn gp_linewise_cursor_lands_after_the_pasted_block() {
    let mut f = Fixture::new("a1\na2\na3\n");
    f.feed(["y", "y", "2", "g", "g", "1", "g", "p"]);
    assert_eq!(
        f.line(),
        3,
        "vim 9.1: 1gp 行级光标 = 粘贴块之后一行（line 4，0 基 3）"
    );
    let mut f2 = Fixture::new("a1\na2\na3\n");
    f2.feed(["y", "y", "2", "g", "g", "2", "g", "p"]);
    assert_eq!(
        f2.line(),
        4,
        "vim 9.1: 2gp 行级光标 = 两份粘贴块之后一行（line 5，0 基 4）"
    );
}

// ---------------------------------------------------------------- C-9（P2）
// 多行 charwise 寄存器的 p/P 光标：落**首个粘贴字符**（do_put MCHAR 多行
// 臂把光标放回 b_op_start；单行 charwise 才是「末字符」）。引擎 put_ex
// 一律停在 prev_grapheme(at+len) = 末字符。
//
// 造寄存器（引擎夹具无 :let）：'b/c/hello'（行 b、c、hello）上 `gg v j y`
// 得 charwise 'b\nc'（oracle：getregtype='v'、getreg strtrans='b^@c'）。
// vim 9.1：`G$p` → 缓冲 ['b','c','hellob','c']，光标 (3,6)=byte 9（'b'，
//     首粘贴字符）；`G0P` → ['b','c','b','chello']，光标 (3,1)=byte 4。
// 引擎：光标落末字符（byte 11 / byte 6）。
// oracle：printf 'b\nc\nhello\n' > b_c9; keys b'\x1bggvjyGG$p:call
//   writefile([string(getpos(".")) . string(getline(1,"$"))], "out")\r:wq\r'
//   → [0,3,6,0]['b','c','hellob','c']。
#[test]
fn multiline_charwise_put_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // charwise 'b\nc'
    assert_eq!(
        f.vim.registers.get('"').map(|r| (r.text.clone(), r.kind)),
        Some(("b\nc".to_owned(), RegisterKind::Charwise)),
        "形状自检：跨行可视 yank 得 charwise 多行寄存器"
    );
    f.feed(["G", "$", "p"]);
    assert_eq!(
        f.text(),
        "b\nc\nhellob\nc\n",
        "形状自检：p 粘出多行"
    );
    assert_eq!(
        f.cursor(),
        9,
        "vim 9.1: 多行 charwise p 光标 = 首个粘贴字符（'hellob' 的 'b'）"
    );
}

// ---------------------------------------------------------------- C-10（P2）
// 同 C-9 的 `P` 形状：oracle `G0P` → 缓冲 ['b','c','b','chello']，光标
// (3,1) = byte 4 = 首粘贴字符。
#[test]
fn multiline_charwise_put_before_cursor_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // charwise 'b\nc'
    f.feed(["G", "0", "P"]);
    assert_eq!(
        f.text(),
        "b\nc\nb\nchello\n",
        "形状自检：P 在行首前粘出（粘贴文本自身带换行再拆两行）"
    );
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: 多行 charwise P 光标 = 首个粘贴字符（(3,1) = byte 4）"
    );
}

// ---------------------------------------------------------------- C-11（P1）
// v/V（非块）模式的 `I`：在**选区起点**行插入一次并退出可视（非块可视的
// I/A 退化为普通命令，落在选区角上；逐行重复是块模式专属）。
// 引擎的 I/A 只在 Block 分支拦截，v/V 下落 Visual trie Miss → 响铃且留在
// 可视模式，后续文本键扩选、字面文本丢失。
//
// 复现：['aa','bb','cc'] 上 `VjI-<Esc>`。
// vim 9.1：['-aa','bb','cc']。
// oracle：printf 'aa\nbb\ncc\n' > b_c11; keys b'\x1bVjI-\x1b:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['-aa','bb','cc']。
#[test]
fn visual_line_i_inserts_at_selection_start_only() {
    let mut f = Fixture::new("aa\nbb\ncc\n");
    f.feed(["V", "j", "I"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "-aa\nbb\ncc\n",
        "vim 9.1: V+I 在选区起点行插入一次并退出可视"
    );
}

// ---------------------------------------------------------------- C-12（P1）
// v/V（非块）模式的 `A`：在**选区终点**行行尾追加一次并退出可视。
//
// 复现：['aa','bb'] 上 `VjA-<Esc>`。
// vim 9.1：['aa','-bb']。
// oracle：printf 'aa\nbb\n' > b_c12; keys b'\x1bVjA-\x1b:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['aa','-bb']。
#[test]
fn visual_line_a_appends_at_selection_end_only() {
    let mut f = Fixture::new("aa\nbb\n");
    f.feed(["V", "j", "A"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "aa\n-bb\n",
        "vim 9.1: V+A 在选区终点行行尾追加一次并退出可视"
    );
}

// ---------------------------------------------------------------- C-13（P2）
// 可视 `P` 不交换寄存器（nv_put_opt：`keep_registers = (cmdchar == 'P')`，
// 选区删除走 blackhole）：被替换文本**丢弃**，粘贴源寄存器原样保留。
// `p` 才交换。引擎 PutReplace 对 p/P 同路（delete_span(None)）——P 侧被
// 替换文本进了 unnamed 和 `"-`。
//
// 复现：`abc def` 上 `wvey` yank 'def'（unnamed='def'），`0` `viwP`——
// 把 'def' 粘到 'abc' 上（探针口径注意：必须粘**不同**文本；把 'def' 粘到
// 'def' 上文本不变，无法与 no-op 区分——初稿在这里空转）。
// vim 9.1：缓冲 'def def'，unnamed 仍 'def'，`"-` 未设。
// 引擎：unnamed='abc'、`"-`='abc'。
// oracle：printf 'abc def\n' > b_c13; keys b'\x1bwvey0viwP:call
//   writefile([string(getreg("\\"")), string(getreg("-"))], "out")\r:wq\r'
//   → 'def' ''。
#[test]
fn visual_capital_p_keeps_the_put_register() {
    let mut f = Fixture::new("abc def\n");
    f.feed(["w", "v", "e", "y"]); // unnamed = 'def'
    f.feed(["0", "v", "i", "w", "P"]); // 'def' 粘到 'abc' 上
    assert_eq!(f.text(), "def def\n", "形状自检：P 粘出 yank 的 'def'");
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("def".to_owned()),
        "vim 9.1: 可视 P 不交换——unnamed 保留粘贴源"
    );
    assert_eq!(
        f.vim.registers.get('-'),
        None,
        "vim 9.1: 可视 P 的被替换文本进 blackhole，\"- 不设"
    );
}

// ---------------------------------------------------------------- C-14（P2）
// 块可视 `P` 同样不写寄存器（nv_put_opt 的 keep_registers 让选区删除走
// `oap->regname = '_'`）：被删块丢弃，unnamed 保留粘贴源。引擎
// block_put_replace 对 p/P 同路（store_block_rows(None)）。
//
// 复现：`XY\nabcdef\nghijkl` 上 `gg yiw`，`j 0 <C-v> j l P`。
// vim 9.1：['XY','XYcdef','XYijkl']，unnamed 仍 'XY'。
// 引擎：unnamed 变被删块 'ab'+'gh'。
// oracle：printf 'XY\nabcdef\nghijkl\n' > b_c15; keys b'\x1bggyiwj0\x16jlP:
//   call writefile([string(getreg("\\"")) . string(getline(1,"$"))],
//   "out")\r:wq\r' → 'XY' ['XY','XYcdef','XYijkl']。
#[test]
fn block_visual_capital_p_keeps_registers() {
    let mut f = Fixture::new("XY\nabcdef\nghijkl\n");
    f.feed(["g", "g", "y", "i", "w"]); // unnamed = 'XY'
    f.feed(["j", "0"]); // 到第二行行首（块从 'abcdef' 起）
    f.feed_raw(Key::ctrl_char('v')); // <C-v>
    f.feed(["j", "l", "P"]);
    assert_eq!(
        f.text(),
        "XY\nXYcdef\nXYijkl\n",
        "形状自检：块 P 粘出 'XY' 两行（首行不在块内保持原样）"
    );
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("XY".to_owned()),
        "vim 9.1: 块可视 P 不交换——unnamed 保留粘贴源"
    );
}

// ---------------------------------------------------------------- C-15（P2）
// 新改动使 redo 分支作废（vim 的 undo 是树：undo 后的新 change 砍掉 redo
// 分支，`<C-r>` 只响铃）。harness 的 HostView 与引擎契约里没有任何
// 「新编辑作废 redo 栈」的路径——`u` 后编辑再 `<C-r>` 会把已撤销状态
// 复活。
//
// 形状取 `A!` 会话（typeahead 通道对**相邻裸改动**会把 xx 合成一个 undo
// 块——u 之后才有 sync 点；插入会话闭合后块是独立的，此形状忠实）：
// `one\n` 上 `A!<Esc>`（'one!'）、`u`（'one'）、`0x`（'ne'）、`<C-r>`。
// vim 9.1：响铃，缓冲仍 'ne\n'；引擎+harness：复活 'one!'。
//
// oracle：printf 'one\n' > b_c16; keys b'\x1bA!\x1bu0x\x12:call
//   writefile([string(getline(1,"$"))], "out")\r:wq\r' → ['ne']。
#[test]
fn redo_is_invalidated_by_a_new_change() {
    let mut f = Fixture::new("one\n");
    f.feed(["A"]);
    f.type_text("!");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one!\n", "形状自检");
    f.feed(["u"]);
    assert_eq!(f.text(), "one\n", "形状自检：u 回到 'one'");
    f.feed(["0", "x"]);
    assert_eq!(f.text(), "ne\n", "形状自检：新改动 'ne'");
    let bells_before = f.host.bells;
    f.feed_raw(Key::ctrl_char('r')); // <C-r>
    assert_eq!(
        f.text(),
        "ne\n",
        "vim 9.1: undo 后的新改动砍掉 redo 分支，<C-r> 无操作"
    );
    assert!(
        f.host.bells > bells_before,
        "vim 9.1: 无可重做时 <C-r> 响铃"
    );
}

// ---------------------------------------------------------------- C-16（P2）
// `c` 后 `']` = 插入文本之后一格（b_op_end 在 ins_esc 随打字增长落账）。
// 引擎 apply() 在删除前把 '] 钉在被删 span 末字节——插入后只被漏斗平移，
// 停在插入文本**内部**（差一字节）。
//
// 复现：`ab cd` 上 `cwXYZ<Esc>`。
// vim 9.1：']=(1,4)（'Z' 之后一格，byte 3）——`` `] `` 钳回 'Z'（byte 2）。
// 引擎：'] 落 byte 4（' '）。
// oracle：printf 'ab cd\n' > b_c17; keys b'\x1bcwXYZ\x1b:call writefile(
//   [string(getpos("']"))], "out")\r:wq\r' → [0,1,4,0]。
#[test]
fn change_marks_end_tracks_the_inserted_text() {
    let mut f = Fixture::new("ab cd\n");
    f.feed(["c", "w"]);
    f.type_text("XYZ");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "XYZ cd\n", "形状自检");
    f.feed(["`", "]"]);
    // 断言修正（同 C-7 的复验）：`] 落 mark 本身 byte 3（'Z' 后一格），
    // 无回退
    assert_eq!(f.cursor(), 3, "oracle：`] 落 '] 本身（byte 3）");
}

// ---------------------------------------------------------------- C-17（P2）
// `o`（开行插入）同样落 `'[`/`']`：'[ = 新行首、'] = 末插入字符之后一格。
// 引擎同 C-7 不设。
//
// oracle：printf 'x\n' > b_c18; keys b'\x1bGoaaaa\x1b:call writefile(
//   [string(getpos("'[")), string(getpos("']"))], "out")\r:wq\r'
//   → [[0,2,1,0]][[0,2,5,0]]。
#[test]
fn open_line_insert_sets_change_marks() {
    let mut f = Fixture::new("x\n");
    f.feed(["G", "o"]);
    f.type_text("aaaa");
    f.feed(["<Esc>"]);
    f.feed(["`", "["]);
    assert_eq!(f.cursor(), 2, "vim 9.1: o 后 '[ = 新行首（(2,1) = byte 2）");
    let bells_before = f.host.bells;
    f.feed(["`", "]"]);
    assert_eq!(
        f.host.bells,
        bells_before,
        "vim 9.1: ']=(2,5) 已设（引擎未设会响铃）"
    );
    // '] byte 6 = 行尾之后一格，跳转钳回末字符 'a'（byte 5）
    assert_eq!(f.cursor(), 5, "vim 9.1: `] 钳回末插入字符 'a'");
}

// ---------------------------------------------------------------- C-18（P2）
// `"=<CR>`（空表达式）重用上一次表达式（get_expr_register 返回缓存的
// 表达式行）：连续粘贴同一求值结果。引擎的 `=` 提示符空输入落空结果，
// 第二次粘贴丢字。
//
// 复现：`zz` 上 `"=6*7<CR>p` 再 `"=<CR>P`。
// vim 9.1：'z4422z'。
// oracle：printf 'zz\n' > b_c19; keys b'\x1b"=6*7\rp"=\rP:call writefile(
//   [string(getline(1,"$"))], "out")\r:wq\r' → ['z4422z']。
#[test]
fn expression_register_empty_prompt_reuses_last_expression() {
    let mut f = Fixture::new("zz\n");
    f.feed(["\"", "="]);
    f.type_text("6*7");
    f.feed(["<CR>"]);
    f.feed(["p"]);
    assert_eq!(f.text(), "z42z\n", "形状自检：首次求值粘贴 '42'");
    f.feed(["\"", "="]);
    f.type_text(""); // 空输入 = 重用上次表达式
    f.feed(["<CR>"]);
    f.feed(["P"]);
    assert_eq!(
        f.text(),
        "z4422z\n",
        "vim 9.1: \"=<CR> 重用上次表达式，P 粘出 '42'"
    );
}

// ---------------------------------------------------------------- C-19（P2）
// undo 恢复可视选区（u_undoredo 交换 uh_visual/当前 b_visual）：`dd` 把
// '</'> 折叠之后 undo，vim 恢复的是**改动前**的选区对，gv 复选复原文本上
// 的原选区。引擎的 last_visual 只被漏斗单向平移，undo 不还原。
//
// 复现：`abcd` 上 `vlly`、`dd`、`u`、`gvy`。
// vim 9.1：@"='abcd'。
// oracle：printf 'abcd\n' > b_c20; keys b'\x1bvllyddugvy:call writefile(
//   [string(getreg("\\""))], "out")\r:wq\r' → 'abcd'。
#[test]
fn undo_restores_the_visual_selection_for_gv() {
    let mut f = Fixture::new("abcd\n");
    f.feed(["v", "l", "l", "y"]);
    f.feed(["d", "d"]);
    f.feed(["u"]);
    f.feed(["g", "v", "y"]);
    // 期望修正（2026-10-09 typeahead 复验）：vll 在 "abcd" 上选的是
    // "abc"（3 字符），gv 复选 yank 也是 'abc'——初稿的 'abcd' 是笔误
    // （报告文本与 writefile 输出不符）。引擎一致，本探针转回归钉。
    assert_eq!(
        f.vim.registers.get('"').map(|r| r.text.clone()),
        Some("abc".to_owned()),
        "oracle 'abc'：dd+u 之后 gv 复选复原文本上的原选区"
    );
}
