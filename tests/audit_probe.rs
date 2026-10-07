//! 第三十一轮审计回归：BUG_AUDIT.md 的 50 项修复回归测试。
//! 每项断言 vim 9.1（typeahead oracle）实证过的语义。

mod common;

use common::{edit, BufferView, Fixture};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use vimcore::host::VimHost;
use vimcore::VimBuffer;
use vimcore::mode::Mode;
use vimcore::state::{Ctx, VimState};

/// 审计专用宿主：完整实现 VimHost，undo/redo 返回值可被伪造（A1 用）。
struct AuditHost {
    text: Rc<RefCell<String>>,
    viewport: (usize, usize),
}

impl VimHost for AuditHost {
    fn viewport(&self) -> (usize, usize) {
        self.viewport
    }
    fn scroll_to_line(&mut self, _line: usize) {}
    fn clipboard_write(&mut self, _text: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, _m: &[Range<usize>], _c: Option<Range<usize>>) {}
    fn changed(&mut self) {}
    fn bell(&mut self) {}
    fn save(&mut self) {}
    fn request_close(&mut self) {}
    fn request_close_forced(&mut self, _f: bool) {}
    fn status_message(&mut self, _m: &str) {}
    fn buffer_name(&self) -> &str {
        "audit"
    }
    fn dispatch_host_action_hinted(&mut self, _id: &str, _s: bool) {}
    fn first_buffer(&mut self) -> bool {
        false
    }
    fn last_buffer(&mut self) -> bool {
        false
    }
    fn begin_undo_group(&mut self, _id: u64, _cursor: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
}

/// A1：viewport 极值不得 panic（motions.rs ScreenTop/ScreenMiddle 溢出）。
#[test]
fn a1_overflow_viewport_no_panic() {
    let max = usize::MAX;
    for keys in [vec!["H"], vec!["M"], vec!["L"], vec!["<C-f>"], vec!["<C-d>"]] {
        let shared = Rc::new(RefCell::new("l1\nl2\nl3\n".to_owned()));
        let mut buf = BufferView(shared.clone());
        let mut host = AuditHost {
            text: shared,
            viewport: (max, max),
        };
        let mut vim = VimState::new();
        for k in &keys {
            let key = vimcore::key::Key::parse(k);
            let mut ctx = Ctx {
                buf: &mut buf,
                host: &mut host,
            };
            let _ = vim.handle_key(&mut ctx, key);
        }
        assert!(vim.cursor_offset() <= buf.len(), "keys {:?} 光标越界", keys);
    }
}

// ------------------------------------------------------------- B 数字

/// B1：hex 字面量 a-f 位视为数字（vim 9.1）。
#[test]
fn b1_hex_letter_digits() {
    let f = edit("v 0x1f w\n", 0, 4, &["<C-a>"]);
    assert_eq!(f.text(), "v 0x20 w\n", "光标在 1：实际 {:?}", f.text());
    let f = edit("v 0x1f w\n", 0, 5, &["<C-a>"]);
    assert_eq!(f.text(), "v 0x20 w\n", "光标在 f：实际 {:?}", f.text());
}

/// B2：非数字位的前向搜索找到完整浮点 token 并浮点 +1。
#[test]
fn b2_float_forward_search() {
    let f = edit("v 1.5 w\n", 0, 3, &["<C-a>"]);
    assert_eq!(f.text(), "v 2.5 w\n", "实际 {:?}", f.text());
}

// ------------------------------------------------------------- C :s 替换侧

/// C1：`$1` 字面输出（vim 的后向引用是 \1，$ 是普通字符）。
#[test]
fn c1_dollar_one_literal() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "$", "1", "/", "<CR>"]);
    assert_eq!(f.text(), "$1x\n", "实际 {:?}", f.text());
}

/// C2：`~` 复用上次替换。
#[test]
fn c2_tilde_reuses_replacement() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "1", "s", "/", "x", "/", "N", "N", "N", "/", "<CR>",
                                     ":", "2", "s", "/", "x", "/", "~", "/", "<CR>"]);
    assert_eq!(f.text(), "NNN1\nNNN2\n", "实际 {:?}", f.text());
}

/// C3：`&` = 整个匹配。
#[test]
fn c3_ampersand_whole_match() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "&", "X", "/", "<CR>"]);
    assert_eq!(f.text(), "xXx\n", "实际 {:?}", f.text());
}

/// C4：`\u`/`\L` 大小写修饰符。
#[test]
fn c4_case_modifiers() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "\\", "u", "y", "/", "<CR>"]);
    assert_eq!(f.text(), "Yx\n", "\\u 实际 {:?}", f.text());
    let f = edit("ABC\n", 0, 0, &[":", "1", "s", "/", "A", "/", "\\", "l", "b", "/", "<CR>"]);
    assert_eq!(f.text(), "bBC\n", "\\l 实际 {:?}", f.text());
    let f = edit("abc def\n", 0, 0, &[":", "1", "s", "/", "a", "b", "c", "/", "\\", "U", "&", "\\", "e", "!", "/", "<CR>"]);
    assert_eq!(f.text(), "ABC! def\n", "\\U&\\e 实际 {:?}", f.text());
}

// ------------------------------------------------------------- D 寄存器

/// D1：`".` 最近插入寄存器。
#[test]
fn d1_dot_register() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i"]);
    f.type_text("hello");
    f.feed(["<Esc>", "\"", ".", "p"]);
    assert_eq!(f.text(), "hellohelloab\n", "p 粘在光标字符后；实际 {:?}", f.text());
}

/// D2：`"%` 文件名寄存器。
#[test]
fn d2_percent_register() {
    let f = edit("ab\n", 0, 0, &["\"", "%", "p"]);
    assert_eq!(f.text(), "atest-bufferb\n", "p 粘在光标字符后；实际 {:?}", f.text());
}

/// D3：`":` 最近命令行寄存器。
#[test]
fn d3_colon_register() {
    let f = edit("ab\n", 0, 0, &[":", "s", "/", "a", "/", "b", "/", "<CR>", "\"", ":", "p"]);
    assert!(f.text().contains("s/a/b/"), "实际 {:?}", f.text());
}

// ------------------------------------------------------------- E Ex 命令

/// E1：裸 `:sort` 全文件排序。
#[test]
fn e1_bare_sort_whole_file() {
    let f = edit("c\na\nb\n", 0, 0, &[":", "s", "o", "r", "t", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "实际 {:?}", f.text());
}

/// E2：`:ce {n}` 居中（CJK 显示宽度）。
#[test]
fn e2_ex_center() {
    let f = edit("中文\n", 0, 0, &[":", "c", "e", " ", "1", "0", "<CR>"]);
    assert_eq!(f.text(), "   中文\n", "实际 {:?}", f.text());
}

/// E3：`:ri {n}` 右对齐。
#[test]
fn e3_ex_right() {
    let f = edit("ab\n", 0, 0, &[":", "r", "i", " ", "1", "0", "<CR>"]);
    assert_eq!(f.text(), "        ab\n", "实际 {:?}", f.text());
}

/// E4：`:le` 左对齐去缩进。
#[test]
fn e4_ex_left() {
    let f = edit("    ab\n", 0, 0, &[":", "l", "e", "<CR>"]);
    assert_eq!(f.text(), "ab\n", "实际 {:?}", f.text());
}

/// E5：`:pu` 粘贴寄存器为行。
#[test]
fn e5_ex_put() {
    let f = edit("l1\nl2\n", 0, 0, &["y", "y", "j", ":", "p", "u", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl1\n", "实际 {:?}", f.text());
}

/// E6：`:retab` et 下展开 tab。
#[test]
fn e6_retab() {
    let f = edit("\ta\n", 0, 0, &[":", "s", "e", "t", " ", "e", "t", "<CR>",
                                 ":", "r", "e", "t", "a", "b", " ", "4", "<CR>"]);
    assert_eq!(f.text(), "    a\n", "实际 {:?}", f.text());
}

/// E7：`:&&` 重复上次替换。
#[test]
fn e7_double_ampersand() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "1", "s", "/", "x", "/", "Y", "/", "<CR>", "j", ":", "&", "&", "<CR>"]);
    assert_eq!(f.text(), "Y1\nY2\n", "oracle: && 在当前行重放 s/x/Y/；实际 {:?}", f.text());
}

/// E8：`:undo` 命令。
#[test]
fn e8_undo_command() {
    let f = edit("ab\n", 0, 0, &["x", ":", "u", "n", "d", "o", "<CR>"]);
    assert_eq!(f.text(), "ab\n", "实际 {:?}", f.text());
}

/// E9：`:redo` 命令。
#[test]
fn e9_redo_command() {
    let f = edit("ab\n", 0, 0, &["x", "u", ":", "r", "e", "d", "o", "<CR>"]);
    assert_eq!(f.text(), "b\n", "实际 {:?}", f.text());
}

/// E10：`g&` 全文件重放替换。
#[test]
fn e10_g_ampersand() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "s", "/", "x", "/", "Y", "/", "<CR>", "g", "&"]);
    assert_eq!(f.text(), "Y1\nY2\n", "实际 {:?}", f.text());
}

/// E11：`:reg {name}` 列指定寄存器。
#[test]
fn e11_reg_named() {
    let f = edit("ab\n", 0, 0, &["\"", "a", "y", "y", ":", "r", "e", "g", " ", "a", "<CR>"]);
    assert!(!f.host.statuses.iter().any(|s| s.contains("E492")),
            "实际 statuses={:?}", f.host.statuses);
}

/// E12：`:delmarks` 删除 mark。
#[test]
fn e12_delmarks() {
    let f = edit("ab\n", 0, 0, &["m", "a", ":", "d", "e", "l", "m", " ", "a", "<CR>"]);
    assert!(!f.host.statuses.iter().any(|s| s.contains("E492")),
            "实际 statuses={:?}", f.host.statuses);
    // 删除后 'a 跳转报错（不落点）
    let f = edit("ab\n", 0, 0, &["m", "a", ":", "d", "e", "l", "m", " ", "a", "<CR>", "'", "a"]);
    assert_eq!(f.line(), 0, "delm 后 'a 不得有落点");
}

// ------------------------------------------------------------- F 范围/搜索偏移

/// F1：`'<,'>` 地址（oracle：V j Esc 后手工 `'<,'>s` 两行都替换）。
#[test]
fn f1_visual_range_substitute() {
    let f = edit("x1\nx2\nx3\n", 0, 0, &["V", "j", "<Esc>", ":", "'", "<", ",", "'", ">", "s", "/", "x", "/", "Y", "/", "<CR>"]);
    assert_eq!(f.text(), "Y1\nY2\nx3\n", "实际 {:?}", f.text());
}

/// F2：`:/pat1/,/pat2/d` 搜索范围。
#[test]
fn f2_search_range_delete() {
    let f = edit("l1\nfoo\nl3\nbar\nl5\n", 0, 0, &[":", "/", "f", "o", "o", "/", ",", "/", "b", "a", "r", "/", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl5\n", "实际 {:?}", f.text());
}

/// F3：`/pat/e` 落匹配尾。
#[test]
fn f3_search_offset_end() {
    let f = edit("foo bar\n", 0, 0, &["/", "f", "o", "o", "/", "e", "<CR>", "x"]);
    assert_eq!(f.text(), "fo bar\n", "x 应删匹配尾 o，实际 {:?}", f.text());
}

// ------------------------------------------------------------- G 可视/块

/// G1：块可视 `x` 删整块宽。
#[test]
fn g1_block_x() {
    let f = edit("abc\nab\na\n", 0, 0, &["<C-v>", "j", "j", "l", "x"]);
    assert_eq!(f.text(), "c\n\n\n", "实际 {:?}", f.text());
}

/// G2：块可视 `D` 每行删到行尾。
#[test]
fn g2_block_D() {
    let f = edit("abcd\nab\na\n", 0, 0, &["<C-v>", "j", "j", "D"]);
    assert_eq!(f.text(), "\n\n\n", "实际 {:?}", f.text());
}

/// G3：块可视 `c` 块列 >0 不错位。
#[test]
fn g3_block_c_offset() {
    let f = edit("abcd\nab\ncd\n", 0, 2, &["<C-v>", "j", "j", "c"]);
    let mut f = f;
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abXd\nabX\ncdX\n", "实际 {:?}", f.text());
}

/// G4：`ß` 大写 = ẞ（U+1E9E，vim 9.1）。
#[test]
fn g4_sharp_s_uppercase() {
    let f = edit("ß x\n", 0, 0, &["g", "U", "U"]);
    assert_eq!(f.text(), "\u{1e9e} X\n", "实际 {:?}", f.text());
}

/// G5：行可视 `p` 字符级寄存器替换选区行（不吞行）。
#[test]
fn g5_visual_line_p_charwise() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["\"", "\"", "y", "i", "w", "j", "V", "p"]);
    assert_eq!(f.text(), "aaa\naaa\nccc\n", "实际 {:?}", f.text());
}

/// G6：`gp` 光标在粘贴文本之后。
#[test]
fn g6_gp_cursor_after() {
    let f = edit("ab cd\n", 0, 0, &["x", "g", "p"]);
    assert_eq!(f.cursor(), 2, "vim gp 光标在粘贴文本后（col3/字节2），实际 {}", f.cursor());
}

// ------------------------------------------------------------- H 插入

/// H1：`C-v` 可引用控制键（Esc/Tab/C-v 等）。
#[test]
fn h1_ctrl_v_control_keys() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>"]);
    f.feed_raw(vimcore::key::Key::escape());
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\u{1b}Xab\n", "C-v<Esc> 应插入字面 ESC（光标随其后）；实际 {:?}", f.text());
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('v'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\u{16}ab\n", "C-v C-v 应插入字面 ^V；实际 {:?}", f.text());
}

/// H2：`C-v u{4hex}` 十六进制字面。
#[test]
fn h2_ctrl_v_unicode() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>", "u", "0", "0", "e", "9", "<Esc>"]);
    assert_eq!(f.text(), "éab\n", "实际 {:?}", f.text());
}

/// H3：`<C-k>` 二合字母。
#[test]
fn h3_digraph() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-k>", "s", "s", "<Esc>"]);
    assert_eq!(f.text(), "ßab\n", "实际 {:?}", f.text());
}

// ------------------------------------------------------------- I 缩进/选项

/// I1：无 autoindent 时 `cc` 清除原缩进。
#[test]
fn i1_cc_clears_indent() {
    let f = edit("    deep\n", 0, 0, &["c", "c"]);
    let mut f = f;
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n", "实际 {:?}", f.text());
}

/// I2：`sw=0` 时 `>>` 使用 ts。
#[test]
fn i2_shiftwidth_zero_uses_ts() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "s", "w", "=", "0", "<CR>", ">", ">"]);
    assert_eq!(f.text(), "    a\n", "实际 {:?}", f.text());
}

// ------------------------------------------------------------- J 键位/信息

/// J1：normal `<Insert>` 进插入模式。
#[test]
fn j1_insert_key_normal() {
    let f = edit("ab\n", 0, 0, &["<Insert>"]);
    assert!(matches!(f.vim.mode(), Mode::Insert), "实际 {:?}", f.vim.mode());
}

/// J2：`ga` 显示字符码。
#[test]
fn j2_ga() {
    let f = edit("a\n", 0, 0, &["g", "a"]);
    assert!(!f.host.statuses.is_empty(), "实际 statuses={:?}", f.host.statuses);
    assert!(f.host.statuses[0].contains("97"), "vim ga 含十进制码：{:?}", f.host.statuses);
}

/// J3：`g8` 显示十六进制字节。
#[test]
fn j3_g8() {
    let f = edit("a\n", 0, 0, &["g", "8"]);
    assert!(!f.host.statuses.is_empty(), "实际 statuses={:?}", f.host.statuses);
    assert!(f.host.statuses[0].contains("61"), "g8 应含 hex 61：{:?}", f.host.statuses);
}

/// J4：`gq` 按 textwidth 重排（gqq 只排当前行——vim 同款；gqj 跨两行合并）。
#[test]
fn j4_gq() {
    let f = edit("aaa bbb\nccc\n", 0, 0, &["g", "q", "j"]);
    assert_eq!(f.text(), "aaa bbb ccc\n", "gqj 合并两行；实际 {:?}", f.text());
}

// ------------------------------------------------------------- K 其他

/// K1：可视 `g C-a` 顺序递增。
#[test]
fn k1_visual_g_ctrl_a() {
    let f = edit("1\n1\n1\n", 0, 0, &["<C-v>", "j", "j", "g", "<C-a>"]);
    assert_eq!(f.text(), "2\n3\n4\n", "实际 {:?}", f.text());
}

/// K2：insert `<S-Tab>` 不插入空格（vim：无操作）。
#[test]
fn k2_shift_tab_insert() {
    let mut f = Fixture::new("    ab\n");
    f.feed(["A"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers { shift: true, ..Default::default() },
        kind: vimcore::key::KeyKind::Named("tab".into()),
    });
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ab\n", "实际 {:?}", f.text());
}

/// K3：cmdline `<C-r>{reg}` 插入寄存器。
#[test]
fn k3_cmdline_ctrl_r() {
    let mut f = Fixture::new("ab\n");
    f.feed(["x"]);
    f.feed([":"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('r'));
    f.feed(["\""]);
    f.feed(["<CR>"]);
    // 寄存器 " = "a" → 命令行执行 ":a" → E492 含 a
    assert!(f.host.statuses.iter().any(|s| s.contains("E492") && s.contains("a")),
            "实际 statuses={:?}", f.host.statuses);
}

/// K4：`_` motion 落首个非空白。
#[test]
fn k4_underscore_motion() {
    let f = edit("  a\n  b\n  c\n", 0, 0, &["j", "_"]);
    assert_eq!(f.cursor(), 6, "应为 line2 'b' 的字节位，实际 {}", f.cursor());
}

// ------------------------------------------------------------- L 响铃

/// L1：`3r.` 行内不足——响铃。
#[test]
fn l1_counted_r_bells() {
    let f = edit("ab\n", 0, 0, &["3", "r", "."]);
    assert_eq!(f.text(), "ab\n");
    assert!(f.host.bells > 0, "实际 bells={}", f.host.bells);
}

/// L2：空行 `r` 响铃。
#[test]
fn l2_r_empty_line_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["r", "x"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert!(f.host.bells > 0, "实际 bells={}", f.host.bells);
}

/// L3：无缩进 `<<` 响铃。
#[test]
fn l3_dedent_bells() {
    let f = edit("ab\n", 0, 0, &["<", "<"]);
    assert!(f.host.bells > 0, "实际 bells={}", f.host.bells);
}

/// L4：空行 `~` 响铃。
#[test]
fn l4_tilde_empty_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["~"]);
    assert!(f.host.bells > 0, "实际 bells={}", f.host.bells);
}

/// L5：空行 `diw` 失败响铃。
#[test]
fn l5_iw_empty_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["d", "i", "w"]);
    assert!(f.host.bells > 0, "实际 bells={}", f.host.bells);
}

// ------------------------------------------------------------- M 粘贴

/// M1：`]p` 按目标行缩进调整。
#[test]
fn m1_paste_indent_adjust() {
    let f = edit("  ab\ncd\n", 0, 0, &["y", "y", "j", "]", "p"]);
    assert_eq!(f.text(), "  ab\ncd\nab\n", "实际 {:?}", f.text());
}
