//! Round-12 review regression (vim 9.1 probe-verified).
//!
//! Coverage: multibyte panic in range scanning, address offset chains, `w`/`b`
//! whitespace-line semantics, `e` at the last word end, `d$`/`dg_` on empty lines, `aw`
//! blank objects, `d-`/`d+` edges, `2*`/`2gN`, `ci"` outside quotes, Ex: `:s` replay
//! with range, `:2,2j`, `:d _`, explicit counts, `@/`, `:set ic` refresh.

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

fn lines(f: &Fixture) -> Vec<String> {
    (0..f.buf.line_count())
        .map(|i| f.buf.line_range(i))
        .map(|r| f.buf.slice(r))
        .collect()
}

// ---- range parsing (cmdline) ------------------------------------------------

#[test]
fn ex_range_multibyte_mark_name_no_panic() {
    // `:'中d` once cut 中 mid-byte and panicked in split_at (user-triggerable)
    let f = edit("中文\nabc\n", 0, 0, &[":", "'", "中", "d", "\r"]);
    // vim 9.1: E78: Unknown mark (non-ASCII mark names are invalid)
    assert_eq!(f.host.statuses.last().map(String::as_str), Some("E78: Unknown mark"));
    assert_eq!(f.buf.line_count(), 2);
}

#[test]
fn ex_range_multibyte_after_quote_in_list_no_panic() {
    // the same crash path for '中 after a `,`
    let f = edit("中文\nabc\ndef\n", 0, 0, &[":", ",", "'", "中", "d", "\r"]);
    assert_eq!(f.buf.line_count(), 3);
}

#[test]
fn ex_range_offset_chain_accumulates() {
    // :5+2+1d deletes line 8 (vim 9.1); the old implementation parsed only the first
    // segment (+2) and deleted line 5
    let f = edit("1\n2\n3\n4\n5\n6\n7\n8\n9\n", 0, 0, &[":", "5", "+", "2", "+", "1", "d", "\r"]);
    assert_eq!(lines(&f), vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n", "7\n", "9\n"]);
}

#[test]
fn ex_range_offset_mixed_signs() {
    // :3-1d deletes line 2 (vim probe)
    let f = edit("1\n2\n3\n4\n", 0, 0, &[":", "3", "-", "1", "d", "\r"]);
    assert_eq!(lines(&f), vec!["1\n", "3\n", "4\n"]);
}

// ---- w / b / e (word.rs) ----------------------------------------------------

#[test]
fn w_skips_whitespace_only_line() {
    // vim stops only on truly empty lines; whitespace-only lines are skipped freely
    // (9.1: w lands on the d of def)
    let f = edit("abc\n   \ndef\n", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 8);
}

#[test]
fn w_still_stops_on_empty_line() {
    // stopping on truly empty lines is unchanged (existing behavior, agrees with vim)
    let f = edit("ab\n\ncd\n", 0, 0, &["w"]);
    assert_eq!(f.line(), 1);
}

#[test]
fn b_skips_whitespace_only_line_upward() {
    let f = edit("abc\n   \ndef\n", 2, 0, &["b"]);
    assert_eq!(f.cursor(), 0); // the a of abc, no stop on the whitespace line
}

#[test]
fn ye_at_last_word_end_yanks_word_only() {
    // the old implementation returned buf.len(), so ye swallowed the trailing newline → "c\n"
    let mut f = edit("abc\n", 0, 2, &["y", "e"]);
    let unnamed = f.vim.registers.lock().unwrap().get('"').cloned().map(|r| r.text.clone());
    assert_eq!(unnamed.as_deref(), Some("c"));
    f.feed(["p"]);
    assert_eq!(f.text(), "abcc\n");
}

#[test]
fn de_at_last_word_end_keeps_newline() {
    let f = edit("abc\n", 0, 2, &["d", "e"]);
    assert_eq!(f.text(), "ab\n");
}

// ---- d$ / dg_ (motions + ops) -----------------------------------------------

#[test]
fn dollar_delete_on_empty_line_is_noop() {
    // the old implementation's Inclusive endpoint swallowed the \n, joining two lines
    // into one (vim: no-op)
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

// ---- aw / iw (objects) ------------------------------------------------------

#[test]
fn daw_on_whitespace_takes_blank_run_plus_next_word() {
    // vim: daw on inter-word blank deletes "   bar", leaving "foo" (the old
    // implementation deleted only the blank run)
    let f = edit("foo   bar\n", 0, 4, &["d", "a", "w"]);
    assert_eq!(lines(&f), vec!["foo\n"]);
}

#[test]
fn yaw_on_whitespace_yanks_blank_run_plus_next_word() {
    let f = edit("foo   bar\n", 0, 4, &["y", "a", "w"]);
    let unnamed = f.vim.registers.lock().unwrap().get('"').cloned().map(|r| r.text.clone());
    assert_eq!(unnamed.as_deref(), Some("   bar"));
}

#[test]
fn daw_on_whitespace_line_merges_next_line_away() {
    // vim: on a whitespace line daw deletes the next line's word together with the
    // newline, turning 3 lines into 1
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
    // aw on a word is unaffected by this batch of changes
    let f = edit("foo bar\n", 0, 0, &["d", "a", "w"]);
    assert_eq!(f.text(), "bar\n");
}

// ---- d- / d+ (motions) ------------------------------------------------------

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

// ---- search jumps ------------------------------------------------------------

#[test]
fn star_count_jumps_count_matches() {
    // 2* lands on the second next match (the old implementation hardcoded 1)
    let f = edit("a b a b a b\n", 0, 0, &["2", "*"]);
    assert_eq!(f.cursor(), 8);
}

#[test]
fn gn_count_backward_selects_distinct_matches() {
    // 2gN once re-selected the same match each step (range.start always contained itself)
    let f = edit("a b a b\n", 0, 7, &["/", "a", "\r", "2", "g", "N"]);
    let (a, b) = f.vim.marks.active_visual().expect("2gN selects a range");
    // backwards from the first match: step one selects the current [0,1), step two wraps
    // to the last [4,5) (wrapscan; the old implementation's step two would re-select
    // [0,1) itself)
    assert_eq!((a, b), (4, 5));
}

// ---- ci" (objects) ------------------------------------------------------------

#[test]
fn ciquote_between_strings_replaces_gap() {
    // vim: the left quote acts as the open quote, paired with the next quote → the gap
    // ' then ' gets replaced
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 13, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "say \"hi\"X\"bye\"\n");
}

#[test]
fn ciquote_before_first_string_takes_first_pair() {
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 0, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\"\n");
}

#[test]
fn ciquote_inside_string_unchanged() {
    let mut f = edit("say \"hi\" then \"bye\"\n", 0, 5, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\"\n");
}

// ---- Ex (:s replay / :j / :d register / counts) -------------------------------

#[test]
fn substitute_replay_honors_explicit_range() {
    // replaying :3,4s acts on lines 3-4 (the old implementation dropped the range and
    // fell to the cursor line)
    let mut f = edit("a\naxa\naxa\naxa\na\n", 0, 0, &[":", "2", ",", "4", "s", "/", "a", "/", "b", "/", "\r"]);
    f.feed(["g", "g"]);
    f.feed([":", "3", ",", "4", "s", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "bxa\n", "bxb\n", "bxb\n", "a\n"]);
}

#[test]
fn substitute_updates_search_state() {
    // after :s, @/ holds the substitute pattern and n searches with it (vim)
    let mut f = edit("axa\nbxb\n", 0, 0, &[":", "s", "/", "x", "/", "Y", "/", "\r"]);
    let slash = f.vim.registers.lock().unwrap().get('/').cloned().map(|r| r.text.clone());
    assert_eq!(slash.as_deref(), Some("x"));
    f.feed(["n"]);
    assert_eq!(f.cursor(), 5); // the second x
}

#[test]
fn join_equal_two_address_range_is_noop() {
    // :2,2j is a no-op (vim :h :j); a single address :2j and a bare :j still join
    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", ",", "2", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "b\n", "c\n"]);

    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "b c\n"]);

    let f = edit("a\nb\nc\n", 0, 0, &[":", "j", "\r"]);
    assert_eq!(lines(&f), vec!["a b\n", "c\n"]);
}

#[test]
fn delete_blackhole_register_stays_empty() {
    // :d _ does not enter "1 (vim blackhole)
    let f = edit("a\nb\nc\n", 0, 0, &[":", "2", "d", " ", "_", "\r"]);
    assert_eq!(f.vim.registers.lock().unwrap().get('1'), None);
    assert_eq!(lines(&f), vec!["a\n", "c\n"]);
}

#[test]
fn delete_explicit_count_one_anchors_at_last_line() {
    // :1,2d 1 deletes line 2 (an explicit count re-anchors even when it is 1; the old
    // implementation deleted 1-2)
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", " ", "1", "\r"]);
    assert_eq!(lines(&f), vec!["a\n", "c\n"]);
}

#[test]
fn yank_explicit_count_one_takes_last_line_only() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "y", " ", "1", "\r"]);
    let zero = f.vim.registers.lock().unwrap().get('0').cloned().map(|r| r.text.clone());
    assert_eq!(zero.as_deref(), Some("b\n"));
}

// ---- :set ic highlight refresh --------------------------------------------------

#[test]
fn set_noic_refreshes_live_highlights() {
    // after :set noic the existing highlights are re-published under the new rule (Foo drops out)
    let mut f = edit("Foo\nfoo\n", 0, 0, &["/", "f", "o", "o", "\r"]);
    assert_eq!(f.host.highlights.len(), 2);
    f.feed([":", "s", "e", "t", " ", "n", "o", "i", "c", "\r"]);
    assert_eq!(f.host.highlights.len(), 1);
}

// ---- UX loop (zero-width search / :5> / visual scrolling / :marks) ---------------

#[test]
fn empty_line_pattern_finds_empty_lines() {
    // /^$ finds empty lines (vim 9.1 ggn evidence); the old implementation filtered out
    // all zero-width matches and reported E486
    let f = edit("ab\n\ncd\n", 0, 0, &["/", "^", "$", "\r"]);
    assert_eq!(f.line(), 1);
    assert_eq!(f.cursor(), 3); // start of the empty line
}

#[test]
fn empty_line_pattern_no_phantom_match_after_trailing_newline() {
    // the phantom position after the trailing newline does not count as a match: n stays
    // wrapped on the single empty line
    let f = edit("ab\n\ncd\n", 0, 0, &["/", "^", "$", "\r", "n"]);
    assert_eq!(f.line(), 1);
}

#[test]
fn shift_in_range_reports_e16_when_out_of_range() {
    // [Round-15 correction] `:>`/`:<` are now implemented (previously E492). vim 9.1 probe:
    // on a 2-line buffer `:5>` is out of range → "E16: Invalid range"; `:2>` shifts right
    // normally (engine default et → 4-space indent; vim default noet → TAB).
    let f = edit("a\nb\n", 0, 0, &[":", "5", ">", "\r"]);
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "got {last:?}");
    let f = edit("a\nb\n", 0, 0, &[":", "2", ">", "\r"]);
    assert_eq!(f.text(), "a\n    b\n", ":2> should shift line 2 right (et → 4 spaces)");
    assert!(
        !f.host.statuses.iter().any(|s| s.starts_with("E492")),
        ":2> is not an unknown command: {:?}",
        f.host.statuses
    );
}

#[test]
fn visual_scroll_commands_keep_selection() {
    // zz/zt/zb in visual mode: scroll report + selection preserved (round11's pending
    // loop closed; vim behavior)
    let mut f = edit("one\ntwo\nthree\nfour\nfive\n", 0, 0, &["V", "j"]);
    f.feed(["z", "z"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::Visual { .. }), "selection survives zz");
    assert_eq!(f.host.scrolled_to.last(), Some(&1));
    f.feed(["z", "t"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::Visual { .. }));
    assert_eq!(f.host.scrolled_to.last(), Some(&1));
    f.feed(["z", "b"]);
    assert!(matches!(f.vim.mode(), vimcore::Mode::Visual { .. }));
    assert_eq!(f.host.scrolled_to.last(), Some(&1));
}

#[test]
fn marks_listing_dedupes_user_set_dot() {
    // after m. sets the . mark, :marks lists the dot mark once only (the old
    // implementation listed one row from items plus one from last_change)
    let mut f = edit("a\nb\n", 0, 0, &["m", "."]);
    f.feed([":", "m", "a", "r", "k", "s", "\r"]);
    let dot_rows = f
        .host
        .statuses
        .iter()
        .filter(|s| s.starts_with(" ."))
        .count();
    assert_eq!(dot_rows, 1, "dot mark listed once, got {:?}", f.host.statuses);
}
