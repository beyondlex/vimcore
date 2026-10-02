//! 第十五轮回归：substitute replacement 转义、越界 Ex 地址、段落边界、
//! 空行 J、quote 对象回退、`N%` 文件百分比、黑洞寄存器、`3R` 重复、
//! redo-register、`<C-h>` 投递形态、visual 标记跳转。全部期望行为经
//! vim 9.1 探针实证（探针脚本见 NOTES 第十五轮）。

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

/// 逐字符展开按键串（`<...>` 记法整体投递）：Ex 命令行测试域。
fn expand(keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        if k.starts_with('<') && k.ends_with('>') && k.len() > 2 {
            out.push(k.to_string());
        } else {
            out.extend(k.chars().map(|c| c.to_string()));
        }
    }
    out
}

fn feed_expanded(f: &mut Fixture, keys: &[&str]) {
    f.feed(expand(keys));
}

// ---- :s replacement 转义（首次执行 + & / :s 重放）------------------------

/// PROBE: vim 9.1 `:s/a/b\/c/` → `b/c`（replacement 里的 `\/` 还原成分隔符；
/// 旧引擎原样保留两个字符）。`&` 与裸 `:s` 重放同款（重放经同一解析器）。
#[test]
fn substitute_replacement_unescapes_separator() {
    // `&`/裸 `:s` 在命令关闭之后键入（`&` 在提示符里只是字面字符）
    for replay in [&["&"][..], &["<CR>", ":", "s", "<CR>"][..]] {
        let mut f = edit("a\n", 0, 0, &[":"]);
        let mut keys: Vec<String> = expand(&["s/a/b\\/c/"]);
        keys.extend(expand(&["<CR>"]));
        f.feed(keys);
        f.feed(expand(replay));
        assert_eq!(
            f.text(),
            "b/c\n",
            "replacement \\/ must fold to / (replay = {replay:?})"
        );
    }
}

/// PROBE: vim 9.1 `:s/a/b\\c/` → `b\c`（`\\` 还原成单个反斜杠）。
#[test]
fn substitute_replacement_unescapes_backslash() {
    let mut f = edit("a\n", 0, 0, &[":"]);
    feed_expanded(&mut f, &["s/a/b\\\\c/", "<CR>"]);
    assert_eq!(f.text(), "b\\c\n");
}

/// 未知转义保持原样（vim 同款）：`\c` 不是 replacement 特殊序列。
#[test]
fn substitute_replacement_keeps_unknown_escape() {
    let mut f = edit("a\n", 0, 0, &[":"]);
    feed_expanded(&mut f, &["s/a/b\\c/", "<CR>"]);
    assert_eq!(f.text(), "b\\c\n");
}

// ---- 越界 Ex 地址（E1247）-------------------------------------------------

/// PROBE: vim 9.1 `:1+<21 个 9>d` → E1247 且不动（旧引擎 unwrap_or(0) 把
/// 偏移吞成 +0，静默删掉第 1 行）。负向同（`:5-<巨值>d`）。
#[test]
fn overflowing_address_offset_reports_e1247() {
    for line in ["1+99999999999999999999d", "5-99999999999999999999d"] {
        let mut f = edit("one\ntwo\nthree\n", 0, 0, &[":"]);
        feed_expanded(&mut f, &[line, "<CR>"]);
        assert_eq!(f.text(), "one\ntwo\nthree\n", "{line} must not act");
        assert!(
            f.host
                .statuses
                .iter()
                .any(|s| s.starts_with("E1247")),
            "{line} must report E1247, got {:?}",
            f.host.statuses.last()
        );
    }
}

// ---- } / { 段落边界只认真空行 ----------------------------------------------

/// PROBE: vim 9.1 `aaa / "   " / bbb` 上 `d}` → 全删剩一个空行（空白行不是
/// 段落边界；旧引擎停在空白行行首）。`{` 反向同理。
#[test]
fn paragraph_motion_skips_whitespace_only_lines() {
    let f = edit("aaa\n   \nbbb\n", 0, 0, &["d", "}"]);
    assert_eq!(f.text(), "", "d}} leaves one empty line (vim [''])");
    let f = edit("aaa\n   \nbbb\n", 2, 0, &["d", "{"]);
    assert_eq!(f.text(), "bbb\n", "d{{ from bbb keeps only bbb (9.1 probe)");
}

// ---- 空行上的 J -----------------------------------------------------------

/// PROBE: vim 9.1 `["", "def"]` 上 `J` → `"def"`（接缝任一侧为空不加空格；
/// 旧引擎得 `" def"`）。`["def", ""]` 同。
#[test]
fn join_on_empty_line_adds_no_space() {
    let f = edit("\ndef\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "def\n");
    let f = edit("def\n\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "def\n");
}

// ---- ci" 越过行末引号回退第一对 -------------------------------------------

/// PROBE: vim 9.1 `say "hi" then "bye" end` 光标在 `"bye"` 之后 → `ci"` 改
/// `hi`（旧行为：响铃 no-op；左引号悬空成闭引号时无右配对）。
#[test]
fn ci_quote_past_last_pair_falls_back_to_first() {
    let mut f = edit("say \"hi\" then \"bye\" end\n", 0, 19, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\" end\n");
}

// ---- N% 文件百分比 ----------------------------------------------------------

/// PROBE: vim 9.1 nv_percent 公式是 1-based ceil `(count*total+99)/100`：
/// 200 行缓冲 `2%` → 第 4 行（0-based 3）、`50%` → 0-based 99、`30%` →
/// 0-based 59。旧 floor 公式在乘积恰为 100 倍数时差一行。
#[test]
fn count_percent_uses_ceil_formula() {
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.push(format!("L{i}"));
    }
    let initial = format!("{}\n", lines.join("\n"));
    for (count, want) in [(2usize, 3usize), (50, 99), (30, 59)] {
        let mut keys: Vec<String> = vec!["g".into(), "g".into()];
        keys.extend(count.to_string().chars().map(|c| c.to_string()));
        keys.push("%".into());
        let mut f = Fixture::new(&initial);
        f.feed(keys);
        assert_eq!(
            f.buf.offset_to_line(f.vim.cursor_offset()),
            want,
            "{count}%@200 must land 0-based line {want}"
        );
    }
}

/// PROBE: vim 9.1 `1%` 是文件百分比（200 行 → 第 2 行），不是括号匹配；
/// 旧 `count > 1` 守卫把键入 1 合并进「无 count」，无括号行上响铃原地。
#[test]
fn typed_one_percent_is_file_position() {
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.push(format!("L{i}"));
    }
    let initial = format!("{}\n", lines.join("\n"));
    let f = edit(&initial, 0, 0, &["g", "g", "1", "%"]);
    assert_eq!(f.buf.offset_to_line(f.vim.cursor_offset()), 1);
    assert_eq!(f.host.bells, 0, "1% is a percentage, not a bracket search");
    // 裸 % 在无括号行仍然响铃原地
    let f = edit("no brackets\n", 0, 0, &["%"]);
    assert_eq!(f.vim.cursor_offset(), 0);
    assert_eq!(f.host.bells, 1);
}

// ---- "_yy 不碰 unnamed ------------------------------------------------------

/// PROBE: vim 9.1 `"_yy` 后 `p` 贴不出任何东西（unnamed 未被触碰）；
/// 旧 fall-through 无条件重指 last。
#[test]
fn blackhole_yank_leaves_unnamed_untouched() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["\"", "_", "y", "y", "G", "$", "p"]);
    assert_eq!(f.text(), "aaa\nbbb\nccc\n");
}

// ---- 3R count-repeat ---------------------------------------------------------

/// PROBE: vim 9.1 `abcdefghij` 上 `3Rab<Esc>` → `abababghij`（Replace 退出时
/// 按字符数覆盖后续文本；旧引擎把 Replace 排除在 count-repeat 之外）。
#[test]
fn count_replace_repeats_overwrite() {
    let mut f = edit("abcdefghij\n", 0, 0, &["3", "R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababghij\n");
}

// ---- redo-register ------------------------------------------------------------

/// PROBE: vim 9.1 `:h redo-register`——`dd dd "1P .` 恢复两行（`.` 递增寄存器
/// 号 "1→"2；旧引擎贴同一寄存器两次）。
#[test]
fn dot_repeats_walk_numbered_register_ring() {
    let f = edit(
        "one\ntwo\nthree\ntail\n",
        0,
        0,
        &["d", "d", "d", "d", "\"", "1", "P", "."],
    );
    assert_eq!(f.text(), "one\ntwo\nthree\ntail\n");
}

// ---- <C-h> 两种投递形态 --------------------------------------------------------

/// gpui 风格宿主投递规范 ctrl 形态 `Key::ctrl_char('h')`；crossterm 风格投递
/// 裸 `\x08`。vim 绑定 C-h ≡ BS——提示符与插入模式两处都曾被静默吞掉。
#[test]
fn ctrl_h_folds_to_backspace_in_both_delivery_forms() {
    // 提示符
    let mut f = Fixture::new("x\n");
    feed_expanded(&mut f, &[":", "a", "b", "c"]);
    f.feed_raw(vimcore::key::Key::parse("<C-h>"));
    assert_eq!(f.vim.cmdline.buffer, "ab");
    // 插入模式
    let mut f = edit("abc\n", 0, 0, &["a"]);
    f.feed_raw(vimcore::key::Key::parse("<C-h>"));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "bc\n");
    // 提示符里的裸 \x08 字节（crossterm 风格文本路径）仍工作
    let mut f = Fixture::new("x\n");
    feed_expanded(&mut f, &[":", "a", "b"]);
    f.feed_raw(vimcore::key::Key::char('\x08'));
    assert_eq!(f.vim.cmdline.buffer, "a");
}

// ---- visual 模式的 'a / `a -----------------------------------------------------

/// PROBE: vim 9.1 visual 中 `'a`/`` `a `` 移动光标、选区跟随（旧行为：单键
/// Miss 响铃并清 pending）。
#[test]
fn visual_mark_jump_extends_selection() {
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    feed_expanded(&mut f, &["m", "a", "V", "j", "'", "a"]);
    assert_eq!(f.host.bells, 0, "'a in visual must not bell");
    assert!(
        matches!(f.vim.mode, vimcore::Mode::Visual { .. }),
        "visual mode must persist"
    );
    assert_eq!(f.vim.cursor_offset(), 0, "cursor jumped back to mark a");
}

// ---- 巨 count 定点终止 -----------------------------------------------------------

/// PROBE: vim 的 motion 重复循环首个不前进的步即停；旧引擎 `999999999e` 在
/// 缓冲末字符上跑满 10⁹ 次全词扫描。
#[test]
fn huge_counts_stop_at_fixed_points() {
    let start = std::time::Instant::now();
    let f = edit("foo\n", 0, 2, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "e"]);
    let f2 = edit("foo bar\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "}"]);
    let f3 = edit("foo\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "b"]);
    assert!(
        start.elapsed().as_millis() < 500,
        "fixed-point counts must not spin (took {:?})",
        start.elapsed()
    );
    assert_eq!(f.vim.cursor_offset(), 2);
    assert_eq!(f2.vim.cursor_offset(), 6, "brace lands on the last char (cursor clamp)");
    assert_eq!(f3.vim.cursor_offset(), 0);
}

// ---- 块寄存器空末行 ----------------------------------------------------------------

/// 宿主块 API yank 的行以 `\n` join（无终止符）；末行为空时文本以 `\n`
/// 结尾，put 侧 trim 曾把空末行吞掉少插一行。
#[test]
fn block_register_trailing_empty_row_survives_put() {
    let mut f = Fixture::new("abc\ncde\nfgh\n");
    // 造一个末行短于块左缘的块（宿主 API：拖选 (0,1)-(2,4)）
    f.vim.set_visual_range(&f.buf, 1, 4);
    f.feed(["y"]);
    f.vim.set_cursor_offset(&f.buf, 0);
    f.feed(["p"]);
    // 行数只增不减：2 行块贴出 2 行
    let text = f.text();
    let line_count = text.lines().count();
    assert!(line_count >= 4, "block put must keep both rows, got {text:?}");
}
