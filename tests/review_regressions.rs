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
    // `/two` arms the pattern; `*` on a punctuation-only line used to jump
    // again with "two" instead of failing
    let mut f = Fixture::at("one two\n...\n", 0, 0);
    f.feed(["/", "t", "w", "o", "<Enter>"]); // on line 1's "two"
    f.feed(["j"]); // line 2: "..."
    let line_before = f.line();
    f.feed(["*"]);
    assert_eq!(
        f.line(),
        line_before,
        "* must not move: no word on the line"
    );
    assert_eq!(
        f.host.highlights.len(),
        1,
        "highlights still describe /two — no stale re-jump happened"
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
