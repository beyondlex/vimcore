//! 第十七轮回归：宏停止的 `q` 残留、`@a` 后的 `.` 语义、`"+` 寄存器
//! roundtrip、句子对象 is/as 空白归属、文本对象 count、`g_` 纯空白行、
//! insert `<C-w>` 空白段、`<</>>`列模型、Ex 裸偏移与 `:s`/`:d` 尾 count。
//! 全部期望行为经 vim 9.1 探针实证（探针与输出见 NOTES 第十七轮 T 系列）。

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

// ---- 宏录制停止的 `q` 残留（T1）---------------------------------------------

/// PROBE T1: `qaxq` 停止后 `J` —— `.` 必须重复 join（vim ['b cd ef']），
/// 旧实现把 [q, J] 一起提交，`.` 重放 MacroRecord 并把引擎楔进录制态。
#[test]
fn macro_stop_q_does_not_leak_into_dot() {
    let mut f = edit("ab\ncd\nef\n", 0, 0, &["q", "a", "x", "q"]);
    assert_eq!(f.text(), "b\ncd\nef\n");
    f.feed(["J"]);
    assert_eq!(f.text(), "b cd\nef\n");
    f.feed(["."]);
    assert_eq!(f.text(), "b cd ef\n", "T1: dot repeats the JOIN only");
    assert!(
        f.vim.macro_recording().is_none(),
        "dot must not reopen a recording session"
    );
}

/// `"b` 前缀 + 录空宏 `"a qq`：宏停止必须清掉寄存器前缀，随后的 `p`
/// 走匿名寄存器（= yy 的镜像，linewise 开新行），而不是残留的空 `"a`。
#[test]
fn register_prefix_cleared_by_macro_stop() {
    let mut f = edit("ab\ncd\n", 0, 0, &["\"", "b", "y", "y"]);
    f.feed(["\"", "a", "q", "q"]);
    f.feed(["p"]);
    assert_eq!(
        f.text(),
        "ab\nab\ncd\n",
        "p reads the unnamed mirror, not the stale empty a prefix"
    );
}

// ---- @a 与 . 的 redo 语义（T2）----------------------------------------------

/// PROBE T2: `qaxq` → `~` → `0@a` → `.` —— vim 重放宏内的 x（['def']），
/// 旧实现 @ 期间抑制提交，`.` 重复的是宏前的 `~`（得到 'Cdef'）。
#[test]
fn dot_after_macro_replays_macro_last_change() {
    let mut f = edit("abcdef\n", 0, 0, &["q", "a", "x", "q"]);
    let mut f = f;
    f.feed(["~"]); // 'b'→'B'
    assert_eq!(f.text(), "Bcdef\n");
    f.feed(["0", "@", "a"]); // 宏内 x：删 'B'
    assert_eq!(f.text(), "cdef\n");
    f.feed(["."]);
    assert_eq!(f.text(), "def\n", "T2: dot repeats the macro's x, not ~");
}

/// `.` 自身重放仍不自我扩展（T2 对照）：`x.` 之后 last_change 停在一份 x。
#[test]
fn dot_replay_still_does_not_self_extend() {
    let mut f = edit("abcdef\n", 0, 0, &["x", "."]);
    assert_eq!(f.text(), "cdef\n", "one repeat, not a self-feeding loop");
}

/// 宏内含 insert 会话时 `.` 同样重复整个会话（录制含 Text 步骤）。
#[test]
fn dot_after_macro_with_insert_replays_insert() {
    let mut f = edit("ab\n", 0, 0, &["q", "a", "i"]);
    f.type_text("X");
    f.feed(["<Esc>", "q"]);
    f.feed(["0"]);
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "XXab\n", "macro inserted X at cursor");
    f.feed(["."]);
    assert_eq!(f.text(), "XXXab\n", "dot repeats the macro's insert session");
}

// ---- "+ 寄存器 roundtrip（T3）-----------------------------------------------

/// PROBE T3（结构缺陷，宿主无关）：`"+yy` 必须推送到宿主剪贴板，`"+p`
/// 贴回同一内容。旧实现写进 get_for_paste 永远读不到的 named 槽。
#[test]
fn clipboard_register_roundtrip() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed(["\"", "+", "y", "y"]);
    assert_eq!(
        f.host.clipboard.as_deref(),
        Some("hello\n"),
        "yank into + reaches the host clipboard"
    );
    f.feed(["j", "\"", "+", "p"]);
    assert_eq!(
        f.text(),
        "hello\nworld\nhello\n",
        "linewise paste reads it back below"
    );

    // 删除同样入剪贴板
    let mut f = Fixture::new("aaa\nbbb\n");
    f.feed(["\"", "+", "d", "d"]);
    assert_eq!(f.host.clipboard.as_deref(), Some("aaa\n"));
    f.feed(["p"]);
    assert_eq!(
        f.text(),
        "bbb\naaa\n",
        "delete into + then linewise put below the cursor line"
    );
}

/// 宿主剪贴板为空（无真实剪贴板的宿主）时回退 named 槽，roundtrip 仍成立。
#[test]
fn clipboard_paste_falls_back_to_named_slot() {
    let mut f = Fixture::new("abc\n");
    f.feed(["\"", "+", "y", "y"]);
    // 把宿主侧剪贴板清空，模拟无剪贴板/被外部清空的宿主
    f.host.clipboard = None;
    f.feed(["\"", "+", "p"]);
    assert_eq!(f.text(), "abc\nabc\n", "named-slot mirror keeps the roundtrip");
}

// ---- 句子对象 is/as（T4）-----------------------------------------------------

/// PROBE T4a: `dis` 止于句末标点 —— "Aaa. Bbb." 光标 A 只删 "Aaa."，
/// 旧实现吞掉句尾空格（得到 "Bbb." 而非 " Bbb."）。
#[test]
fn inner_sentence_stops_at_terminator() {
    let mut f = edit("Aaa. Bbb.\n", 0, 0, &["d", "i", "s"]);
    assert_eq!(f.text(), " Bbb.\n", "T4a: trailing space stays with next");
    let mut f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["5", "l", "d", "i", "s"]);
    assert_eq!(f.text(), "Aaa.  Ccc.\n", "middle sentence, both spaces kept");
}

/// PROBE T4b: `das` = 句子+尾随空白（句中）；末句无尾随时回退取前导空白。
#[test]
fn outer_sentence_trailing_then_leading_fallback() {
    let mut f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["5", "l", "d", "a", "s"]);
    assert_eq!(f.text(), "Aaa. Ccc.\n", "mid sentence takes its trailing ws");
    let mut f = edit("Aaa. Bbb.\n", 0, 0, &["5", "l", "d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n", "last sentence falls back to leading ws");
}

/// PROBE T4c: `as` 的尾随空白在段界（空行）封顶 —— 空行保留。
#[test]
fn outer_sentence_blank_line_is_boundary() {
    let mut f = edit("Aaa.\n\nBbb.\n", 0, 0, &["d", "a", "s"]);
    assert_eq!(f.text(), "\nBbb.\n", "T4c: the empty line survives");
    // 光标在 Bbb 上：无同行前导空白 → 只删句子（空行与换行都保留；
    // vim 探针 ['Aaa.', '', '|'] 逐字节一致）
    let mut f = edit("Aaa.\n\nBbb.\n", 2, 0, &["d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n\n\n", "sentence only; gap whitespace stays");
    // 同行的前导空白可以带上（vim ['Aaa.', '', 'Bbb.']）
    let mut f = edit("Aaa.\n\nBbb. Ccc.\n", 2, 9, &["d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n\nBbb.\n", "same-line leading space goes too");
}

/// PROBE T4d: `2das` 删两句（连各自尾随空白）。
#[test]
fn outer_sentence_count_two() {
    let mut f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["2", "d", "a", "s"]);
    assert_eq!(f.text(), "Ccc.\n");
}

// ---- 文本对象 count（T5）-----------------------------------------------------

/// PROBE T5a: `d2aw` 删两个词（旧实现等价 `daw`）。
#[test]
fn count_repeats_text_object() {
    let mut f = edit("foo bar baz\n", 0, 0, &["d", "2", "a", "w"]);
    assert_eq!(f.text(), "baz\n", "T5a: d2aw deletes two words");
    let mut f = edit("foo bar baz\n", 0, 0, &["2", "d", "a", "w"]);
    assert_eq!(f.text(), "baz\n", "pre-count multiplies too");
    let mut f = edit("foo bar baz\n", 0, 0, &["c", "3", "a", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X\n", "c3aw replaces all three");
}

/// PROBE T5b: count 的怪癖由重复扫描自然复现 —— `d2iw`=diw（"foo "）、
/// `3iw` 到 "foo bar"。
#[test]
fn inner_word_count_matches_vim_quirk() {
    let mut f = edit("foo bar baz\n", 0, 0, &["d", "2", "i", "w"]);
    assert_eq!(f.text(), "bar baz\n", "2iw covers 'foo ' like vim");
    let mut f = edit("foo bar baz\n", 0, 0, &["d", "3", "i", "w"]);
    assert_eq!(f.text(), " baz\n", "3iw covers 'foo bar' (blank stays)");
}

/// PROBE T5c: visual 下同样扩展；`v2i(` 攀到外层括号。
#[test]
fn visual_object_count_extends() {
    let mut f = edit("foo bar baz\n", 0, 0, &["v", "2", "a", "w", "y", "0", "P"]);
    assert_eq!(f.text(), "foo bar foo bar baz\n", "v2aw yanks two words");
    let mut f = edit("f (1 + (2)) g\n", 0, 7, &["v", "2", "i", "(", "y", "0", "P"]);
    assert_eq!(f.text(), "1 + (2)f (1 + (2)) g\n", "v2i( climbs one level");
}

// ---- g_ 于纯空白行（T6）------------------------------------------------------

/// PROBE T6: `dg_` 在 "   " 上 c1/c2/c3 分别删 1/2/3 字符 —— span 是
/// [line_start, cursor)。旧实现 Inclusive(line_start) 把 c1 也删成空行。
#[test]
fn g_underscore_whitespace_line_spans_to_cursor() {
    let mut f = edit("   \nfoo\n", 0, 0, &["d", "g", "_"]);
    assert_eq!(f.text(), "  \nfoo\n", "c1 deletes one char");
    let mut f = edit("   \nfoo\n", 0, 1, &["d", "g", "_"]);
    assert_eq!(f.text(), " \nfoo\n", "c2 deletes two");
    let mut f = edit("   \nfoo\n", 0, 2, &["d", "g", "_"]);
    assert_eq!(f.text(), "\nfoo\n", "c3 deletes three");
    let mut f = edit("   \n", 0, 1, &["c", "g", "_"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X \n", "cg_ replaces the run through the cursor");
    // 非空白行不变：g_ 落最后非空白字符
    let mut f = edit("a  b\n", 0, 1, &["d", "g", "_"]);
    assert_eq!(f.text(), "a\n");
}

// ---- insert <C-w> 空白段（T7）-----------------------------------------------

/// PROBE T7: 行中空白上 `<C-w>` 一笔删掉行首到光标（自动缩进清除）；
/// 旧实现落到并线分支每次剥 1 字节。并线只发生在 col 1。
#[test]
fn ctrl_w_deletes_whitespace_run_in_one_stroke() {
    let mut f = edit("x\n  y\n", 1, 2, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\ny\n", "the whole run goes in one stroke");

    // 半段空白：删到行首为止
    let mut f = edit("x\n  y\n", 1, 1, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n y\n", "only the run before the cursor");

    // 打字后的 C-w 照常删词
    let mut f = edit("x\n  y\n", 1, 2, &["i"]);
    f.type_text("hello");
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n  y\n", "typed word deleted, indent kept");

    // 行中非空白前缀照常删词，不并线
    let mut f = edit("ab\ncd\n", 1, 1, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\nd\n");
}

// ---- <</>>列模型（T8）-------------------------------------------------------

fn shift_fixture(initial: &str, line: usize, col: usize, opts: &str) -> Fixture {
    let mut f = Fixture::at(initial, line, col);
    // 通过 :set 走真实选项管道（noet/ts/sw）
    for c in opts.chars() {
        f.feed([&c.to_string()]);
    }
    f.feed(["<CR>"]);
    f
}

/// PROBE T8a: noet ts=8 sw=4 的左移重表达 —— `\tx`→`    x`、
/// `\t\tx`→`\t    x`（1 tab + 4 空格）。旧实现剥字节得 `\tx`。
#[test]
fn shift_left_reexpresses_tabs() {
    let mut f = shift_fixture("\tx\n", 0, 0, ":set noet sw=4 ts=8");
    f.feed(["<", "<"]);
    assert_eq!(f.text(), "    x\n", "TABx << 4 spaces");
    let mut f = shift_fixture("\t\tx\n", 0, 0, ":set noet sw=4 ts=8");
    f.feed(["<", "<"]);
    assert_eq!(f.text(), "\t    x\n", "TABTABx << one tab + 4 spaces");
    let mut f = shift_fixture(" \tx\n", 0, 0, ":set noet sw=4 ts=8");
    f.feed(["<", "<"]);
    assert_eq!(f.text(), "    x\n", "mixed space+tab collapses to column");
}

/// PROBE T8b: 右移按 tabstop 重表达 —— noet 下凑整 tab 余量空格；
/// col 5 + sw 4 = col 9 → `\t `（1 tab + 1 空格）。
#[test]
fn shift_right_reexpresses_by_tabstop() {
    let mut f = shift_fixture("x\n", 0, 0, ":set noet sw=4 ts=8");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "    x\n", "noet right shift uses spaces when 0 tabs fit");
    let mut f = shift_fixture("     x\n", 0, 0, ":set noet sw=4 ts=8");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "\t x\n", "col 9 = one tab + one space");
    let mut f = shift_fixture("\tx\n", 0, 0, ":set noet sw=4 ts=4");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "\t\tx\n", "ts=4: whole tab again");
    let mut f = shift_fixture("x\n", 0, 0, ":set et sw=4");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "    x\n", "et: spaces");
}

/// PROBE T8c: 纯空白行参与移位（`<<` 清空、`>>` 追加），真空行跳过。
#[test]
fn shift_on_whitespace_only_and_empty_lines() {
    let mut f = shift_fixture("   \n", 0, 0, ":set et sw=4");
    f.feed(["<", "<"]);
    assert_eq!(f.text(), "\n", "ws-only line: << empties it");
    let mut f = shift_fixture("   \n", 0, 0, ":set et sw=4");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "       \n", "ws-only line: >> appends sw");
    let mut f = shift_fixture("\nfoo\n", 0, 0, ":set et sw=4");
    f.feed([">", ">"]);
    assert_eq!(f.text(), "\nfoo\n", "EMPTY line stays empty");
}

// ---- Ex 裸偏移与尾 count（T9/T10）-------------------------------------------

/// PROBE T9: 裸 `+`/`-` = ±1（vim :h :range "If a number is omitted, 1 is
/// used"）。旧实现对空数字串报 E1247。
#[test]
fn ex_bare_plus_minus_offsets() {
    let mut f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "+", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nccc\n", ":+d deletes the next line");
    let mut f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "bbb\nccc\n", ":-d deletes the previous line");
    let mut f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "5", "+", "d", "<CR>"]);
    assert_eq!(
        f.text(),
        "aaa\nbbb\nccc\n",
        ":5+d past the end runs nothing (vim E16)"
    );
    assert!(f.host.statuses.iter().any(|s| s.contains("E16")));
    let mut f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", ".", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "bbb\nccc\n", ":.-d deletes the line above");
    // 链式偏移里的裸符号：:+-d = +1-1 = 当前行
    let mut f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", "+", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nccc\n", ":+-d re-anchors at the cursor line");
    // 数字溢出仍报 E1247（round15 行为保持）
    let mut f = edit("aaa\nbbb\n", 0, 0, &[":", "1", "+", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nbbb\n", "huge offset still refused");
    assert!(f.host.statuses.iter().any(|s| s.contains("E1247")));
}

/// PROBE T10a: `:d2` 直连数字 = `:d 2`（vim ex 解析：数字终止命令名）。
#[test]
fn ex_packed_count_for_delete() {
    let mut f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "d", "2", "<CR>"]);
    assert_eq!(f.text(), "ccc\n", ":d2 deletes two lines");
}

/// PROBE T10b: `:s` 尾 count = 从范围末行起数 N 行。
#[test]
fn ex_substitute_trailing_count() {
    let mut f = edit("aaa\naaa\naaa\n", 0, 0, &[":", "s", "/", "a", "/", "X", "/", " ", "3", "<CR>"]);
    assert_eq!(f.text(), "Xaa\nXaa\nXaa\n", "count 3 from line 1");
    let mut f = edit("aaa\naaa\naaa\n", 0, 0, &[
        ":", "1", ",", "2", "s", "/", "a", "/", "X", "/", "2", "<CR>",
    ]);
    assert_eq!(f.text(), "aaa\nXaa\nXaa\n", ":1,2s count 2 = lines 2-3");
    let mut f = edit("aaa\naaa\n", 0, 0, &[
        ":", "s", "/", "a", "/", "X", "/", "g", "2", "<CR>",
    ]);
    assert_eq!(f.text(), "XXX\nXXX\n", "g flag + count combine");
}

// ---- :nohlsearch 缩写前缀（T11）---------------------------------------------

/// vim 缩写规则：nohlsearch 的全部无歧义前缀可用（旧实现停在 nohls）。
#[test]
fn nohlsearch_prefix_abbreviations() {
    for spelling in ["noh", "nohl", "nohls", "nohlse", "nohlsea", "nohlsear", "nohlsearc"] {
        let mut f = edit("foo\n", 0, 0, &["/", "o", "o", "<CR>"]);
        f.feed([":"]);
        for c in spelling.chars() {
            f.feed([&c.to_string()]);
        }
        f.feed(["<CR>"]);
        assert!(
            f.host.highlights.is_empty(),
            "{spelling} must clear highlights (statuses: {:?})",
            f.host.statuses
        );
    }
}
