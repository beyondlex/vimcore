//! Round-17 regressions: the leftover `q` when macro recording stops, the `.`
//! semantics after `@a`, the `"+` register roundtrip, sentence object is/as
//! whitespace ownership, text object counts, `g_` on whitespace-only lines, insert
//! `<C-w>` whitespace runs, the `<</>>` column model, bare Ex offsets and the
//! trailing count of `:s`/`:d`.
//! Every expected behaviour was proven with vim 9.1 probes (the probes and their
//! output are in NOTES, round 17, the T series).

mod common;

use common::{edit, Fixture};
use vimcore::key::Key;

// ---- the leftover `q` when macro recording stops (T1) ----------------------

/// PROBE T1: after `qaxq` stops, `J` -- `.` must repeat the join (vim gives
/// ['b cd ef']); the old implementation committed [q, J] together, so `.` replayed
/// the MacroRecord and wedged the engine into recording state.
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

/// The `"b` prefix plus an empty recorded macro `"a qq`: stopping the macro must
/// clear the register prefix, so the following `p` reads the unnamed register
/// (= the mirror of yy, linewise opening a new line) instead of the stale empty
/// `"a`.
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

// ---- the redo semantics of @a and . (T2) ----------------------------------

/// PROBE T2: `qaxq` -> `~` -> `0@a` -> `.` -- vim replays the x inside the macro
/// (['def']); the old implementation suppressed the commit while @ ran, so `.`
/// repeated the pre-macro `~` (giving 'Cdef').
#[test]
fn dot_after_macro_replays_macro_last_change() {
    let f = edit("abcdef\n", 0, 0, &["q", "a", "x", "q"]);
    let mut f = f;
    f.feed(["~"]); // 'b' -> 'B'
    assert_eq!(f.text(), "Bcdef\n");
    f.feed(["0", "@", "a"]); // the x inside the macro: deletes 'B'
    assert_eq!(f.text(), "cdef\n");
    f.feed(["."]);
    assert_eq!(f.text(), "def\n", "T2: dot repeats the macro's x, not ~");
}

/// `.` replaying itself still does not self-extend (the T2 control): after `x.`
/// the last_change stays at a single x.
#[test]
fn dot_replay_still_does_not_self_extend() {
    let f = edit("abcdef\n", 0, 0, &["x", "."]);
    assert_eq!(f.text(), "cdef\n", "one repeat, not a self-feeding loop");
}

/// When the macro contains an insert session, `.` repeats the whole session as
/// well (the recording includes the Text step).
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

// ---- the "+ register roundtrip (T3) ---------------------------------------

/// PROBE T3 (a structural defect, host-independent): `"+yy` must push to the host
/// clipboard and `"+p` must paste the same content back. The old implementation
/// wrote into a named slot that get_for_paste can never read.
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

    // deletes go into the clipboard as well
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

/// With an empty host clipboard (a host without a real clipboard) it falls back to
/// the named slot, so the roundtrip still holds.
#[test]
fn clipboard_paste_falls_back_to_named_slot() {
    let mut f = Fixture::new("abc\n");
    f.feed(["\"", "+", "y", "y"]);
    // clear the host-side clipboard to emulate a host without one / one emptied by
    // the outside world
    f.host.clipboard = None;
    f.feed(["\"", "+", "p"]);
    assert_eq!(f.text(), "abc\nabc\n", "named-slot mirror keeps the roundtrip");
}

// ---- the sentence objects is/as (T4) ---------------------------------------

/// PROBE T4a: `dis` stops at the sentence terminator -- with the cursor on the A
/// of "Aaa. Bbb." it deletes only "Aaa."; the old implementation swallowed the
/// sentence's trailing space (leaving "Bbb." instead of " Bbb.").
#[test]
fn inner_sentence_stops_at_terminator() {
    let f = edit("Aaa. Bbb.\n", 0, 0, &["d", "i", "s"]);
    assert_eq!(f.text(), " Bbb.\n", "T4a: trailing space stays with next");
    let f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["5", "l", "d", "i", "s"]);
    assert_eq!(f.text(), "Aaa.  Ccc.\n", "middle sentence, both spaces kept");
}

/// PROBE T4b: `das` = the sentence plus its trailing whitespace (mid-paragraph);
/// the last sentence, having no trailing whitespace, falls back to the leading one.
#[test]
fn outer_sentence_trailing_then_leading_fallback() {
    let f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["5", "l", "d", "a", "s"]);
    assert_eq!(f.text(), "Aaa. Ccc.\n", "mid sentence takes its trailing ws");
    let f = edit("Aaa. Bbb.\n", 0, 0, &["5", "l", "d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n", "last sentence falls back to leading ws");
}

/// PROBE T4c: the trailing whitespace of `as` is capped at a paragraph boundary
/// (an empty line) -- the empty line survives.
#[test]
fn outer_sentence_blank_line_is_boundary() {
    let f = edit("Aaa.\n\nBbb.\n", 0, 0, &["d", "a", "s"]);
    assert_eq!(f.text(), "\nBbb.\n", "T4c: the empty line survives");
    // with the cursor on Bbb: no same-line leading whitespace -> only the sentence
    // is deleted (both the empty line and the newline survive; byte-for-byte equal
    // to the vim probe ['Aaa.', '', '|'])
    let f = edit("Aaa.\n\nBbb.\n", 2, 0, &["d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n\n\n", "sentence only; gap whitespace stays");
    // leading whitespace on the same line can be taken along (vim ['Aaa.', '',
    // 'Bbb.'])
    let f = edit("Aaa.\n\nBbb. Ccc.\n", 2, 9, &["d", "a", "s"]);
    assert_eq!(f.text(), "Aaa.\n\nBbb.\n", "same-line leading space goes too");
}

/// PROBE T4d: `2das` deletes two sentences (each with its own trailing
/// whitespace).
#[test]
fn outer_sentence_count_two() {
    let f = edit("Aaa. Bbb. Ccc.\n", 0, 0, &["2", "d", "a", "s"]);
    assert_eq!(f.text(), "Ccc.\n");
}

// ---- text object counts (T5) -----------------------------------------------

/// PROBE T5a: `d2aw` deletes two words (the old implementation was equivalent to
/// `daw`).
#[test]
fn count_repeats_text_object() {
    let f = edit("foo bar baz\n", 0, 0, &["d", "2", "a", "w"]);
    assert_eq!(f.text(), "baz\n", "T5a: d2aw deletes two words");
    let f = edit("foo bar baz\n", 0, 0, &["2", "d", "a", "w"]);
    assert_eq!(f.text(), "baz\n", "pre-count multiplies too");
    let mut f = edit("foo bar baz\n", 0, 0, &["c", "3", "a", "w"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X\n", "c3aw replaces all three");
}

/// PROBE T5b: vim's quirk with counts falls out of the repeated scan naturally --
/// `d2iw`=diw ("foo "), and `3iw` reaches "foo bar".
#[test]
fn inner_word_count_matches_vim_quirk() {
    let f = edit("foo bar baz\n", 0, 0, &["d", "2", "i", "w"]);
    assert_eq!(f.text(), "bar baz\n", "2iw covers 'foo ' like vim");
    let f = edit("foo bar baz\n", 0, 0, &["d", "3", "i", "w"]);
    assert_eq!(f.text(), " baz\n", "3iw covers 'foo bar' (blank stays)");
}

/// PROBE T5c: the same extension happens in visual mode; `v2i(` climbs to the
/// outer parenthesis.
#[test]
fn visual_object_count_extends() {
    let f = edit("foo bar baz\n", 0, 0, &["v", "2", "a", "w", "y", "0", "P"]);
    assert_eq!(f.text(), "foo bar foo bar baz\n", "v2aw yanks two words");
    let f = edit("f (1 + (2)) g\n", 0, 7, &["v", "2", "i", "(", "y", "0", "P"]);
    assert_eq!(f.text(), "1 + (2)f (1 + (2)) g\n", "v2i( climbs one level");
}

// ---- g_ on whitespace-only lines (T6) -------------------------------------

/// PROBE T6: on "   ", `dg_` from c1/c2/c3 deletes 1/2/3 characters -- the span is
/// [line_start, cursor). The old implementation used Inclusive(line_start) and
/// turned even c1 into an empty line.
#[test]
fn g_underscore_whitespace_line_spans_to_cursor() {
    let f = edit("   \nfoo\n", 0, 0, &["d", "g", "_"]);
    assert_eq!(f.text(), "  \nfoo\n", "c1 deletes one char");
    let f = edit("   \nfoo\n", 0, 1, &["d", "g", "_"]);
    assert_eq!(f.text(), " \nfoo\n", "c2 deletes two");
    let f = edit("   \nfoo\n", 0, 2, &["d", "g", "_"]);
    assert_eq!(f.text(), "\nfoo\n", "c3 deletes three");
    let mut f = edit("   \n", 0, 1, &["c", "g", "_"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "X \n", "cg_ replaces the run through the cursor");
    // non-blank lines are unchanged: g_ lands on the last non-blank character
    let f = edit("a  b\n", 0, 1, &["d", "g", "_"]);
    assert_eq!(f.text(), "a\n");
}

// ---- insert <C-w> whitespace runs (T7) ------------------------------------

/// PROBE T7: on whitespace in the middle of a line, `<C-w>` removes the whole run
/// from the start of the line to the cursor in one stroke (autoindent cleared); the
/// old implementation fell into the join-lines branch and stripped 1 byte per call.
/// Joining lines only happens at col 1.
#[test]
fn ctrl_w_deletes_whitespace_run_in_one_stroke() {
    let mut f = edit("x\n  y\n", 1, 2, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\ny\n", "the whole run goes in one stroke");

    // a partial run: deleted up to the start of the line
    let mut f = edit("x\n  y\n", 1, 1, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n y\n", "only the run before the cursor");

    // C-w after typed text deletes the word as usual
    let mut f = edit("x\n  y\n", 1, 2, &["i"]);
    f.type_text("hello");
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "x\n  y\n", "typed word deleted, indent kept");

    // a non-blank prefix mid-line deletes the word as usual, without joining lines
    let mut f = edit("ab\ncd\n", 1, 1, &["i"]);
    f.feed_raw(Key::ctrl_char('w'));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\nd\n");
}

// ---- the <</>> column model (T8) ------------------------------------------

fn shift_fixture(initial: &str, line: usize, col: usize, opts: &str) -> Fixture {
    let mut f = Fixture::at(initial, line, col);
    // go through :set so the real option pipeline is exercised (noet/ts/sw)
    for c in opts.chars() {
        f.feed([&c.to_string()]);
    }
    f.feed(["<CR>"]);
    f
}

/// PROBE T8a: the left-shift re-expression under noet ts=8 sw=4 -- `\tx` ->
/// `    x`, `\t\tx` -> `\t    x` (1 tab + 4 spaces). The old implementation
/// stripped bytes and got `\tx`.
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

/// PROBE T8b: the right shift re-expresses by tabstop -- under noet the remainder
/// past a whole tab becomes spaces; col 5 + sw 4 = col 9 -> `\t ` (1 tab + 1 space).
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

/// PROBE T8c: whitespace-only lines take part in the shift (`<<` empties them,
/// `>>` appends), truly empty lines are skipped.
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

// ---- bare Ex offsets and trailing counts (T9/T10) --------------------------

/// PROBE T9: a bare `+`/`-` = ±1 (vim :h :range, "If a number is omitted, 1 is
/// used"). The old implementation reported E1247 for the empty digit string.
#[test]
fn ex_bare_plus_minus_offsets() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "+", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nccc\n", ":+d deletes the next line");
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "bbb\nccc\n", ":-d deletes the previous line");
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "5", "+", "d", "<CR>"]);
    assert_eq!(
        f.text(),
        "aaa\nbbb\nccc\n",
        ":5+d past the end runs nothing (vim E16)"
    );
    assert!(f.host.statuses.iter().any(|s| s.contains("E16")));
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", ".", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "bbb\nccc\n", ":.-d deletes the line above");
    // bare signs inside a chained offset: :+-d = +1-1 = the current line
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &[":", "+", "-", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nccc\n", ":+-d re-anchors at the cursor line");
    // a numeric overflow still reports E1247 (the round-15 behaviour is kept)
    let f = edit("aaa\nbbb\n", 0, 0, &[":", "1", "+", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "9", "d", "<CR>"]);
    assert_eq!(f.text(), "aaa\nbbb\n", "huge offset still refused");
    assert!(f.host.statuses.iter().any(|s| s.contains("E1247")));
}

/// PROBE T10a: `:d2` with a directly attached digit = `:d 2` (the vim ex parser
/// ends the command name at a digit).
#[test]
fn ex_packed_count_for_delete() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &[":", "d", "2", "<CR>"]);
    assert_eq!(f.text(), "ccc\n", ":d2 deletes two lines");
}

/// PROBE T10b: the trailing count of `:s` = N lines counted from the last line of
/// the range.
#[test]
fn ex_substitute_trailing_count() {
    let f = edit("aaa\naaa\naaa\n", 0, 0, &[":", "s", "/", "a", "/", "X", "/", " ", "3", "<CR>"]);
    assert_eq!(f.text(), "Xaa\nXaa\nXaa\n", "count 3 from line 1");
    let f = edit("aaa\naaa\naaa\n", 0, 0, &[
        ":", "1", ",", "2", "s", "/", "a", "/", "X", "/", "2", "<CR>",
    ]);
    assert_eq!(f.text(), "aaa\nXaa\nXaa\n", ":1,2s count 2 = lines 2-3");
    let f = edit("aaa\naaa\n", 0, 0, &[
        ":", "s", "/", "a", "/", "X", "/", "g", "2", "<CR>",
    ]);
    assert_eq!(f.text(), "XXX\nXXX\n", "g flag + count combine");
}

/// The report format of `:s` matches vim: with the number of replacements <=
/// 'report' (default 2) it stays silent; 3 or more reads
/// "{n} substitutions on {m} lines" (singular line).
#[test]
fn substitute_report_format_matches_vim() {
    // one replacement = silent (vim's v:statusmsg is empty)
    let mut f = Fixture::new("aaa\n");
    f.feed([":", "s", "/", "a", "/", "X", "/", "<CR>"]);
    assert_eq!(f.text(), "Xaa\n");
    assert_eq!(f.host.statuses, Vec::<String>::new(), "single sub is silent");
    // two replacements on one line (the g flag) <= the report threshold = silent
    // (oracle osr2)
    let mut f = Fixture::new("aa\n");
    f.feed([":", "s", "/", "a", "/", "X", "/", "g", "<CR>"]);
    assert_eq!(f.text(), "XX\n");
    assert_eq!(f.host.statuses, Vec::<String>::new(), "2 subs is silent");
    // three replacements across lines = "on N lines"
    let mut f = Fixture::new("a1\na2\na3\n");
    f.feed([":", "1", ",", "3", "s", "/", "a", "/", "X", "/", "<CR>"]);
    assert_eq!(f.host.statuses, ["3 substitutions on 3 lines"]);
}

// ---- the :nohlsearch abbreviation prefixes (T11) ---------------------------

/// vim's abbreviation rule: every unambiguous prefix of nohlsearch works (the old
/// implementation stopped at nohls).
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
