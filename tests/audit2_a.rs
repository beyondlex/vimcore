//! audit2_a：normal-mode motions / counts / text objects 独立审计（2026-10-07）。
//!
//! 每个测试对应一个新发现，断言的是 **vim 9.1（`-Nu NONE -N -s` typeahead
//! 实证）的期望结果**，因此在当前工作树上应当失败。oracle 探针逐条用
//! `printf … > /tmp/*.txt; vim -Nu NONE -N -i NONE -s …; cat 输出` 复核，
//! 证据写在各测试的注释里（1-based col 已换算回 0-based 字节偏移）。

mod common;

use common::edit;
use vimcore::buffer::VimBuffer;

// ---------------------------------------------------------------- 1. `|` 与 tab

/// **发现 1（P1）**：`{count}|` 的列号把 TAB 当 1 个显示格；vim 按 'ts'=8 展开。
/// oracle：`ab<Tab>c` 上 `5|` → col=3（停在 TAB 上，vcol 5 落在 tab 的 3..10
/// 格区间内）；引擎落到 offset 3（'c'，第 4 列）。
#[test]
fn pipe_column_ignores_tabstop() {
    let f = edit("ab\tc", 0, 0, &["5", "|"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: 5| 停在 TAB（col 3 1-based）");
}

// ---------------------------------------------------------------- 2. `d2aw` 于空行

/// **发现 2（P1）**：文本对象 count 的重复扫描在空行处断链（probe 落回本行
/// 尾词，range 无进展即 break）。vim 把空行本身当一个 word，2aw 会穿过它。
/// oracle：`foo␊␊bar␊` 光标 foo 上 `d2aw` → 缓冲全空（n=1，l1 为空）；
/// 引擎只剩 `\n\nbar\n`。
#[test]
fn d2aw_count_stops_at_empty_line() {
    let f = edit("foo\n\nbar\n", 0, 0, &["d", "2", "a", "w"]);
    assert_eq!(
        f.buf.line_count(),
        1,
        "vim 9.1: d2aw 穿过空行删掉 foo 与 bar，缓冲只剩 1 个空行"
    );
    assert_eq!(f.buf.line_content(0), "");
}

// ------------------------------------- 3. `daw` 于行尾空白 + 空行 + 下一词

/// **发现 3（P1）**：光标在行尾空白上时 `aw` 的空白+下一词搜索越过空行吞掉
/// 下面的词；vim 在空行（本身是一个 word）处停住，只删 `   \n`。
/// oracle：`a   ␊␊b␊` 光标在最后一个空格 `daw` → `a␊b␊`（n=2）；
/// 引擎得 `a`（b 也被吞，还丢了行结构）。
#[test]
fn daw_after_trailing_ws_eats_next_word_across_empty_line() {
    let f = edit("a   \n\nb\n", 0, 3, &["d", "a", "w"]);
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: 只删 '   \\n'，保留空行与 b");
}

// ------------------------------------ 4. `daw` 于纯空白行 + 空行 + 下一词

/// **发现 4（P1）**：光标在纯空白行（含 tab 行）上时同样吞掉空行之后的词。
/// oracle：`foo␊   ␊␊bar␊` 光标在空白行 `daw` → `foo␊bar␊`（n=2，
/// 删的是 `   \n\n`）；tab 版 `foo␊\t␊␊bar␊` → `foo␊bar␊`。引擎得 `foo␊`
/// （bar 被吞）。
#[test]
fn daw_on_ws_only_line_eats_next_word_across_empty_line() {
    let f = edit("foo\n   \n\nbar\n", 1, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "foo\nbar\n", "vim 9.1: 删 '   \\n\\n'，bar 保留");
}

// --------------------------------------- 5. `daw` 于无下一词的空白（EOF 方向）

/// **发现 5a（P1）**：光标在行尾空白、其后没有任何 word（直接到 EOF/空行
/// 尾部）时 vim 的 `daw` 是**无操作**（找不到「下一个词」，对象失败）；引擎
/// 的 blank_run_plus_next_word 把 `\n` 当普通空白续扫，把空白连同换行删掉。
/// oracle：`a   ␊` 光标在空格上 `daw` → 缓冲原样（n=1，l1="a   "）；
/// 引擎得 `a`（连换行都没了）。
#[test]
fn daw_trailing_ws_without_next_word_is_noop() {
    let f = edit("a   \n", 0, 3, &["d", "a", "w"]);
    assert_eq!(f.text(), "a   \n", "vim 9.1: 无下一词时 daw 不动");
}

/// **发现 5b（P1）**：纯空白**末行**（无换行结尾）同理。oracle：
/// `foo␊   ` 光标在空白行 `daw` → 原样（n=2，l1=foo l2="   "）；引擎得
/// `foo␊`（删掉了三个空格）。
#[test]
fn daw_ws_only_last_line_without_next_word_is_noop() {
    let f = edit("foo\n   ", 1, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "foo\n   ", "vim 9.1: 无下一词时 daw 不动");
}

// ---------------------------------------------------------------- 6. `cw` 与 U+3000

/// **发现 6（P1）**：`cw` 的「不含尾随空白」特例只剥 `' '`/`'\t'`；vim 的 cw
/// 等价 ce——**任何**尾随空白都不动。全角空格 U+3000 被引擎当词后空白删掉。
/// oracle：`a　b␊`（U+3000）`cwX<Esc>` → `X　b`；引擎得 `Xb`。
#[test]
fn cw_keeps_wide_space_vim_ce_semantics() {
    let mut f = edit("a\u{3000}b\n", 0, 0, &["c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X\u{3000}b\n", "vim 9.1: cw=ce，U+3000 保留");
}

// ---------------------------------------------------------------- 7. tag 大小写

/// **发现 7a（P1）**：`it`/`at` 的开闭标签名匹配是大小写敏感的（`rposition`
/// 按名字相等）；vim 9.1 对 HTML 标签大小写不敏感。
/// oracle：`<P>x</p>␊` 光标在 x `dit` → `<P></p>`；引擎原样不动。
#[test]
fn tag_object_matches_open_tag_case_insensitively() {
    let f = edit("<P>x</p>\n", 0, 3, &["d", "i", "t"]);
    assert_eq!(f.text(), "<P></p>\n", "vim 9.1: P/p 视为同名，删出 x");
}

/// **发现 7b（P1）**：反方向同理（小写开、大写闭）。oracle：
/// `<p>x</P>␊` `dit` → `<p></P>`；引擎原样不动。
#[test]
fn tag_object_matches_close_tag_case_insensitively() {
    let f = edit("<p>x</P>\n", 0, 3, &["d", "i", "t"]);
    assert_eq!(f.text(), "<p></P>\n", "vim 9.1: p/P 视为同名，删出 x");
}

// ---------------------------------------------------------------- 8. section motions

/// **发现 8（P2）**：`]]`/`[[`（及 `][`/`[]`/`]m`/`[m` 族）完全未绑定——
/// tables.rs 无行，按下响铃原地不动。vim 9.1 内建：`]]` 到下一个第一列
/// `{`/`}`，`[[` 到上一个第一列 `{`（`}` 在第一列不拦 `[[`）。
/// oracle：`foo␊{␊bar␊}␊baz␊` 上 `]]` → line 2（`{` 行）；`G[[` → line 2。
#[test]
fn section_motions_brace_column_unbound() {
    let f = edit("foo\n{\nbar\n}\nbaz\n", 0, 0, &["]", "]"]);
    assert_eq!(f.line(), 1, "vim 9.1: ]] 落到第一列开花括号的行");
    let f = edit("foo\n{\nbar\n}\nbaz\n", 4, 0, &["[", "["]);
    assert_eq!(f.line(), 1, "vim 9.1: [[ 落到第一列开花括号的行");
}

// ------------------------------------------- 9. `d2aw` 的 count 在换行处断链

/// **发现 9a（P1）**：首个对象是行尾词时，重复扫描从 span.end（换行位）落回
/// 本行末词、range 无进展即 break——count 静默失效。vim 的第 2 个 aw 跨过
/// 换行取下一行的词。
/// oracle：`ab cd␊ef gh␊` 光标在 cd 前的空格上 `d2aw` → `abgh`（n=1，
/// 删了 ` cd\nef `）；引擎得 `ab␊ef gh␊`（只删 ` cd`）。
#[test]
fn d2aw_count_stops_at_line_break() {
    let f = edit("ab cd\nef gh\n", 0, 2, &["d", "2", "a", "w"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: 两行并成一行 abgh");
    assert_eq!(f.buf.line_content(0), "abgh");
}

/// **发现 9b（P1）**：标点词变体。oracle：`ab!␊cd␊` 光标在 `!` 上
/// `2daw` → `ab`（n=1，删了 `!\ncd`）；引擎得 `ab␊cd␊`（只删 `!`）。
#[test]
fn d2aw_punct_line_final_stops_at_line_break() {
    let f = edit("ab!\ncd\n", 0, 2, &["2", "d", "a", "w"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: 只剩一行 ab");
    assert_eq!(f.buf.line_content(0), "ab");
}

// ---------------------------------------------- 10. `2cw` 跨空行

/// **发现 10（P1）**：`2cw` 的落点在第 2 行第 1 列（空行后）时，span 被下行
/// 的「列 1 落点」规则钳回本行行尾，cw 剥尾随空白后只剩光标词；vim 的 2cw
/// 一路改到第 2 个词的**末尾**（`b \n\ncd` 整段删掉）。
/// oracle：`ab ␊␊cd␊` 光标在 b 上 `2cwX<Esc>` → `aX`（n=1）；引擎得
/// `aX ␊␊cd␊`。
#[test]
fn c2w_across_empty_line_reaches_last_word() {
    let mut f = edit("ab \n\ncd\n", 0, 1, &["2", "c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.line_count(), 1, "vim 9.1: 改写吞掉空行与 cd");
    assert_eq!(f.buf.line_content(0), "aX");
}

// --------------------------------------------------- 11. `j` 与 tab 的列保持

/// **发现 11（P1）**：`j`/`k`（及 `<C-d>`/`<C-u>`/`<C-f>`/`<C-b>`）的列保持
/// 用 unicode-width，TAB 记 1 格；vim 保持的是**虚拟列**（'ts'=8 展开）。
/// oracle：`a<Tab>b␊0123456789x␊` 上 `w`（col 3，virtcol 9）再 `j` →
/// line 2 col 9（'8'）；引擎落在 '2'（col 3）。
#[test]
fn j_preserves_virtcol_with_tabstop() {
    let f = edit("a\tb\n0123456789x\n", 0, 2, &["j"]);
    assert_eq!(f.cursor(), 12, "vim 9.1: j 保持 virtcol 9 → 第 9 列 '8'");
}
