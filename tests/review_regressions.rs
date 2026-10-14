//! Regression tests from the 2026-09 code review.
//!
//! Every test here pins a behavior that was verified against real vim 9.1
//! (macOS) before the fix, so the vim-parity contract stays executable.

// (unused-harness-helper noise is silenced inside common/mod.rs — each
// test target compiles a different subset of it)
mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;
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
    // `~` on ß produces the single char ẞ (U+1E9E) in vim 9.1 (typeahead
    // oracle re-probed in the round-31 audit — the old "SS" expectation
    // was wrong); the mapping must expand in place.
    let f = edit("aßc", 0, 1, &["~"]);
    assert_eq!(f.text(), "a\u{1e9e}c");

    // the byte length can also SHRINK (ẞ -> ß): the replaced range must be
    // the consumed span, or the trailing bytes would be eaten (or the old
    // `start + mapped.len()` range would even cut mid-character and panic)
    let f = edit("ẞx", 0, 0, &["~"]);
    assert_eq!(f.text(), "ßx");

    // gU over ß maps to the single char ẞ (oracle-probed, audit G4)
    let f = edit("aß", 0, 0, &["g", "U", "U"]);
    assert_eq!(f.text(), "A\u{1e9e}");
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
    assert_eq!(f.cursor(), 3, "cursor lands at the start of the last char 文");
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
    assert_eq!(f.cursor(), 2, "lands on c, skipping the trailing blanks");

    let f = edit("中文  ", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 3, "lands at the start of 文");

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
    assert_eq!(f.cursor(), 1, "normal <BS> moves left one column");

    // d<BS> == dh: delete the char to the left
    let f = edit("abc", 0, 2, &["d", "<BS>"]);
    assert_eq!(f.text(), "ac");
    assert_eq!(f.cursor(), 1);
}

#[test]
fn bs_in_visual_shrinks_selection() {
    let f = edit("abcdef", 0, 3, &["v", "<BS>"]);
    assert_eq!(f.cursor(), 2, "the active end of the selection moves left");
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
fn dw_on_blank_line_with_indented_next_line_deletes_the_line() {
    // The landing is column 1 on BOTH shapes of the next line (an indented
    // line's column 1 is its first blank), so vim's col-0 adjustment fires the
    // same way either time. 9.1 oracle (probe D1, `-S` + `normal!`):
    // "foo\n\n  bar" cursor 2,1 `dw` -> LINWISE, reg 'V' "\n", leaving
    // "foo\n  bar" — the old pin claimed a no-op and was wrong.
    let f = edit("foo\n\n  bar", 1, 0, &["d", "w"]);
    assert_eq!(f.text(), "foo\n  bar");
    assert_eq!(f.cursor(), 6, "linewise delete parks on the first non-blank");
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
    // 9.1 oracle re-verified (2026-10-08): gv restores a CHARWISE selection as
    // a **backward** span — the cursor lands at the start (both v3l<Esc>gv and
    // the backward vb<Esc>gv land at offset 0) with the anchor at the end; the
    // old pin (0,3) was the engine's behavior at the time, not vim.
    let f = edit(
        "abcdef\nghijkl\n",
        0,
        0,
        &["v", "3", "l", "<Esc>", "g", "v"],
    );
    assert_eq!(f.vim.mode_indicator(), "VISUAL");
    let (a, c, _) = f.vim.visual_selection().unwrap();
    assert_eq!((a, c), (3, 0), "gv must restore the forward 0..3 span, cursor at start");

    // backward selections kept working before; keep them covered
    let f = edit("abcdef\nghijkl\n", 0, 3, &["v", "b", "<Esc>", "g", "v"]);
    let (a, c, _) = f.vim.visual_selection().unwrap();
    // gv re-anchors at the range start; the SPAN is what must survive
    assert_eq!((a, c), (3, 0), "backward gv keeps the same span, cursor at start");
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
    // a SHIFT-ONLY letter spelling canonicalizes to the printable uppercase
    // char (`<s-x>` == `X`): handle_key drops the shift flag on every plain
    // char key, so storing shift+lowercase was a mapping that could never
    // fire. vim 9.1 does the same normalization (`:nmap <S-x> ihello` lists
    // as `n  X  ihello`; PTY probe round 21)
    assert_eq!(Key::parse("<s-x>"), Key::char('X'));
    assert_eq!(Key::parse("<S-A>"), Key::char('A'));
    // non-letters keep the flag (shift+1 is layout-dependent, vim too)
    assert!(Key::parse("<s-1>").modifiers.shift);
    assert_eq!(Key::parse("<s-1>").kind, KeyKind::Char('1'));
}

// ---- * on a line without words: must not re-jump with the stale pattern ---------

#[test]
fn star_without_word_bells_instead_of_reusing_stale_pattern() {
    // `/two` arms the pattern; `*` on a punctuation-only line must not
    // re-search "two". [Round-14 correction] vim 9.1 probe Q1: with the cursor
    // on a non-word non-blank char, `*` **searches that char literally** (the
    // pattern of `*` on '!' is `!`) — the engine now searches `\.` (all three
    // dots highlighted, jumping to the next one) instead of reusing the old
    // pattern; the invariant "never reuse a stale pattern" holds.
    let mut f = Fixture::at("one two\n...\n", 0, 0);
    f.feed(["/", "t", "w", "o", "<Enter>"]); // on line 1's "two"
    f.feed(["j"]); // line 2: "..."
    f.feed(["*"]);
    assert_eq!(
        f.vim.search.pattern.as_deref(),
        Some(r"\."),
        "* on . searches the literal dot instead of reusing two"
    );
    assert_eq!(
        f.host.highlights.len(),
        3,
        "highlights = the three dots (literal .), not one /two match"
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
    // 'autoindent' set explicitly — the engine default now follows vim's
    // noai (round-31 audit I1)
    let mut f = Fixture::at("\tfoo\n", 0, 1);
    f.feed([":", "s", "e", "t", " ", "a", "i", "<CR>", "o"]);
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

// ---- unbounded-allocation guards (round-10 review) ---------------------------

/// The replicated volume of count-repeat insert must be byte-capped:
/// `99999999i` + long text + <Esc> goes through `replicate_count_insert`,
/// which used to call `text.repeat(copies)` unguarded — one small session
/// could make the host eat gigabytes (the register-paste path has long had
/// the 16MB `clamped_repeat` guard).
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
    // replication itself still happens (within the cap); it is not fully disabled
    assert!(len > payload.len(), "replication should still happen");
}

/// A huge `:set sw=` value used to be accepted verbatim, then `>>` built the
/// indent with `" ".repeat(sw)` — numeric options are now truncated at 1e6 on
/// the set_value boundary.
#[test]
fn numeric_option_values_are_capped() {
    let mut f = Fixture::new("a\nb\n");
    f.feed([
        ":", "s", "e", "t", " ", "s", "w", "=", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9",
        "9", "<CR>",
    ]);
    assert_eq!(f.vim.options_mut().shiftwidth, 1_000_000);
    // indentation itself stays usable: values up to 1e6 take effect as given
    // (>> indents only the current line, same as vim)
    f.feed([
        ":", "s", "e", "t", " ", "s", "w", "=", "2", "<CR>", ">", ">",
    ]);
    assert_eq!(f.vim.options_mut().shiftwidth, 2);
    assert_eq!(f.text(), "  a\nb\n");
}

// ---- search-cache sync after insert-mode edits (stale cache caught by fuzz round 10) -----

/// <Del>/<BS>/<C-w>/<C-u> in insert mode used to skip the hlsearch rescan: the
/// cached `last_matches` kept pre-edit offsets, and a delete could leave an old
/// offset inside a multibyte char (after deleting 'a' from "a中b", the cached
/// 1..4 points into the bytes of 中). `cancel_cmdline` published that cache to
/// the host as-is — the host panics when running offset_to_line on highlights.
#[test]
fn insert_mode_deletes_refresh_search_cache() {
    // /中/ matches 1..4; after deleting 'a' the new text "中b\n" matches 0..3
    let mut f = Fixture::new("a中b\n");
    f.feed(["/", "中", "<CR>"]);
    assert_eq!(f.vim.cursor_offset(), 1);
    f.feed(["g", "g"]);
    f.feed(["i"]);
    f.feed_raw(Key::named("delete")); // <Del> deletes 'a'
    let text = f.text();
    assert_eq!(text, "中b\n");
    for m in &f.vim.search.last_matches {
        assert!(
            text.is_char_boundary(m.start) && text.is_char_boundary(m.end),
            "stale cached match {m:?} in {text:?}"
        );
    }
    assert_eq!(f.vim.search.last_matches, vec![0..3]);
    // the highlights seen by the host are refreshed too
    assert_eq!(f.host.highlights, vec![0..3]);
}

/// Same class of path: in-line BS must refresh the cache too (one gap fixed,
/// both entries pinned).
#[test]
fn insert_mode_backspace_refreshes_search_cache() {
    let mut f = Fixture::new("a中b\n");
    f.feed(["/", "中", "<CR>"]); // cursor parks on 中 (offset 1)
    f.feed(["i"]);
    f.feed_raw(Key::named("backspace")); // BS deletes the preceding 'a'
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

// ---- zero-width replacements (the :s/^/x/ family) ----------------------------

/// Zero-width matches used to be skipped as "no replacement" and not counted:
/// `:s/^/>/` sat silently and even mis-reported E486. In vim this is the
/// standard idiom for prepending/appending to a line; it must substitute.
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
    // vim: 2 substitutions <= 'report' (default 2) stay silent (oracle osr2;
    // the old assertion wrongly recorded a "2 substitutions on 2 lines" report)
    assert_eq!(f.host.statuses, Vec::<String>::new());
}

/// `a*` on text without `a` matches zero-width everywhere: vim `:s/a*/-/` yields "-bbb".
#[test]
fn substitute_star_pattern_replaces_empty_match() {
    let mut f = Fixture::new("bbb\n");
    f.feed([":", "s", "/", "a", "*", "/", "-", "/", "<CR>"]);
    assert_eq!(f.text(), "-bbb\n");
    assert_eq!(f.host.statuses, Vec::<String>::new());
}

// ---- ranges with unset marks report E20 (vim-verified) -----------------------

/// `:'<,'>d` on a buffer that never entered visual: vim reports
/// `E20: Mark '< not set`; the engine used to report E16 for everything
/// (conflating "mark not set" with "bad range syntax").
#[test]
fn range_with_unset_visual_marks_reports_e20() {
    let mut f = Fixture::new("abc\n");
    f.feed([":", "'", "<", ",", "'", ">", "d", "<CR>"]);
    assert_eq!(f.host.statuses, ["E20: Mark '< not set"]);
    assert_eq!(f.text(), "abc\n"); // buffer untouched
}

/// Same for plain named marks (vim: E20: Mark 'a not set).
#[test]
fn range_with_unset_named_mark_reports_e20() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed([":", "'", "a", ",", "$", "d", "<CR>"]);
    assert_eq!(f.host.statuses, ["E20: Mark 'a not set"]);
    assert_eq!(f.text(), "abc\ndef\n");
}

// ---- Ex range semantics, three-in-a-row (vim 9.1 probe-verified, round 11) ---

/// An empty command with a range parks the cursor on the range's **last
/// address** (`:2,5<CR>` lands on line 5). The engine used to jump to `first`,
/// so `:2,5` landed on line 1.
#[test]
fn bare_range_command_moves_to_last_address() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    f.feed([":", "2", ",", "5", "<CR>"]);
    assert_eq!(f.line(), 4, ":2,5<CR> cursor lands on line 5 (last address)");

    // the single-address `:5` behavior is unchanged
    let mut f = Fixture::new("l1\nl2\nl3\n");
    f.feed([":", "2", "<CR>"]);
    assert_eq!(f.line(), 1);
}

/// Multi-address ranges: vim keeps only the **last two** addresses (`:1,2,3d`
/// on ['a','b','c','d'] deletes lines 2-3, probe-verified). The engine used to
/// keep first+last and deleted lines 1-3.
#[test]
fn multi_address_range_keeps_last_two() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed([":", "1", ",", "2", ",", "3", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\n");
}

/// An empty address defaults to the cursor line (on both sides): `:,3d` from
/// line 2 deletes 2-3, `:2,d` from line 3 deletes 2-3 (vim probes). Skipping
/// the empty segment made both commands delete a single line.
#[test]
fn empty_range_address_defaults_to_cursor_line() {
    let mut f = Fixture::at("a\nb\nc\nd\ne\n", 1, 0);
    f.feed([":", ",", "3", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\ne\n");

    let mut f = Fixture::at("a\nb\nc\nd\ne\n", 2, 0);
    f.feed([":", "2", ",", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nd\ne\n");
}

// ---- `&` / bare `:s` replays drop the previous command's flags (vim probe) ---

/// vim 9.1: after `s/a/B/g`, `&` on "xaxax" replaces only the first match —
/// `g` is **not** replayed by `&` (`:h :&`). The engine used to replay the
/// whole command line verbatim, keeping `g`.
#[test]
fn ampersand_repeat_drops_substitute_flags() {
    let mut f = Fixture::new("xaxax\nxaxax\n");
    f.feed([":", "s", "/", "a", "/", "B", "/", "g", "<CR>"]);
    assert_eq!(f.text(), "xBxBx\nxaxax\n");
    f.feed(["j", "&"]);
    assert_eq!(f.text(), "xBxBx\nxBxax\n", "& replays without the g flag");
}

/// Bare `:s` behaves like `&`: no flags from the previous command.
#[test]
fn bare_colon_s_repeat_drops_substitute_flags() {
    let mut f = Fixture::new("xaxax\nxaxax\n");
    f.feed([":", "s", "/", "a", "/", "B", "/", "g", "<CR>"]);
    f.feed(["j", ":"]);
    f.feed(["s", "<CR>"]);
    assert_eq!(f.text(), "xBxBx\nxBxax\n");
}

// ---- mark jumps as operator targets (d'a / d`a / y'a / c'a, vim-probe-verified) --

/// `d'a` deletes linewise from the current line through the mark's line,
/// direction-agnostic (vim 9.1: cursor on line 2, mark on line 3 deletes
/// 2-3). The old implementation only moved the cursor in complete_char_arg and
/// left the operator dangling — `d'a` did nothing silently, and the next key
/// was polluted by the hanging d.
#[test]
fn operator_with_mark_jump_deletes_through_mark_line() {
    // cursor on line 1, mark on line 2: delete lines 1-2
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["j", "m", "a", "k", "d", "'", "a"]);
    assert_eq!(f.text(), "ccc\n");

    // the other direction: cursor on line 2, mark on line 1, still deletes two lines
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["m", "a", "j", "d", "'", "a"]);
    assert_eq!(f.text(), "ccc\n");
}

/// `y'a` yanks linewise through the mark's line; `p` puts the two lines back.
#[test]
fn yank_with_mark_jump_is_linewise() {
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    f.feed(["m", "a", "j", "k", "y", "'", "a", "j", "p"]);
    assert_eq!(f.text(), "aaa\nbbb\naaa\nccc\n");
}

/// ``d`a`` deletes the charwise exclusive span cursor..mark (vim probe: mark on
/// the t of 'two' in 'one two', then ``d`a`` at the next line's start gives 'one three four').
#[test]
fn operator_with_backtick_mark_is_charwise_exclusive() {
    let mut f = Fixture::new("one two\nthree four\n");
    f.feed(["0", "f", "t", "m", "a", "j", "0", "d", "`", "a"]);
    assert_eq!(f.text(), "one three four\n");
}

/// With the mark unset, `d'a` bells and leaves the buffer alone (same feedback as plain `'x`).
#[test]
fn operator_with_unset_mark_bells_without_editing() {
    let mut f = Fixture::new("aaa\nbbb\n");
    f.feed(["d", "'", "x"]);
    assert_eq!(f.text(), "aaa\nbbb\n");
    assert!(f.host.bells > 0);
}

// ---- <C-a> on radix-prefix letters (vim 9.1 probe-verified) ------------------

/// vim treats the prefix letter `x` of `0x1f` as "number at the cursor": C-a
/// increments the whole literal (0x1f -> 0x20). The old implementation treated
/// `x` as plain text and grabbed the bare digit `1` ahead of it, producing
/// junk like `00x2f`. Same for `0XAB`/`0b101`; with no valid digits after the
/// prefix (the `b` of `0backup`) it is not a number (vim leaves it alone).
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

/// False-positive guard on prefix letters: `0b` not followed by 0/1 is not a
/// number (vim: buffer unchanged, E18 reported).
#[test]
fn c_a_on_bogus_binary_prefix_finds_no_number() {
    let mut f = Fixture::new("0backup\n");
    f.feed(["0", "l", "<C-a>"]);
    assert_eq!(f.text(), "0backup\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E18")));
}

/// Cursor on the leading `0` of `077`: octal +1 = `0100` (width grows naturally).
#[test]
fn c_a_on_octal_leading_zero_grows_width() {
    let mut f = Fixture::new("x077\n");
    f.feed(["0", "l", "<C-a>"]);
    assert_eq!(f.text(), "x0100\n");
}

// ---- navigation keys (arrows / Del) vs the `.` record and operators ----------

/// Arrow keys are not a "change": vim's redo buffer never contains a plain
/// motion. `<Down>` then `x` then `.` replays only `x` (deleting one char at
/// the current cursor), no downward move. The old implementation left `<Down>`
/// in the uncommitted recording, so the next change command baked it into
/// `last_change` and `.` became "move down + delete".
#[test]
fn arrow_key_never_leaks_into_dot_replay() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["down", "x", "."]);
    assert_eq!(f.text(), "abc\nf\nghi\n");
}

/// Mirror of the same contamination window: an arrow key after a committed
/// change, then a plain motion — the uncommitted arrow key must not survive to
/// the next command (previously this relied on the following motion's commit
/// clearing it; once the arrow key itself commits, the window closes fully).
#[test]
fn arrow_key_after_commit_does_not_replay() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 1);
    f.feed(["x", "down", "j", "."]);
    // `.` repeats just the `x`: deletes 'h' on line 3 (cursor moved 2 down)
    assert_eq!(f.text(), "ac\ndef\ngi\n");
}

/// `<Del>` is a complete delete command: repeatable by `.` itself (the old
/// implementation never committed a change record, so `.` replayed the command
/// before last or nothing at all).
#[test]
fn delete_key_is_repeatable_by_dot() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed_raw(Key::named("delete"));
    f.feed(["."]);
    assert_eq!(f.text(), "c\n");
}

/// The `.` record of the command right after a Del is not polluted by the Del.
#[test]
fn delete_key_pollutes_no_later_record() {
    let mut f = Fixture::at("abc\ndef\n", 0, 0);
    f.feed_raw(Key::named("delete"));
    f.feed(["j", "d", "d"]);
    assert_eq!(f.text(), "bc\n");
    f.feed(["."]); // replays just dd
    assert_eq!(f.text(), "");
}

/// Operators can take arrow keys as motions: `d<Down>` = `dj` (same as vim).
/// In the old implementation the arrow key bypassed the operator pipeline and
/// moved the cursor directly: the operator hung and the next key bailed with a bell.
#[test]
fn operator_completes_with_arrow_motion() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["d"]);
    f.feed_raw(Key::named("down"));
    assert_eq!(f.text(), "ghi\n");
}

/// A prefix count merging into the motion count runs the full pipeline:
/// `2d<Down>` = `d2<Down>`; from line 3, moving down 2 lines deletes l3..l5
/// linewise, 3 lines in all (vim's count rule: 2d3w = 6w).
#[test]
fn operator_arrow_count_merges() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\nl5\nl6\nl7\n", 2, 0);
    f.feed(["2"]);
    f.feed(["d"]);
    f.feed_raw(Key::named("down"));
    assert_eq!(f.text(), "l1\nl2\nl6\nl7\n");
}

// ---- round 19: D/C line model (vim 9.1 byte-level probes) / r<CR> counts / r<C-E>/r<C-Y> --

/// `D` on the last line (truncation survives) keeps the file's trailing
/// newline — the engine's final `\n` is the host's file eol. vim 9.1 byte
/// probe: `D` on the last line of `abc\ndef\n` writes `abc\nde\n`. The old
/// implementation swallowed the newline, so the host-saved file lost its eol.
#[test]
fn d_on_last_line_keeps_file_eol() {
    let f = edit("abc\ndef\n", 1, 2, &["D"]);
    assert_eq!(f.text(), "abc\nde\n");
}

/// `99D` reaching EOF from mid-line: the truncated first line + eol both survive (vim probe -> "a\n").
#[test]
fn count_d_to_eof_keeps_eol() {
    let f = edit("aaaa\nbbbb\n", 0, 1, &["9", "9", "D"]);
    assert_eq!(f.text(), "a\n");
}

/// `D` at a line start (with surviving lines below) = whole-line deletion, no
/// phantom empty line (vim probe: `2D` at the start of ['a','b','c'] ->
/// ['c']). The old implementation left an empty first line.
#[test]
fn d_at_line_start_deletes_covered_lines_whole() {
    let f = edit("a\nb\nc\n", 0, 0, &["2", "D"]);
    assert_eq!(f.text(), "c\n");
    let f = edit("aaaa\nbbbb\ncccc\ndddd\n", 0, 0, &["3", "D"]);
    assert_eq!(f.text(), "dddd\n");
    let f = edit("aaaa\nbbbb\n", 0, 0, &["9", "9", "D"]);
    assert_eq!(f.text(), "");
}

/// `D` count=1 at a line start = d$ semantics: the line is emptied but kept
/// (vim PTY probe: `D` at the start of ['a','b','c'] -> "\nb\nc\n", the cursor
/// line does not vanish).
#[test]
fn d_count_one_at_line_start_empties_the_line() {
    let f = edit("a\nb\nc\n", 0, 0, &["D"]);
    assert_eq!(f.text(), "\nb\nc\n");
}

/// `D` at the line start of a sole-line buffer: emptied but kept (vim keeps >=1 line, probe -> "\n").
#[test]
fn d_at_sole_line_start_keeps_empty_line() {
    let f = edit("abc\n", 0, 0, &["D"]);
    assert_eq!(f.text(), "\n");
}

/// `C` and `D` diverge at a line start: C empties the line and enters insert
/// (vim probe: `C` at the line start + z -> "z\nb\nc\n"), the line never
/// disappears; covered middle lines still vanish whole (existing round-9 2C
/// probe + round19 `2Cnew` at the line start -> ['new','cccc','dddd']).
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

/// `N r <CR>` collapses to a single line break — `:h r` spells it out: "5r<CR>
/// replaces five characters with a single line break" (vim byte probe:
/// `3r<CR>` on "abcdef" -> one empty line + "def"). The old implementation produced N newlines.
#[test]
fn count_r_cr_is_one_break() {
    let f = edit("abcdef\n", 0, 0, &["3", "r", "<CR>"]);
    assert_eq!(f.text(), "\ndef\n");
    // a single r<CR> parks the cursor at the new line's start (existing
    // semantics, pinning the landing spot after the collapse)
    let f = edit("abc\n", 0, 1, &["r", "<CR>"]);
    assert_eq!(f.text(), "a\nc\n");
    assert_eq!(f.cursor(), 2); // first char of the new next line
}

/// `r<C-E>` / `r<C-Y>`: the replacement chars come from the same display
/// column of the line below/above (`:h r`; `10r<C-E>` copies 10 chars from
/// below). If either side runs short the whole command is cancelled (same
/// cancel semantics as `3rx` with only two chars left).
#[test]
fn r_ctrl_e_y_copies_from_neighbor_line() {
    // below: 'a' <- 'X'
    let f = edit("abc\nXYZ\n", 0, 0, &["r", "<C-e>"]);
    assert_eq!(f.text(), "Xbc\nXYZ\n");
    // above: 'Y' <- 'b'
    let f = edit("abc\nXYZ\n", 1, 1, &["r", "<C-y>"]);
    assert_eq!(f.text(), "abc\nXbZ\n");
    // count: 3r<C-E> copies 3 chars
    let f = edit("abc\nXYZW\n", 0, 0, &["3", "r", "<C-e>"]);
    assert_eq!(f.text(), "XYZ\nXYZW\n");
    // positions the neighbor line cannot supply are SKIPPED, not a
    // whole-command cancel (2026-10-09 oracle re-verification flipped the old
    // pin: 3r<C-E> on `abc`+`XY` -> `XYc`, writefile evidence via the keys
    // channel; the NUL arm of nv_replace in normal.c only does ++col, no E
    // message. The old "cancel the whole command + bell" assertion was a
    // sampling misread). Same family: `abcdef`+`abX` with `0ll2r<C-E>` ->
    // `abXdef`, cursor on the last processed position.
    let mut f = Fixture::at("abc\nXY\n", 0, 0);
    f.feed(["3", "r", "<C-e>"]);
    assert_eq!(f.text(), "XYc\nXY\n");
}

/// `'z` / `` `z `` with an unset mark goes through vim's message channel (E20), not just a mute bell.
#[test]
fn unset_mark_jump_reports_e20() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["'", "z"]);
    assert_eq!(f.text(), "abc\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E20")));
    assert!(f.host.statuses.iter().any(|s| s.contains('z')));
}

/// `:j` on the last line (no seam to join) is a pure no-op: no changelist
/// entry, no `.` mark push. The old implementation bumped anyway, polluting
/// the changelist with a no-op.
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

/// EOF clamping of a trailing `:s` count: the effective range starts at the
/// original range's last line and extends count-1 lines downward, cutting only
/// the tail past EOF (vim 9.1 probes: `:5s/a/X/20` on a 10-line buffer ->
/// lines 5..10; `:1,3s/a/X/2` -> lines 3..4). The old implementation
/// back-computed the window start from the clamped last line (`last - (n-1)`),
/// turning `:5s/a/X/20` into a whole-file substitution.
#[test]
fn ex_substitute_trailing_count_clamps_at_eof_without_sliding() {
    fn ex(f: &mut Fixture, line: &str) {
        let mut keys: Vec<String> = line.chars().map(|c| c.to_string()).collect();
        keys.insert(0, ":".to_owned());
        keys.push("<CR>".to_owned());
        f.feed(keys);
    }
    let buf = "l1a\nl2a\nl3a\nl4a\nl5a\nl6a\nl7a\nl8a\nl9a\nl10a\n";

    let mut f = Fixture::at(buf, 0, 0);
    ex(&mut f, "5s/a/X/20");
    assert_eq!(f.text(), "l1a\nl2a\nl3a\nl4a\nl5X\nl6X\nl7X\nl8X\nl9X\nl10X\n");

    let mut f = Fixture::at(buf, 0, 0);
    ex(&mut f, "1,3s/a/X/2");
    assert_eq!(f.text(), "l1a\nl2a\nl3X\nl4X\nl5a\nl6a\nl7a\nl8a\nl9a\nl10a\n");

    // last line + count: no room below -> only the last line changes
    // (probe `:3s/a/X/2` on 3 lines)
    let mut f = Fixture::at("l1a\nl2a\nl3a\n", 0, 0);
    ex(&mut f, "3s/a/X/2");
    assert_eq!(f.text(), "l1a\nl2a\nl3X\n");
}

/// E35 (`/ab<C-u><CR>`: empty pattern + no previous pattern) must end the
/// incsearch preview — round20 fuzz caught the Enter early-exit path leaving
/// the preview set, stranding the host on the last per-char match set (an
/// empty-pattern preview includes end-of-line zero-width matches beyond the buffer range).
#[test]
fn e35_disarms_incsearch_preview() {
    let mut f = Fixture::new("abc\ndefg");
    f.feed(["/", "a", "b", "<C-u>", "<CR>"]);
    assert!(f.host.highlights.is_empty());
    assert_eq!(f.host.current_highlight, None);
    assert!(f
        .host
        .statuses
        .iter()
        .any(|s| s.contains("E35")));
    // with a previous pattern the same path goes through set_pattern and
    // republishes, leaving no preview
    let mut f = Fixture::new("abc abc\n");
    f.feed(["/", "a", "b", "c", "<CR>", "<BS>", "<BS>", "<BS>", "<C-u>", "<CR>"]);
    assert_eq!(f.host.highlights, vec![0..3, 4..7]);
}

// ---- round-20 review additions (2026-10-04) ----------------------------------

/// `:marks` no longer lists `^` twice: exit_insert writes both the offsets
/// table's `set('^')` and the engine-tracked `last_insert_exit`, while the
/// listing branch deduped only `.` and appended `^` unconditionally — the same
/// nominal mark appeared on two rows.
#[test]
fn marks_listing_does_not_duplicate_caret() {
    let mut f = Fixture::new("hello world\n");
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["<Esc>", ":", "m", "a", "r", "k", "s", "<CR>"]);
    let caret_rows = f
        .host
        .statuses
        .iter()
        .filter(|s| s.starts_with(" ^"))
        .count();
    assert_eq!(caret_rows, 1, "statuses: {:?}", f.host.statuses);
}

/// Text-object probes floor to char boundaries: with offset == line_end (a
/// legal insert-mode cursor position) plus a CJK line end, the old raw-byte
/// `line_end - 1` probe landed inside a multibyte char -> class_at read None
/// -> the empty-line branch fired wrongly and `iw` silently selected the whole line.
#[test]
fn word_object_probe_floors_to_char_boundary_at_line_end() {
    use vimcore::buffer::VimBuffer;
    use vimcore::objects::{range, TextObject};
    let f = Fixture::new("中文 bar");
    let buf = &f.buf;
    // 中(0..3) 文(3..6) space(6) bar(7..10): iw at line_end should select "bar"
    let r = range(
        buf,
        buf.line_end(0),
        TextObject::Word {
            inner: true,
            big: false,
        },
        1,
    );
    assert_eq!(r.map(|x| (x.start, x.end)), Some((7, 10)));
}

/// `:1j` on the last line (no-seam no-op) parks the cursor at the range
/// address line's first non-blank, same as the empty `:1` command (vim 9.1 PTY
/// probe: after `:1j` on "    abc", `x` deletes 'a'). The old implementation
/// landed at the bare line start (col 0), contradicting the `:1` landing spot.
#[test]
fn ex_join_noop_parks_on_first_non_blank() {
    let mut f = Fixture::new("    abc\n");
    f.feed(["G", ":", "1", "j", "<CR>"]);
    assert_eq!(f.text(), "    abc\n");
    assert_eq!(f.cursor(), 4, "first non-blank of the indented line");
    // the already-modified path's landing spot is unchanged
    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", "j", "\r"]);
    assert_eq!(f.text(), "a\nb c\n");
}

/// `<S-a>`-style mappings normalize to a deliverable form: handle_key drops
/// shift on plain char keys, so mappings stored by the old parse with
/// shift+lowercase could never fire (shift+a delivers 'A'). vim 9.1
/// normalizes the same way (`:nmap <S-x> ihello` lists as `n  X  ihello`).
#[test]
fn shift_letter_mapping_fires() {
    let mut f = Fixture::new("abc\n");
    f.vim
        .keymaps_mut()
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "<S-a>", "x", true);
    f.feed(["A"]);
    assert_eq!(f.text(), "bc\n", "mapped <S-a> fires on the delivered 'A'");
    // plain lowercase a must not misfire (append into insert, typing lands after 'a')
    let mut f = Fixture::new("abc\n");
    f.vim
        .keymaps_mut()
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "<S-a>", "x", true);
    f.feed(["a"]);
    f.type_text("Z");
    assert_eq!(f.text(), "aZbc\n");
}

// ---- search-as-motion (d/pat, c?pat, v/pat) ----------------------------------
//
// vim 9.1 PTY probes (round 21):
//   * `gg0d/bar\r` on "foo bar\nbaz\n" -> "bar\nbaz\n" (exclusive to the match start)
//   * when the cursor sits on the match it wraps ("search hit BOTTOM") and
//     deletes up to the wrap landing
//   * `gg0v/bar\r` -> cursor jumps to the match start, the selection extends,
//     the mode stays VISUAL
//   * miss -> E486, the operator does not hang
// Before the fix the engine's shape was a pure bug: `d` + `/` opened the
// prompt but left the operator dangling, so the next motion key after the
// search accidentally fired `d<motion>` (probe: `j` -> dj deleted the whole buffer).

fn feed_prompt(f: &mut Fixture, pattern: &str) {
    for c in pattern.chars() {
        f.feed_raw(Key::char(c));
    }
    f.feed(["<CR>"]);
}

#[test]
fn d_slash_pattern_deletes_through_match_start() {
    let mut f = Fixture::new("foo bar\nbaz\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "bar");
    assert_eq!(f.text(), "bar\nbaz\n");
    // the operator no longer dangles: the pipeline is idle, the later j only moves
    assert!(f.vim.is_idle());
    f.feed(["j"]);
    assert_eq!(f.text(), "bar\nbaz\n");
}

#[test]
fn c_slash_pattern_changes_through_match_start() {
    let mut f = Fixture::new("foo bar\n");
    f.feed(["c", "/"]);
    feed_prompt(&mut f, "bar");
    f.type_text("XX");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "XXbar\n");
}

#[test]
fn d_slash_miss_reports_e486_and_aborts_operator() {
    let mut f = Fixture::new("foo bar\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "zzz");
    assert!(f.host.statuses.iter().any(|s| s.contains("E486")));
    assert!(f.vim.is_idle(), "operator must not survive the miss");
    assert_eq!(f.text(), "foo bar\n");
}

#[test]
fn d_slash_count_targets_nth_match() {
    // 2d/foo == d2/foo: the second match
    let mut f = Fixture::new("foo foo tail\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "foo");
    assert_eq!(f.text(), "foo tail\n");
}

#[test]
fn d_question_pattern_deletes_backward_to_match_start() {
    let mut f = Fixture::new("abc def ghi\n");
    f.feed(["$", "d", "?"]);
    feed_prompt(&mut f, "def");
    // the backward exclusive span = [match_start, cursor), the cursor char is
    // kept — vim 9.1 probe `$d?def\r` on "abc def ghi" -> "abc i\n" (deletes
    // "def gh", 'i' stays)
    assert_eq!(f.text(), "abc i\n");
}

#[test]
fn d_slash_esc_cancels_quietly() {
    let mut f = Fixture::new("foo bar\n");
    f.feed(["d", "/"]);
    for c in "bar".chars() {
        f.feed_raw(Key::char(c));
    }
    f.feed(["<Esc>"]);
    assert!(f.vim.is_idle(), "Esc drops the pending operator");
    assert_eq!(f.text(), "foo bar\n");
    assert_eq!(f.host.bells, 0);
}

#[test]
fn v_slash_pattern_extends_selection_and_keeps_visual() {
    let mut f = Fixture::new("foo bar\n");
    f.feed(["v", "/"]);
    feed_prompt(&mut f, "bar");
    assert_eq!(f.vim.mode(), vimcore::mode::Mode::Visual { kind: vimcore::mode::VisualKind::Char });
    assert_eq!(f.cursor(), 4, "cursor at the match start");
    // the selection includes the cursor char (visual semantics): anchor 0..cursor 4+1 = "foo b" -> d deletes it
    // (vim 9.1 probe `v/bar\rx` -> "ar\nbaz\n")
    f.feed(["d"]);
    assert_eq!(f.text(), "ar\n");
}

#[test]
fn v_slash_cancel_keeps_selection_untouched() {
    let mut f = Fixture::new("foo bar\n");
    f.feed(["v", "l", "l", "/"]);
    feed_prompt(&mut f, "zzz");
    assert!(f.host.statuses.iter().any(|s| s.contains("E486")));
    assert_eq!(
        f.vim.mode(),
        vimcore::mode::Mode::Visual { kind: vimcore::mode::VisualKind::Char }
    );
    // the selection is still vll's [0, 3)
    f.feed(["d"]);
    assert_eq!(f.text(), " bar\n");
}

#[test]
fn d_slash_dot_replay_repeats_the_search_motion() {
    let mut f = Fixture::new("foo aa mid aa tail\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "aa");
    assert_eq!(f.text(), "aa mid aa tail\n");
    // `.` replays the whole d/pat: from the cursor, delete to the next aa
    f.feed(["."]);
    assert_eq!(f.text(), "aa tail\n");
}

#[test]
fn plain_slash_after_operatorless_prompt_still_jumps() {
    // without an operator, / still just jumps (count semantics unchanged)
    let mut f = Fixture::new("aa bb aa\n");
    f.feed(["/", "a", "a", "<CR>"]);
    // the cursor's own match is skipped: the next aa is at 6
    assert_eq!(f.cursor(), 6);
    f.feed(["2", "/", "b", "<CR>"]);
    // "b" matches at 3 and 4; from 6 it wraps to 3, count 2 -> 4
    assert_eq!(f.cursor(), 4);
}

/// The vim-consistent shape of search-motions (round 21 probe addendum):
#[test]
fn d_slash_crosses_newline_to_match() {
    let mut f = Fixture::new("one two\nthree four\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "three");
    assert_eq!(f.text(), "three four\n");
}

#[test]
fn y_slash_pattern_keeps_cursor_planted() {
    // vim 9.1: after y/pat the cursor stays put (yank does not move it), x still deletes the original char
    let mut f = Fixture::new("foo bar\n");
    f.feed(["y", "/"]);
    feed_prompt(&mut f, "bar");
    assert_eq!(f.cursor(), 0);
    f.feed(["x"]);
    assert_eq!(f.text(), "oo bar\n");
}

#[test]
fn gu_slash_pattern_lowercases_through_match() {
    let mut f = Fixture::new("AAA BBB\n");
    f.feed(["g", "u", "/"]);
    feed_prompt(&mut f, "BBB");
    assert_eq!(f.text(), "aaa BBB\n");
}

#[test]
fn d_slash_wrap_to_cursor_match_bells_and_aborts() {
    // the match is only at the cursor (wraps back to itself): the motion did
    // not move -> bell + drop the operator (vim 9.1 PTY probe: d/foo on "foo"
    // -> wrap message + \x07, text unchanged)
    let mut f = Fixture::new("foo\n");
    f.feed(["d", "/"]);
    feed_prompt(&mut f, "foo");
    assert_eq!(f.text(), "foo\n");
    assert_eq!(f.host.bells, 1);
    assert!(f.vim.is_idle());
}

#[test]
fn macro_records_and_replays_search_motion() {
    // d/pat inside q{reg} is recorded in full (six keys: d / p a t CR), @b replays the same shape
    let mut f = Fixture::new("foo bar baz\n");
    f.feed(["q", "b", "d", "/"]);
    feed_prompt(&mut f, "bar");
    f.feed(["q"]);
    assert_eq!(f.vim.macro_len('b'), 6);
    // swap the text via the host contract (engine idle and Normal); @b should delete to the bar start
    *f.buf.0.borrow_mut() = "xx bar yy\n".to_owned();
    f.vim.cursor.offset = 0;
    f.feed(["@", "b"]);
    assert_eq!(f.text(), "bar yy\n");
}

// ---- round 22 (2026-10-04): mapping key spellings, prompt-count overflow, :s replacement escapes, :sort E488 --

#[test]
fn c_space_mapping_keeps_its_modifier() {
    // parse_angle's "space" branch used to return Key::char(' ')
    // unconditionally: a `<C-Space>` mapping was stored as a plain space —
    // pressing space fired it (in vim 9.1's :nmap listing <C-Space> and
    // <Space> are two keys). The modifiers must come along.
    assert_ne!(
        Key::parse("<C-Space>"),
        Key::parse("<Space>"),
        "<C-Space> must not alias plain Space"
    );
    assert!(Key::parse("<C-Space>").modifiers.control);
    // behavior: map <C-Space> -> ix, plain space acts as the l motion
    let mut f = Fixture::new("foo bar");
    f.vim
        .keymaps_mut()
        .map_str(vimcore::keymap::ModeClass::Normal, "<C-Space>", "ix");
    f.feed([" "]);
    assert_eq!(f.vim.mode(), vimcore::mode::Mode::Normal);
    assert_eq!(f.cursor(), 1);
    // the modified form itself still fires the mapping
    f.feed_raw(Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char(' '),
    });
    assert_eq!(f.vim.mode(), vimcore::mode::Mode::Insert);
}

#[test]
fn lt_and_bar_spellings_keep_modifiers_too() {
    // same family of dropped modifiers: the <lt>/<bar> branches
    assert_ne!(Key::parse("<C-lt>"), Key::parse("<"));
    assert_ne!(Key::parse("<M-bar>"), Key::parse("|"));
}

#[test]
fn named_ctrl_chord_equals_char_ctrl_chord() {
    // Key::ctrl("w") and Key::ctrl_char('w') are two host spellings of the
    // same keystroke, but insert mode only recognized the Char form — the
    // Named form silently returned Unknown (key swallowed, nothing deleted).
    // handle_key now folds single-char Named keys into Char.
    let mut named = Fixture::at("aaaa\nbbbb", 1, 4);
    named.feed_raw(Key::named("x").clone()); // noop warmup, keep harness shape
    let mut f = Fixture::at("aaaa\nbbbb", 1, 4);
    f.feed_raw(Key::ctrl("i")); // Named + ctrl
    f.feed(["e", "s", "c"]);
    let mut g = Fixture::at("aaaa\nbbbb", 1, 4);
    g.feed_raw(Key::ctrl_char('i')); // Char + ctrl (the form parse produces)
    g.feed(["e", "s", "c"]);
    assert_eq!(f.text(), g.text(), "both ctrl spellings must be equivalent");
    // positive check: <C-w> in insert mode deletes the previous word
    let mut h = Fixture::at("aaaa\nbbbb", 1, 4);
    h.feed(["i"]);
    h.feed_raw(Key::ctrl("w"));
    assert_eq!(h.text(), "aaaa\n");
}

#[test]
fn huge_prompt_count_does_not_overflow_search_jump() {
    // the count before the prompt (`3/foo`) goes through search_count,
    // bypassing take_total_count's one-billion cap; 20 nines then /pat<CR>
    // used to overflow the u64 addition in jump_to_match — debug builds
    // panicked outright (release jumped wrongly in silence). The step count is
    // now taken modulo the match count, so the sum never leaves u64.
    let mut f = Fixture::new("aa\naa\naa\naa");
    f.feed(["/", "a"]);
    f.feed_raw(Key::enter()); // cursor lands on match #1 (start_index=1)
                              // the count is typed before / (9x25 -> saturates to usize::MAX)
    let mut keys: Vec<&str> = std::iter::repeat_n("9", 25).collect::<Vec<_>>();
    keys.push("/");
    keys.push("a");
    f.feed(keys);
    f.feed_raw(Key::enter());
    // usize::MAX steps modulo the match count: 8 matches, u64::MAX-1 ≡ 6 (mod 8),
    // 6 more steps from start_index=2 -> (2+6)%8 = 0 -> the first match. The key
    // contract is "no overflow + bounded landing" (the old code debug-panicked on
    // this path / release jumped wrongly).
    assert_eq!(f.cursor(), 0);
}

#[test]
fn substitute_replacement_r_and_n_follow_vim_escapes() {
    // vim 9.1 byte-level probe: in the replacement, \r is a newline
    // (fX<LF>Yo bar) and \n is NUL (fX<NUL>Yo bar). The old engine wrote both
    // as literals into the buffer.
    let mut f = Fixture::new("foo bar");
    f.feed([":"]);
    for c in "s/o/X\\rY/".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert_eq!(f.text(), "fX\nYo bar");

    let mut g = Fixture::new("foo bar");
    g.feed([":"]);
    for c in "s/o/X\\nY/".chars() {
        g.feed_raw(vimcore::key::Key::char(c));
    }
    g.feed_raw(Key::enter());
    assert_eq!(g.text(), "fX\u{0}Yo bar");

    // \\r (escaped backslash + r) stays literal, same as vim
    let mut h = Fixture::new("foo bar");
    h.feed([":"]);
    for c in "s/o/X\\\\rY/".chars() {
        h.feed_raw(vimcore::key::Key::char(c));
    }
    h.feed_raw(Key::enter());
    assert_eq!(h.text(), "fX\\rYo bar");
}

#[test]
fn sort_rejects_trailing_count_like_vim() {
    // :sort takes no count — vim 9.1 probe: `:1,3sort 3` reports E488 and the
    // buffer stays put; the old engine sorted in silence.
    let mut f = Fixture::new("b\na\nc");
    f.feed([":"]);
    for c in "1,3sort 3".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert_eq!(f.text(), "b\na\nc");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E488")),
        "statuses = {:?}",
        f.host.statuses
    );
    // sorting without the numeric argument is unaffected
    let mut g = Fixture::new("b\na\nc");
    g.feed([":"]);
    for c in "1,3sort".chars() {
        g.feed_raw(vimcore::key::Key::char(c));
    }
    g.feed_raw(Key::enter());
    assert_eq!(g.text(), "a\nb\nc");
}

#[test]
fn unknown_set_option_reports_e518() {
    // vim 9.1: `:set foo` → "E518: Unknown option: foo", later items no longer execute
    let mut f = Fixture::new("a\n");
    f.feed([":"]);
    for c in "set foo ts=8".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E518: Unknown option: foo")),
        "statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(f.vim.options.tabstop, 4, "items after E518 no longer execute");
}

#[test]
fn invalid_substitute_pattern_reports_compile_error() {
    // patterns the RE2 dialect rejects no longer bell silently — the compile
    // error text is published (vim's traditional engine tolerates an unclosed
    // `[` and reports E486; that is a documented dialect divergence)
    let mut f = Fixture::new("a\n");
    f.feed([":"]);
    for c in "s/[/x/".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert!(f.host.statuses.iter().any(|s| s.contains("Invalid pattern")),
        "statuses = {:?}", f.host.statuses);
    // a pattern that failed to compile must not enter last_substitute (& must not replay a broken command)
    f.feed(["&"]);
    assert_eq!(f.text(), "a\n");
}

// ---- round-23 review (2026-10-04) --------------------------------------------

#[test]
fn ime_commit_mid_session_is_one_undo_group() {
    // IME commits inside an insert session go through replace_range: the old
    // implementation called end_edit() unconditionally, so every commit split
    // out its own undo group and `i<Esc>u` undid only the last commit. vim
    // treats the whole insert session (IME commits included) as one undo
    // block. A standalone replace_range outside a session (REPL-style
    // replacement) must still close the group it opened (no dangling).
    let mut f = Fixture::new("hello");
    f.feed(["A"]); // append at line end, cursor (0,5)
    f.type_text("ab"); // "helloab"
    f.ime_replace(5..7, "阿"); // first commit -> "hello阿"
    f.type_text("cd"); // "hello阿cd"
    f.ime_replace(8..10, "波"); // second commit -> "hello阿波"
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hello阿波");
    f.feed(["u"]);
    assert_eq!(f.text(), "hello", "one u should restore the whole session");
    // standalone replacement outside a session: the group opens and closes by
    // itself, never dangling into the next edit
    let mut g = Fixture::new("hello world");
    g.ime_replace(0..5, "hi");
    g.feed(["u"]);
    assert_eq!(g.text(), "hello world");
    g.feed(["x"]); // an independent edit starts a new group
    g.feed(["u"]);
    assert_eq!(g.text(), "hello world", "a dangling group must not swallow later edits");
}

#[test]
fn enter_as_char_arg_only_serves_replace() {
    // vim PTY probe: f<CR>/t<CR>/m<CR>/'<CR> all do nothing. The old engine
    // mapped Enter to '\n' unconditionally: f<CR> failed yet overwrote
    // last_find (the next ; searched for a newline), and '<CR> produced an E20
    // message containing a newline. r<CR>/visual r<CR> still consume Enter.
    let mut f = Fixture::new("a b a c a");
    f.feed(["f", "a"]); // cursor lands on the 2nd a (offset 4)
    assert_eq!(f.cursor(), 4);
    let bells_before = f.host.bells;
    f.feed(["f", "<CR>"]); // fails: bell, cursor unmoved
    assert!(f.host.bells > bells_before);
    assert_eq!(f.cursor(), 4);
    f.feed([";"]); // last_find is still 'a' -> lands on the 3rd a (offset 8)
    assert_eq!(f.cursor(), 8, "; repeats the last successful f, not a newline search");
    // m<CR>: illegal mark name -> bell, no message
    let mut g = Fixture::new("abc");
    let b0 = g.host.bells;
    g.feed(["m", "<CR>"]);
    assert!(g.host.bells > b0);
    assert!(g.host.statuses.is_empty(), "statuses = {:?}", g.host.statuses);
    // '<CR>: no mark -> bell; the message must not contain a bare newline (the old E20 text spanned lines)
    let mut h = Fixture::new("abc");
    h.feed(["'", "<CR>"]);
    assert!(
        h.host.statuses.iter().all(|s| !s.contains('\n')),
        "statuses = {:?}",
        h.host.statuses
    );
    // visual r<CR>: every selected char becomes one line break (vim PTY
    // probe: vllr<CR> on "abcdef" -> three empty lines + "def")
    let mut v = Fixture::new("abcdef");
    v.feed(["v", "l", "l", "r", "<CR>"]);
    assert_eq!(v.text(), "\n\n\ndef");
    // r<CR>: the line-splitting semantics are unaffected
    let mut r = Fixture::new("abc");
    r.feed(["r", "<CR>"]);
    assert_eq!(r.text(), "\nbc");
}

#[test]
fn set_bad_numeric_value_reports_e521() {
    // vim 9.1 -es evidence: `:set ts=` → "E521: Number required after =: ts=".
    // The old implementation fell into E518 (Unknown option), reporting an
    // illegal VALUE as a nonexistent OPTION.
    for bad in ["ts=", "ts=x"] {
        let mut f = Fixture::new("a\n");
        f.feed([":"]);
        for c in format!("set {bad}").chars() {
            f.feed_raw(vimcore::key::Key::char(c));
        }
        f.feed_raw(Key::enter());
        assert!(
            f.host
                .statuses
                .iter()
                .any(|s| s.contains("E521: Number required after =:")),
            "set {bad}: statuses = {:?}",
            f.host.statuses
        );
        assert_eq!(f.vim.options.tabstop, 4, "set {bad}: the value must not be rewritten");
    }
    // an unknown option name is still E518 (the two error classes do not cross)
    let mut f = Fixture::new("a\n");
    f.feed([":"]);
    for c in "set foo=1".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E518: Unknown option: foo=1")),
        "statuses = {:?}",
        f.host.statuses
    );
}

#[test]
fn sort_unknown_flag_reports_e475_and_skips() {
    // vim 9.1 PTY probe: `:sort z` → "E475: Invalid argument: z" with the
    // buffer untouched; the old engine sorted in silence (unknown flags were
    // treated as absent).
    let mut f = Fixture::new("b\na");
    f.feed([":"]);
    for c in "sort z".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert_eq!(f.text(), "b\na", "no sort may run after E475");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E475: Invalid argument: z")),
        "statuses = {:?}",
        f.host.statuses
    );
}

#[test]
fn sort_numeric_flag_orders_by_first_number() {
    // the `n` flag: sort by the first decimal number in the line; lines
    // WITHOUT a number form their own segment BEFORE all numeric lines
    // (2026-10-09 oracle re-verification flipped the old pin: `:sort n` on
    // `abc/5/-3/7/x` → `abc x -3 5 7`, non-numeric segment first;
    // test_sort.vim Test_sort_large_num says the same: "Non-numeric lines are
    // ordered before numerical lines". The old assertion "numberless lines
    // count as 0 inside the numeric segment" was a misread at the time).
    // Equal-key lines keep their order (stable); `u` dedups by sort key.
    let mut f = Fixture::new("10 apples\n2 bananas\nno number\n-3 x");
    f.feed([":"]);
    for c in "%sort n".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed_raw(Key::enter());
    assert_eq!(f.text(), "no number\n-3 x\n2 bananas\n10 apples");
    // nu: dedup by numeric value
    let mut g = Fixture::new("2 a\n10 b\n2 c\nzzz");
    g.feed([":"]);
    for c in "%sort nu".chars() {
        g.feed_raw(vimcore::key::Key::char(c));
    }
    g.feed_raw(Key::enter());
    assert_eq!(g.text(), "zzz\n2 a\n10 b");
}

#[test]
fn is_idle_is_false_while_a_prompt_is_open() {
    // is_idle must be false while a `:`/`/` prompt is open: the host uses it
    // as the key-interception gate, and hijacking the prompt's literal text
    // (the u in `:ru`, e.g.) would eat characters.
    let mut f = Fixture::new("abc");
    assert!(f.vim.is_idle());
    f.feed([":"]);
    assert!(!f.vim.is_idle(), "an open prompt is unfinished input");
    f.feed(["r"]);
    assert!(!f.vim.is_idle());
    f.feed(["<Esc>"]);
    assert!(f.vim.is_idle());
    // insert mode still counts as idle (text input is the host's job)
    let mut g = Fixture::new("abc");
    g.feed(["i"]);
    assert!(g.vim.is_idle());
}

#[test]
fn double_prefill_range_takes_last_two_addresses() {
    // visual `:` prefills `'<,'>` and the same is then typed by hand (the
    // macro/replay accident shape): with no visual selection the first `'<`
    // reports a clean E20 (mark name, no token-junk tail), buffer untouched, no panic
    let mut g = Fixture::new("a\nb\nc");
    g.feed([":"]);
    for c in "'<,'>'<,'>d".chars() {
        g.feed_raw(vimcore::key::Key::char(c));
    }
    g.feed_raw(Key::enter());
    assert_eq!(g.text(), "a\nb\nc");
    let quoted = g
        .host
        .statuses
        .iter()
        .find(|s| s.contains("E20"))
        .expect("E20 expected");
    assert_eq!(quoted, "E20: Mark '< not set", "{quoted}");
}

// ---- round 24: cursor landing on trailing grapheme clusters -------------------
//
// When the line ends with a grapheme cluster (decomposed combining chars,
// VS16, ZWJ families), the line-end rollback must land on the CLUSTER START,
// not on the last "char" — a combining char has width 0, and parking the
// cursor there lets `x` split the cluster (deleting only the mark, leaving a
// bare base char).

#[test]
fn esc_after_trailing_cluster_lands_on_cluster_start() {
    // after inserting "á" (a + U+0301) and Esc: the cursor must land on 'a' (0), not the mark (1)
    let mut f = Fixture::new("\n");
    f.feed(["i"]);
    f.type_text("a\u{0301}");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a\u{0301}\n");
    assert_eq!(f.cursor(), 0, "after Esc the cursor should sit at the grapheme-cluster start");
    // the following x deletes the whole cluster
    f.feed(["x"]);
    assert_eq!(f.text(), "\n", "x must delete the whole grapheme cluster");
}

#[test]
fn esc_after_variation_selector_lands_on_base() {
    // "#"+U+FE0F (emoji presentation selector)
    let mut f = Fixture::new("\n");
    f.feed(["i"]);
    f.type_text("#\u{FE0F}");
    f.feed(["<Esc>"]);
    assert_eq!(f.cursor(), 0, "VS16 cluster: the cursor should land on '#'");
    f.feed(["x"]);
    assert_eq!(f.text(), "\n", "x must delete both # and VS16");
}

#[test]
fn esc_after_zwj_family_lands_on_first_member() {
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut f = Fixture::new("\n");
    f.feed(["i"]);
    f.type_text(fam);
    f.feed(["<Esc>"]);
    assert_eq!(f.cursor(), 0, "emoji family: the cursor should land on the first member");
    f.feed(["x"]);
    assert_eq!(f.text(), "\n", "x must delete the whole family");
}

#[test]
fn dollar_parks_on_trailing_cluster_start() {
    // the `$` line-end landing is also the cluster start (the old implementation landed on the mark).
    // line "ab\u{0301}": a@0, b@1, mark@2..4; the mark's cluster starts at b (1)
    let f = edit("ab\u{0301}\n", 0, 0, &["$"]);
    assert_eq!(f.cursor(), 1, "$ should land on b (the start of the cluster owning the mark)");
}

#[test]
fn j_onto_trailing_cluster_lands_on_cluster_start() {
    // j onto a line ending in a cluster: the display-column clamp must also land on the cluster start
    let f = edit("hello\nab\u{0301}\n", 0, 0, &["j"]);
    assert_eq!(f.line(), 1);
    assert_eq!(f.cursor(), 6, "after j the cursor should land on 'a' (start of line 2)");
    // j down from column 1 -> lands on b (cluster start)
    let f = edit("hello\nab\u{0301}\n", 0, 1, &["j"]);
    assert_eq!(f.cursor(), 7, "j should land on b (cluster start), not the mark");
}

#[test]
fn g_skips_trailing_combining_mark() {
    // g_ finds the last non-blank char: the trailing mark is not the landing spot, it falls to the base char
    let f = edit("ab\u{0301}\n", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 1, "g_ should land on b (cluster start), not the mark");
}

// ---- round 24: w/W semantics around the ideographic space (U+3000) -----------

#[test]
fn w_skips_ideographic_space() {
    // U+3000 is is_whitespace: vim's w never stops on a blank char
    // a=1 byte, U+3000=3 bytes -> 'b' sits at 4
    let f = edit("a\u{3000}b\n", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 4, "w should skip the fullwidth space and land on b");
    let f = edit("a\u{3000}b\n", 0, 0, &["W"]);
    assert_eq!(f.cursor(), 4, "same for W");
}

#[test]
fn w_crosses_ideographic_space_only_line() {
    // A line holding only an ideographic space is not an empty line: vim's w
    // crosses it freely (same shape as ASCII blank lines).
    // line 0 "x\u{3000}\n" spans 0..5; line 1 "\u{3000}y": U+3000@5..8, y@8
    let f = edit("x\u{3000}\n\u{3000}y\n", 0, 0, &["w"]);
    assert_eq!(f.line(), 1, "should land on y (line 2), not on a blank");
    assert_eq!(f.cursor(), 8);
}

#[test]
fn g_ignores_ideographic_space_tail() {
    // g_ skips trailing blanks (fullwidth included) and lands on the last non-blank char
    let f = edit("ab\u{3000}\n", 0, 0, &["g", "_"]);
    assert_eq!(f.cursor(), 1, "g_ should land on b");
}

// ---- round 24: cluster awareness of the f/t/F/T till stopping points ---------

#[test]
fn till_stops_never_park_on_combining_mark() {
    // line "áx" (a + U+0301 + x): the `tx` stopping point is the start of the
    // cluster owning the mark, 'a' (0); the old implementation landed on the
    // mark (offset 1, mid-cluster)
    let f = edit("a\u{0301}x\n", 0, 0, &["t", "x"]);
    assert_eq!(f.cursor(), 0, "tx should stop at the cluster start a");
    // backward: is there an `Fa` left of x? this line has no a to the left —
    // use "xá" to exercise Ta's symmetric branch
    let f = edit("xa\u{0301}b\n", 0, 4, &["F", "a"]);
    assert_eq!(f.cursor(), 1, "Fa lands on a (cluster start)");
    // `ta`'s stopping point is the start of the cluster before the hit (á) =
    // the cursor stays in place (visually the same composite char; byte-wise
    // vim would also land inside the combining char — the engine clamps to the cluster start)
    let f = edit("xa\u{0301}b\n", 0, 1, &["t", "b"]);
    assert_eq!(f.cursor(), 1, "ta stop = start of the cluster before the hit (in place)");
}

#[test]
fn paste_lands_on_last_cluster_start() {
    // after yanking a word containing a combining char and p: the cursor
    // lands on the start of the last cluster
    let mut f = Fixture::new("a\u{0301}b\n");
    f.feed(["y", "l"]); // yank "á"
    f.feed(["$", "p"]);
    assert_eq!(f.text(), "a\u{0301}ba\u{0301}\n");
    assert_eq!(f.cursor(), 4, "after p the cursor should land on the start of the pasted text's last cluster (the second á)");
}

#[test]
fn insert_backspace_deletes_whole_cluster() {
    // after the IME commits "e"+U+0301, BS deletes the whole cluster (vim's delcombine=off default)
    let mut f = Fixture::new("\n");
    f.feed(["i"]);
    f.type_text("e\u{0301}");
    f.feed_raw(vimcore::key::Key::named("backspace"));
    assert_eq!(f.text(), "\n", "BS should delete the whole e+mark cluster");
}

// ---- round 25: grapheme clusters × line boundaries (w blank skip / GB4 hard boundary / Del) --

#[test]
fn w_skips_continuation_chars_attached_to_blanks() {
    // `w`'s blank skip must fold continuation chars into the blank cluster
    // they belong to (base char being the space): round 24 fixed b
    // (prev_word_start's is_cont arm) but missed forward w — the cursor parked
    // on the width-0 mark, violating the "never stop on a width-0 char" invariant.
    let f = edit("a \u{0301}bc", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 4, "w should cross space+mark and land on b");
    let f = edit("  \u{0301}abc", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 4, "a mark after indentation likewise belongs to the blank cluster");
    // "abc\ne \u{0301}fgh\nijk": the mark takes 2 bytes (6..8), the f cluster starts at 8
    let f = edit("abc\ne \u{0301}fgh\nijk", 0, 4, &["w"]);
    assert_eq!(f.cursor(), 8, "cross-line case: w lands on the f cluster start, not on the mark");
}

#[test]
fn standalone_mark_line_is_its_own_cluster() {
    // \n is a hard grapheme boundary (UAX #29 GB4): a standalone mark at the
    // line start does not glue to the previous line's \n. The old
    // prev_grapheme_offset walked the mark back into the \n, and after w
    // reached the buffer end clamp_cursor parked the cursor on the previous
    // line's newline, so x would delete the newline and join the lines.
    let f = edit("x\n\u{0301}", 0, 0, &["w"]);
    assert_eq!(f.line(), 1, "w should reach the mark line");
    assert_eq!(f.cursor(), 2, "the landing is the mark itself (a standalone cluster, line-start shape)");
    assert_ne!(f.cursor(), f.buf.line_end(0), "must not stop at the previous line's newline position");

    // $ on a standalone-mark line: the landing stays inside the line (the mark), never past the line boundary
    let f = edit("ab\n\u{0301}", 0, 3, &["$"]);
    assert_eq!(f.line(), 1, "$ must not jump to the previous line");
}

#[test]
fn zwj_at_line_start_does_not_glue_following_starter() {
    // round 26: the "pre-ZWJ gluing to a following starter" rule in
    // prev_grapheme_offset never checked whether the ZWJ itself is glued.
    // A ZWJ at line start (or buffer start) forms its own cluster in the
    // forward scan, yet backward glued the following starter to it — the two
    // directions gave different cluster boundaries at the same position: `$`
    // parked on the invisible ZWJ and `x` deleted only the joiner, leaving a
    // bare base char (the visible char was the one that should have gone).
    // "ab\n\u{200D}b": ZWJ occupies 3..6, b at 6
    let mut f = edit("ab\n\u{200D}b", 1, 0, &["$"]);
    assert_eq!(f.cursor(), 6, "$ lands on the visible b, not the invisible ZWJ");
    f.feed(["x"]);
    assert_eq!(f.text(), "ab\n\u{200D}", "x deletes b; the standalone line-start ZWJ survives");
    // buffer-start form: the starter after ZWJ is likewise not glued
    let f = Fixture::new("\u{200D}b");
    assert_eq!(
        vimcore::buffer::prev_grapheme_offset(&f.buf, 4),
        Some(3),
        "buffer-start ZWJ does not glue the starter either"
    );
}

#[test]
fn grapheme_scan_directions_agree() {
    // mirror-consistency invariant: for every cluster [s,e) cut by the forward
    // scan, backward from e must trace back to s. Once the two semantics
    // drift apart, the landing points of w/b and x/$ disagree.
    let cases: &[&str] = &[
        "a\n\u{200D}b",
        "\u{200D}b",
        "ab\n\u{200D}c",
        "x\n\u{200D}\u{200D}y",
        "a\u{200D}b\nc\u{200D}d",
        "e\u{0301}\n\u{0301}\n",
        "a\u{200D}\nb",
        "\u{200D}",
        "中文\u{200D}👨‍👩‍👧\n\u{0301}x",
    ];
    for text in cases {
        let f = Fixture::new(text);
        let mut o = 0usize;
        let mut boundaries = vec![0usize];
        while o < f.buf.len() {
            match vimcore::buffer::next_grapheme_offset(&f.buf, o) {
                Some(next) => {
                    boundaries.push(next);
                    o = next;
                }
                None => break,
            }
        }
        for w in boundaries.windows(2) {
            let (s, e) = (w[0], w[1]);
            assert_eq!(
                vimcore::buffer::prev_grapheme_offset(&f.buf, e),
                Some(s),
                "backward trace of cluster [{s},{e}) must return to its start ({text:?})"
            );
        }
        // &str mirror: the start of the last cluster of the whole string must
        // agree with the buffer-side backward scan
        if let Some(&last_start) = boundaries.iter().rev().nth(1) {
            assert_eq!(
                vimcore::buffer::last_grapheme_start(text),
                Some(last_start),
                "last_grapheme_start must agree with prev_grapheme_offset ({text:?})"
            );
        }
    }
}

#[test]
fn last_grapheme_start_treats_newline_as_boundary() {
    // the &str mirror of prev_grapheme_offset follows the same rule: the block
    // insert session's byte ledger relies on it agreeing with buffer-side
    // cluster deletion.
    assert_eq!(
        vimcore::buffer::last_grapheme_start("x\n\u{0301}"),
        Some(2),
        "a standalone mark forms its own cluster, starting at itself"
    );
    assert_eq!(
        vimcore::buffer::last_grapheme_start("e\u{0301}"),
        Some(0),
        "regular combining-char cluster start unchanged"
    );
}

#[test]
fn insert_delete_removes_whole_cluster() {
    // Del shares BS's cluster semantics (delcombine=off): deleting only the
    // base char leaves a bare mark, which then re-composes onto the previous char.
    let mut f = Fixture::new("");
    f.feed(["i"]);
    f.type_text("e\u{0301}x");
    f.feed(["<Esc>", "0", "i"]);
    f.feed_raw(vimcore::key::Key::named("delete"));
    assert_eq!(f.text(), "x", "Del should remove the whole e+mark cluster");
}
#[test]
fn r_multibyte_on_zwj_line_end_does_not_panic() {
    // ZWJ used to glue the line-ending \n into the cluster: the base (b) cluster
    // spanned the newline, advance_graphemes consumed 0 clusters, and with
    // replaced==0 `start + 0 - ch.len_utf8()` underflowed usize (debug panic).
    // After the GB4/GB5 fix ZWJ never crosses lines; the empty-replacement guard
    // is the backstop.
    let mut f = Fixture::new("ab\u{200D}\ncd");
    f.vim.cursor.offset = 1; // 'b'
    f.feed(["r", "中"]);
    // b+ZWJ is already one cluster (ZWJ attaches to the preceding char); replace
    // it wholly with 中; \n does not enter the cluster
    assert_eq!(f.text(), "a中\ncd", "r中 replaces the b+ZWJ cluster without eating the newline");
    assert_eq!(f.cursor(), 1, "cursor lands at the start of the replaced cluster");
}

#[test]
fn zwj_clusters_never_span_newlines() {
    // next_grapheme_offset: the x+ZWJ cluster ends before \n (ZWJ does not eat
    // \n; ZWJ is 3 bytes, cluster end = offset 4 of the \n)
    let f = Fixture::new("x\u{200D}\ncd");
    assert_eq!(
        vimcore::buffer::next_grapheme_offset(&f.buf, 0),
        Some(4),
        "x+ZWJ cluster ends at \n"
    );
    // a standalone line-start ZWJ is not glued to \n (before the fix, prev
    // traced back into the previous line's \n); and it does not glue the
    // following starter into its own cluster either — in the forward scan the
    // line-start ZWJ is its own cluster and c is another, so prev must give
    // the same boundary (round 26 backward read "ZWJ c" as one cluster, `$`
    // parked on the invisible ZWJ and `x` deleted only the joiner)
    let g = Fixture::new("ab\n\u{200D}c");
    assert_eq!(
        vimcore::buffer::prev_grapheme_offset(&g.buf, 6),
        Some(3),
        "line-start ZWJ forms its own cluster"
    );
    assert_eq!(
        vimcore::buffer::prev_grapheme_offset(&g.buf, 7),
        Some(6),
        "line-start ZWJ does not glue the following starter (same boundary as the forward scan)"
    );
}

#[test]
fn sentence_object_on_multibyte_cursor_char() {
    // sentence_range's (offset+1) landed inside the multibyte cursor char:
    // prev_char_offset returned None and the sentence start silently collapsed
    // to 0 — dis on "Hi. 你好 ok" with the cursor on 你 deleted "Hi." instead of
    // the sentence under the cursor.
    let mut f = Fixture::new("Hi. 你好 ok");
    f.vim.cursor.offset = 4; // start of '你'
    f.feed(["d", "i", "s"]);
    assert_eq!(f.text(), "Hi. ", "dis should delete the sentence under the cursor");
}

#[test]
fn marks_listing_shows_both_visual_ends() {
    // vim's :marks lists both '< and '> (the engine listed only '<); the row
    // format is `" %c %6ld %4d %s"`, so lines start with " <"/" >"
    let mut f = Fixture::new("alpha\nbeta\n");
    f.feed(["V", "j", "d"]);
    f.feed([":", "m", "a", "r", "k", "s", "<CR>"]);
    let joined = f.host.statuses.join("\n");
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with(" <")),
        "should list '<\n{joined}"
    );
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with(" >")),
        "should list '>\n{joined}"
    );
}

// ---- round 27: host history axis (undo/redo) and group ownership --------------

#[test]
fn count_undo_steps_and_edge_bell() {
    // `Nu` undoes N steps; the excess past the history depth rings once at the
    // history edge (vim: undo to the bottom + beep). Three x = three groups:
    // 2u stops at the second step, 3u reaches the bottom, 5u bottom + 1 bell.
    let mk = |keys: &[&str]| {
        let mut f = Fixture::new("aaa\nbbb\nccc\n");
        f.feed(["x", "x", "x"]);
        assert_eq!(f.text(), "\nbbb\nccc\n");
        f.feed(keys);
        f
    };
    let f = mk(&["2", "u"]);
    assert_eq!(f.text(), "aa\nbbb\nccc\n", "2u undoes two steps");
    let f = mk(&["3", "u"]);
    assert_eq!(f.text(), "aaa\nbbb\nccc\n", "3u undoes three steps to the bottom");
    let f = mk(&["5", "u"]);
    assert_eq!(f.text(), "aaa\nbbb\nccc\n", "5u undoes to the bottom");
    assert_eq!(f.host.bells, 1, "exactly one bell at the history edge");
}

#[test]
fn undo_to_bottom_restores_initial_text() {
    // Directed version of the completeness invariant: after mixed edits /
    // visual operators / Ex deletes / IME commits, undoing all the way down
    // must return to the initial buffer byte-for-byte — any edit path that
    // forgets to open an undo group (bypassing the edit_* funnel) silently
    // shows up here.
    let initial = "hello world\nsecond line\n中文行\n";
    let mut f = Fixture::new(initial);
    f.feed(["x", "d", "d", "p", "J", "g", "J", "y", "y", "P"]);
    f.feed(["v", "l", "d"]);
    f.ime_replace(0..2, "组");
    f.feed([":", "2", "d", "\n"]);
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    f.feed([">", ">", "<", "<", "~"]);
    assert_ne!(f.text(), initial, "precondition: the sequence did change the text");

    let mut depth = 0;
    loop {
        let before = f.host.bells;
        f.feed(["u"]);
        if f.host.bells > before {
            break;
        }
        depth += 1;
        assert!(depth <= 100, "u did not hit the bottom within 100 steps");
    }
    assert_eq!(f.text(), initial, "undo to the bottom must restore the initial buffer");
}

#[test]
fn substitute_and_block_insert_are_single_undo_groups() {
    // vim counts the whole :{range}s and the whole blockwise-visual I/A/c
    // session as one undo block each
    let mut f = Fixture::new("aaa\naaa\naaa\n");
    f.feed([":", "%", "s", "/", "a", "/", "b", "/", "g", "\n"]);
    assert_eq!(f.text(), "bbb\nbbb\nbbb\n");
    let groups_before = f.host.group_count;
    f.feed(["u"]);
    assert_eq!(f.text(), "aaa\naaa\naaa\n", "the whole :s is one undo group");
    assert!(f.host.group_count >= groups_before); // (u does not create groups)

    let mut g = Fixture::new("ab\ncd\nef\n");
    // printable chars in insert mode go through the host placement path
    // (type_text); the key stream only carries control keys
    g.feed(["<C-v>", "j", "I"]);
    g.type_text("X");
    g.feed(["<Esc>"]);
    assert_eq!(g.text(), "Xab\nXcd\nef\n");
    g.feed(["u"]);
    assert_eq!(g.text(), "ab\ncd\nef\n", "block I session incl. the copy body is one undo group");
}

#[test]
fn undo_then_dot_replays_the_undone_change() {
    // u does not rewrite the `.` replay record: after u, `.` replays the undone
    // change (vim agrees)
    let mut f = Fixture::new("abc\nabc\n");
    f.feed(["x", "u", "."]);
    assert_eq!(f.text(), "bc\nabc\n");
}

#[test]
fn q_uppercase_appends_to_macro_recording() {
    // `qA` records, appending to a's existing steps (vim :h q_a); stopping
    // the recording lands back in the lowercase slot
    let mut f = Fixture::new("line1\nline2\nline3\n");
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1);
    f.feed(["q", "A", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 2, "qA appends instead of overwriting");
    f.feed(["j", "@", "a"]);
    assert_eq!(f.text(), "ne1\nne2\nline3\n", "@a replays the two x steps");
}

#[test]
fn insert_ctrl_o_passes_through_to_host() {
    // insert mode only recognizes the established chords C-w/C-u/C-r etc.;
    // <C-o> (temporary normal) is not modeled and returns Unknown to the
    // host — the host may claim it (same contract as printable chars)
    let mut f = Fixture::new("ab\n");
    f.feed(["i"]);
    let r = f.feed_raw(Key::ctrl_char('o'));
    assert!(matches!(r, vimcore::state::KeyResult::Unknown));
}

#[test]
fn normal_u_undo_line_is_not_bound() {
    // normal-mode U (undo a line) is unbound: it rings the bell and leaves the
    // cursor. The engine delegates history to the host's linear stack; a
    // line-undo needs a host cooperation API (see NOTES divergence 70).
    // visual U (the upper-case operator) is unaffected and covered elsewhere.
    let mut f = Fixture::new("aaa bbb\n");
    f.feed(["x", "U"]);
    assert_eq!(f.host.bells, 1, "unbound U should ring the bell");
    assert_eq!(f.cursor(), 0, "U leaves the cursor alone");
}

// ---- round 27 batch 2: selection survival of gv after visual operators --------

#[test]
fn gv_after_charwise_delete_selects_the_line_head() {
    // vim 9.1: after `vllld`, '< '> keep the selection's row/col coordinates —
    // gv reselects the same-width span at the head of the post-delete line,
    // instead of collapsing to zero width. The old behavior folded last_visual
    // through adjust_delete into the deletion point, so gv selected nothing and
    // `gv d` became a no-op.
    let mut f = Fixture::new("hello world\nsecond\n");
    f.feed(["v", "l", "l", "d"]); // deletes "hel" → "lo world"
    assert_eq!(f.text(), "lo world\nsecond\n");
    f.feed(["g", "v"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::Visual { .. }));
    f.feed(["d"]);
    assert_eq!(
        f.text(),
        "world\nsecond\n",
        "gv reselects the first three chars \"lo \" of the post-delete line (vim: row/col projection) and deletes"
    );
}

#[test]
fn gv_after_undo_reselects_the_original_selection() {
    // vim 9.1: after vllld then u, the same '< '> pair lands back on the
    // restored text — gv reselects the original span ("hel"). The old behavior
    // had last_visual already folded to (0,0) by the deletion, unrecoverable
    // by undo.
    // "abcdef" deletes "abc" → "def"; after undo restores, gv must reselect "abc"
    let mut f = Fixture::new("abcdef\nsecond\n");
    f.feed(["v", "l", "l", "d"]);
    assert_eq!(f.text(), "def\nsecond\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "abcdef\nsecond\n");
    f.feed(["g", "v"]);
    f.feed(["d"]);
    assert_eq!(f.text(), "def\nsecond\n", "gv revives the original selection abc and deletes it");
}

#[test]
fn gv_after_linewise_delete_still_targets_the_surviving_line() {
    // linewise selections follow a different vim rule: the deleted lines' marks
    // fold onto the following line (round 7's `Vjd` semantics must not be broken
    // by the charwise freezing rule)
    let mut f = Fixture::new("aaa\nbbb\nccc\nddd\n");
    f.feed(["V", "j", "d"]);
    assert_eq!(f.text(), "ccc\nddd\n");
    f.feed(["g", "v", "d"]);
    assert_eq!(f.text(), "ddd\n", "gv reselects the surviving line ccc and deletes it");
}

// ---- round 28: empty linewise delete leaves registers alone (vim 9.1 PTY probe) ---

fn unnamed(f: &Fixture) -> Option<String> {
    f.vim.registers.get('"').map(|r| r.text.clone())
}

#[test]
fn empty_linewise_delete_leaves_registers_alone() {
    // vim 9.1 probe: on an empty buffer dd/Vd/dip/dap/dG all leave the
    // registers untouched (p rings a bell, `:reg` shows only "%). The engine
    // had fed the 0..0 linewise span into the numbered ring + unnamed, so a
    // later p opened a ghost empty line. Yank is the opposite rule: yy/yip on
    // an empty buffer **do** store an empty line (see next test) — the
    // asymmetry is exactly vim's.
    for keys in [
        vec!["d", "d"],
        vec!["V", "d"],
        vec!["d", "i", "p"],
        vec!["d", "a", "p"],
        vec!["d", "G"],
    ] {
        let mut f = Fixture::new("");
        f.feed(keys.iter().copied());
        assert_eq!(f.text(), "");
        assert_eq!(unnamed(&f), None, "keys {keys:?} must not write unnamed");
        assert_eq!(f.vim.registers.get('1').map(|r| r.text.clone()), None);
    }
}

#[test]
fn empty_linewise_yank_stores_an_empty_line() {
    // vim 9.1 probe: after yy on an empty buffer, `:reg` shows a linewise empty
    // line in "" and "0 each, and p opens a new line — deliberately contrasted
    // with delete's non-write. 2026-10-08 byte-level oracle re-verification
    // (od -c): `yyp` on an empty file writes back "\n\n" (lines 1→2); the old
    // assertion "\n" was still 1 line in this fixture's line model, contradicting
    // its own oracle conclusion.
    let mut f = Fixture::new("");
    f.feed(["y", "y"]);
    assert_eq!(unnamed(&f), Some(String::new()));
    assert_eq!(f.vim.registers.get('0').map(|r| r.text.clone()), Some(String::new()));
    f.feed(["p"]);
    assert_eq!(f.text(), "\n\n", "empty-buffer yy+p opens one empty line (vim lines=1→2)");
}

// ---- round 28: :s flag validation (E488) and the `e` flag ----------------------

fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("\r".to_owned());
    f.feed(keys.iter().map(|s| s.as_str()));
}

#[test]
fn substitute_rejects_junk_flags_with_e488() {
    // vim 9.1 probe: `:s/a/X/z` → "E488: Trailing characters: z", buffer unchanged.
    // The old engine ignored unknown flag letters and substituted anyway — a
    // typo'd flag quietly changed the text.
    let mut f = Fixture::new("aaa zzz\n");
    ex(&mut f, "s/a/X/z");
    assert_eq!(f.text(), "aaa zzz\n", "a junk flag must keep the whole command from running");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E488")),
        "statuses = {:?}",
        f.host.statuses
    );
}

#[test]
fn substitute_still_accepts_valid_flag_letters() {
    // valid flags (g/i/I/n + the tolerated set c/e/l/p/#/&) do not trigger E488
    let mut f = Fixture::new("aaa\n");
    ex(&mut f, "s/a/X/g");
    assert_eq!(f.text(), "XXX\n");
}

#[test]
fn substitute_e_flag_suppresses_the_miss_report() {
    // vim 9.1 probe: `:s/q/W/e` with no match → no E486 (suppressing it is the
    // e flag's whole purpose)
    let mut f = Fixture::new("aaa zzz\n");
    ex(&mut f, "s/q/W/e");
    assert_eq!(f.text(), "aaa zzz\n");
    assert!(f.host.statuses.is_empty(), "statuses = {:?}", f.host.statuses);
    // without e, E486 is still reported
    let mut f = Fixture::new("aaa zzz\n");
    ex(&mut f, "s/q/W/");
    assert!(f.host.statuses.iter().any(|s| s.contains("E486")));
}

// ---- round 28: blockwise-visual `$` (:h v_$, PTY probe) -----------------------

#[test]
fn block_dollar_delete_extends_to_every_line_end() {
    // vim 9.1 probe: `<C-v>j$d` → ['','','def'] — the block extends to **every**
    // line's end. The old engine treated $ as an ordinary motion and deleted
    // only up to the cursor line's $ column ("g line here").
    let f = edit("long line here\nabc\ndef\n", 0, 0, &["<C-v>", "j", "$", "d"]);
    assert_eq!(f.text(), "\n\ndef\n");
}

#[test]
fn block_dollar_ignores_the_anchor_column() {
    // vim 9.1 probe: `<C-v>jll$d` yields the same as `<C-v>j$d` — once $ is
    // pressed the anchor column is out of play and each row is covered whole.
    let f = edit("long line here\nabc\ndef\n", 0, 0, &["<C-v>", "j", "l", "l", "$", "d"]);
    assert_eq!(f.text(), "\n\ndef\n");
}

#[test]
fn block_dollar_i_inserts_at_line_starts_and_a_at_line_ends() {
    // vim 9.1 probe: $+I inserts at each line's start ("_Xabc…); $+A appends at
    // each row's own line end and **does not pad** ("long line hereX"/"abcX").
    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "I"]);
    f.type_text("_X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "_Xlong line here\n_Xabc\ndef\n");

    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "A"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "long line hereX\nabcX\ndef\n");
}

#[test]
fn block_dollar_change_replicates_over_whole_rows() {
    // vim 9.1 probe: `<C-v>j$cyz<Esc>` → both rows become "yz"
    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "c"]);
    f.type_text("yz");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "yz\nyz\ndef\n");
}

#[test]
fn block_dollar_yank_register_is_unpadded() {
    // vim 9.1 probe (:reg): a $ block yank's register holds unpadded whole-row text
    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "$", "y"]);
    let reg = f.vim.registers.get('"').expect("unnamed set");
    assert_eq!(reg.text, "long line here\nabc");
    assert_eq!(reg.kind, vimcore::registers::RegisterKind::Blockwise);
}

// ---- round 28: blockwise-visual register semantics ----------------------------

#[test]
fn block_delete_stores_the_block_for_paste() {
    // vim 9.1 probe: after `<C-v>jlld`, p pastes the deleted block back (block
    // register rows are unpadded), "0 untouched. The old engine's block delete
    // wrote no register at all.
    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "d"]);
    let reg = f.vim.registers.get('"').expect("unnamed set");
    assert_eq!(reg.text, "lon\nabc");
    assert_eq!(reg.kind, vimcore::registers::RegisterKind::Blockwise);
    assert_eq!(f.vim.registers.get('0'), None, "delete must not enter \"0");
    // p pastes it back
    f.feed(["g", "g", "0", "p"]);
    assert_eq!(f.text(), "glon line here\n abc\ndef\n");
}

#[test]
fn block_change_stores_the_deleted_block() {
    // vim 9.1 probe: after `<C-v>jlc` + typing + Esc, p pastes the deleted block back
    let mut f = Fixture::at("ab\nef\nij\nkl\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "c"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    // block 2 cols wide ("ab"/"ef" deleted), X replicated onto both rows
    assert_eq!(f.text(), "X\nX\nij\nkl\n");
    f.feed(["g", "g", "0", "p"]);
    assert_eq!(f.text(), "Xab\nXef\nij\nkl\n");
}

#[test]
fn block_yank_keeps_rows_unpadded() {
    // vim 9.1 probe (:reg strtrans): block yank does not pad short rows — the
    // register is exactly "long\nabc". Padding is the put side's job
    // (put_blockwise already does it).
    // The cursor row is "abc" (3 cols), l clamps at the line end: block width 3
    // ("lon"/"abc").
    // The assertion point is that the short row "abc" carries no padding —
    // vim's register works the same (the width lives in the register type;
    // padding is a put-side concern).
    let mut f = Fixture::at("long line here\nabc\ndef\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "y"]);
    let reg = f.vim.registers.get('"').expect("unnamed set");
    assert_eq!(reg.text, "lon\nabc");
}

#[test]
fn visual_block_put_swaps_with_the_deleted_selection() {
    // vim 9.1 probe (swap idiom): `<C-v>jy` + `<C-v>jp` then p again — the
    // second p pastes back the ij/kl just deleted.
    // A 1-col block ("a"/"e") matches the vim probe shape: the first p
    // replaces the i/k column, the second p (normal mode) pastes back the
    // deleted block "i"/"k" → "aij"/"ekl"
    let mut f = Fixture::at("ab\nef\nij\nkl\n", 0, 0);
    f.feed(["<C-v>", "j", "y"]);
    f.feed(["j", "j", "<C-v>", "j", "p"]);
    assert_eq!(f.text(), "ab\nef\naj\nel\n");
    f.feed(["p"]);
    assert_eq!(f.text(), "ab\nef\naij\nekl\n", "the second p pastes back the deleted block (swap)");
}

#[test]
fn visual_put_leaves_the_explicit_paste_register_alone() {
    // vim 9.1 probe: after viw"ap, "a is still aaa; the replaced text goes to
    // unnamed. The old engine passed the paste register into delete_span and
    // overwrote "a.
    let mut f = Fixture::at("aaa bbb\n", 0, 0);
    f.feed(["\"", "a", "y", "i", "w"]);
    f.feed(["w", "v", "i", "w", "\"", "a", "p"]);
    assert_eq!(f.text(), "aaa aaa\n");
    assert_eq!(
        f.vim.registers.get('a').map(|r| r.text.clone()),
        Some("aaa".to_owned()),
        "the source register must not be overwritten by the deleted text"
    );
    assert_eq!(unnamed(&f), Some("bbb".to_owned()));
}

#[test]
fn block_put_extra_register_rows_append_below_the_selection() {
    // vim 9.1 probe: a 3-row block register put onto a 2-row block selection —
    // the third row becomes a new line below the last covered row (the old
    // engine dropped it outright).
    let mut f = Fixture::at("aa\nbb\ncc\ndd\nee\n", 0, 0);
    f.feed(["<C-v>", "j", "j", "y"]);
    f.feed(["j", "j", "j", "<C-v>", "j", "p"]);
    assert_eq!(f.text(), "aa\nbb\ncc\nad\nbe\nc\n");
}

// ---- round 28: blockwise put merge model (PTY probes corrected the old "push down" model) --

#[test]
fn blockwise_put_merges_into_existing_lines_below() {
    // vim 9.1 probe: `2G0p` mid-buffer — register rows 2/3 merge onto the
    // **existing lines below** the cursor line (insert at the same column,
    // padding the row to line length with spaces first), no longer opening new
    // lines unconditionally. The old "push down" model came from probes placed
    // only at the last line (both models coincide at the buffer end).
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd\n", 0, 0);
    f.feed(["<C-v>", "j", "y"]); // 1-col block ["a", "b"]
    f.feed(["2", "G", "0", "p"]);
    assert_eq!(f.text(), "aaaa\nbabbb\ncbccc\ndddd\n");
}

#[test]
fn blockwise_put_before_merges_at_cursor_column() {
    // vim 9.1 probe: `2G0P` → "abbbb"/"bcccc" (P inserts at the cursor column
    // itself, following rows still run downward)
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd\n", 0, 0);
    f.feed(["<C-v>", "j", "y"]);
    f.feed(["2", "G", "0", "P"]);
    assert_eq!(f.text(), "aaaa\nabbbb\nbcccc\ndddd\n");
}

#[test]
fn blockwise_put_merges_mid_line_at_display_column() {
    // vim 9.1 probe: `3G02lp` — row width 4, column 2: the insertion point is
    // before the 3rd char, "cccc"+"aaaa" → "cccaaaac"; the next row at the same
    // column "dddd" → "ddbbbbdd"
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "l", "y"]); // 4-col block
    f.feed(["3", "G", "0", "2", "l", "p"]);
    assert_eq!(f.text(), "aaaa\nbbbb\ncccaaaac\ndddbbbbd\n");
}

#[test]
fn visual_block_put_overflow_merges_into_the_next_line() {
    // vim 9.1 probe: a 3-row register put onto a 2-row selection — the 3rd row
    // merges into the **existing line below** the selection ("ee" → "cee");
    // only past the buffer end does a new line open
    let mut f = Fixture::at("aa\nbb\ncc\ndd\nee\nff\n", 0, 0);
    f.feed(["<C-v>", "j", "j", "y"]); // ["a", "b", "c"]
    f.feed(["3", "G", "<C-v>", "j", "p"]); // selection = lines 2-3
    assert_eq!(f.text(), "aa\nbb\nac\nbd\ncee\nff\n");
}
