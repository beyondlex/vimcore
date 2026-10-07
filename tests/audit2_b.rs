//! audit2_b：第二轮算子/寄存器/粘贴审计（2026-10-07）。
//! 每个发现一个 `#[test]`，断言 vim 9.1（`-Nu NONE -N -s` typeahead oracle，
//! 本机实证）的预期结果，因此当前全部失败。
//!
//! 发现清单（严重度 / 编号对应回复正文）：
//! 1. 空 linewise 寄存器在无尾换行的行上 p/P 少粘一行（P1）
//! 2. 无尾换行末行上 `p` 吞掉寄存器自带的末尾 `\n`（P1）
//! 3. 无尾换行末行上 `]p` 把粘贴内容并进当前行（P1）
//! 4. linewise 寄存器按 count 重复时不补行分隔符（P1）
//! 5. 大小写算子用全 Unicode 映射（ﬁ→FI、ǰ→J̌、İ→i̇），vim 用简单映射（P2）
//! 6. `gP` 行级粘贴光标落在粘贴块首行，vim 落在块之后一行（P2）
//! 7. `"{reg}` 前缀在 operator-pending 下被接受（vim 取消算子并响铃）（P2）
//! 8. 只读寄存器（`"%` `":` `".` `"/`）可被 `"{reg}` 前缀写入（vim 拒绝）（P2）
//! 9. `"Add` 后未命名寄存器指向合并结果，vim 指向新追加片段（P3）
//! 10. `"=` 表达式寄存器缺失（vim 求值并粘出结果）（P2）

mod common;

use common::edit;
use vimcore::VimBuffer;

/// 1.（P1）空 linewise 寄存器 put 少一行。
/// oracle：空文件（1 空行）上 `yyp` → line("$")==2；`yy3p` → 4；`yyP` → 2。
/// 引擎：`p`/`P` 分支把寄存器自带的 `\n` 当作「与当前行的分隔」消费掉，
/// 空寄存器被强制成 `"\n"` 后 strip_suffix 归零，一行都没落进缓冲。
#[test]
fn f1_empty_linewise_put_on_empty_buffer_loses_line() {
    // yy + p：vim 2 行（"\n\n"），引擎仍是 1 行（"\n"）
    let f = edit("", 0, 0, &["y", "y", "p"]);
    assert_eq!(f.text(), "\n\n", "yy p 于空缓冲应得到两个空行");

    // yy + P：vim 2 行
    let f = edit("", 0, 0, &["y", "y", "P"]);
    assert_eq!(f.text(), "\n\n", "yy P 于空缓冲应得到两个空行");

    // yy + 3p：vim 1+3=4 行
    let f = edit("", 0, 0, &["y", "y", "3", "p"]);
    assert_eq!(f.text(), "\n\n\n\n", "yy 3p 于空缓冲应得到四个空行");
}

/// 2.（P1）`p` 于无尾换行的末行下粘贴时，把换行结尾的寄存器文本的
/// 末尾 `\n` strip 掉当分隔符，最后一个被粘行丢了文件末尾换行。
/// oracle：文件 "a\n" 上 `ddp` + `:wq` → 文件字节 `\na\n`；`dd3p` → `\na\na\na\n`。
/// 引擎：`"\na"` / `"\na\na\na"`（末行 noeol，静默丢字节）。
#[test]
fn f2_put_below_noel_line_drops_final_newline() {
    let f = edit("a\n", 0, 0, &["d", "d", "p"]);
    assert_eq!(f.text(), "\na\n", "dd p 应得 [\"\"] 与 [\"a\"]（末行带 eol）");

    let f = edit("a\n", 0, 0, &["d", "d", "3", "p"]);
    assert_eq!(f.text(), "\na\na\na\n", "dd 3p 应得 [\"\",a,a,a]");
}

/// 3.（P1）`]p` 粘贴位置取 `line_range(cur).end.min(len)`——末行无尾换行时
/// 该点就是缓冲末尾，粘贴文本直接拼进行内。
/// oracle：noeol 文件 [ab, cd] 上 `yy j ]p` → [ab, cd, ab]。
/// 引擎："ab\ncdab\n"（"ab" 并进 "cd" 行）。
#[test]
fn f3_bracket_p_below_noel_line_glues() {
    let f = edit("ab\ncd", 0, 0, &["y", "y", "j", "]", "p"]);
    assert_eq!(f.text(), "ab\ncd\nab", "]p 应在末行下开新行粘出 ab");
}

/// 4.（P1）linewise 寄存器按 count 重复用 `text.repeat(count)`——从 noeol
/// 末行 yank 出的 linewise 文本没有自带的 `\n`，重复后各行水平粘连。
/// oracle：noeol 文件 "abc" 上 `yy3p` → [abc,abc,abc,abc]。
/// 引擎："abc\nabcabcabc"。
#[test]
fn f4_linewise_count_repeat_glues_without_separators() {
    let f = edit("abc", 0, 0, &["y", "y", "3", "p"]);
    assert_eq!(f.text(), "abc\nabc\nabc\nabc", "3p 应粘出三个独立行");
}

/// 5.（P2）大小写算子走 Rust 的 full Unicode 映射；vim 9.1 用简单映射，
/// 无简单映射的字符保持不变：
///   `~` 于 ﬁ（U+FB01）→ vim 不变 "ﬁx"，引擎 "FIx"；
///   `gul` 于 İ（U+0130）→ vim "ix"（普通 i），引擎 "i\u{307}x"；
///   `~` 于 ǰ（U+01F0）→ vim 不变 "ǰx"，引擎 "J\u{30c}x"。
/// （oracle：三例 getline(1) 逐一实证。ß→ẞ 已单列特判，本项是其余同族。）
#[test]
fn f5_case_operators_full_vs_simple_unicode() {
    let f = edit("\u{fb01}x", 0, 0, &["~"]);
    assert_eq!(f.text(), "\u{fb01}x", "~ 不得展开连字 ﬁ");

    let f = edit("\u{0130}x", 0, 0, &["g", "u", "l"]);
    assert_eq!(f.text(), "ix", "gu 于 İ 应得普通 i，不带组合点");

    let f = edit("\u{01f0}x", 0, 0, &["~"]);
    assert_eq!(f.text(), "\u{01f0}x", "~ 不得把 ǰ 拆成 J+组合符");
}

/// 6.（P2）`gP` 行级粘贴光标：vim 落在「粘贴文本之后」= 粘贴块的下一行
/// （oracle：[aaa,bbb,ccc] 上 `jyyGgP` → cursor line 4；2 行块 → line 5）。
/// 引擎落在粘贴块首行（line 3，1-based）。
#[test]
fn f6_gp_linewise_cursor_after_block() {
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &["y", "y", "G", "g", "P"]);
    // 粘贴后缓冲 [aaa,bbb,bbb,ccc]；vim 光标在 1-based 第 4 行（块之后）
    assert_eq!(f.line(), 3, "gP 光标应落在粘贴块之后的行（0-based 3）");
}

/// 7.（P2）operator-pending 下不接受 `"{reg}` 前缀。
/// oracle："foo bar" 上 `d"aw<Esc>` → 算子被 `"` 取消（响铃），随后 `a` 进入
/// 插入模式、`w` 被打字 → 缓冲 "fwoo bar"，寄存器 a 保持为空。
/// 引擎：把 "foo " 删进寄存器 a，缓冲变 "bar"。
#[test]
fn f7_register_prefix_rejected_under_operator() {
    let f = edit("foo bar", 0, 0, &["d", "\"", "a", "w", "<Esc>"]);
    assert_eq!(f.text(), "fwoo bar", "d\"aw 不得当作删除执行");
}

/// 8.（P2）只读寄存器不得经 `"{reg}` 前缀写入。
/// oracle（逐一实证）：
///   `"%dd` → 拒绝，缓冲保持 "abc"（1 行）；
///   `":dd`、`".dd`、`"/dd` → 同样拒绝（`.dd` 后 getreg('.') 仍是旧插入文本）。
/// 引擎：`"%dd` 照删不误（缓冲变空），还把删除内容塞进名为 % 的槽。
#[test]
fn f8_readonly_registers_reject_prefix_write() {
    let f = edit("abc\n", 0, 0, &["\"", "%", "d", "d"]);
    assert_eq!(f.text(), "abc\n", "\"%dd 必须被拒绝，缓冲不变");
}

/// 9.（P3）`"Add` 之后未命名寄存器的内容。
/// oracle：[ab,cd,ef] 上 `"ayy` `j` `"Add` → getreg('"') == "ef\n"（新删除的
/// 片段），寄存器 a == "ab\ncd\n"（合并）。引擎把未命名寄存器指向合并后的
/// 整体，`p` 会粘出两行。
#[test]
fn f9_uppercase_append_unnamed_gets_new_piece() {
    let f = edit("ab\ncd\nef\n", 1, 0, &[
        "\"", "a", "y", "y", "j", "\"", "A", "d", "d", "p",
    ]);
    assert_eq!(
        f.text(),
        "ab\ncd\nef\n",
        "p 应只粘出新追加的 ef 一行（未命名=新片段，非合并体）"
    );
}

/// 10.（P2）`"=` 表达式寄存器缺失。
/// oracle："ab" 上 `"=2+3<CR>p` → vim 弹出 `=` 提示行求值 2+3，p 粘出 "5"
/// → 缓冲 "a5b"。引擎无表达式寄存器，`p` 落空（响铃），缓冲不变。
#[test]
fn f10_expression_register_put() {
    let f = edit("ab", 0, 0, &["\"", "=", "2", "+", "3", "<CR>", "p"]);
    assert_eq!(f.text(), "a5b", "\"=2+3<CR>p 应粘出求值结果 5");
}
