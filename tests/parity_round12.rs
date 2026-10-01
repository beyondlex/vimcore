//! 第十二轮检视回归(vim 9.1 探针实证)。
//!
//! 覆盖:范围扫描多字节 panic、地址偏移链、`w`/`b` 空白行语义、`e` 末词尾、
//! `d$`/`dg_` 空行、`aw` 空白对象、`d-`/`d+` 边缘、`2*`/`2gN`、`ci"` 引号外、
//! Ex:`:s` 带范围重放、`:2,2j`、`:d _`、显式 count、`@/`、`:set ic` 刷新。

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

fn lines(f: &Fixture) -> Vec<String> {
    (0..f.buf.line_count())
        .map(|i| f.buf.line_range(i))
        .map(|r| f.buf.slice(r))
        .collect()
}

// ---- 范围解析(cmdline)------------------------------------------------

#[test]
fn ex_range_multibyte_mark_name_no_panic() {
    // `:'中d` 曾把 中 切在字节中间,split_at panic(用户可触发)
    let f = edit("中文\nabc\n", 0, 0, &[":", "'", "中", "d", "\r"]);
    // vim 9.1: E78: Unknown mark(非 ASCII mark 名非法)
    assert_eq!(f.host.statuses.last().map(String::as_str), Some("E78: Unknown mark"));
    assert_eq!(f.buf.line_count(), 2);
}

#[test]
fn ex_range_multibyte_after_quote_in_list_no_panic() {
    // `,` 之后的 '中 同款崩溃路径
    let f = edit("中文\nabc\ndef\n", 0, 0, &[":", ",", "'", "中", "d", "\r"]);
    assert_eq!(f.buf.line_count(), 3);
}

#[test]
fn ex_range_offset_chain_accumulates() {
    // :5+2+1d = 删第 8 行(vim 9.1);旧实现只解析第一段(+2)删第 5 行
    let f = edit("1\n2\n3\n4\n5\n6\n7\n8\n9\n", 0, 0, &[":", "5", "+", "2", "+", "1", "d", "\r"]);
    assert_eq!(lines(&f), vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n", "7\n", "9\n"]);
}

#[test]
fn ex_range_offset_mixed_signs() {
    // :3-1d 删第 2 行(vim 探针)
    let f = edit("1\n2\n3\n4\n", 0, 0, &[":", "3", "-", "1", "d", "\r"]);
    assert_eq!(lines(&f), vec!["1\n", "3\n", "4\n"]);
}

// ---- w / b / e(word.rs)------------------------------------------------

#[test]
fn w_skips_whitespace_only_line() {
    // vim 只停真空行;纯空白行自由跳过(9.1: w 落 def 的 d)
    let f = edit("abc\n   \ndef\n", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 8);
}

#[test]
fn w_still_stops_on_empty_line() {
    // 真空行停驻不变(既有行为,vim 一致)
    let f = edit("ab\n\ncd\n", 0, 0, &["w"]);
    assert_eq!(f.line(), 1);
}

#[test]
fn b_skips_whitespace_only_line_upward() {
    let f = edit("abc\n   \ndef\n", 2, 0, &["b"]);
    assert_eq!(f.cursor(), 0); // abc 的 a,不在空白行停
}

#[test]
fn ye_at_last_word_end_yanks_word_only() {
    // 旧实现返回 buf.len(),ye 吞尾部换行得 "c\n"
    let mut f = edit("abc\n", 0, 2, &["y", "e"]);
    let unnamed = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(unnamed.as_deref(), Some("c"));
    f.feed(&["p"]);
    assert_eq!(f.text(), "abcc\n");
}

#[test]
fn de_at_last_word_end_keeps_newline() {
    let f = edit("abc\n", 0, 2, &["d", "e"]);
    assert_eq!(f.text(), "ab\n");
}

// ---- d$ / dg_(motions + ops)-------------------------------------------

#[test]
fn dollar_delete_on_empty_line_is_noop() {
    // 旧实现 Inclusive 端点吞 \n,两行并一行(vim: no-op)
    let f = edit("\nabc\n", 0, 0, &["d", "$"]);
    assert_eq!(lines(&f), vec!["\n", "abc\n"]);
}

#[test]
fn g_underscore_delete_on_whitespace_line_is_noop() {
    let f = edit("\nabc\n", 0, 0, &["d", "g", "_"]);
    assert_eq!(lines(&f), vec!["\n", "abc\n"]);
}

#[test]
fn dollar_delete_still_deletes_to_line_end() {
    let f = edit("abcdef\n", 0, 2, &["d", "$"]);
    assert_eq!(f.text(), "ab\n");
}

// ---- aw / iw(objects)---------------------------------------------------

#[test]
fn daw_on_whitespace_takes_blank_run_plus_next_word() {
    // vim: daw 于词间空白删 "   bar" 剩 "foo"(旧实现只删空白段)
    let f = edit("foo   bar\n", 0, 4, &["d", "a", "w"]);
    assert_eq!(lines(&f), vec!["foo\n"]);
}

#[test]
fn yaw_on_whitespace_yanks_blank_run_plus_next_word() {
    let f = edit("foo   bar\n", 0, 4, &["y", "a", "w"]);
    let unnamed = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(unnamed.as_deref(), Some("   bar"));
}

#[test]
fn daw_on_whitespace_line_merges_next_line_away() {
    // vim: 空白行上 daw 连下一行词与换行一并删除,3 行变 1 行
    let f = edit("foo\n   \nbar\n", 1, 0, &["d", "a", "w"]);
    assert_eq!(lines(&f), vec!["foo\n"]);
}

#[test]
fn daw_on_empty_line_merges_next_line_away() {
    let f = edit("foo\n\nbar\n", 1, 0, &["d", "a", "w"]);
    assert_eq!(lines(&f), vec!["foo\n"]);
}

#[test]
fn daw_on_word_unchanged() {
    // 词上的 aw 行为不受本批改动影响
    let f = edit("foo bar\n", 0, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "bar\n");
}

// ---- d- / d+(motions)----------------------------------------------------

#[test]
fn d_minus_on_first_line_is_noop() {
    let f = edit("a\nb\n", 0, 0, &["d", "-"]);
    assert_eq!(f.buf.line_count(), 2);
}

#[test]
fn d_plus_on_last_line_is_noop() {
    let f = edit("a\nb\n", 1, 0, &["d", "+"]);
    assert_eq!(f.buf.line_count(), 2);
}

#[test]
fn d_minus_mid_still_deletes_two_lines() {
    let f = edit("a\nb\nc\n", 1, 0, &["d", "-"]);
    assert_eq!(lines(&f), vec!["c\n"]);
}

// ---- 搜索跳转 ------------------------------------------------------------

#[test]
fn star_count_jumps_count_matches() {
    // 2* 落第二个下一匹配(旧实现硬编码 1)
    let f = edit("a b a b a b\n", 0, 0, &["2", "*"]);
    assert_eq!(f.cursor(), 8);
}

#[test]
fn gn_count_backward_selects_distinct_matches() {
    // 2gN 曾每步重选同一匹配(range.start 恒被自身包含)
    let f = edit("a b a b\n", 0, 7, &["/", "a", "\r", "2", "g", "N"]);
    let (a, b) = f.vim.marks.active_visual().expect("2gN selects a range");
    // 从第一个匹配反向:第一步选当前 [0,1),第二步 wrap 到最后的 [4,5)
    // (wrapscan;旧实现第二步会重选 [0,1) 自身)
    assert_eq!((a, b), (4, 5));
}

// ---- ci"(objects)---------------------------------------------------------

#[test]
fn ciquote_between_strings_replaces_gap() {
    // vim: 左引号作开引号,配下一个引号 → ' then ' 被替换
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 13, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(&["<Esc>"]);
    assert_eq!(f.text(), "say \"hi\"X\"bye\"\n");
}

#[test]
fn ciquote_before_first_string_takes_first_pair() {
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 0, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(&["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\"\n");
}

#[test]
fn ciquote_inside_string_unchanged() {
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 5, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(&["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\"\n");
}

// ---- Ex(:s 重放 / :j / :d 寄存器 / count)---------------------------------

#[test]
fn substitute_replay_honors_explicit_range() {
    // :3,4s 重放作用于 3-4(旧实现丢范围落光标行)
    let mut f = edit("a\naxa\naxa\naxa\na\n", 0, 0, &[":", "2", ",", "4", "s", "/", "a", "/", "b", "/", "\r"]);
    f.feed(&["g", "g"]);
    f.feed(&[":", "3", ",", "4", "s", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "bxa\n", "bxb\n", "bxb\n", "a\n"]);
}

#[test]
fn substitute_updates_search_state() {
    // :s 后 @/ 为替换模式,n 搜之(vim)
    let mut f = edit("axa\nbxb\n", 0, 0, &[":", "s", "/", "x", "/", "Y", "/", "\r"]);
    let slash = f.vim.registers.get('/').map(|r| r.text.clone());
    assert_eq!(slash.as_deref(), Some("x"));
    f.feed(&["n"]);
    assert_eq!(f.cursor(), 5); // 第二个 x
}

#[test]
fn join_equal_two_address_range_is_noop() {
    // :2,2j no-op(vim :h :j);单地址 :2j 与裸 :j 照常接
    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", ",", "2", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "b\n", "c\n"]);

    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "b c\n"]);

    let f = edit("a\nb\nc\n", 0, 0, &[":", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a b\n", "c\n"]);
}

#[test]
fn delete_blackhole_register_stays_empty() {
    // :d _ 不进 "1(vim 黑洞)
    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", "d", " ", "_", "\r"]);
    assert_eq!(f.vim.registers.get('1'), None);
    assert_eq!(lines(&f), vec!["a\n", "c\n"]);
}

#[test]
fn delete_explicit_count_one_anchors_at_last_line() {
    // :1,2d 1 删第 2 行(显式 count 哪怕 1 也重锚定;旧实现删 1-2)
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", " ", "1", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "c\n"]);
}

#[test]
fn yank_explicit_count_one_takes_last_line_only() {
    let mut f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "y", " ", "1", "\r"]);
    let zero = f.vim.registers.get('0').map(|r| r.text.clone());
    assert_eq!(zero.as_deref(), Some("b\n"));
}

// ---- :set ic 刷新高亮 ------------------------------------------------------

#[test]
fn set_noic_refreshes_live_highlights() {
    // :set noic 后既有高亮按新规则重发布(Foo 掉出)
    let mut f = edit("Foo\nfoo\n", 0, 0, &["/", "f", "o", "o", "\r"]);
    assert_eq!(f.host.highlights.len(), 2);
    f.feed(&[":", "s", "e", "t", " ", "n", "o", "i", "c", "\r"]);
    assert_eq!(f.host.highlights.len(), 1);
}
