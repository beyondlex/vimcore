//! audit2_c：insert/replace 内部、`.` 重放、宏录制/重放 独立审计（2026-10-07）。
//!
//! 每个发现一个 `#[test]`，断言写的是 **vim 9.1（`-Nu NONE -N -i NONE -n -s`
//! typeahead 实证）的期望结果**，因此在当前工作树上应当失败。oracle 证据
//! 写在各测试注释里（`printf '%b' … > keys; vim -Nu NONE -N -i NONE -n -s
//! keys buf`；缓冲内容用 `:call writefile([getline(1)…])` 采样）。
//!
//! 范围：src/insert_mode.rs + state.rs 的宏/`.`/计数插入部分。

mod common;

use common::Fixture;
use std::matches;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;
use vimcore::mode::Mode;

// ------------------------------------------------ insert 控制键缺位（i_CTRL-T/D/Y/E/A/@/=）

/// **发现 1（P2）**：insert `<C-t>`（shiftwidth 缩进）完全未绑定——ctrl 弦表
/// 只认 w/u/r，其余落入 Unknown 静默丢弃。vim 9.1 在行首插入一个
/// shiftwidth（noet+sw=ts=8 → 一个 TAB），光标不动。
/// oracle：`abc` 上 `iab<C-t><Esc>` → getline(1) = "\tababc"；
/// 引擎得 "ababc"。
#[test]
fn ctrl_t_inserts_shiftwidth_indent() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["i"]);
    f.type_text("ab");
    f.feed_raw(Key::ctrl_char('t'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\tababc\n", "vim 9.1: i<C-t> 在行首插入一个 shiftwidth");
}

/// **发现 2（P2）**：insert `<C-d>`（去缩进）同样未绑定。vim 9.1 删除一个
/// shiftwidth；`0<C-d>`（insert 中先按 0 再 C-d）删除全部缩进且不落 0 字。
/// oracle：`"        abc"` 上 `i<C-d>x<Esc>` → "xabc"（col=1）；
/// `i0<C-d><Esc>` → "abc"（0<C-d> 全删）。引擎两个形状都不动
/// （得 "x        abc" / "0        abc"）。
#[test]
fn ctrl_d_removes_shiftwidth_indent() {
    let mut f = Fixture::at("        abc\n", 0, 0);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('d'));
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "xabc\n", "vim 9.1: i<C-d> 删一个 shiftwidth");

    let mut f = Fixture::at("        abc\n", 0, 0);
    f.feed(["i"]);
    f.type_text("0");
    f.feed_raw(Key::ctrl_char('d'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc\n", "vim 9.1: 0<C-d> 删除全部缩进");
}

/// **发现 3（P2）**：insert `<C-y>`（从上一行同列抄字符）未绑定。
/// oracle：`abcdef\nxy\n` 行2 col1 `ix` 后 `<C-y>`×3 → getline(2)="xxcdey"
/// （按插入点列 2/3/4 抄 c、d、e；上一行抄尽后为空操作）。引擎得 "xxy"。
#[test]
fn ctrl_y_copies_char_from_line_above() {
    let mut f = Fixture::at("abcdef\nxy\n", 1, 1);
    f.feed(["i"]);
    f.type_text("x");
    for _ in 0..3 {
        f.feed_raw(Key::ctrl_char('y'));
    }
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcdef\nxxcdey\n", "vim 9.1: i<C-y> 逐字符抄上一行");
}

/// **发现 4（P2）**：insert `<C-e>`（从下一行同列抄字符）未绑定。
/// oracle：`xyz\nab\n` 行1 col0 `i` 后 `<C-e>`×4 → getline(1)="abxyz"
/// （抄 'a'、'b'，下一行抄尽后为空操作）。引擎缓冲原样。
#[test]
fn ctrl_e_copies_char_from_line_below() {
    let mut f = Fixture::at("xyz\nab\n", 0, 0);
    f.feed(["i"]);
    for _ in 0..4 {
        f.feed_raw(Key::ctrl_char('e'));
    }
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abxyz\nab\n", "vim 9.1: i<C-e> 逐字符抄下一行");
}

/// **发现 5（P2）**：insert `<C-a>`（重新插入最近一次插入的文本）未绑定。
/// oracle：`foo\nbar\n` 上 `ihello <Esc>jA<C-a>!<Esc>` → getline(2)
/// = "barhello !"（多行文本同理照搬，C-a 在 A 会话里重放 "hello "）。
/// 引擎得 "bar!"（C-a 丢弃）。
#[test]
fn ctrl_a_inserts_last_inserted_text() {
    let mut f = Fixture::at("foo\nbar\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hello ");
    f.feed(["<Esc>", "j", "A"]);
    f.feed_raw(Key::ctrl_char('a'));
    f.type_text("!");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hello foo\nbarhello !\n", "vim 9.1: i<C-a> 重放上次插入文本");
}

/// **发现 6（P2）**：insert `<C-@>`（重放上次插入文本并退出 insert）未绑定。
/// oracle：`x` 上 `ihi<Esc>i<C-@><Esc>` → "hhiix" 且 mode=n
/// （C-@ 重放 "hi" 后即退 normal，随后的 Z 不再进缓冲——ca12 证）。
/// 引擎：C-@ 静默丢弃，仍留在 insert 模式（"hix"）。
#[test]
fn ctrl_at_inserts_last_insert_then_leaves_insert() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hi");
    f.feed(["<Esc>", "i"]);
    f.feed_raw(Key::ctrl_char('@'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hhiix\n", "vim 9.1: i<C-@> 重放上次插入文本");
    assert!(
        matches!(f.vim.mode(), Mode::Normal),
        "vim 9.1: i<C-@> 之后应已退出 insert 模式"
    );
}

/// **发现 7（P2）**：insert `<C-r>=`（表达式寄存器）未实现——引擎把 `=`
/// 当普通寄存器名查不到、静默吞掉，随后的表达式按键全部按文本插入。
/// oracle：`x` 上 `i<C-r>=1+1<CR><Esc>` → getline(1)="2x"（vim 打开 `=`
/// 提示符、求值、插结果）；引擎得 "1+1\nx\n"。
#[test]
fn ctrl_r_equals_expression_register() {
    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["="]);
    f.type_text("1+1");
    f.feed(["<CR>", "<Esc>"]);
    assert_eq!(f.text(), "2x\n", "vim 9.1: i<C-r>= 求值表达式并插入结果");
}

// ------------------------------------------------ `".` 寄存器的账目规则

/// **发现 8（P1）**：空的 insert 会话应当**清空** `".`（以及 `<C-a>`/`<C-@>`
/// 用的同一份 last-insert），引擎却保留旧账。
/// oracle：`ihi<Esc>i<Esc>` 后 `string(@.)` = ''（dot2 探针；`i<Esc>` 后
/// `<C-a>`/`<C-@>` 均不插任何内容，ba6 探针）。引擎在空会话时不清
/// `insert_session_text`，`i<C-r>.` 把陈旧的 "hi" 粘出来。
#[test]
fn empty_insert_session_clears_last_insert_register() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("hi");
    f.feed(["<Esc>", "i", "<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ZZ\nZZ\n", "vim 9.1: 空会话后 i<C-r>. 不粘任何内容");
}

/// **发现 9（P1）**：会话内 `<C-w>`/`<C-u>` 删掉的文本必须从 `".` 里去掉，
/// 引擎的 `insert_session_text` 只进不出。
/// oracle：`ifo<C-w><Esc>` 后 `i<C-a><Esc>` 什么都不插（ba1 探针，
/// `i<C-u>` 同 rg4）——即 `".` 为空。引擎 `i<C-r>.` 粘出 "fo"。
#[test]
fn ctrl_w_and_ctrl_u_trim_last_insert_register() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("fo");
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ZZ\nZZ\n", "vim 9.1: C-w 删掉的文本不再进 `\".`");
}

/// **发现 10（P2）**：会话内真实退格的删除要以**可重放的退格键**记账：
/// vim 的 `".` 存 "abc" + K_BS 内部码（strtrans 探针 rg1：`iabc<BS><Esc>`
/// 后 `@.` = "abc<80>kb"），`i<C-r>.`/`<C-a>` 重放时退格**再次执行**——
/// net 效果是 "ab"（rg2：`ZZ` 行粘出 "ZabZ"）。引擎粘出未删过的 "abc"。
#[test]
fn backspace_in_session_replays_as_backspace_from_last_insert() {
    let mut f = Fixture::at("ZZ\nZZ\n", 0, 0);
    f.feed(["i"]);
    f.type_text("abc");
    f.feed(["<BS>", "<Esc>", "j", "i"]);
    f.feed_raw(Key::ctrl_char('r'));
    f.feed(["."]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abZZ\nZabZ\n", "vim 9.1: `\".` 里的退格重放时再次执行");
}

// ------------------------------------------------ 宏录制 / 重放

/// **发现 11（P2）**：`q"`（录制进无名寄存器，`q{0-9a-zA-Z"}` 的合法形）被
/// 引擎拒绝（非 alphanumeric → 响铃待机）；vim 9.1 接受。
/// oracle：`ZZ` 上 `q"ix<Esc>q` 后 `@"` → "xxZZ"（qa1b 探针；
/// `strtrans(@")` = "ix^["，qa4）。
#[test]
fn q_quote_records_into_unnamed_register() {
    let mut f = Fixture::at("ZZ\n", 0, 0);
    f.feed(["q", "\"", "i"]);
    f.type_text("x");
    f.feed(["<Esc>", "q", "@", "\""]);
    assert_eq!(f.text(), "xxZZ\n", "vim 9.1: q\" 录进无名寄存器，@\" 重放");
}

/// **发现 12（P1）**：可视行选区里 `@{reg}` 应当执行寄存器（宏的第一条可视
/// 命令作用于活动选区），引擎未绑定（响铃、选区原样）。
/// oracle：`aa\nbb\ncc\n` 上 `qaxq<Esc>jV@a` → 缓冲变 ["a","cc"]
/// （vr8 探针：选中的 "bb" 行被重放的 `x` 整行删除，cur=2,1）。
#[test]
fn at_in_linewise_visual_runs_the_register() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["q", "a", "x", "q", "<Esc>", "j", "V", "@", "a"]);
    assert_eq!(
        f.text(),
        "a\ncc\n",
        "vim 9.1: V 中 @a 用重放的 x 删掉选区行"
    );
    assert_eq!(f.buf.line_count(), 2, "vim 9.1: bb 行整行删除");
}

// ------------------------------------------------ `.` 重放

/// **发现 13（P1）**：`.` 重放 `gi` 开头的插入时，引擎把 [g,i] 原样重跑、
/// 跳回 `^` 位；vim 的 `.` 把插入文本落在**当前光标**（gi 的跳转不是变更的
/// 一部分）。
/// oracle：`foo bar\nqux\n` 上 `giX<Esc>ww.` → ["Xfoo bar","Xqux"]
/// （gi2b 探针）。引擎得 "XXfoo bar\nqux"（X 插回了 `^`）。
#[test]
fn dot_after_gi_inserts_at_current_cursor() {
    let mut f = Fixture::at("foo bar\nqux\n", 0, 0);
    f.feed(["g", "i"]);
    f.type_text("X");
    f.feed(["<Esc>", "w", "w", "."]);
    assert_eq!(f.text(), "Xfoo bar\nXqux\n", "vim 9.1: . 把 X 插在当前光标");
}

/// **发现 14（P1）**：块插入（`<C-v>jI…<Esc>`）之后 `.` 必须把变更记进
/// redo——vim 重放原键序（块可视 + I + 文本 + Esc），即把文本按块重新插到
/// 光标行与块内各行；引擎用 recording_blocked 把**整条**录制丢弃，`.` 重放
/// 的是更早的无关变更（此处无 → 纯响铃）。
/// oracle：`aa\nbb\ncc\n` 上 `<C-v>jI-<Esc>w.` → ["--aa","--bb","cc"]
/// （b5 探针：光标行与块行各得一个 "-"）。
#[test]
fn dot_after_block_insert_redoes_block_insert() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("-");
    f.feed(["<Esc>", "w", "."]);
    assert_eq!(
        f.text(),
        "--aa\n--bb\ncc\n",
        "vim 9.1: . 重放块插入（光标行 + 块行）"
    );
}

/// **发现 15（P2）**：计数插入的重复条件比 vim 严——引擎要求光标停在
/// `anchor + expanded.len()`，一次退格就放弃重复；vim 9.1 退格**不**取消
/// 重复（方向键才取消，ar3 探针：`3ifoo<Left><Esc>` 确不重复）。
/// oracle：`zz` 上 `3ifo<BS><Esc>` → "fffzz"（rg6：每轮 "fo"+BS 净得 "f"，
/// 重复照发）。引擎光标漂移 → 跳过复制，得 "fzz"。
#[test]
fn count_insert_repeat_survives_backspace() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("fo");
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(f.text(), "fffzz\n", "vim 9.1: 退格不取消计数插入的重复");
}
