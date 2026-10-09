//! audit6_c：寄存器（编号环/具名/特殊）/ 可视算子 p/P/J / undo-redo /
//! 插入模式控制键 / 宏录制与重放 —— 第五轮独立审计（域 C，智能体 C）。
//!
//! 每个 `#[test]` 断言 **vim 9.1 oracle 的期望结果**；在当前工作树上失败
//! 的项即候选 bug（oracle 证据通道写在各例注释里，待主会话复验）。
//! 跑法：`cargo test --test audit6_c -- --ignored --nocapture`。
//!
//! oracle 通道：
//! * typeahead `vim -Nu NONE -N -i NONE -n -s <keys> <buf>`（控制键写原始
//!   字节 \x12=C-r、\x16=C-v、\x19=C-y；寄存器经 getreg/getregtype 采样，
//!   缓冲以 :w 回读为准）。
//! * 唯一文件名（swap 残留吞按键）；引擎分歧默认值在 oracle 侧先 :set 钉平
//!   （本文件涉及 et/sw，探针内同步 :set pin）。
//!
//! ## 已证伪（读码 + 探针与 oracle 一致，免下轮重查）
//! - 连续小删在 `"-` 上是**覆盖**不是累积（vim src/ops.c op_delete：小删走
//!   `get_yank_register('-')` + op_yank，y_append=FALSE）——引擎 store_delete
//!   覆盖一致。
//! - `i_CTRL-V_digit` 的五种进制形态（十进制/o O/x X/u/U）引擎与
//!   insert.txt 表格一致；`0b` 二进制**不存在于 vim**（get_literal 无此分支）。
//! - `q{0-9}` 录制、`qaq` 清寄存器、`@:` 认 count、`@@` 基本回放、宏对
//!   纯铃失败继续（note_bell 不中止）——引擎与 vim 一致。
//! - undo/redo 到边界的响铃（undo.c `beep_flush()`）与消息形状一致。
//! - `":` 寄存器存「冒号后整条命令行」（含范围、不含冒号；
//!   ex_getln.c new_last_cmdline = ccline.cmdbuff）。
//! - `0<C-d>` 的「0 必须是本行刚打的字符」判定（edit.c 无此约束的形状
//!   见 E-C-11 的正例）。

mod common;

use common::Fixture;
use vimcore::key::Key;
use vimcore::registers::{RegisterKind, UNNAMED};

// ---------------------------------------------------------------- E-C-1（P2）
// i_CTRL-R 的字面形（`C-r C-o{reg}`、`C-r C-r{reg}`、`C-r C-p{reg}`）缺失：
// vim 的 `C-r` 特殊形让下一个键做寄存器名并**按字面**插入
// （edit.c ins_ctrl_r：`C-o`/`C-r` 设 literally 位，`C-p` 再加 indent 位）。
// 引擎 insert_key 的 C-r pending 只认 printable_char()——ctrl 修饰的键被
// 静默吃掉（pending 清空、无插入），随后的寄存器名字符成了普通打字。
//
// 复现："foo" 上 `"ayiw`（a='foo'），`i<C-r><C-o>a<Esc>`。
// vim 9.1：'foofoo'；引擎：'foo'（a 被丢弃）。
// oracle：printf 'foo\n' > b_ec1; keys
//   b'\x1b"ayiw\x16\x12\x0fa\x1b:call writefile([getline(1,"$")],"out")\r:wq\r'
//   （\x16\x12\x0f = C-v 转义？不——-s 键文件里 \x12=C-r、\x0f=C-o 直接
//   原始字节）→ ['foofoo']。
#[test]
#[ignore]
fn insert_ctrl_r_literal_forms_paste_the_register() {
    let mut f = Fixture::new("foo\n");
    f.feed(["\"", "a", "y", "i", "w"]); // a = 'foo'
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed_raw(Key::ctrl_char('o')); // literal form
    f.feed(["a"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "foofoo\n",
        "vim 9.1: i<C-r><C-o>a 按字面插入寄存器 a（引擎把 <C-o> 吃掉、'a' 落空）"
    );

    let mut f2 = Fixture::new("foo\n");
    f2.feed(["\"", "a", "y", "i", "w"]);
    f2.feed(["i"]);
    f2.feed_raw(Key::ctrl_char('r'));
    f2.feed_raw(Key::ctrl_char('r')); // C-r C-r{reg} 同族
    f2.feed(["a"]);
    f2.feed(["<Esc>"]);
    assert_eq!(
        f2.text(),
        "foofoo\n",
        "vim 9.1: i<C-r><C-r>a 同为字面形（引擎同样吃键）"
    );
}

// ---------------------------------------------------------------- E-C-2（P2）
// 编号环移位必须**无条件**整槽平移：vim shift_delete_registers 是
// `y_regs[n] = y_regs[n-1]`（空寄存器照样平移过去），`q{0-9}` 录制的
// 编号寄存器在下一次大删除时会被**清空**。引擎的 shift_numbered_ring
// 只在 from 槽存在时复制——HashMap 里「不存在的槽」不被平移，录制内容
// 穿环幸存，`"2p` 粘出幽灵宏文本。
//
// 复现："hello\nworld\n" 上 `q2xq`（录制并执行 x，"2='x'），`dd`，
// 再 `"2p`。
// vim 9.1：dd 的环移把 "2 清空 → `"2p` 粘不出东西，缓冲 'world'。
// 引擎："2 幸存 → 'wxorld'。
// oracle：printf 'hello\nworld\n' > b_ec2; keys
//   b'\x1bq2xqdd"2p:call writefile([getline(1,"$")],"out")\r:wq\r'
//   → ['world']。
#[test]
#[ignore]
fn ring_shift_clears_recorded_numbered_registers() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed(["q", "2", "x", "q"]); // "2 = 'x'（录制即执行）
    assert_eq!(
        f.vim.registers.get('2').map(|r| r.text.clone()),
        Some("x".to_owned()),
        "形状自检：q2 落编号寄存器"
    );
    f.feed(["d", "d"]); // 大删除 → 环移
    f.feed(["\"", "2", "p"]);
    assert_eq!(
        f.text(),
        "world\n",
        "vim 9.1: dd 的环移用空 \"1 覆写 \"2——录制的宏内容穿环幸存是引擎缺陷"
    );
}

// ---------------------------------------------------------------- E-C-3（P2）
// `"A` 追加的**行模型**：vim 的追加是把新文本按「寄存器行」接到旧行序列
// 尾（register.c op_yank 的 append 块），只在合并类型为 MCHAR 时把旧块
// 末行与新块首行**字符串拼接**（cpo 无 'j'）。两条推论：
// (a) 旧 charwise + 新 blockwise → 合并 charwise，行为 ['fooa','c']；
// (b) 旧 blockwise + 新 charwise → 合并仍 blockwise，行为 ['a','c','zz']。
// 引擎 append_to_named 是「整段文本拼接」（linewise 除外）：(a) kind 错成
// Blockwise（粘贴走矩形模型）、(b) 行被并成 'a\nczz'（少一行）。
//
// oracle (a)：printf 'foo\nab\ncd\n' > b_ec3a; keys
//   b'\x1b"ayiw\x16jy"Ay:call writefile([getregtype("a"),strtrans(getreg("a"))],"out")\r:wq\r'
//   → 'v' 'fooa^@c'。
// oracle (b)：printf 'ab\ncd\nzz\n' > b_ec3b; keys
//   b'\x1b"a\x16jyyiw"Ay:call writefile([getregtype("a"),strtrans(getreg("a"))],"out")\r:wq\r'
//   → "\x161"（CTRL-V 宽 1） 'a^@c^@zz'。
#[test]
#[ignore]
fn uppercase_append_charwise_then_blockwise_stays_charwise() {
    let mut f = Fixture::new("foo\nab\ncd\n");
    f.feed(["\"", "a", "y", "i", "w"]); // a = 'foo' (charwise)
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["j", "y"]); // block 2×1: 'a'/'c'
    f.feed(["\"", "A", "y"]); // 追加块
    let r = f.vim.registers.get('a');
    assert_eq!(
        r.as_ref().map(|r| r.text.clone()),
        Some("fooa\nc".to_owned()),
        "形状自检（文本面）"
    );
    assert_eq!(
        r.map(|r| r.kind),
        Some(RegisterKind::Charwise),
        "vim 9.1: 旧 charwise + 新 blockwise → 合并仍 charwise（getregtype='v'），粘贴不走矩形"
    );
}

#[test]
#[ignore]
fn uppercase_append_blockwise_then_charwise_keeps_rows() {
    let mut f = Fixture::new("ab\ncd\nzz\n");
    f.feed(["\"", "a"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["j", "y"]); // a = block 2×1: 'a'/'c'
    f.feed(["y", "i", "w"]); // yank 'zz' 进 unnamed
    f.feed(["\"", "A"]); // 追加 unnamed（charwise 'zz'）
    let r = f.vim.registers.get('a');
    assert_eq!(
        r.map(|r| (r.text.clone(), r.kind)),
        Some(("a\nc\nzz".to_owned(), RegisterKind::Blockwise)),
        "vim 9.1: 旧 blockwise + 新 charwise → 三行矩形 ['a','c','zz']（引擎并成 'a\\nczz' 两行）"
    );
}

// ---------------------------------------------------------------- E-C-4（P2）
// 可视 `p` × 多行 charwise 寄存器的光标：do_put 的 MCHAR 多行臂
// （register.c「put cursor on first inserted character」
// `curwin->w_cursor = new_cursor`）落在**首个粘贴字符**——与普通 p 的
// audit5 C-9 同一规则，但可视 PutReplace 臂没同步（停在末字符）。
//
// 复现："b\nc\nhello\n" 上 `ggvjy`（charwise 'b\nc'），`Gviwp`。
// vim 9.1：缓冲 ['b','c','b','c']，光标 (3,1) = byte 4。
// 引擎：byte 6（末字符）。
// oracle：printf 'b\nc\nhello\n' > b_ec4; keys
//   b'\x1bggvjyGviwp:call writefile([string(getpos("."))],"out")\r:wq\r'
//   → [0,3,1,0]。
#[test]
#[ignore]
fn visual_put_multiline_charwise_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]); // unnamed = 'b\nc'
    f.feed(["G", "v", "i", "w", "p"]);
    assert_eq!(f.text(), "b\nc\nb\nc\n", "形状自检");
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: 可视 p 多行 charwise 光标 = 首个粘贴字符（(3,1)=byte 4）"
    );
}

// ---------------------------------------------------------------- E-C-5（P2)
// 同 E-C-4 的 `P`（keep_registers）形状：nv_put_opt 黑洞删除选区后仍走
// 同一个 do_put 多行臂 → 光标落首个粘贴字符。引擎 PutReplaceKeep 的
// charwise 臂同样停在末字符。
//
// oracle：printf 'b\nc\nhello\n' > b_ec5; keys
//   b'\x1bggvjyGviwP:call writefile([string(getpos("."))],"out")\r:wq\r'
//   → [0,3,1,0]。
#[test]
#[ignore]
fn visual_put_keep_multiline_charwise_cursor_lands_on_first_char() {
    let mut f = Fixture::new("b\nc\nhello\n");
    f.feed(["g", "g", "v", "j", "y"]);
    f.feed(["G", "v", "i", "w", "P"]);
    assert_eq!(f.text(), "b\nc\nb\nc\n", "形状自检");
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: 可视 P 多行 charwise 光标 = 首个粘贴字符"
    );
}

// ---------------------------------------------------------------- E-C-6（P2）
// 可视 `J` 无视前缀 count：v_J 的行数取**选区行数**（do_pending_operator
// 的 `oap->line_count = end.lnum - start.lnum + 1`；OP_JOIN 臂只做
// `< 2 → 2` 的下限钳制，cap->count0 不参与）。引擎 VisualCmd::Join 取
// `max(选区行数, count)` → `Vj3J` 多接一行。
//
// 复现："a\nb\nc\nd\n" 上 `Vj3J`。
// vim 9.1：'ab\nc\nd'（只接选中的两行）；引擎：'abc\nd'。
// oracle：printf 'a\nb\nc\nd\n' > b_ec6; keys
//   b'\x1bVj3J:call writefile([getline(1,"$")],"out")\r:wq\r'
//   → ['ab','c','d']。
#[test]
#[ignore]
fn visual_join_ignores_the_typed_count() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["V", "j", "3", "J"]);
    assert_eq!(
        f.text(),
        "ab\nc\nd\n",
        "vim 9.1: 可视 J 的行数 = 选区行数，count 被无视"
    );
}

// ---------------------------------------------------------------- E-C-7（P2）
// 块寄存器 `gp` 的 PUT_CURSEND 光标：do_put MBLOCK 臂里
// `PUT_CURSEND → curwin->w_cursor = b_op_end; col++`——落在**最后一行
// 粘贴内容之后一格**。引擎 put_blockwise 无 leave_after 参数（put_ex 直接
// return 进块臂），光标停在首粘贴字符。
//
// 复现："ab\ncd\n" 上 `<C-v>y`（1×1 块 'a'），`2G0gp`。
// vim 9.1：缓冲 'ab\ncad'，光标 (2,3)（粘出的 'a' 之后一格 = 'd'）。
// 引擎：光标在 'a' 上（(2,2)）。
// oracle：printf 'ab\ncd\n' > b_ec7; keys
//   b'\x1b\x16y2G0gp:call writefile([string(getpos(".")).string(getline(1,"$"))],"out")\r:wq\r'
//   → [0,2,3,0] ['ab','cad']。
#[test]
#[ignore]
fn gp_blockwise_cursor_lands_after_the_pasted_block() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["y"]); // 1×1 块 'a'
    f.feed(["2", "G", "0", "g", "p"]);
    assert_eq!(f.text(), "ab\ncad\n", "形状自检");
    assert_eq!(
        f.cursor(),
        5,
        "vim 9.1: 块 gp 光标 = 最后粘贴内容之后一格（(2,3)=byte 5）"
    );
}

// ---------------------------------------------------------------- E-C-8（P3）
// i_CTRL-V 十进制 >255 钳制：get_literal 的 `if (cc > 255 && unicode == 0)
// cc = 255`——`C-v 300` 插入 'ÿ'（255）。引擎 char::from_u32(300) 得 U+012C。
//
// oracle：printf 'x\n' > b_ec8; keys b'\x1bi\x16300\x1b:call
//   writefile([strtrans(getline(1))],"out")\r:wq\r' → 'xÿ'。
#[test]
#[ignore]
fn ctrl_v_decimal_value_clamps_at_255() {
    let mut f = Fixture::new("x\n");
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["3", "0", "0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "x\u{ff}\n",
        "vim 9.1: C-v 300 钳到 255（'ÿ'）；引擎插 U+012C"
    );
}

// ---------------------------------------------------------------- E-C-9（P3）
// C-v 数值 0 = 插入**换行**（get_literal：`if (cc == 0) cc = '\n'`——NUL
// 内部存为 NL），且终结数字的非数字键 vungetc 照常处理：`A<C-v>0<Esc>`
// 插出一个换行然后 Esc 正常退出插入。引擎把 0 当「未完成的数字串」，
// Esc 只取消（D6 行为）——缓冲原样。
//
// oracle：printf 'x\n' > b_ec9; keys b'\x1bA\x160\x1b:call
//   writefile([getline(1,"$")],"out")\r:wq\r' → ['x','']。
#[test]
#[ignore]
fn ctrl_v_zero_value_inserts_a_line_break() {
    let mut f = Fixture::new("x\n");
    f.feed(["A"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "x\n\n",
        "vim 9.1: C-v 0 的值为 0 → NL → 换行；引擎按未完成数字取消"
    );
}

// ------------------------------------------------------------ E-C-10（P3）
// C-v 前缀后的非法终结键**照常处理**（get_literal：i==0 时 `cc = nc`；
// insert.txt「the 'invalid' character is dealt with in the normal way」）：
// `i<C-v>xg<Esc>` 插入 'g'。引擎对空数字串 from_str_radix("")=0 → 插 NUL
// （缓冲被塞进控制字节，数据污染），'g' 变普通打字。
//
// oracle：printf '' > b_ec10; keys b'\x1bi\x16xg\x1b:call
//   writefile([strtrans(getline(1))],"out")\r:wq\r' → 'g'。
#[test]
#[ignore]
fn ctrl_v_invalid_terminator_is_typed_normally() {
    let mut f = Fixture::new("");
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    f.feed(["x"]);
    f.feed(["g"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "g",
        "vim 9.1: C-v x 后的非法键 'g' 照常插入；引擎插 NUL 并丢 'g'"
    );
}

// ---------------------------------------------------------------- E-C-11（P3)
// `0<C-d>` 移除**全部**缩进（edit.c 的 0_CTRL-D 臂把缩进删到 0，
// `:h i_0_CTRL-D`「delete all indent」），不只是 shiftwidth 一档。引擎
// insert_shift_indent 的 typed_zero 分支只删掉 '0' 后走普通 C-d（±sw）。
//
// 复现："      abc"（6 空格缩进）上 `:set sw=4` `I0<C-d><Esc>`。
// vim 9.1：'abc'（全缩进移除）；引擎：'  abc'（只移一档 4 列）。
// oracle：printf '      abc\n' > b_ec11; keys b'\x1b:set sw=4\rI0\x04\x1b:call
//   writefile([getline(1)],"out")\r:wq\r' → 'abc'（\x04=C-d）。
#[test]
#[ignore]
fn zero_ctrl_d_removes_all_indent() {
    let mut f = Fixture::new("      abc\n");
    f.feed([":", "s", "e", "t", " ", "s", "w", "=", "4", "<CR>"]);
    f.feed(["I", "0"]);
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: 0<C-d> 移除全部缩进；引擎只移 shiftwidth 一档"
    );
}

// ---------------------------------------------------------------- E-C-12（P3)
// `^<C-d>`：移除**全部**缩进且光标列不动（edit.c 同臂的 '^' 分支），
// '^' 本身不进缓冲。引擎把 '^' 当普通打字。
//
// oracle：printf '      abc\n' > b_ec12; keys b'\x1b:set sw=4\rI^\x04\x1b:call
//   writefile([getline(1)],"out")\r:wq\r' → 'abc'。
#[test]
#[ignore]
fn caret_ctrl_d_removes_all_indent_without_typing_caret() {
    let mut f = Fixture::new("      abc\n");
    f.feed([":", "s", "e", "t", " ", "s", "w", "=", "4", "<CR>"]);
    f.feed(["I", "^"]);
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: ^<C-d> 移除全部缩进且 '^' 是命令字符；引擎打了字面 '^'"
    );
}

// ---------------------------------------------------------------- E-C-13（P2)
// `@@` 必须把 `@:` 计入「上一个寄存器」（normal.c nv_at：任何寄存器名——
// 含 ':'——都 `lastc = cap->nchar`）。引擎 MacroPlay 的 `:` 臂执行完不更新
// last_macro_played，`@@` 重放的是更早的宏或响铃。
//
// 复现："a\na\n" 上 `qax<Esc>q`，`:s/a/z/<CR>`，`@:`，`@@`。
// vim 9.1：@@ 重放 @: → 第二行未动（line1 'zz' 已无 'a'，E486），缓冲
// 'zz\na'。引擎：@@ 重放宏 a → 'xzz\na'。
// oracle：printf 'a\na\n' > b_ec13; keys
//   b'\x1bqax\x1bq:s/a/z/\r@:@@:call writefile([getline(1,"$")],"out")\r:wq\r'
//   → ['zz','a']。
#[test]
#[ignore]
fn at_at_after_at_colon_repeats_the_ex_command() {
    let mut f = Fixture::new("a\na\n");
    f.feed(["q", "a", "x", "<Esc>", "q"]); // a = 'ix<Esc>'
    f.feed([":", "s", "/", "a", "/", "z", "/", "<CR>"]); // line1 'za'
    f.feed(["@", ":"]); // 重放 :s → line1 'zz'
    assert_eq!(f.text(), "zz\na\n", "形状自检");
    f.feed(["@", "@"]); // 应重放 @:（line1 无 'a' → 无操作）
    assert_eq!(
        f.text(),
        "zz\na\n",
        "vim 9.1: @@ 重复的是 @:（nv_at 的 lastc 收 ':'）；引擎重放了宏 a"
    );
}

// ---------------------------------------------------------------- E-C-14（P3)
// 空寄存器粘贴的报文：do_put 对 y_size==0 报 `E353: Nothing in register x`
// （semsg；含普通模式 p）。引擎 put_ex 空寄存器只 note_bell——状态栏无
// E353 文本。
//
// oracle：冷寄存器上 `p`（PTY 通道可见 E353 回显；typeahead 侧以
// v:errmsg 采样亦可）。
#[test]
#[ignore]
fn paste_from_empty_register_reports_e353() {
    let mut f = Fixture::new("a\n");
    f.feed(["p"]); // unnamed 未设
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E353")),
        "vim 9.1: 空寄存器 p 报 E353: Nothing in register \"（引擎只响铃无报文）"
    );
}

// ================================================================ 证伪探针
// （oracle 已证引擎一致；失败即说明引擎回退，按回归处理）

// 证伪-1：`":` 存冒号后整条命令行（含范围）。oracle：`:1,2d` 后 `":p`
// 粘出 '1,2d'。
#[test]
#[ignore]
fn falsified_colon_register_holds_range_without_colon() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "1", ",", "2", "d", "<CR>"]);
    f.feed(["\"", ":", "p"]);
    assert_eq!(f.text(), "1,2dc\n", "vim 9.1: \": = '1,2d'（无冒号、含范围）");
}

// 证伪-2：`c` 的小删也写 `"-`（op_delete 同路）。
#[test]
#[ignore]
fn falsified_change_writes_small_delete_register() {
    let mut f = Fixture::new("ab cd\n");
    f.feed(["c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("ab".to_owned()),
        "vim 9.1: cw 的被删文本进 \"-（cw 不带尾随空白）"
    );
}

// 证伪-3：连续小删**覆盖** `"-`（op_delete 每次重新 op_yank，无累积）。
#[test]
#[ignore]
fn falsified_consecutive_small_deletes_overwrite_dash() {
    let mut f = Fixture::new("aa bb cc\n");
    f.feed(["d", "w", "d", "w"]);
    assert_eq!(
        f.vim.registers.get('-').map(|r| r.text.clone()),
        Some("bb ".to_owned()),
        "vim 9.1: 第二次 dw 覆写 \"-（非拼接）"
    );
}

// 证伪-4：`@@` 基本回放（无 @: 干扰时）。
#[test]
#[ignore]
fn falsified_at_at_replays_previous_macro() {
    let mut f = Fixture::new("q\n");
    f.feed(["q", "a", "x", "<Esc>", "q"]);
    f.feed(["@", "a"]);
    f.feed(["@", "@"]);
    assert_eq!(f.text(), "xxxq\n", "vim 9.1: @@ 重复 @a");
}

// 证伪-5：`3u` / `2<C-r>` 计数步进（undo.c 的 count 循环）。
#[test]
#[ignore]
fn falsified_undo_redo_count_steps() {
    let mut f = Fixture::new("a b c\n");
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["<Esc>", "w", "i"]);
    f.type_text("Y");
    f.feed(["<Esc>", "w", "i"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    f.feed(["3", "u"]);
    assert_eq!(f.text(), "a b c\n", "3u 撤三个插入会话");
    f.feed_raw(Key::ctrl_char('r'));
    f.feed_raw(Key::ctrl_char('r'));
    assert_eq!(f.text(), "aX bY c\n", "2<C-r> 重做两步");
}

// 证伪-6：`"/` 最近搜索模式寄存器。
#[test]
#[ignore]
fn falsified_search_register_holds_pattern() {
    let mut f = Fixture::new("foo\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed(["\"", "/", "p"]);
    assert_eq!(f.text(), "ffoooo\n", "vim 9.1: \"/ = 'foo'，p 粘出");
}

// 证伪-7：`p` 不改寄存器（unnamed 保持源内容）。
#[test]
#[ignore]
fn falsified_put_leaves_register_untouched() {
    let mut f = Fixture::new("xy\n");
    f.feed(["y", "i", "w"]);
    f.feed(["w", "p"]);
    f.feed(["p"]);
    assert_eq!(
        f.vim.registers.get(UNNAMED).map(|r| r.text.clone()),
        Some("xy".to_owned()),
        "vim 9.1: 粘贴不写寄存器"
    );
}

// 证伪-8：`:s` 各自一个 undo 组（连续两条 :s，一条 u 只撤最后一条）。
#[test]
#[ignore]
fn falsified_ex_substitutes_are_separate_undo_groups() {
    let mut f = Fixture::new("a\na\na\n");
    f.feed([":", "%", "s", "/", "a", "/", "b", "/", "<CR>"]);
    f.feed([":", "s", "/", "b", "/", "c", "/", "<CR>"]);
    assert_eq!(f.text(), "c\nb\nb\n", "形状自检");
    f.feed(["u"]);
    assert_eq!(f.text(), "b\nb\nb\n", "vim 9.1: u 只撤最后一条 :s");
}

// 证伪-9：`qaq` 清寄存器、`@a` 空转不残留（execute_command 的 stop 臂
// store_named_plain(reg, "")）。
#[test]
#[ignore]
fn falsified_empty_recording_clears_the_register() {
    let mut f = Fixture::new("x\n");
    f.feed(["q", "a", "i", "y"]);
    f.feed(["<Esc>", "q"]); // a = 'iy<Esc>'
    f.feed(["q", "a", "q"]); // 空录制覆盖
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "x\n", "vim 9.1: qaq 清空 \"a，@a 无操作");
}
