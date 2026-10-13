//! audit5_a: domain A (Ex commands / cmdline editing / search / `:s` / `:set` /
//! addresses & ranges) — fourth independent audit round (2026-10-09).
//!
//! Method: behavioral assertions ported from `~/code/github/vim/src/testdir/`
//! (test_substitute / test_cmdline / test_search / test_excmd / test_sort /
//! test_options) + an attack-surface checklist (count=0/huge, reversed
//! addresses, empty patterns, the `~`/`&` family, `\n` patterns, `\/`/`\?`/`\&`
//! addresses, `|` command separation, cmdline `<C-r>=`/`<C-w>`, `:set` odd
//! values). Every probe's expectation was empirically verified through the
//! local `/usr/bin/vim` (9.1 patches 1-1752) typeahead channel:
//! `vim -Nu NONE -N -i NONE -n -s keys buf`, with the keys file ending in
//! `:call writefile(...)` + `:wq`; message-family samples via v:errmsg /
//! `execute()`. The engine's default divergences (ignorecase=on, ts=4) are
//! pinned flat with `:set` first in probes that touch options.
//!
//! Each test asserts the **expected vim 9.1 result** and should fail on the
//! current work tree (turning into a regression once fixed).
//!
//! ## Falsified (oracle proved the engine agrees with vim; no re-check needed)
//! - falsified: **bare numeric count prefilling the `:` range** — vim
//!   `gg4:yank<CR>` gives `"l1\nl2\nl3\nl4\n` (the count becomes `.,.+3`,
//!   oracle u8); the engine's `Char(':')` branch prefills `.,.+(n-1)`
//!   (state.rs from 3705) — consistent, not a bug.
//! - falsified: **slot selection for the `:s//` empty pattern** — oracle p4/p5:
//!   after an intervening search, `:s//Q/` uses the **search** pattern
//!   ('ccc'→'Q aaa'); with no intervention it uses the **replacement** pattern
//!   ('aaa'→'ccc Q') — i.e. vim's RE_LAST "most recent user" model. The
//!   engine's single search.pattern slot produces the same results on both
//!   paths (`:s` then set_pattern).
//! - falsified: **tie stability of `:sort! n`** — vim's `!` reverses the whole
//!   stable-ascending result (ties included): [1a,1b,2] → [2,1b,1a] (oracle
//!   u17); the engine's sort_by_key then reverse has the same shape. `:sort n`
//!   leaves ties in place (u18: [1a,1b,2]).
//! - falsified: **the number-extraction rule of `:sort n`** — `x-22` yields
//!   -22, ` 123b` yields 123, and lines without numbers compare as 0: this
//!   half matches vim (vim's exception is only "non-numeric lines as a whole
//!   come first", see the sort_n probe); consistent with the test_sort.vim
//!   expectations and the numeric_sort_key reading of the source.
//! - falsified (source reading): `//<CR>` empty pattern with a closing
//!   delimiter — split_search_offset returns pattern="" for an empty tail and
//!   reuses the last search, consistent with vim; cmdline `<BS>` deletes per
//!   char via String::pop, multibyte-safe.
//! - falsified: **normal `&`** — vim `:s/a/B/` + `j` + `&` gives [Baa,Baa]
//!   (oracle u22, `&` = replay on the current line without flags); the engine's
//!   NormalCmd::RepeatSubstitute (state.rs:5335) behaves the same; the probe
//!   went green and was withdrawn.
//! - falsified: **`:2,3sort u` is silent when nothing changes** — when the
//!   sorted range equals the original order vim emits no message (u21 agrees
//!   on both sides).
//! - falsified: **the family beyond `2/foo/yank`** — `:/foo/,/bar/d` with both
//!   ends matching and no wrapping ambiguity: the visible result of the `,`
//!   chain equals vim's (the divergence is only in address anchoring, visible
//!   through the yank in the A7 probe).

mod common;

use common::Fixture;

/// Feed one Ex command line into the prompt char by char (each `f.feed` item
/// is "one key").
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

// ---------------------------------------------------------------- 1 (P2)
// `:sort n`: lines without numbers must all sort before every numeric line
// (vim splits into two segments by "has a number or not" and sorts within each);
// the engine keys numberless lines as 0 and mixes them into the middle of the
// numeric segment.
//
// Repro: ['abc','5','-3','7','x'], `:%sort n`.
// vim 9.1: `abc x -3 5 7` (non-numeric segment abc/x first, ascending numeric
// segment after).
// Engine: numeric_sort_key('abc')=0, ('x')=0 → stable sort gives `-3 abc x 5 7`
//     (abc/x squeezed into the middle below -3).
// oracle: cd /tmp/orc5 && printf 'abc\n5\n-3\n7\nx\n' > b1 && :sort n + writefile
//   → o1 = abc~x~-3~5~7 (same shape as the test_sort.vim
//   Test_sort_large_num assertion: "Non-numeric lines are ordered before
//   numerical lines").
#[test]
fn sort_n_orders_nonnumeric_lines_first() {
    let mut f = Fixture::new("abc\n5\n-3\n7\nx\n");
    ex(&mut f, "%sort n");
    assert_eq!(
        f.text(),
        "abc\nx\n-3\n5\n7\n",
        "vim 9.1: :sort n puts the whole numberless segment first, not mixed into the numeric run as 0"
    );
}

// ---------------------------------------------------------------- 2 (P3)
// After `:sort u` deduplicates, vim reports "N fewer lines" on the message
// channel; the engine is silent.
//
// Repro: [1,1,2,2,3,3], `:%sort u`.
// vim 9.1: buffer [1,2,3] + message "3 fewer lines" (same wording asserted by
//     test_sort.vim Test_sort_cmd_report).
// Engine: ex_sort dedupes without emitting any status_message.
// oracle: `:let m = execute('%sort u')` → m = "3 fewer lines", buffer 1,2,3 (u13).
#[test]
fn sort_u_reports_lines_removed() {
    let mut f = Fixture::new("1\n1\n2\n2\n3\n3\n");
    ex(&mut f, "%sort u");
    assert_eq!(f.text(), "1\n2\n3\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("3 fewer lines")),
        "vim 9.1: :sort u reports '3 fewer lines'; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 3 (P2)
// `:s/foo/bar/0`: count 0 is an error (E939 Positive count required) and the
// whole command does not run; the engine treats 0 as "no count" and substitutes
// anyway — a command that should have been rejected changed the text.
//
// Repro: 'foo', `:s/foo/bar/0`.
// vim 9.1: text 'foo' unchanged + E939 (test_substitute.vim Test_substitute_count:
//   assert_fails('s/foo/bar/0', 'E939:')).
// Engine: `Some(0)` falls into the `_ => range` arm and substitution continues →
//   'bar', no message.
// oracle: o2 = foo~'E939: Positive count required'.
#[test]
fn substitute_count_zero_is_e939_noop() {
    let mut f = Fixture::new("foo\n");
    ex(&mut f, "s/foo/bar/0");
    assert_eq!(f.text(), "foo\n", "vim 9.1: count 0 refuses to run, text unchanged");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E939")),
        "vim 9.1: E939: Positive count required; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 4 (P3)
// The `g` flag of `:s` toggles on each occurrence: `:s/foo/FOO/gg` = two g's
// cancel = replace only the first match per line (test_substitute.vim
// Test_substitute_gdefault asserts s/foo/FOO/gg → 'FOO bar foo'). The engine's
// flags.contains('g') is always true.
//
// Repro: 'foo bar foo', `:set noic` then `:s/foo/FOO/gg`.
// vim 9.1: 'FOO bar foo'. Engine: 'FOO bar FOO'.
// oracle: o3 = FOO bar foo.
#[test]
fn doubled_g_flag_toggles_global_off() {
    let mut f = Fixture::new("foo bar foo\n");
    ex(&mut f, "set noic");
    ex(&mut f, "s/foo/FOO/gg");
    assert_eq!(
        f.text(),
        "FOO bar foo\n",
        "vim 9.1: gg cancels, replacing only the first match per line"
    );
}

// ---------------------------------------------------------------- 5 (P2)
// The `r` flag of `:s`: with an empty pattern it reuses the last [search]
// pattern (not the replacement pattern). The engine does not know `r`, reports
// E488 and runs nothing.
//
// Repro: ['aaa','ccc aaa','ccc']; `:1s/aaa/B/`, `/ccc`, `:2s//Q/r`.
// vim 9.1: line 2 'ccc aaa' → 'Q aaa' (r = RE_SEARCH = 'ccc').
// Engine: E488: Trailing characters: r, line 2 unchanged.
// oracle: p6 = B~Q aaa~ccc~'' (no E message).
#[test]
fn r_flag_reuses_last_search_pattern() {
    let mut f = Fixture::new("aaa\nccc aaa\nccc\n");
    ex(&mut f, "1s/aaa/B/");
    f.feed(["/", "c", "c", "c", "<CR>"]);
    ex(&mut f, "2s//Q/r");
    assert_eq!(
        f.text(),
        "B\nQ aaa\nccc\n",
        "vim 9.1: the r flag makes the empty pattern reuse the last search pattern"
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E488")),
        "vim 9.1: r is a legal flag, no E488; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 6 (P2)
// `:%s/\n//` is an idiom vim specializes into join (do_sub's `\n` fast path,
// ex_cmds.c:4165 onward); the engine substitutes per line and `\n` never
// matches within a line → E486, the command is dead.
//
// Repro: ['l1','l2','l3'], `:%s/\n//`.
// vim 9.1: 'l1l2l3' (same shape as test_substitute.vim Test_substitute_join).
// Engine: E486: Pattern not found: \n, all three lines unchanged.
// oracle: u8 = l1l2l3~''.
#[test]
fn substitute_newline_pattern_joins_lines() {
    let mut f = Fixture::new("l1\nl2\nl3\n");
    ex(&mut f, "%s/\\n//");
    assert_eq!(
        f.text(),
        "l1l2l3\n",
        "vim 9.1: :%s/\\n// joins the whole range into one line"
    );
}

// ---------------------------------------------------------------- 7 (P1)
// The [concatenation] syntax of a numeric address with a search address in
// `2/foo/`, plus second-address search anchoring: both layers are missing:
//
// a) syntax layer: vim's get_address ends with `while (*cmd == \'/\' || *cmd ==
//    \'?\')` — when a search address follows a number directly it merges into
//    the same address (the `N/{pattern}` shape of `:h :range`), no comma
//    needed. The engine's range character table requires a `,` before `/` after
//    a number, so `2/foo/` is parsed whole as one address base → E16: Invalid
//    range, the command is dead (scratch-verified: both `2/foo/y a` and
//    `2/foo/d` give E16; `1,/foo/y a` passes instead).
// b) anchoring layer (the next layer after fixing a): the search starts from
//    the [previous address], not from the cursor (ex_docmd.c get_address: "When
//    search follows another address, start from there"); the engine's base_line
//    always uses cursor_line.
//
// Repro: ['foo 1','foo 2','foo 3'], cursor on line 1; `:2/foo/y a`.
// vim 9.1: address = line 3; "a = "foo 3\n" (same assertion as test_cmdline.vim
//     Test_lnum_and_pattern_as_range).
// Engine: E16, "a never written (get('a') = None).
// oracle: o5 = 'foo 3' (getreg("a")).
#[test]
fn second_address_search_anchors_at_first_address() {
    let mut f = Fixture::at("foo 1\nfoo 2\nfoo 3\n", 0, 0);
    ex(&mut f, "2/foo/y a");
    let got = f.vim.registers.get('a').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("foo 3\n"),
        "vim 9.1: the second address /foo/ searches from address 2, landing on line 3"
    );
}

// ---------------------------------------------------------------- 8 (P2)
// The `\/`, `\?`, `\&` address shapes are missing: `\` is not in the engine's
// range character table, so the whole command falls to E492 and nothing runs.
// vim: `\/` = "the next matching line of the last search pattern" (following
// the previous address as the start), `\?` backward, `\&` uses the last
// replacement pattern.
//
// Repro: ['a','b','c','d']; `/b` (cursor line 2), `:2,\/d`.
// vim 9.1: searching forward from line 2 wraps back to line 2 itself → delete
// line 2 → 'a c d', cursor line 2.
// Engine: E492: Not an editor command: \/d, all four lines unchanged.
// oracle: u1 = a~c~d~''~2 (same family as test_cmdline.vim
//   Test_cmdline_search_range: 1,\/s/b/B/, \?,4s/c/C/, 1,\&s/b/B/).
#[test]
fn backslash_search_addresses_supported() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["/", "b", "<CR>"]);
    ex(&mut f, "2,\\/d");
    assert_eq!(
        f.text(),
        "a\nc\nd\n",
        "vim 9.1: `:2,\\/d` deletes the line hit by \\/"
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: the \\/ address produces no E492; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 9 (P2)
// `:dl` is `:delete` + the `l` flag (delete and print), not an unknown command
// E492 (same family in test_excmd.vim Test_ex_delete: dl/dell/delel/deletl/
// deletel/dp…). The engine's DELETE_SPELLINGS lacks the l/p/# flag shapes →
// E492 and the line is not deleted.
//
// Repro: ['foo',"\tbar"], `:dl`.
// vim 9.1: deletes the current line → leaves '\tbar' and prints `^Ibar$`
// (q2: no v:errmsg).
// Engine: E492, both lines unchanged.
// oracle: q2 = b~c~d~'' (deleting the current line foo from a,b,c,d).
#[test]
fn dl_deletes_current_line() {
    let mut f = Fixture::new("foo\n\tbar\n");
    ex(&mut f, "dl");
    assert_eq!(f.text(), "\tbar\n", "vim 9.1: :dl deletes the current line");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: :dl is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 10 (P2)
// The `|` command separator (:h :bar): after commands where "-bar is allowed"
// such as `:d`/`:s`/`:y`, a `|` starts the next command and both run. The
// engine reports E488 trailing characters for the `|…` and runs not even the
// first command — one command drags the whole line down.
//
// Repro: ['a','b','c','d'], `:1,2d|j`.
// vim 9.1: after deleting a,b the join of the rest → one line 'c d'.
// Engine: E488: Trailing characters: |j, all four lines unchanged.
// oracle: u19 = 'c d' (getline leaves one line); u20 (`:1s/a/B/|j`) confirms.
#[test]
fn bar_runs_both_ex_commands() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "1,2d|j");
    assert_eq!(
        f.text(),
        "c d\n",
        "vim 9.1: `:1,2d|j` deletes lines 1-2 then joins the remaining lines"
    );
}

// ---------------------------------------------------------------- 11 (P3)
// The bare-quote address `:'` (mark name missing): vim is silent without error
// (o9: v:errmsg '', cursor unmoved; test_cmdline.vim Test_tick_mark_in_range
// asserts `:'<CR>` does not fail); the engine reports E16: Invalid range + rings.
//
// oracle: cd /tmp/orc5; keys `:let v:errmsg=''|:'|writefile` → o9 = ''~1.
#[test]
fn bare_tick_address_is_silent() {
    let mut f = Fixture::new("x\ny\n");
    f.feed([":", "'", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        "vim 9.1: `:'` produces no message at all; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 12 (P3)
// `:set ts=0`: vim reports E487: Argument must be positive: ts=0 and rejects
// it (ts keeps its old value); the engine silently accepts ts=0 (later column
// arithmetic all falls back through .max(1), TAB width 1 cell).
//
// Repro: `:set ts=6`, `:set ts=0`, `:set ts?`.
// vim 9.1: E487 + `  tabstop=6` (o10).
// Engine: no message + `  tabstop=0`.
#[test]
fn set_ts_zero_rejected_e487() {
    let mut f = Fixture::new("ab\n");
    ex(&mut f, "set ts=6");
    ex(&mut f, "set ts=0");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E487: Argument must be positive")),
        "vim 9.1: `:set ts=0` reports E487; actual statuses = {:?}",
        f.host.statuses
    );
    ex(&mut f, "set ts?");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=6")),
        "vim 9.1: after the rejection ts stays 6; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 13 (P3)
// `:set ts=2000000` exceeds the option cap: vim reports E474: Invalid
// argument: ts=2000000 and rejects it; the engine silently clamps to
// MAX_OPTION_VALUE(1000000) — quietly becoming a different value.
//
// Repro: `:set ts=6`, `:set ts=2000000`, `:set ts?`.
// vim 9.1: E474 + tabstop=6 (u14).
// Engine: no message + tabstop=1000000.
#[test]
fn set_ts_over_limit_rejected_e474() {
    let mut f = Fixture::new("ab\n");
    ex(&mut f, "set ts=6");
    ex(&mut f, "set ts=2000000");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E474: Invalid argument")),
        "vim 9.1: `:set ts=2000000` reports E474; actual statuses = {:?}",
        f.host.statuses
    );
    ex(&mut f, "set ts?");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop=6")),
        "vim 9.1: after the rejection ts stays 6; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 14 (P3)
// `:s` count above 2147483647: vim reports E1510: Value too large and does not
// run (the E1510 assertion in test_substitute.vim Test_substitute_count); the
// engine accepts it wholesale as a usize and really substitutes.
//
// Repro: ['a','b'], `:s/./b/2147483647`.
// vim 9.1: 'a','b' unchanged + E1510 (u9).
// Engine: line 1 becomes 'b', no message.
#[test]
fn substitute_count_overflow_is_e1510() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "s/./b/2147483647");
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: an out-of-range count refuses to run");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E1510")),
        "vim 9.1: E1510: Value too large: 2147483647; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 15 (P2)
// `:s` does not accept a space as delimiter: `:s a b` is E146 (Regular
// expressions can't be delimited by letters) in vim and does nothing; the
// engine treats the space as a delimiter and really substitutes — a data
// rewrite.
//
// Repro: 'a b', `:set noic`, `:s/!/Q/` (seeding the last replacement), `:s a b`.
// vim 9.1: 'a b' unchanged + E146 (q4 same shape: 'aYb' + E146).
// Engine: pattern='a', replacement='b' → 'b b'.
#[test]
fn space_delimited_substitute_is_e146() {
    let mut f = Fixture::new("a b\n");
    ex(&mut f, "set noic");
    ex(&mut f, "s/!/Q/");
    ex(&mut f, "s a b");
    assert_eq!(f.text(), "a b\n", "vim 9.1: the space delimiter is rejected, text unchanged");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E146")),
        "vim 9.1: E146: Regular expressions can't be delimited by letters; \
         actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 16 (P3)
// `\` also cannot serve as a `:s` delimiter: `:s\foo\bar\` makes vim report
// E10: \ should be followed by /, ? or & (do_sub's check_regexp_delim, q3) and
// not run; the engine parses `\` as a delimiter for the pattern (getting the
// regex foo\bar\) and lands on a different error / no-op.
//
// Repro: 'foobar', `:s\foo\bar\`.
// vim 9.1: E10 + text unchanged. Engine: some message other than E10.
#[test]
fn backslash_delimited_substitute_is_e10() {
    let mut f = Fixture::new("foobar\n");
    ex(&mut f, "s\\foo\\bar\\");
    assert_eq!(f.text(), "foobar\n", "vim 9.1: the \\ delimiter is rejected, text unchanged");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E10:")),
        "vim 9.1: E10: \\ should be followed by /, ? or &; actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 17 (P3)
// Cmdline `<C-r>=` opens the expression prompt, evaluates and inserts the
// result (c_CTRL-R_=); on a failed evaluation it reports E121/E15 and inserts
// nothing. The engine treats `=` as an ordinary register name and pastes a
// stale value (nothing if never set), after which the following keys become
// command text → E492.
//
// Repro: `:`, `<C-r>`, `=`, `foo`, `<CR>`, `<CR>`.
// vim 9.1: E121: Undefined variable: foo, cmdline resets to empty, the second
//     CR runs empty (u11).
// Engine: `=` has no stored value → no insertion, 'foo' enters the cmdline →
//     E492: Not an editor command: foo.
#[test]
fn cmdline_ctrl_r_equals_opens_expression_prompt() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "<C-r>", "=", "f", "o", "o", "<CR>", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E121") || s.contains("E15")),
        "vim 9.1: evaluating `<C-r>=foo` fails with E121/E15; actual statuses = {:?}",
        f.host.statuses
    );
    // assertion intent corrected (2026-10-09 re-verification): the draft
    // asserted that "foo" must not appear in statuses — but vim's own E121
    // message quotes the variable name (E121: Undefined variable: foo), so
    // that assertion would not pass the oracle either. The intent is that
    // `foo` must not become [command text] (the old engine reported
    // E492: Not an editor command: foo); the assertion now checks for no E492.
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: 'foo' must not become cmdline text (no E492); actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 18 (P3)
// Cmdline `<C-w>` deletes only the one [alphanumeric word] before the cursor:
// 'set ts=4' + `<C-w>` gives 'set ts=' ('4' deleted, '=' kept); the engine
// deletes the whole non-blank run giving ''.
//
// Repro: `:`, 'set ts=4', `<C-w>`, 'z', `<CR>`.
// vim 9.1: cmdline 'set ts=z' → E521: Number required after =: ts=z (u6).
// Engine: cmdline 'z' → E492: Not an editor command: z.
#[test]
fn cmdline_ctrl_w_deletes_alnum_word_only() {
    let mut f = Fixture::new("ab\n");
    f.feed([
        ":", "s", "e", "t", " ", "t", "s", "=", "4", "<C-w>", "z", "<CR>",
    ]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E521") && s.contains("ts=z")),
        "vim 9.1: <C-w> deletes only '4', leaving 'set ts=z' on the cmdline → E521; \
         actual statuses = {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------- 19 (P3)
// `:y` with a huge count: vim accepts it and clamps to the buffer end
// (`:2y 77777777777777777777` = line 2 through the last line, silent; same
// shape as test_excmd.vim Test_address_line_overflow); the engine's
// parse_reg_count uses usize, the 20-digit number overflows → E488 trailing
// characters, the whole command rejected.
//
// Repro: ['l0','l1','l2','l3','l4'], `:2y 77777777777777777777`.
// vim 9.1: "= "l1\nl2\nl3\nl4\n", reports "4 lines yanked" (clamped to 4 lines
// > the report threshold 2; the audit draft's "no message" missed op_yank's
// report, corrected on retest).
// Engine: E488: Trailing characters: 77777777777777777777, register unchanged.
#[test]
fn huge_yank_count_clamps_silently() {
    let mut f = Fixture::new("l0\nl1\nl2\nl3\nl4\n");
    ex(&mut f, "2y 77777777777777777777");
    let got = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("l1\nl2\nl3\nl4\n"),
        "vim 9.1: a huge count clamps to the buffer end and yanks as usual"
    );
    assert!(
        f.host.statuses.iter().any(|s| s.contains("4 lines yanked")),
        "vim 9.1: clamped to 4 lines > the report threshold, '4 lines yanked' \
         (oracle hugey); actual statuses = {:?}",
        f.host.statuses
    );
}

// (The draft's item 20, "normal `&` unbound", was falsified through the engine:
// NormalCmd::RepeatSubstitute (state.rs:5335) implements the flagless replay;
// the probe went green and was withdrawn, the evidence moved into the
// falsified list.)

