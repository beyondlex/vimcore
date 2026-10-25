//! Vim-parity regressions, round two (2026-09 crossterm_vim integration review).
//!
//! Each test pins one behavior aligned with vim 9.1: `n`/`N` direction, `*` word
//! bounds, `t`/`T` line bounds, uppercase register append, empty text objects,
//! `C-e`/`C-y`, `%` percentage, `:N` line jump, `:s` case flags, mark ranges, `@:`,
//! Replace backspace, gq display width, `:q` forced flag, shared search history.
//!
//! Note: every element of `Fixture::feed` goes through `Key::parse` — a multi-char
//! string becomes a `Named` key! Apart from single chars and tokens like `<CR>`,
//! keys must be fed one char at a time.

mod common;

use common::{edit, Fixture};
use vimcore::key::{parse_key_sequence, Key};
use vimcore::registers::{RegisterKind, CLIPBOARD, UNNAMED, YANK};

fn feed(edit: &mut Fixture, keys: &str) {
    for k in parse_key_sequence(keys) {
        edit.feed_raw(k);
    }
}

// ---- n / N respect the direction of the last search ------------------------------

#[test]
fn n_after_question_search_goes_backward() {
    // vim: `?` + `n` repeats in the backward direction; the old implementation
    // hardcoded forward
    let mut f = Fixture::at("b b b", 0, 4);
    feed(&mut f, "?b<CR>");
    assert_eq!(f.cursor(), 2);
    f.feed(["n"]);
    assert_eq!(f.cursor(), 0, "`n` after `?` must keep going backward");
    f.feed(["N"]);
    assert_eq!(f.cursor(), 2, "`N` mirrors back to forward");
}

#[test]
fn n_after_slash_search_stays_forward() {
    let mut f = Fixture::at("b b b", 0, 0);
    feed(&mut f, "/b<CR>");
    assert_eq!(f.cursor(), 2);
    f.feed(["n"]);
    assert_eq!(f.cursor(), 4);
    f.feed(["N"]);
    assert_eq!(f.cursor(), 2);
}

// ---- * / # on non-word characters --------------------------------------------------

#[test]
fn star_on_whitespace_finds_next_word_on_line() {
    // vim: when the cursor is not on a word, `*` uses the nearest word after the
    // cursor on the same line — the old implementation swept the leading whitespace
    // into the pattern too (searched " bar")
    let mut f = Fixture::at("foo bar baz", 0, 3); // cursor on the space after foo
    feed(&mut f, "*");
    assert_eq!(f.cursor(), 4, "should jump to bar");
    assert_eq!(
        f.vim.search.pattern.as_deref(),
        Some(r"\bbar\b"),
        "the pattern must not contain whitespace"
    );
}

#[test]
fn star_on_last_word_char_of_buffer() {
    // the old word_bounds_at returned an empty span on the buffer's last char → empty pattern
    let mut f = Fixture::at("a b c", 0, 4);
    feed(&mut f, "*");
    assert_eq!(f.vim.search.pattern.as_deref(), Some(r"\bc\b"));
}

// ---- t / T line bounds ---------------------------------------------------------------

#[test]
fn till_target_at_line_start_fails_instead_of_crossing_lines() {
    // the x of `tx` sits at line start: vim fails with a bell; the old implementation
    // parked the cursor on the same line's `\n`
    let f = edit("xabc\nxabc", 1, 2, &["t", "x"]);
    assert_eq!(f.line(), 1, "on a failed motion the cursor should stay on the original line");
}

#[test]
fn backward_till_hit_at_line_end_fails() {
    // the z of `Tz` is the last char of the line: nothing to the right can host the
    // landing, so vim fails
    let f = edit("abcz", 0, 0, &["T", "z"]);
    assert_eq!(f.cursor(), 0, "on a failed motion the cursor does not move");
}

#[test]
fn backward_till_within_line_still_works() {
    // `Ta` stops one char to the right of a
    let f = edit("abcd", 0, 3, &["T", "a"]);
    assert_eq!(f.cursor(), 1);
}

#[test]
fn till_finds_within_line_still_works() {
    // `t` lands on the char just before the target
    let f = edit("abcb", 0, 0, &["t", "b"]);
    assert_eq!(f.cursor(), 0);
    let f = edit("abcb", 0, 2, &["t", "b"]);
    assert_eq!(f.cursor(), 2);
}

// ---- uppercase register append -------------------------------------------------------

#[test]
fn uppercase_register_appends() {
    let mut f = Fixture::at("one\ntwo\nthree\n", 0, 0);
    f.feed(["\"", "a", "y", "y", "j", "\"", "A", "y", "y"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a");
    assert_eq!(reg.text, "one\ntwo\n");
    assert_eq!(reg.kind, RegisterKind::Linewise);
    // unnamed points at the merged whole (vim: the unnamed register is an alias of
    // the last written register)
    let unnamed = f.vim.registers.lock().unwrap().get(UNNAMED).cloned().unwrap();
    assert_eq!(unnamed.text, "one\ntwo\n");
}

#[test]
fn uppercase_charwise_append() {
    let mut f = Fixture::at("abcd", 0, 0);
    f.feed(["\"", "a", "y", "l", "l", "\"", "A", "y", "l"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().unwrap();
    assert_eq!(reg.text, "ab", "charwise append should concatenate into the same register");
    assert_eq!(reg.kind, RegisterKind::Charwise);
}

#[test]
fn uppercase_delete_appends_and_pastes() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["\"", "a", "d", "d"]); // "a = "aa\n", bb,cc remain
    f.feed(["\"", "A", "d", "d"]); // delete bb → "a = "aa\nbb\n", cc remains
    f.feed(["\"", "a", "p"]); // paste out the merged two lines
    assert_eq!(f.text(), "cc\naa\nbb\n", "the appended deletes should paste out as one block");
}

// ---- empty text objects -----------------------------------------------------------------

#[test]
fn ci_quote_on_empty_pair_enters_insert() {
    let mut f = Fixture::at("\"\"", 0, 1);
    f.feed(["c", "i", "\""]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "\"x\"", "insertion should work inside empty quotes");
}

#[test]
fn ci_paren_on_empty_pair_enters_insert() {
    let mut f = Fixture::at("f()", 0, 2);
    f.feed(["c", "i", "("]);
    f.type_text("y");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "f(y)", "insertion should work inside empty parens");
}

#[test]
fn ci_paren_with_cursor_on_open_paren() {
    // cursor sitting on the `(`: vim still selects the block containing it
    let mut f = Fixture::at("(ab)", 0, 0);
    f.feed(["c", "i", "("]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "(x)", "with the cursor on ( the inside of the parens should be selected");
}

#[test]
fn visual_i_paren_on_empty_pair_collapses() {
    let mut f = Fixture::at("()", 0, 0);
    f.feed(["v", "i", "("]);
    assert_eq!(f.cursor(), 1, "an empty selection should collapse to the inner span's start");
}

// ---- C-e / C-y and Tab --------------------------------------------------------------

#[test]
fn ctrl_e_and_ctrl_y_scroll_the_view() {
    let text = (0..20)
        .map(|i| format!("l{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut f = Fixture::new(&text);
    f.host.viewport = (0, 4); // 5-line viewport
    f.feed(["G"]);
    f.host.viewport = (0, 4);
    f.host.scrolled_to.clear();
    let before = f.line();
    f.feed(["<C-y>"]);
    assert!(
        f.host.scrolled_to.contains(&before),
        "C-y must keep the cursor line visible (view moves up)"
    );
}

#[test]
fn tab_jumps_forward_like_c_i() {
    let text = "one\ntwo\nthree\nfour".to_owned();
    let mut f = Fixture::at(&text, 1, 0);
    f.feed(["G"]); // jump: line 1 -> line 3
    assert_eq!(f.line(), 3);
    f.feed(["<C-o>"]); // back to line 1
    assert_eq!(f.line(), 1);
    f.feed(["<Tab>"]); // forward again — same entry as <C-i>
    assert_eq!(f.line(), 3, "Tab should equal C-i (jumplist forward)");
}

// ---- % count as percentage ------------------------------------------------------------

#[test]
fn percent_with_count_jumps_to_file_percentage() {
    let text = (0..101)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let f = edit(&text, 0, 0, &["5", "0", "%"]);
    assert_eq!(f.line(), 50, "50% should go to line 51 (0-based 50)");
    let f = edit(&text, 10, 0, &["1", "0", "0", "%"]);
    assert_eq!(f.line(), 100);
}

// ---- :N line jump ----------------------------------------------------------------------

#[test]
fn colon_number_jumps_to_line() {
    let text = "a\nb\nc\nd\n".to_owned();
    let f = edit(&text, 0, 0, &[":", "3", "<CR>"]);
    assert_eq!(f.line(), 2);
}

// ---- :s i / I flags ---------------------------------------------------------------------

#[test]
fn substitute_i_flag_forces_case_insensitive() {
    // smartcase is on by default: a pattern with uppercase matches case-sensitively;
    // the i flag overrides that
    let mut f = Fixture::at("Foo foo FOO", 0, 0);
    f.feed([
        ":", "s", "/", "F", "o", "o", "/", "X", "/", "g", "i", "<CR>",
    ]);
    assert_eq!(f.text(), "X X X", "the i flag should force case-insensitive matching");
}

#[test]
fn substitute_capital_i_flag_forces_case_sensitive() {
    let mut f = Fixture::at("Foo foo FOO", 0, 0);
    f.feed([
        ":", "%", "s", "/", "f", "o", "o", "/", "X", "/", "g", "I", "<CR>",
    ]);
    assert_eq!(f.text(), "Foo X FOO", "the I flag should force case-sensitive matching");
}

// ---- marks in Ex ranges -----------------------------------------------------------------

#[test]
fn mark_range_substitute() {
    let mut f = Fixture::at("x\nfoo\nfoo\nfoo\nx", 0, 0);
    f.feed(["j", "m", "a", "j", "m", "b"]); // 'a=line1, 'b=line2
    f.feed([
        ":", "'", "a", ",", "'", "b", "s", "/", "f", "o", "o", "/", "b", "a", "r", "/", "<CR>",
    ]);
    assert_eq!(f.text(), "x\nbar\nbar\nfoo\nx", "both lines of the 'a,'b range should be substituted");
}

// ---- @: repeats the last Ex command ------------------------------------------------------

#[test]
fn at_colon_repeats_last_ex_command() {
    let mut f = Fixture::at("foo\nbar\nfoo\nbaz\nfoo\nqux", 0, 0);
    f.feed([":", "s", "/", "f", "o", "o", "/", "X", "/", "<CR>"]);
    assert_eq!(f.text(), "X\nbar\nfoo\nbaz\nfoo\nqux");
    f.feed(["j", "j", "@", ":"]); // line 2
    assert_eq!(f.text(), "X\nbar\nX\nbaz\nfoo\nqux");
    f.feed(["j", "j", "@", ":"]); // line 4
    assert_eq!(f.text(), "X\nbar\nX\nbaz\nX\nqux");
}

// ---- Replace-mode Backspace restores -----------------------------------------------------

#[test]
fn replace_mode_backspace_restores_overwritten_chars() {
    // BS restores the original chars that were overwritten: Z covered c, so first c is
    // restored, then b
    let mut f = Fixture::at("abcdef", 0, 0);
    f.feed(["R"]);
    f.type_text("XYZ");
    assert_eq!(f.text(), "XYZdef");
    f.feed_raw(Key::named("backspace"));
    f.feed_raw(Key::named("backspace"));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Xbcdef", "BS should restore c then b in turn");
    assert_eq!(f.cursor(), 0);

    // three BSs fully undo this replacement run
    let mut f = Fixture::at("abcdef", 0, 0);
    f.feed(["R"]);
    f.type_text("XYZ");
    for _ in 0..3 {
        f.feed_raw(Key::named("backspace"));
    }
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcdef", "char-by-char BS should fully restore the original text");
}

#[test]
fn replace_mode_backspace_past_line_end_deletes() {
    // the appended part (past end of line) has no original to restore: BS deletes normally
    let mut f = Fixture::at("abc", 0, 0);
    f.feed(["R"]);
    f.type_text("abcdefgh");
    assert_eq!(f.text(), "abcdefgh");
    f.feed_raw(Key::named("backspace"));
    f.feed_raw(Key::named("backspace"));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcdef");
}

// ---- gq wrapping by display width -----------------------------------------------------------

#[test]
fn gq_wraps_by_display_width_for_cjk() {
    // each word is 2 CJK chars = 4 display columns; at width 10 each line fits at most two words (9 columns)
    let mut f = Fixture::at("中中 中中 中中 中中 中中\n", 0, 0);
    f.vim.options_mut().textwidth = 10;
    f.feed(["g", "q", "q"]);
    for line in f.text().lines() {
        let w: usize = line.chars().map(vimcore::buffer::char_display_width).sum();
        assert!(w <= 10, "after wrapping each line's display width must not exceed textwidth: {line}");
    }
    assert_eq!(f.text().lines().count(), 3, "5 words should wrap into 3 lines of 2+2+1");
}

// ---- :q / :q! forced flag --------------------------------------------------------------------

#[test]
fn plain_q_is_not_forced_but_q_bang_is() {
    let mut f = Fixture::at("x", 0, 0);
    f.feed([":", "q", "<CR>"]);
    assert!(f.host.close_requested);
    assert!(!f.host.close_forced, ":q should not carry the forced flag");

    let mut f = Fixture::at("x", 0, 0);
    f.feed([":", "q", "!", "<CR>"]);
    assert!(f.host.close_requested);
    assert!(f.host.close_forced, ":q! should carry the forced flag");

    let mut f = Fixture::at("x", 0, 0);
    f.feed(["Z", "Z"]);
    assert!(f.host.close_forced, "ZZ closes forced after writing");
    let mut f = Fixture::at("x", 0, 0);
    f.feed(["Z", "Q"]);
    assert!(f.host.close_forced, "ZQ equals :q!");
}

// ---- / and ? share the search history -----------------------------------------------------------

#[test]
fn search_history_is_shared_between_slash_and_question() {
    let mut f = Fixture::at("abc abc", 0, 0);
    feed(&mut f, "/abc<CR>");
    feed(&mut f, "?"); // open the ? prompt without executing
    f.feed_raw(Key::named("up"));
    assert_eq!(f.vim.cmdline.buffer, "abc", "? should be able to browse the history of /");
}

// ---- misc: register-group regression guard --------------------------------------------------------

#[test]
fn yank_register_roundtrip_intact() {
    let mut f = Fixture::at("hello", 0, 0);
    f.feed(["y", "y"]);
    assert_eq!(
        f.vim.registers.lock().unwrap().get(YANK).map(|r| r.text.clone()),
        Some("hello".into())
    );
    assert_eq!(
        f.vim.registers.lock().unwrap().get(CLIPBOARD),
        None,
        "an explicit yank must not touch the + register"
    );
}
