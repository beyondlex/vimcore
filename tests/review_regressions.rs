//! Regression tests from the 2026-09 code review.
//!
//! Every test here pins a behavior that was verified against real vim 9.1
//! (macOS) before the fix, so the vim-parity contract stays executable.

// (unused-harness-helper noise is silenced inside common/mod.rs — each
// test target compiles a different subset of it)
mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

// ---- gq format operator ------------------------------------------------------

#[test]
fn gq_on_later_line_does_not_delete_text() {
    // format_lines used to feed the span's byte OFFSET into its line-index
    // loop: for any line past the first the loop was empty and gq REPLACED
    // the whole paragraph with nothing. `gqq` on line 1 must reflow it.
    let mut f = Fixture::at("keep\nsecond line here is long enough to wrap\n", 1, 0);
    f.vim.options_mut().textwidth = 12;
    f.feed(["g", "q", "q"]);
    assert_eq!(
        f.text(),
        "keep\nsecond line\nhere is long\nenough to\nwrap\n"
    );
    // vim: cursor on the first non-blank of the LAST formatted line
    assert_eq!(f.line(), 4);
}

#[test]
fn gq_cursor_lands_on_last_formatted_line() {
    let mut f = Fixture::at("the quick brown fox jumps over the lazy dog\nnext\n", 0, 0);
    f.vim.options_mut().textwidth = 20;
    f.feed(["g", "q", "g", "q"]);
    assert_eq!(
        f.text(),
        "the quick brown fox\njumps over the lazy\ndog\nnext\n"
    );
    assert_eq!(f.cursor(), 40); // the "dog" line
}

// ---- C-a / C-x number increment ----------------------------------------------

#[test]
fn c_a_on_last_digit_increments_whole_number() {
    // The digit run's START must be found: cursor on `9` of `129` used to
    // read only the `9` (129+1 producing 1210).
    let f = edit("x129y", 0, 3, &["<C-a>"]);
    assert_eq!(f.text(), "x130y");
    assert_eq!(f.cursor(), 3); // last digit of 130 ("x130": '0' at byte 3)

    // mid-number behaves the same
    let f = edit("x129y", 0, 2, &["<C-a>"]);
    assert_eq!(f.text(), "x130y");
}

#[test]
fn c_a_ignores_numbers_before_cursor() {
    // vim: nothing "at or after the cursor" -> E18, buffer unchanged.
    // The old code walked BACKWARDS and incremented the 123.
    let f = edit("123 abc", 0, 5, &["<C-a>"]);
    assert_eq!(f.text(), "123 abc");
}

#[test]
fn c_a_at_line_end_uses_trailing_number() {
    // the engine parks the cursor at the line-end byte (visually on the
    // last char), which must still count as "on" the trailing number
    let f = edit("n 42\n", 0, 4, &["<C-a>"]);
    assert_eq!(f.text(), "n 43\n");
}

// ---- J join bookkeeping --------------------------------------------------------

#[test]
fn join_updates_marks_through_edit_replace() {
    // The `J` separator replacement used to call the raw buffer, skipping
    // the mark/search-generation sync every other edit funnels through.
    // Mark `a` sits on `t` of `  two` (offset 6); after `J` the newline +
    // indent collapse and the `t` lands at offset 4.
    let mut f = Fixture::at("one\n  two\n", 0, 0);
    f.feed(["j", "0", "w", "m", "a"]); // mark on the 't' of "two"
    f.feed(["g", "g", "J"]);
    assert_eq!(f.text(), "one two\n");
    assert_eq!(f.vim.marks.get('a'), Some(4));
}

// ---- linewise put cursor --------------------------------------------------------

#[test]
fn linewise_put_cursor_on_first_pasted_line() {
    // vim 9.1 (verified for p/P, 1..4 lines): the cursor lands on the FIRST
    // line of the put text, first non-blank — not the last pasted line.
    let mut f = Fixture::at("alpha\nbeta\ngamma\n", 0, 0);
    f.feed(["y", "y", "j", "p"]);
    assert_eq!(f.text(), "alpha\nbeta\nalpha\ngamma\n");
    assert_eq!(f.cursor(), 11); // start of the pasted "alpha"

    // P pastes above: cursor on the pasted line as well
    let mut f = Fixture::at("alpha\nbeta\n", 1, 0);
    f.feed(["y", "y", "k", "P"]);
    assert_eq!(f.text(), "beta\nalpha\nbeta\n");
    assert_eq!(f.cursor(), 0); // the pasted "beta" is now line 0
}

// ---- ? + empty Enter repeats backward --------------------------------------------

#[test]
fn question_enter_repeats_search_backward() {
    // vim: an empty pattern re-runs the last search in the direction of the
    // CURRENT prompt. `?` + Enter used to repeat forward.
    let mut f = Fixture::at("b b b", 0, 0);
    f.feed(["/", "b", "<CR>"]); // forward to the second b (offset 2)
    assert_eq!(f.cursor(), 2);
    f.feed(["?", "<CR>"]);
    assert_eq!(f.cursor(), 0, "? + Enter must search backward");
    assert!(!f.vim.search.forward);
}

// ---- multi-char case mappings (ß ↔ SS) -------------------------------------------

#[test]
fn toggle_char_expands_and_shrinks_multi_char_case_mapping() {
    // `~` on ß produces SS in vim; the mapping must expand in place.
    let f = edit("aßc", 0, 1, &["~"]);
    assert_eq!(f.text(), "aSSc");

    // the byte length can also SHRINK (ẞ -> ß): the replaced range must be
    // the consumed span, or the trailing bytes would be eaten (or the old
    // `start + mapped.len()` range would even cut mid-character and panic)
    let f = edit("ẞx", 0, 0, &["~"]);
    assert_eq!(f.text(), "ßx");

    // gU over ß expands too
    let f = edit("aß", 0, 0, &["g", "U", "U"]);
    assert_eq!(f.text(), "ASS");
}

// ---- dw over trailing blanks keeps the newline ------------------------------------

#[test]
fn dw_on_trailing_blanks_keeps_newline() {
    // vim's exclusive-motion column-1 rule moves the end to the last char
    // of the previous line; the newline itself survives. The old span ended
    // at line_end + 1 and joined the lines.
    let f = edit("foo   \nbar", 0, 3, &["d", "w"]);
    assert_eq!(f.text(), "foo\nbar");
}

// ---- is_idle (the VimEdit local-undo gate) -----------------------------------------

#[test]
fn is_idle_reflects_pending_command_input() {
    let mut f = Fixture::new("foo\n");
    assert!(f.vim.is_idle());

    f.feed(["g"]);
    assert!(!f.vim.is_idle(), "`g` is a live two-key prefix");

    f.feed(["u"]); // completes the `gu` operator
    assert!(!f.vim.is_idle(), "operator waits for its motion");

    f.feed(["w"]); // guw executes
    assert!(f.vim.is_idle());

    f.feed(["\""]); // register prefix
    assert!(!f.vim.is_idle());
    f.feed(["a", "d", "d"]); // "add executes
    assert!(f.vim.is_idle());

    f.feed(["g"]);
    f.feed(["<Esc>"]);
    assert!(f.vim.is_idle(), "Esc clears pending state");
}

// ---- space key / <Space> leader normalization ---------------------------------------

#[test]
fn space_leader_from_vimrc_space_notation() {
    // `let mapleader = "<Space>"` used to shatter into S,p,a,c,e: the leader
    // notation was substituted back into the raw string and re-parsed.
    let text = "let mapleader = \"<Space>\"\nmap <Leader>q :action T.Q<CR>\n";
    let mut f = Fixture::at("foo\n", 0, 0);
    let config = vimcore::config::parse(text);
    f.vim.apply_config(&config);
    f.feed_raw(Key::named("space"));
    f.feed(["q"]);
    assert_eq!(f.host.actions, vec!["T.Q".to_owned()]);
}

#[test]
fn space_key_matches_char_space_mappings() {
    // Named("space") keystrokes (the gpui path) canonicalize to Char(' '),
    // so a mapping declared either spelling fires from either path. The RHS
    // avoids insert typing (headless fixtures place text via the IME path).
    let mut f = Fixture::at("foo\n", 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "<Space>x", "x", true);
    f.feed_raw(Key::named("space"));
    f.feed(["x"]);
    assert_eq!(f.text(), "oo\n");
}

// ---- $ / g_ land on a character start (multibyte-safe) -----------------------

#[test]
fn dollar_lands_on_last_char_not_mid_char() {
    // `$` used to return `line_end - 1` in bytes, which is INSIDE 文 for
    // `中文`: the cursor sat mid-char, the block cursor vanished, and `x`
    // deleted nothing (real vim: `$x` removes 文).
    let mut f = edit("中文", 0, 0, &["$"]);
    assert_eq!(f.cursor(), 3, "光标落在末字符 文 的起点");
    f.feed(["x"]);
    assert_eq!(f.text(), "中");

    // ASCII behavior is unchanged (end - 1 == start of the 1-byte last char)
    let f = edit("abc", 0, 0, &["$"]);
    assert_eq!(f.cursor(), 2);
}

#[test]
fn dollar_delete_span_covers_wide_last_char() {
    // `d$` from the first char must remove the WHOLE line content
    let f = edit("中文x", 0, 0, &["d", "$"]);
    assert_eq!(f.text(), "");

    // 2$ reaches the second line's last char even when multibyte
    let f = edit("ab\n中文", 0, 0, &["2", "$"]);
    assert_eq!(f.cursor(), 6);
}

#[test]
#[allow(non_snake_case)]
fn g__skips_trailing_blanks_and_lands_on_char_start() {
    // g_ goes to the last NON-BLANK char; it also used to return `end - 1`,
    // i.e. the trailing blank itself (and mid-char after a wide char).
    let f = edit("abc  ", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 2, "落在 c 上，跳过行尾空格");

    let f = edit("中文  ", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 3, "落在 文 的起点");

    let f = edit("中文", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 3);

    // all-blank line: g_ stays at the line start (vim behavior)
    let f = edit("a\n   \nb", 1, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 2);
}

// ---- <BS> as the h-twin motion + <C-c> cancels (terminal-key parity) ---------

#[test]
fn bs_moves_left_in_normal_and_operator_pending() {
    // vim notation: <BS> == <C-h> == h outside insert/cmdline. The tries had
    // no binding at all — normal <BS> only rang the bell.
    let f = edit("abc", 0, 2, &["<BS>"]);
    assert_eq!(f.cursor(), 1, "normal <BS> 左移一格");

    // d<BS> == dh: delete the char to the left
    let f = edit("abc", 0, 2, &["d", "<BS>"]);
    assert_eq!(f.text(), "ac");
    assert_eq!(f.cursor(), 1);
}

#[test]
fn bs_in_visual_shrinks_selection() {
    let f = edit("abcdef", 0, 3, &["v", "<BS>"]);
    assert_eq!(f.cursor(), 2, "选区活动端左移");
    assert!(matches!(f.vim.mode(), vimcore::Mode::Visual { .. }));
}

#[test]
fn ctrl_c_cancels_cmdline_like_esc() {
    let mut f = Fixture::at("foo\n", 0, 0);
    f.feed([":", "x"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::CommandLine { .. }));
    f.feed_raw(Key::parse("<C-c>"));
    assert!(matches!(f.vim.mode(), vimcore::Mode::Normal));
    assert_eq!(f.vim.cmdline.buffer, "");

    // search prompts behave the same, including an empty prompt
    f.feed(["/", "a"]);
    f.feed_raw(Key::parse("<C-c>"));
    assert!(matches!(f.vim.mode(), vimcore::Mode::Normal));
}

#[test]
fn ctrl_c_exits_visual() {
    let f = edit("abcdef", 0, 1, &["v", "l", "l", "<C-c>"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::Normal));
}

// ---- dw/cw crossing into an indented next line (verified against vim 9.1) ----

#[test]
fn dw_into_indented_next_line_never_joins() {
    // cursor on the trailing blanks; the landing is NOT column 1, so the
    // generic col-1 rules never fired and the span used to swallow the
    // newline + indent ("foobar")
    let f = edit("foo   \n  bar", 0, 3, &["d", "w"]);
    assert_eq!(f.text(), "foo\n  bar");
}

#[test]
fn dw_on_last_word_of_line_stops_at_line_end() {
    let f = edit("foo bar", 0, 4, &["d", "w"]);
    assert_eq!(f.text(), "foo ");
}

#[test]
fn d2w_landing_column1_empties_line_instead_of_deleting_it() {
    // vim keeps the (now empty) line: end moves to EOL of the start line
    let f = edit("foo x\nbar baz\ncorge", 0, 0, &["d", "2", "w"]);
    assert_eq!(f.text(), "\nbar baz\ncorge");
}

#[test]
fn dw_on_blank_line_with_indented_next_line_is_a_noop() {
    // vim: no join, no line deletion — the clamped span is empty
    let f = edit("foo\n\n  bar", 1, 0, &["d", "w"]);
    assert_eq!(f.text(), "foo\n\n  bar");
}

#[test]
fn dw_on_blank_line_with_column0_next_line_deletes_the_line() {
    // column-1 landing from a blank start line: LINEWISE over the blanks
    let f = edit("foo\n\nbar", 1, 0, &["d", "w"]);
    assert_eq!(f.text(), "foo\nbar");
}

#[test]
fn d3w_landing_mid_line_joins_like_vim() {
    // deeper crossings DO span the newline
    let f = edit("foo bar\nbaz qux\ncorge", 0, 0, &["d", "3", "w"]);
    assert_eq!(f.text(), "qux\ncorge");
}

#[test]
fn cw_on_trailing_blanks_keeps_newline_and_indent() {
    let mut f = edit("ab  \n  cd", 0, 2, &["c", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abX\n  cd");
}

// ---- n/N with counts (verified against vim 9.1) --------------------------------

/// `/two<CR>` jumps to line 1's match; counts then walk from there, like
/// probing vim with `let @/ = "two"` + an explicit cursor position.
#[test]
fn count_n_steps_forward_wrapping() {
    let f = edit(
        "one two\nthree two\nfour two\nfive two",
        0,
        0,
        &["/", "t", "w", "o", "<Enter>", "2", "n"],
    );
    assert_eq!(f.line(), 2, "2n from the first match lands on line 3");
}

#[test]
fn count_n_steps_backward() {
    let f = edit(
        "one two\nthree two\nfour two\nfive two",
        0,
        0,
        &["/", "t", "w", "o", "<Enter>", "2", "N"],
    );
    assert_eq!(
        f.line(),
        2,
        "2N wraps past the first match: line 4 is one step back, line 3 is two"
    );
}

#[test]
fn plain_n_backward_wraps_to_last_match() {
    let f = edit(
        "one two\nthree two\nfour two\nfive two",
        0,
        0,
        &["/", "t", "w", "o", "<Enter>", "N"],
    );
    assert_eq!(f.line(), 3, "N from the first match wraps to the last");
}

// ---- :s cursor placement (verified against vim 9.1) ----------------------------

#[test]
fn substitute_cursor_lands_on_last_substituted_line_first_non_blank() {
    // line 2 has no match, so the last substituted line is line 1;
    // vim puts the cursor at its first non-blank (col 3), regardless of
    // where the match sits in the line
    let f = edit(
        "  hello world\n  foo x\n  bar",
        0,
        0,
        &[":", "%", "s", "/", "o", "/", "0", "/", "<CR>"],
    );
    assert_eq!(f.text(), "  hell0 world\n  f0o x\n  bar");
    assert_eq!(f.line(), 1);
    assert_eq!(
        f.cursor(),
        16,
        "first non-blank of line 2 ('  f0o x') is byte 16"
    );
}

#[test]
fn substitute_cursor_survives_length_changing_earlier_lines() {
    // line 1 grows by one byte per substitution, shifting line 2; the old
    // code computed the cursor from stale pre-edit offsets and could land
    // mid-line (or mid-character on CJK)
    let f = edit(
        "ooo\noz\n",
        0,
        0,
        &[":", "%", "s", "/", "o", "/", "0", "0", "/", "g", "<CR>"],
    );
    assert_eq!(f.text(), "000000\n00z\n");
    assert_eq!(f.line(), 1);
    assert_eq!(f.cursor(), 7, "first non-blank of line 2");
}

// ---- gv after a forward (cursor right of anchor) selection ----------------------

#[test]
fn gv_restores_forward_selection() {
    // exit_visual used to re-read the selection AFTER parking the cursor on
    // the selection start, collapsing forward selections to one char
    let f = edit(
        "abcdef\nghijkl\n",
        0,
        0,
        &["v", "3", "l", "<Esc>", "g", "v"],
    );
    assert_eq!(f.vim.mode_indicator(), "VISUAL");
    let (a, c, _) = f.vim.visual_selection().unwrap();
    assert_eq!((a, c), (0, 3), "gv must restore the forward 0..3 selection");

    // backward selections kept working before; keep them covered
    let f = edit("abcdef\nghijkl\n", 0, 3, &["v", "b", "<Esc>", "g", "v"]);
    let (a, c, _) = f.vim.visual_selection().unwrap();
    // gv re-anchors at the range start; the SPAN is what must survive
    assert_eq!((a, c), (0, 3), "backward gv keeps the same span");
}

// ---- <c-a> lowercase modifier spellings ------------------------------------------

#[test]
fn angle_keys_accept_lowercase_modifier_prefixes() {
    use vimcore::key::{Key, KeyKind};
    // rc files use <c-a>, <C-a>, <C-A> interchangeably; the old parser was
    // case-sensitive and turned <c-a> into Named("c-a")
    assert_eq!(Key::parse("<c-a>"), Key::parse("<C-a>"));
    assert_eq!(Key::parse("<C-a>").kind, KeyKind::Char('a'));
    assert!(Key::parse("<c-a>").modifiers.control);
    assert!(Key::parse("<C-S-a>").modifiers.control);
    assert!(Key::parse("<C-S-a>").modifiers.shift);
    // a plain <s-x> keeps the shift flag on the char
    assert!(Key::parse("<s-x>").modifiers.shift);
    assert_eq!(Key::parse("<s-x>").kind, KeyKind::Char('x'));
}

// ---- * on a line without words: must not re-jump with the stale pattern ---------

#[test]
fn star_without_word_bells_instead_of_reusing_stale_pattern() {
    // `/two` arms the pattern; `*` on a punctuation-only line must not
    // re-search "two". 【第十四轮修正】vim 9.1 探针 Q1：光标在非词非空白
    // 字符上时 `*` **搜该字符的字面**（`*` on '!' 的 pattern 是 `!`）——
    // 现在引擎搜 `\.`（三个点全高亮、跳到下一个点），不再复用旧 pattern；
    // 不变式「不复用陈旧 pattern」保持。
    let mut f = Fixture::at("one two\n...\n", 0, 0);
    f.feed(["/", "t", "w", "o", "<Enter>"]); // on line 1's "two"
    f.feed(["j"]); // line 2: "..."
    f.feed(["*"]);
    assert_eq!(
        f.vim.search.pattern.as_deref(),
        Some(r"\."),
        "* 在 . 上搜字面点，而非复用 two"
    );
    assert_eq!(
        f.host.highlights.len(),
        3,
        "高亮 = 三个点（字面 .），不是 /two 的一个匹配"
    );
}

// ---- absurd counts must not panic -----------------------------------------------

#[test]
fn huge_counts_saturate_instead_of_overflowing() {
    let f = edit(
        "ab\ncd\n",
        0,
        0,
        &[
            "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9",
            "9", "9", "9", "d", "d",
        ],
    );
    // saturates far past the line count; dd clamps to the last line
    assert_eq!(f.text(), "");

    let f = edit("x99999999999999999999\n", 0, 1, &["<C-a>"]);
    // literal overflows i64: saturates at i64::MAX instead of wrapping
    // negative (the exact rendering is unspecified; must not panic)
    assert!(f.text().starts_with('x'));
}

// ---- visual-block A pads short rows (verified against vim 9.1) ------------------

#[test]
fn block_append_pads_short_rows_to_block_edge() {
    // block cols 3-5; row "ab" is short: vim pads it with 3 spaces then
    // appends — old code just appended at the line end ("abX")
    let mut f = Fixture::at("long1\nab\nlong2\n", 0, 2);
    f.feed(["<C-v>", "j", "e", "A"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "long1X\nab   X\nlong2X\n");
}

#[test]
fn block_insert_on_short_row_lands_at_line_end_without_padding() {
    // `I` on a short row: vim does NOT pad, it inserts at the line end
    let mut f = Fixture::at("long1\nab\nlong2\n", 0, 2);
    f.feed(["<C-v>", "j", "e", "I"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    // X lands at the block's LEFT edge (col 3), short rows at their end
    assert_eq!(f.text(), "loXng1\nabX\nloXng2\n");
}

// ---- o/O copies the indent verbatim (tabs stay tabs) ----------------------------

#[test]
fn open_line_preserves_tab_indent() {
    let mut f = Fixture::at("\tfoo\n", 0, 1);
    f.feed(["o"]);
    f.type_text("bar");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\tfoo\n\tbar\n");
}

// ---- blockwise register p/P in normal mode (verified against vim 9.1) -----------

#[test]
fn blockwise_put_creates_padded_rows_below() {
    // yank the 2x2 block (ab/cd), put after cursor col 2 of "gh":
    // row 1 joins the cursor line, row 2 becomes a padded new line below
    let mut f = Fixture::at("ab\ncd\nef\ngh\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "y"]); // block ["ab", "cd"]
    f.feed(["G"]); // last line
    f.feed(["l", "p"]); // col 2, put after
    assert_eq!(f.text(), "ab\ncd\nef\nghab\n  cd\n");
    assert_eq!(
        f.cursor(),
        11,
        "cursor on the first pasted char (line 4, byte 11)"
    );
}

#[test]
fn blockwise_put_before_inserts_at_cursor_column() {
    let mut f = Fixture::at("ab\ncd\nef\ngh\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "y"]);
    f.feed(["G", "P"]);
    assert_eq!(f.text(), "ab\ncd\nef\nabgh\ncd\n");
}

#[test]
fn blockwise_put_count_repeats_rows_horizontally() {
    let mut f = Fixture::at("ab\ngh", 0, 0);
    f.feed(["<C-v>", "l", "y"]);
    f.feed(["G", "2", "p"]);
    // p puts one column right of the cursor char: "g" | "abab" | "h"
    assert_eq!(f.text(), "ab\ngababh");
}

// ---- r<CR> replaces with a line break (vim splits the line) ----------------------

#[test]
fn r_enter_splits_the_line() {
    let f = edit("abc\ndef\n", 0, 1, &["r", "<CR>"]);
    assert_eq!(f.text(), "a\nc\ndef\n");
}

// ---- <Space> moves right like l --------------------------------------------------

#[test]
fn space_moves_right_like_l() {
    let f = edit("ab\ncd\n", 0, 0, &[" "]);
    assert_eq!(f.cursor(), 1);
    // and stops at the line end, like vim
    let f = edit("ab\ncd\n", 0, 1, &[" "]);
    assert_eq!(f.cursor(), 1);
    // operator form: d<Space> is dl
    let f = edit("abc", 0, 0, &["d", " "]);
    assert_eq!(f.text(), "bc");
}

// ---- random-key fuzz: engine invariants under adversarial sequences -------------
//
// Deterministic xorshift hammering over CJK/ASCII/emoji buffers, mixing the
// IME typing path. Guards the class of bugs found in review: mid-character
// offsets leaking into host callbacks (visual-range ends, stored marks,
// undo/redo cursors, span end-1 arithmetic).

#[test]
fn fuzz_random_key_sequences_hold_invariants() {
    // deterministic pseudo-random key hammering over CJK/ASCII mixed buffer
    let keys: Vec<&str> = [
        "h", "j", "k", "l", "w", "b", "e", "0", "$", "^", "g", "G", "d", "c", "y", "p", "P", "x",
        "X", "s", "S", "D", "C", "r", "a", "i", "o", "O", "v", "V", "<C-v>", "u", "<Esc>", "J",
        "gJ", ">", "<", "gu", "gU", "g~", "gqq", "~", "f", "t", "F", "T", ";", "%", "n", "N", "*",
        "#", "iw", "aw", "i\"", "a\"", "i(", "a(", "it", "ip", "ap", "dd", "dw", "yy", "cc", "cw",
        "gg", "zz", "zt", "zb", "m", "`", "'", "q", "@", ".", "<C-a>", "<C-x>", "<C-r>", "<C-o>",
        "<C-i>", "1", "2", "3", "9", "/", "?", "<CR>", ":", "noh", "w", "q", "<C-e>", "<C-y>",
        "<C-f>", "<C-b>", "<C-d>", "<C-u>", "ge", "g_",
    ]
    .to_vec();
    let mut state: u64 = 0x5EED_2026;
    for round in 0..500 {
        let initial = match round % 7 {
            0 => "",
            1 => "a",
            2 => "中",
            3 => "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
            4 => "x\n",
            5 => "éA→ ç\nｱｲｳ\n\tindent",
            _ => "你好, world 123 -45\n#tag\"",
        };
        let mut f = common::Fixture::new(initial);
        // jump cursor somewhere interesting
        let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
        f.vim.set_cursor_offset(&f.buf, cur);
        for _ in 0..160 {
            let k = keys[(fuzz_xorshift(&mut state) as usize) % keys.len()];
            if std::env::var("FUZZ_TRACE").is_ok() {
                eprintln!("key={k} text={:?} cur={}", f.text(), f.vim.cursor_offset());
            }
            // multi-char entries ("dd", "iw", "<C-a>") must become REAL key
            // sequences — feeding them through `feed([k])` would parse the
            // whole string as one junk Named key and silently test nothing
            for key in vimcore::key::parse_key_sequence(k) {
                f.feed_raw(key);
            }
            // exercise the IME text path too: while in insert/replace, type
            // some text the way a host delivers composed input
            if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                && fuzz_xorshift(&mut state).is_multiple_of(4)
            {
                f.type_text("tx中");
            }
            // invariants
            let text = f.text();
            assert!(std::str::from_utf8(text.as_bytes()).is_ok());
            assert!(vimcore::buffer::VimBuffer::line_count(&f.buf) >= 1);
            let co = f.vim.cursor_offset();
            assert!(
                co <= text.len(),
                "cursor {co} past end in round {round} after {k}"
            );
            if co < text.len() {
                assert!(
                    text.is_char_boundary(co),
                    "cursor {co} mid-char in round {round} after {k} (text {text:?})"
                );
            }
            // stored mark offsets must stay addressable too (they are floored
            // at use, but a corrupt one means the adjust funnels missed)
            for (name, off) in f.vim.marks.items() {
                assert!(
                    off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
                    "mark {name} at {off} invalid in round {round} after {k} (text {text:?})"
                );
            }
            if let Some((lo, hi, _)) = f.vim.marks.last_visual {
                assert!(
                    lo <= text.len()
                        && (lo == text.len() || text.is_char_boundary(lo))
                        && hi <= text.len()
                        && (hi == text.len() || text.is_char_boundary(hi)),
                    "last_visual {lo}..{hi} invalid in round {round} after {k} (text {text:?})"
                );
            }
        }
    }
}

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// ---- unbounded-allocation guards (第十轮审查) ---------------------------------

/// count-repeat insert 的复制体积必须有字节上限：`99999999i` + 长文本 + <Esc>
/// 走 `replicate_count_insert`，过去对 `text.repeat(copies)` 不设防，一个小会话
/// 就能让宿主吃掉数 GB（寄存器粘贴路径早有 16MB 的 `clamped_repeat` 防线）。
#[test]
fn count_insert_repeat_volume_is_byte_capped() {
    let mut f = Fixture::new("x\n");
    let payload = "abcdefghij".repeat(10); // 100 bytes typed once
    f.feed(["9", "9", "9", "9", "9", "9", "9", "9", "i"]); // count 99999999
    f.type_text(&payload);
    f.feed(["<Esc>"]);
    let len = f.text().len();
    assert!(
        len < 17 * 1024 * 1024,
        "count-insert replicated {len} bytes — no cap"
    );
    // 复制本身仍然发生（cap 之内），不等于完全禁用
    assert!(len > payload.len(), "replication should still happen");
}

/// `:set sw=` 巨值曾被原样接受，随后 `>>` 用 `" ".repeat(sw)` 生成缩进——
/// 现在数值选项在 set_value 边界截到 1e6。
#[test]
fn numeric_option_values_are_capped() {
    let mut f = Fixture::new("a\nb\n");
    f.feed([
        ":", "s", "e", "t", " ", "s", "w", "=", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9",
        "9", "<CR>",
    ]);
    assert_eq!(f.vim.options_mut().shiftwidth, 1_000_000);
    // 缩进本身可用：1e6 以内的值原样生效（>> 只缩进当前行，vim 同款）
    f.feed([
        ":", "s", "e", "t", " ", "s", "w", "=", "2", "<CR>", ">", ">",
    ]);
    assert_eq!(f.vim.options_mut().shiftwidth, 2);
    assert_eq!(f.text(), "  a\nb\n");
}

// ---- insert 模式编辑后的搜索缓存同步（fuzz round 10 抓到的陈旧缓存） ---------

/// insert 模式的 <Del>/<BS>/<C-w>/<C-u> 过去不重跑 hlsearch 扫描：缓存的
/// `last_matches` 留着编辑前的偏移，删除恰好让旧偏移落进多字节字符中间
/// （"a中b" 删 'a' 后缓存的 1..4 指进 中 的字节内部）。`cancel_cmdline` 会把
/// 这份缓存原样发布给宿主——宿主对高亮做 offset_to_line 即 panic。
#[test]
fn insert_mode_deletes_refresh_search_cache() {
    // /中/ 命中 1..4；删掉 'a' 后新文本 "中b\n" 的命中是 0..3
    let mut f = Fixture::new("a中b\n");
    f.feed(["/", "中", "<CR>"]);
    assert_eq!(f.vim.cursor_offset(), 1);
    f.feed(["g", "g"]);
    f.feed(["i"]);
    f.feed_raw(Key::named("delete")); // <Del> 删 'a'
    let text = f.text();
    assert_eq!(text, "中b\n");
    for m in &f.vim.search.last_matches {
        assert!(
            text.is_char_boundary(m.start) && text.is_char_boundary(m.end),
            "stale cached match {m:?} in {text:?}"
        );
    }
    assert_eq!(f.vim.search.last_matches, vec![0..3]);
    // 宿主看到的高亮同样已刷新
    assert_eq!(f.host.highlights, vec![0..3]);
}

/// 同类路径：行内 BS 也必须刷新缓存（修的是同一缺口，两条入口都钉住）。
#[test]
fn insert_mode_backspace_refreshes_search_cache() {
    let mut f = Fixture::new("a中b\n");
    f.feed(["/", "中", "<CR>"]); // 光标停在 中 (offset 1)
    f.feed(["i"]);
    f.feed_raw(Key::named("backspace")); // BS 删掉前面的 'a'
    let text = f.text();
    assert_eq!(text, "中b\n");
    for m in &f.vim.search.last_matches {
        assert!(
            text.is_char_boundary(m.start) && text.is_char_boundary(m.end),
            "stale cached match {m:?} in {text:?}"
        );
    }
    assert_eq!(f.vim.search.last_matches, vec![0..3]);
}

// ---- 零宽替换（:s/^/x/ 家族）------------------------------------------------

/// 零宽匹配过去被当作"无替换"跳过且不计入 hits：`` :s/^/>/ `` 静默不动还误报
/// E486。vim 里这是行首/行尾插入的标准惯用法，必须照常替换。
#[test]
fn substitute_at_caret_prepends_to_the_line() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed([":", "s", "/", "^", "/", ">", "/", "<CR>"]);
    assert_eq!(f.text(), ">hello\nworld\n");
    // vim: exactly one substitution reports nothing (status stays empty)
    assert_eq!(f.host.statuses, Vec::<String>::new());
}

#[test]
fn substitute_dollar_appends_across_range() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed([":", "%", "s", "/", "$", "/", "<", "/", "<CR>"]);
    assert_eq!(f.text(), "hello<\nworld<\n");
    assert_eq!(f.host.statuses, ["2 substitutions on 2 lines"]);
}

/// `a*` 在无 `a` 文本上每处都是零宽匹配：vim `:s/a*/-/` 得 "-bbb"。
#[test]
fn substitute_star_pattern_replaces_empty_match() {
    let mut f = Fixture::new("bbb\n");
    f.feed([":", "s", "/", "a", "*", "/", "-", "/", "<CR>"]);
    assert_eq!(f.text(), "-bbb\n");
    assert_eq!(f.host.statuses, Vec::<String>::new());
}

// ---- 未设置 mark 的范围报错（vim 实证 E20）-----------------------------------

/// `:'<,'>d` 在从未进过 visual 的缓冲上：vim 报 `E20: Mark '< not set`，
/// 引擎过去一律 E16（把"mark 未设置"和"范围语法错误"混为一谈）。
#[test]
fn range_with_unset_visual_marks_reports_e20() {
    let mut f = Fixture::new("abc\n");
    f.feed([":", "'", "<", ",", "'", ">", "d", "<CR>"]);
    assert_eq!(f.host.statuses, ["E20: Mark '< not set"]);
    assert_eq!(f.text(), "abc\n"); // 缓冲不动
}

/// 普通命名 mark 同理（vim: E20: Mark 'a not set）。
#[test]
fn range_with_unset_named_mark_reports_e20() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed([":", "'", "a", ",", "$", "d", "<CR>"]);
    assert_eq!(f.host.statuses, ["E20: Mark 'a not set"]);
    assert_eq!(f.text(), "abc\ndef\n");
}

// ---- Ex 范围语义三连（vim 9.1 探针实证，第十一轮）---------------------------

/// 空命令带范围：vim 光标落范围的**最后一个地址**（`:2,5<CR>` 落第 5 行）。
/// 引擎过去跳到 `first`，`:2,5` 落第 1 行。
#[test]
fn bare_range_command_moves_to_last_address() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    f.feed([":", "2", ",", "5", "<CR>"]);
    assert_eq!(f.line(), 4, ":2,5<CR> 光标落第 5 行（末地址）");

    // 单地址 `:5` 行为不变
    let mut f = Fixture::new("l1\nl2\nl3\n");
    f.feed([":", "2", "<CR>"]);
    assert_eq!(f.line(), 1);
}

/// 多地址范围：vim 只保留**最后两个**地址（`:1,2,3d` 于 ['a','b','c','d']
/// 删第 2-3 行，探针实证）。引擎过去保留首尾，删 1-3 行。
#[test]
fn multi_address_range_keeps_last_two() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed([":", "1", ",", "2", ",", "3", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\n");
}

/// 空地址默认当前行（两侧皆然）：`:,3d` 于第 2 行删 2-3、`:2,d` 于第 3 行
/// 删 2-3（vim 探针）。跳过空段让两条命令都只删了单行。
#[test]
fn empty_range_address_defaults_to_cursor_line() {
    let mut f = Fixture::at("a\nb\nc\nd\ne\n", 1, 0);
    f.feed([":", ",", "3", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\ne\n");

    let mut f = Fixture::at("a\nb\nc\nd\ne\n", 2, 0);
    f.feed([":", "2", ",", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\ne\n");
}

// ---- `&` / 裸 `:s` 重放丢弃上一条的旗标（vim 探针）--------------------------

/// vim 9.1：`s/a/B/g` 之后 `&` 于 "xaxax" 只替换首个匹配——`g` **不**随
/// `&` 重放（`:h :&`）。引擎过去原样重放整条命令行，`g` 被保留。
#[test]
fn ampersand_repeat_drops_substitute_flags() {
    let mut f = Fixture::new("xaxax\nxaxax\n");
    f.feed([":", "s", "/", "a", "/", "B", "/", "g", "<CR>"]);
    assert_eq!(f.text(), "xBxBx\nxaxax\n");
    f.feed(["j", "&"]);
    assert_eq!(f.text(), "xBxBx\nxBxax\n", "& 重放不带 g 旗标");
}

/// 裸 `:s` 与 `&` 同款：不带上一条的旗标。
#[test]
fn bare_colon_s_repeat_drops_substitute_flags() {
    let mut f = Fixture::new("xaxax\nxaxax\n");
    f.feed([":", "s", "/", "a", "/", "B", "/", "g", "<CR>"]);
    f.feed(["j", ":"]);
    f.feed(["s", "<CR>"]);
    assert_eq!(f.text(), "xBxBx\nxBxax\n");
}

// ---- mark 跳转作为算子目标（d'a / d`a / y'a / c'a，vim 探针实证）-------------

/// `d'a` 行级删除「当前行..mark 行」，与方向无关（vim 9.1：光标第 2 行、
/// mark 第 3 行时删 2-3 行）。旧实现在 complete_char_arg 里只挪光标，
/// 算子悬空——`d'a` 静默不动，下一个键还会被悬空的 d 污染。
#[test]
fn operator_with_mark_jump_deletes_through_mark_line() {
    // 光标第 1 行、mark 第 2 行：删 1-2 行
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["j", "m", "a", "k", "d", "'", "a"]);
    assert_eq!(f.text(), "ccc\n");

    // 反方向：光标第 2 行、mark 第 1 行，删的也是两行
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["m", "a", "j", "d", "'", "a"]);
    assert_eq!(f.text(), "ccc\n");
}

/// `y'a` 行级 yank 到 mark 行；`p` 把两行贴回去。
#[test]
fn yank_with_mark_jump_is_linewise() {
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["m", "a", "j", "k", "y", "'", "a", "j", "p"]);
    assert_eq!(f.text(), "aaa\nbbb\naaa\nccc\n");
}

/// ``d`a`` 字级删「光标..mark」排他区间（vim 探针：'one two' 的 two 的 t
/// 上设 mark，下一行行首 ``d`a`` → 'one three four'）。
#[test]
fn operator_with_backtick_mark_is_charwise_exclusive() {
    let mut f = Fixture::new("one two\nthree four\n");
    f.feed(["0", "f", "t", "m", "a", "j", "0", "d", "`", "a"]);
    assert_eq!(f.text(), "one three four\n");
}

/// mark 未设置时 `d'a` 响铃且缓冲不动（plain `'x` 同款反馈）。
#[test]
fn operator_with_unset_mark_bells_without_editing() {
    let mut f = Fixture::new("aaa\nbbb\n");
    f.feed(["d", "'", "x"]);
    assert_eq!(f.text(), "aaa\nbbb\n");
    assert!(f.host.bells > 0);
}

// ---- <C-a> 光标落在进制前缀字母上（vim 9.1 探针实证）-------------------------

/// vim 把 `0x1f` 的前缀字母 `x` 也视为「数字在光标处」：C-a 增的是整个
/// 字面量（0x1f → 0x20）。旧实现把 `x` 当普通文本，向前抓到裸数字 `1`，
/// 拼出 `00x2f` 这样的垃圾。`0XAB`/`0b101` 同理；前缀后无合法数位时
/// （`0backup` 的 `b`）不算数字（vim 不动）。
#[test]
fn c_a_on_radix_prefix_letter_increments_whole_literal() {
    let mut f = Fixture::new("v 0x1f\n");
    f.feed(["0", "l", "l", "l", "<C-a>"]);
    assert_eq!(f.text(), "v 0x20\n");

    let mut f = Fixture::new("v 0XAB\n");
    f.feed(["0", "l", "l", "l", "<C-a>"]);
    assert_eq!(f.text(), "v 0XAC\n");

    let mut f = Fixture::new("v 0b101\n");
    f.feed(["0", "l", "l", "l", "2", "<C-a>"]);
    assert_eq!(f.text(), "v 0b111\n");
}

/// 前缀字母的假阳性守卫：`0b` 后面不是 0/1 时不构成数字（vim: 缓冲不动，
/// E18 报告）。
#[test]
fn c_a_on_bogus_binary_prefix_finds_no_number() {
    let mut f = Fixture::new("0backup\n");
    f.feed(["0", "l", "<C-a>"]);
    assert_eq!(f.text(), "0backup\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E18")));
}

/// 光标在 `077` 的前导 `0` 上：八进制 +1 = `0100`（宽度自然生长）。
#[test]
fn c_a_on_octal_leading_zero_grows_width() {
    let mut f = Fixture::new("x077\n");
    f.feed(["0", "l", "<C-a>"]);
    assert_eq!(f.text(), "x0100\n");
}

// ---- navigation keys (arrows / Del) vs the `.` record and operators ----------

/// 方向键不属于「修改」，vim 的 redo 缓冲永远不含纯 motion：`<Down>` 后 `x`
/// 再 `.`，只重放 `x`（在当前光标删一个字符），不下移。旧实现把
/// `<Down>` 留在未提交的 recording 里，被下一条修改命令一并烤进
/// `last_change`，`.` 变成「下移+删字符」。
#[test]
fn arrow_key_never_leaks_into_dot_replay() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["down", "x", "."]);
    assert_eq!(f.text(), "abc\nf\nghi\n");
}

/// 同一污染窗口的镜像：修改提交后按方向键、再按一个纯 motion，
/// 未提交的方向键不会活到下一条命令（此前依赖「后续 motion 的提交顺带
/// 清掉」，方向键自身提交后该窗口彻底关闭）。
#[test]
fn arrow_key_after_commit_does_not_replay() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 1);
    f.feed(["x", "down", "j", "."]);
    // `.` repeats just the `x`: deletes 'h' on line 3 (cursor moved 2 down)
    assert_eq!(f.text(), "ac\ndef\ngi\n");
}

/// `<Del>` 是完整的删除命令：自身可被 `.` 重复（旧实现从不提交
/// change record，`.` 重放的还是上上条命令或什么都没有）。
#[test]
fn delete_key_is_repeatable_by_dot() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed_raw(Key::named("delete"));
    f.feed(["."]);
    assert_eq!(f.text(), "c\n");
}

/// Del 删除后紧跟的下一命令的 `.` 记录不被 Del 污染。
#[test]
fn delete_key_pollutes_no_later_record() {
    let mut f = Fixture::at("abc\ndef\n", 0, 0);
    f.feed_raw(Key::named("delete"));
    f.feed(["j", "d", "d"]);
    assert_eq!(f.text(), "bc\n");
    f.feed(["."]); // replays just dd
    assert_eq!(f.text(), "");
}

/// 操作符可以吃方向键作为 motion：`d<Down>` = `dj`（vim 同款）。旧实现里
/// 方向键绕过 operator 管线直接移动光标，操作符悬空、下一个键响铃取消。
#[test]
fn operator_completes_with_arrow_motion() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["d"]);
    f.feed_raw(Key::named("down"));
    assert_eq!(f.text(), "ghi\n");
}

/// 前缀计数并入 motion 计数走完整管线：`2d<Down>` = `d2<Down>`，从第 3 行
/// 下移 2 行 → linewise 删 l3..l5 共 3 行（vim 计数规则：2d3w = 6w）。
#[test]
fn operator_arrow_count_merges() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\nl5\nl6\nl7\n", 2, 0);
    f.feed(["2"]);
    f.feed(["d"]);
    f.feed_raw(Key::named("down"));
    assert_eq!(f.text(), "l1\nl2\nl6\nl7\n");
}

// ---- 第十九轮：D/C 行模型（vim 9.1 字节级探针）/ r<CR> 计数 / r<C-E>/r<C-Y> ----

/// `D` 在末行（截断存活）保留文件的结尾换行——引擎的尾 `\n` 就是宿主的
/// 文件 eol，vim 9.1 字节探针：`abc\ndef\n` 末行 `D` → 文件 `abc\nde\n`。
/// 旧实现吞掉换行，宿主保存后文件丢 eol。
#[test]
fn d_on_last_line_keeps_file_eol() {
    let f = edit("abc\ndef\n", 1, 2, &["D"]);
    assert_eq!(f.text(), "abc\nde\n");
}

/// `99D` 从行中触达缓冲末：截断的首行 + eol 一起存活（vim 探针 → "a\n"）。
#[test]
fn count_d_to_eof_keeps_eol() {
    let f = edit("aaaa\nbbbb\n", 0, 1, &["9", "9", "D"]);
    assert_eq!(f.text(), "a\n");
}

/// `D` 于行首（下方有存活行）= 整行删除，不留幻影空行（vim 探针：
/// ['a','b','c'] 行首 `2D` → ['c']）。旧实现留下一个空首行。
#[test]
fn d_at_line_start_deletes_covered_lines_whole() {
    let f = edit("a\nb\nc\n", 0, 0, &["2", "D"]);
    assert_eq!(f.text(), "c\n");
    let f = edit("aaaa\nbbbb\ncccc\ndddd\n", 0, 0, &["3", "D"]);
    assert_eq!(f.text(), "dddd\n");
    let f = edit("aaaa\nbbbb\n", 0, 0, &["9", "9", "D"]);
    assert_eq!(f.text(), "");
}

/// `D` count=1 于行首 = d$ 语义：行被清空但保留（vim PTY 探针
/// ['a','b','c'] 行首 `D` → "\nb\nc\n"，光标行不消失）。
#[test]
fn d_count_one_at_line_start_empties_the_line() {
    let f = edit("a\nb\nc\n", 0, 0, &["D"]);
    assert_eq!(f.text(), "\nb\nc\n");
}

/// 唯一行缓冲的行首 `D`：行清空但保留（vim 保持 ≥1 行，探针 → "\n"）。
#[test]
fn d_at_sole_line_start_keeps_empty_line() {
    let f = edit("abc\n", 0, 0, &["D"]);
    assert_eq!(f.text(), "\n");
}

/// `C` 与 `D` 在行首分化：C 清空行进入插入（vim 探针 `C` 行首 + z →
/// "z\nb\nc\n"），行永不消失；中间覆盖行仍然整行消失（round-9 既有 2C
/// 探针 + round19 `2Cnew` 行首探针 → ['new','cccc','dddd']）。
#[test]
fn c_at_line_start_types_on_the_emptied_line() {
    let mut f = Fixture::at("a\nb\nc\n", 0, 0);
    f.feed(["C"]);
    f.type_text("z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "z\nb\nc\n");
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd\n", 0, 0);
    f.feed(["2", "C"]);
    f.type_text("new");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "new\ncccc\ndddd\n");
}

/// `N r <CR>` 塌缩为单个换行——`:h r` 明文 "5r<CR> replaces five
/// characters with a single line break"（vim 字节探针：`3r<CR>` 于
/// "abcdef" → 一个空行 + "def"）。旧实现产出 N 个换行。
#[test]
fn count_r_cr_is_one_break() {
    let f = edit("abcdef\n", 0, 0, &["3", "r", "<CR>"]);
    assert_eq!(f.text(), "\ndef\n");
    // 单个 r<CR> 光标落新行首（既有语义，顺带钉住塌缩后的落点）
    let f = edit("abc\n", 0, 1, &["r", "<CR>"]);
    assert_eq!(f.text(), "a\nc\n");
    assert_eq!(f.cursor(), 2); // first char of the new next line
}

/// `r<C-E>` / `r<C-Y>`：替换字符取自下/上行同显示列（`:h r`；
/// `10r<C-E>` 复制下方 10 个字符）。任一侧不够则整条取消（同
/// `3rx` 只剩两字符的取消语义）。
#[test]
fn r_ctrl_e_y_copies_from_neighbor_line() {
    // 下方：'a' ← 'X'
    let f = edit("abc\nXYZ\n", 0, 0, &["r", "<C-e>"]);
    assert_eq!(f.text(), "Xbc\nXYZ\n");
    // 上方：'Y' ← 'b'
    let f = edit("abc\nXYZ\n", 1, 1, &["r", "<C-y>"]);
    assert_eq!(f.text(), "abc\nXbZ\n");
    // 计数：3r<C-E> 复制 3 个字符
    let f = edit("abc\nXYZW\n", 0, 0, &["3", "r", "<C-e>"]);
    assert_eq!(f.text(), "XYZ\nXYZW\n");
    // 下方行太短 → 整条取消 + 响铃
    let mut f = Fixture::at("abc\nXY\n", 0, 0);
    f.feed(["3", "r", "<C-e>"]);
    assert_eq!(f.text(), "abc\nXY\n");
    assert!(f.host.bells > 0);
}

/// 未设 mark 的 `'z` / `` `z `` 走 vim 的消息通道（E20），不再只有哑铃。
#[test]
fn unset_mark_jump_reports_e20() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["'", "z"]);
    assert_eq!(f.text(), "abc\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E20")));
    assert!(f.host.statuses.iter().any(|s| s.contains('z')));
}

/// `:j` 于末行（无接缝可接）是纯 no-op：不进 changelist、不推 `.` mark。
/// 旧实现照常 bump，changelist 被无操作污染。
#[test]
fn ex_join_at_eof_is_a_clean_noop() {
    let mut f = Fixture::at("a\nb\nc\n", 0, 0);
    f.feed(["x"]); // a real change: "\nb\nc\n", changelist = the x site
    f.feed([":", "3", "j", "<CR>"]); // last line: no seam to join
    assert_eq!(f.text(), "\nb\nc\n");
    // `.` still replays the x — the no-op join took over neither the
    // changelist nor the `.` mark (cursor re-anchored so the replay lands
    // on "b" regardless of where :3j parked it)
    f.feed(["g", "g", "j", "."]);
    assert_eq!(f.text(), "\n\nc\n");
}
