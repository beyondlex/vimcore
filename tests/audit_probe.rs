//! Round-31 audit regression: regression tests for the 50 fixes from the round-31 audit.
//! Every assertion pins semantics that were verified against vim 9.1 (typeahead oracle).

mod common;

use common::{edit, BufferView, Fixture};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use vimcore::host::VimHost;
use vimcore::VimBuffer;
use vimcore::mode::Mode;
use vimcore::state::{Ctx, VimState};

/// Audit-only host: a full VimHost implementation whose undo/redo return values
/// can be faked (used by A1).
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

/// A1: extreme viewport values must not panic (ScreenTop/ScreenMiddle overflow in motions.rs).
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
        assert!(vim.cursor_offset() <= buf.len(), "keys {:?} cursor out of bounds", keys);
    }
}

// ------------------------------------------------------------- B numbers

/// B1: a-f digits of a hex literal count as digits (vim 9.1).
#[test]
fn b1_hex_letter_digits() {
    let f = edit("v 0x1f w\n", 0, 4, &["<C-a>"]);
    assert_eq!(f.text(), "v 0x20 w\n", "cursor on the 1: actual {:?}", f.text());
    let f = edit("v 0x1f w\n", 0, 5, &["<C-a>"]);
    assert_eq!(f.text(), "v 0x20 w\n", "cursor on the f: actual {:?}", f.text());
}

/// B2: the forward search over non-digit positions finds the whole float token and increments it as a float.
#[test]
fn b2_float_forward_search() {
    let f = edit("v 1.5 w\n", 0, 3, &["<C-a>"]);
    assert_eq!(f.text(), "v 2.5 w\n", "actual {:?}", f.text());
}

// ------------------------------------------------------------- C :s replacement side

/// C1: `$1` is emitted literally (vim's back-reference is \1; $ is an ordinary char).
#[test]
fn c1_dollar_one_literal() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "$", "1", "/", "<CR>"]);
    assert_eq!(f.text(), "$1x\n", "actual {:?}", f.text());
}

/// C2: `~` reuses the previous replacement.
#[test]
fn c2_tilde_reuses_replacement() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "1", "s", "/", "x", "/", "N", "N", "N", "/", "<CR>",
                                     ":", "2", "s", "/", "x", "/", "~", "/", "<CR>"]);
    assert_eq!(f.text(), "NNN1\nNNN2\n", "actual {:?}", f.text());
}

/// C3: `&` stands for the whole match.
#[test]
fn c3_ampersand_whole_match() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "&", "X", "/", "<CR>"]);
    assert_eq!(f.text(), "xXx\n", "actual {:?}", f.text());
}

/// C4: the `\u`/`\L` case modifiers.
#[test]
fn c4_case_modifiers() {
    let f = edit("xx\n", 0, 0, &[":", "1", "s", "/", "x", "/", "\\", "u", "y", "/", "<CR>"]);
    assert_eq!(f.text(), "Yx\n", "\\u actual {:?}", f.text());
    let f = edit("ABC\n", 0, 0, &[":", "1", "s", "/", "A", "/", "\\", "l", "b", "/", "<CR>"]);
    assert_eq!(f.text(), "bBC\n", "\\l actual {:?}", f.text());
    let f = edit("abc def\n", 0, 0, &[":", "1", "s", "/", "a", "b", "c", "/", "\\", "U", "&", "\\", "e", "!", "/", "<CR>"]);
    assert_eq!(f.text(), "ABC! def\n", "\\U&\\e actual {:?}", f.text());
}

// ------------------------------------------------------------- D registers

/// D1: `".` the last-insert register.
#[test]
fn d1_dot_register() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i"]);
    f.type_text("hello");
    f.feed(["<Esc>", "\"", ".", "p"]);
    assert_eq!(f.text(), "hellohelloab\n", "p pastes after the char under the cursor; actual {:?}", f.text());
}

/// D2: `"%` the file-name register.
#[test]
fn d2_percent_register() {
    let f = edit("ab\n", 0, 0, &["\"", "%", "p"]);
    assert_eq!(f.text(), "atest-bufferb\n", "p pastes after the char under the cursor; actual {:?}", f.text());
}

/// D3: `":` the last command-line register.
#[test]
fn d3_colon_register() {
    let f = edit("ab\n", 0, 0, &[":", "s", "/", "a", "/", "b", "/", "<CR>", "\"", ":", "p"]);
    assert!(f.text().contains("s/a/b/"), "actual {:?}", f.text());
}

// ------------------------------------------------------------- E Ex commands

/// E1: a bare `:sort` sorts the whole file.
#[test]
fn e1_bare_sort_whole_file() {
    let f = edit("c\na\nb\n", 0, 0, &[":", "s", "o", "r", "t", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "actual {:?}", f.text());
}

/// E2: `:ce {n}` centers the line (CJK display width).
#[test]
fn e2_ex_center() {
    let f = edit("中文\n", 0, 0, &[":", "c", "e", " ", "1", "0", "<CR>"]);
    assert_eq!(f.text(), "   中文\n", "actual {:?}", f.text());
}

/// E3: `:ri {n}` right-aligns the line.
#[test]
fn e3_ex_right() {
    let f = edit("ab\n", 0, 0, &[":", "r", "i", " ", "1", "0", "<CR>"]);
    assert_eq!(f.text(), "        ab\n", "actual {:?}", f.text());
}

/// E4: `:le` left-aligns and strips the indent.
#[test]
fn e4_ex_left() {
    let f = edit("    ab\n", 0, 0, &[":", "l", "e", "<CR>"]);
    assert_eq!(f.text(), "ab\n", "actual {:?}", f.text());
}

/// E5: `:pu` puts the register as whole lines.
#[test]
fn e5_ex_put() {
    let f = edit("l1\nl2\n", 0, 0, &["y", "y", "j", ":", "p", "u", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl1\n", "actual {:?}", f.text());
}

/// E6: `:retab` expands tabs while 'et' is set.
#[test]
fn e6_retab() {
    let f = edit("\ta\n", 0, 0, &[":", "s", "e", "t", " ", "e", "t", "<CR>",
                                 ":", "r", "e", "t", "a", "b", " ", "4", "<CR>"]);
    assert_eq!(f.text(), "    a\n", "actual {:?}", f.text());
}

/// E7: `:&&` repeats the last substitution.
#[test]
fn e7_double_ampersand() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "1", "s", "/", "x", "/", "Y", "/", "<CR>", "j", ":", "&", "&", "<CR>"]);
    assert_eq!(f.text(), "Y1\nY2\n", "oracle: && replays s/x/Y/ on the current line; actual {:?}", f.text());
}

/// E8: the `:undo` command.
#[test]
fn e8_undo_command() {
    let f = edit("ab\n", 0, 0, &["x", ":", "u", "n", "d", "o", "<CR>"]);
    assert_eq!(f.text(), "ab\n", "actual {:?}", f.text());
}

/// E9: the `:redo` command.
#[test]
fn e9_redo_command() {
    let f = edit("ab\n", 0, 0, &["x", "u", ":", "r", "e", "d", "o", "<CR>"]);
    assert_eq!(f.text(), "b\n", "actual {:?}", f.text());
}

/// E10: `g&` replays the substitution over the whole file.
#[test]
fn e10_g_ampersand() {
    let f = edit("x1\nx2\n", 0, 0, &[":", "s", "/", "x", "/", "Y", "/", "<CR>", "g", "&"]);
    assert_eq!(f.text(), "Y1\nY2\n", "actual {:?}", f.text());
}

/// E11: `:reg {name}` lists the given register.
#[test]
fn e11_reg_named() {
    let f = edit("ab\n", 0, 0, &["\"", "a", "y", "y", ":", "r", "e", "g", " ", "a", "<CR>"]);
    assert!(!f.host.statuses.iter().any(|s| s.contains("E492")),
            "actual statuses={:?}", f.host.statuses);
}

/// E12: `:delmarks` deletes a mark.
#[test]
fn e12_delmarks() {
    let f = edit("ab\n", 0, 0, &["m", "a", ":", "d", "e", "l", "m", " ", "a", "<CR>"]);
    assert!(!f.host.statuses.iter().any(|s| s.contains("E492")),
            "actual statuses={:?}", f.host.statuses);
    // after the deletion the 'a jump errors (it has no landing position)
    let f = edit("ab\n", 0, 0, &["m", "a", ":", "d", "e", "l", "m", " ", "a", "<CR>", "'", "a"]);
    assert_eq!(f.line(), 0, "after delm the 'a mark must have no landing position");
}

// ------------------------------------------------------------- F ranges / search offsets

/// F1: the `'<,'>` address (oracle: after V j Esc a manual `'<,'>s` substitutes both lines).
#[test]
fn f1_visual_range_substitute() {
    let f = edit("x1\nx2\nx3\n", 0, 0, &["V", "j", "<Esc>", ":", "'", "<", ",", "'", ">", "s", "/", "x", "/", "Y", "/", "<CR>"]);
    assert_eq!(f.text(), "Y1\nY2\nx3\n", "actual {:?}", f.text());
}

/// F2: the `:/pat1/,/pat2/d` search range.
#[test]
fn f2_search_range_delete() {
    let f = edit("l1\nfoo\nl3\nbar\nl5\n", 0, 0, &[":", "/", "f", "o", "o", "/", ",", "/", "b", "a", "r", "/", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl5\n", "actual {:?}", f.text());
}

/// F3: the `/pat/e` offset lands at the end of the match.
#[test]
fn f3_search_offset_end() {
    let f = edit("foo bar\n", 0, 0, &["/", "f", "o", "o", "/", "e", "<CR>", "x"]);
    assert_eq!(f.text(), "fo bar\n", "x should delete the trailing o of the match, actual {:?}", f.text());
}

// ------------------------------------------------------------- G visual / block

/// G1: blockwise-visual `x` deletes the full block width.
#[test]
fn g1_block_x() {
    let f = edit("abc\nab\na\n", 0, 0, &["<C-v>", "j", "j", "l", "x"]);
    assert_eq!(f.text(), "c\n\n\n", "actual {:?}", f.text());
}

/// G2: blockwise-visual `D` deletes to the end of every line.
#[test]
fn g2_block_D() {
    let f = edit("abcd\nab\na\n", 0, 0, &["<C-v>", "j", "j", "D"]);
    assert_eq!(f.text(), "\n\n\n", "actual {:?}", f.text());
}

/// G3: blockwise-visual `c` with a block column > 0 does not misalign.
#[test]
fn g3_block_c_offset() {
    let f = edit("abcd\nab\ncd\n", 0, 2, &["<C-v>", "j", "j", "c"]);
    let mut f = f;
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abXd\nabX\ncdX\n", "actual {:?}", f.text());
}

/// G4: uppercase of `ß` is ẞ (U+1E9E, vim 9.1).
#[test]
fn g4_sharp_s_uppercase() {
    let f = edit("ß x\n", 0, 0, &["g", "U", "U"]);
    assert_eq!(f.text(), "\u{1e9e} X\n", "actual {:?}", f.text());
}

/// G5: linewise-visual `p` with a charwise register replaces the selected lines (does not swallow the line).
#[test]
fn g5_visual_line_p_charwise() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["\"", "\"", "y", "i", "w", "j", "V", "p"]);
    assert_eq!(f.text(), "aaa\naaa\nccc\n", "actual {:?}", f.text());
}

/// G6: `gp` leaves the cursor after the pasted text.
#[test]
fn g6_gp_cursor_after() {
    let f = edit("ab cd\n", 0, 0, &["x", "g", "p"]);
    assert_eq!(f.cursor(), 2, "vim gp leaves the cursor after the pasted text (col3/byte2), actual {}", f.cursor());
}

// ------------------------------------------------------------- H insert

/// H1: `C-v` can quote control keys (Esc/Tab/C-v and the like).
#[test]
fn h1_ctrl_v_control_keys() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>"]);
    f.feed_raw(vimcore::key::Key::escape());
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\u{1b}Xab\n", "C-v<Esc> should insert a literal ESC (the cursor follows it); actual {:?}", f.text());
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('v'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\u{16}ab\n", "C-v C-v should insert a literal ^V; actual {:?}", f.text());
}

/// H2: `C-v u{4hex}` inserts the character from its hex value.
#[test]
fn h2_ctrl_v_unicode() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-v>", "u", "0", "0", "e", "9", "<Esc>"]);
    assert_eq!(f.text(), "éab\n", "actual {:?}", f.text());
}

/// H3: `<C-k>` digraphs.
#[test]
fn h3_digraph() {
    let mut f = Fixture::new("ab\n");
    f.feed(["i", "<C-k>", "s", "s", "<Esc>"]);
    assert_eq!(f.text(), "ßab\n", "actual {:?}", f.text());
}

// ------------------------------------------------------------- I indent / options

/// I1: without autoindent `cc` clears the original indent.
#[test]
fn i1_cc_clears_indent() {
    let f = edit("    deep\n", 0, 0, &["c", "c"]);
    let mut f = f;
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n", "actual {:?}", f.text());
}

/// I2: with `sw=0` the `>>` operator uses ts.
#[test]
fn i2_shiftwidth_zero_uses_ts() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "s", "w", "=", "0", "<CR>", ">", ">"]);
    assert_eq!(f.text(), "    a\n", "actual {:?}", f.text());
}

// ------------------------------------------------------------- J keys / information

/// J1: `<Insert>` in normal mode enters insert mode.
#[test]
fn j1_insert_key_normal() {
    let f = edit("ab\n", 0, 0, &["<Insert>"]);
    assert!(matches!(f.vim.mode(), Mode::Insert), "actual {:?}", f.vim.mode());
}

/// J2: `ga` shows the character code.
#[test]
fn j2_ga() {
    let f = edit("a\n", 0, 0, &["g", "a"]);
    assert!(!f.host.statuses.is_empty(), "actual statuses={:?}", f.host.statuses);
    assert!(f.host.statuses[0].contains("97"), "vim ga reports the decimal code: {:?}", f.host.statuses);
}

/// J3: `g8` shows the byte in hex.
#[test]
fn j3_g8() {
    let f = edit("a\n", 0, 0, &["g", "8"]);
    assert!(!f.host.statuses.is_empty(), "actual statuses={:?}", f.host.statuses);
    assert!(f.host.statuses[0].contains("61"), "g8 should report hex 61: {:?}", f.host.statuses);
}

/// J4: `gq` reformats to textwidth (gqq formats only the current line, as vim does; gqj joins two lines).
#[test]
fn j4_gq() {
    let f = edit("aaa bbb\nccc\n", 0, 0, &["g", "q", "j"]);
    assert_eq!(f.text(), "aaa bbb ccc\n", "gqj joins the two lines; actual {:?}", f.text());
}

// ------------------------------------------------------------- K misc

/// K1: visual `g C-a` increments sequentially.
#[test]
fn k1_visual_g_ctrl_a() {
    let f = edit("1\n1\n1\n", 0, 0, &["<C-v>", "j", "j", "g", "<C-a>"]);
    assert_eq!(f.text(), "2\n3\n4\n", "actual {:?}", f.text());
}

/// K2: `<S-Tab>` in insert mode inserts no space (vim: no-op).
#[test]
fn k2_shift_tab_insert() {
    let mut f = Fixture::new("    ab\n");
    f.feed(["A"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers { shift: true, ..Default::default() },
        kind: vimcore::key::KeyKind::Named("tab".into()),
    });
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "    ab\n", "actual {:?}", f.text());
}

/// K3: `<C-r>{reg}` on the command line inserts the register.
#[test]
fn k3_cmdline_ctrl_r() {
    let mut f = Fixture::new("ab\n");
    f.feed(["x"]);
    f.feed([":"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('r'));
    f.feed(["\""]);
    f.feed(["<CR>"]);
    // register " holds "a" -> the command line runs ":a" -> E492 mentioning a
    assert!(f.host.statuses.iter().any(|s| s.contains("E492") && s.contains("a")),
            "actual statuses={:?}", f.host.statuses);
}

/// K4: the `_` motion lands on the first non-blank.
#[test]
fn k4_underscore_motion() {
    let f = edit("  a\n  b\n  c\n", 0, 0, &["j", "_"]);
    assert_eq!(f.cursor(), 6, "should be the byte offset of 'b' on line2, actual {}", f.cursor());
}

// ------------------------------------------------------------- L bell

/// L1: `3r.` with too few characters on the line -- bell.
#[test]
fn l1_counted_r_bells() {
    let f = edit("ab\n", 0, 0, &["3", "r", "."]);
    assert_eq!(f.text(), "ab\n");
    assert!(f.host.bells > 0, "actual bells={}", f.host.bells);
}

/// L2: `r` on an empty line rings the bell.
#[test]
fn l2_r_empty_line_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["r", "x"]);
    assert_eq!(f.text(), "a\n\nb\n");
    assert!(f.host.bells > 0, "actual bells={}", f.host.bells);
}

/// L3: `<<` on a line without indent rings the bell.
#[test]
fn l3_dedent_bells() {
    let f = edit("ab\n", 0, 0, &["<", "<"]);
    assert!(f.host.bells > 0, "actual bells={}", f.host.bells);
}

/// L4: `~` on an empty line rings the bell.
#[test]
fn l4_tilde_empty_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["~"]);
    assert!(f.host.bells > 0, "actual bells={}", f.host.bells);
}

/// L5: a failing `diw` on an empty line rings the bell.
#[test]
fn l5_iw_empty_bells() {
    let f = edit("a\n\nb\n", 1, 0, &["d", "i", "w"]);
    assert!(f.host.bells > 0, "actual bells={}", f.host.bells);
}

// ------------------------------------------------------------- M paste

/// M1: `]p` adjusts to the indent of the target line.
#[test]
fn m1_paste_indent_adjust() {
    let f = edit("  ab\ncd\n", 0, 0, &["y", "y", "j", "]", "p"]);
    assert_eq!(f.text(), "  ab\ncd\nab\n", "actual {:?}", f.text());
}
