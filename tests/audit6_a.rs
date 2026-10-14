//! audit6_a: domain A (Ex commands / cmdline editing / Ex addresses & ranges /
//! search / `:s` / `:set` / `:g`) — fifth independent audit round
//! (2026-10-09), agent A. 27 new bug probes (red) + 7 falsification/regression
//! pins (green).
//!
//! Method: corners the previous round (audit5_a) did not sweep — the missing
//! surface of `:m`/`:t`/`:>`/`:=`/`:p`/`:l`/`:nu`/`:k`/`:@`, `'[`/`']` booking
//! for `:y`/`:d`/`:pu`, the line-count reports of `:d`/`:y`/`:pu` (misc1.c
//! msgmore / "N lines yanked"), the `:s` report threshold (do_sub_msg's
//! sub_nsubs > p_report), the TAB folding of `:le {indent}` (an audit4
//! open item), cmdline cursor editing (c_CTRL-B), cmdline `<C-k>` digraphs,
//! the second colon of `::`/`:5:`, count=0 for `:y`/`:d`/`:j` (E939), the
//! `:reg`/`:marks` headers.
//!
//! Every expectation is empirically verified through the local `/usr/bin/vim`
//! (9.1 patches 1-1752) typeahead channel (sampling in /tmp/audit6_o; the key
//! order and output are written in each comment; an o-prefixed name = the
//! writefile output file name). The engine's default divergences
//! (ignorecase=on, ts=4, tw=78) are pinned flat with `:set` first in probes
//! that touch options. All probes turn into regressions (zeroed after the
//! fifth-round fixes).
//!
//! ## Falsified (oracle proved the engine agrees with vim 9.1; no re-check
//! ## needed next round)
//! - `:?a?+1y b` (backward-search address + offset + command): oracle obs
//!   b=b — engine agrees.
//! - `:0d`: oracle od0 = [b,c] with no error (address 0 is legal for line
//!   commands, deletes the first line) — engine agrees.
//! - cursor parks on the range's first line after `:2,4sort` (oracle osc
//!   p=2 1) — engine agrees.
//! - `:%s/zz//n` with zero matches reports E486 (oracle onf) — engine agrees.
//! - `<C-r>/` pastes the last search into the cmdline (oracle ocr =
//!   E492: foo) — engine agrees.
//! - the `:dl` family = :delete + the l/p/# print flags (change.txt
//!   ":dl delete and list"; oracle odl4: `:1,3dl` reports "3 fewer lines" and
//!   prints `d$`, register l empty) — audit5 A-7's direction was right; the
//!   printed [line] and the report shape have an independent bug (E-A-27).
//! - the numeric argument of `:2y 7` is a COUNT, not register 7 (the
//!   `!((argt & EX_COUNT) && VIM_ISDIGIT)` rule at ex_docmd.c:2373; oracle
//!   oy7a = "3 lines yanked", the yank range matches the engine's count path)
//!   — audit5 A-18's clamping shape holds.
//!
//! ## Related-domain notes (for the B/C domains and the main session)
//! - normal-mode `3dd`/`3yy` also produce msgmore reports in vim
//!   ("3 fewer lines" / "3 lines yanked"); the engine is equally silent
//!   (ad-hoc probe measured statuses=[]). Same root cause as E-A-17/E-A-19;
//!   fixing the reporting layer should cover these too.
//! - the 2-substitution reports pinned by tests/engine.rs
//!   ("2 substitutions on 1 line") and tests/review_regressions.rs
//!   ("2 substitutions on 2 lines") conflict with the oracle (E939 — see the
//!   E-A-16 comment); those tests must be corrected when the fix lands.

mod common;

use common::Fixture;
use vimcore::VimBuffer;

/// Feed one Ex command line into the prompt char by char (each `f.feed` item
/// is "one key").
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

/// The cursor line (0-based).
fn line_of(f: &Fixture) -> usize {
    f.line()
}

// ------------------------------------------------------------- E-A-1 (P2)
// `:m`/`:move` regression pin (the family was once missing and fell to E492;
// now implemented by `ex_copymove` in src/cmdline.rs). Sibling line commands
// (:d/:y/:j/:sort/:pu/:s) share the same range machinery.
//
// oracle (om, typeahead): buf [a,b,c,d], keys `:2m0<CR>…`:
//   text [b,a,c,d], no error, cursor p=1 1 (ex_cmds.c ex_copymove/do_move).
#[test]
fn move_command() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2m0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2m0` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "b\na\nc\nd\n",
        "vim 9.1: `:2m0` moves line 2 to the top of the file (oracle om)"
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(0),
        "vim 9.1: after `:m` the cursor lands at the start of the moved line (oracle om p=1 1)"
    );
}

// ------------------------------------------------------------- E-A-2 (P2)
// The landing semantics of `:m`: the cursor lands on the first non-blank of
// the moved line (begline at the end of do_move).
//
// oracle (om2): buf ['  x','a','b'], `:1m2<CR>`:
//   text [a,'  x',b], cursor p=2 3 (line 2 col 3 = the 'x' of the moved line).
#[test]
fn move_cursor_lands_on_moved_line_first_non_blank() {
    let mut f = Fixture::new("  x\na\nb\n");
    ex(&mut f, "1m2");
    assert_eq!(f.text(), "a\n  x\nb\n", "oracle om2");
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1) + 2,
        "vim 9.1: cursor = first non-blank of the moved line (oracle om2 p=2 3)"
    );
}

// ------------------------------------------------------------- E-A-3 (P2)
// `:t`/`:co`/`:copy` regression pin: `:2t0` copies line 2 to the top of the
// file (the family was once missing; now implemented by `ex_copymove`).
//
// oracle (ot): buf [a,b,c,d], `:2t0<CR>`:
//   [b,a,b,c,d], cursor p=1 1 (start of the new copied line).
#[test]
fn copy_command() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2t0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2t0` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(f.text(), "b\na\nb\nc\nd\n", "oracle ot");
    assert_eq!(
        f.cursor(),
        f.buf.line_start(0),
        "vim 9.1: `:t` parks the cursor at the start of the copied line (oracle ot p=1 1)"
    );
}

// ------------------------------------------------------------- E-A-4 (P2)
// The Ex indent commands `:>`/`:<<`/`:{range}>{count}` are missing (:h :>);
// the engine returns E492. normal `>>`/`<<` is implemented; the Ex side is a
// same-family absence.
//
// oracle (osh): buf [a,b,c], `:set sw=4 ts=4 et`, `:2,3><CR>`:
//   [a,'    b','    c'], cursor p=3 5 = first non-blank of the range's
//   [last address] line (the generic Ex-command parking rule: line2 + begline).
#[test]
fn ex_shift_right_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "set sw=4");
    ex(&mut f, "set ts=4");
    ex(&mut f, "set et");
    ex(&mut f, "2,3>");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:2,3>` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "a\n    b\n    c\n",
        "oracle osh: lines 2-3 each shift right by one shiftwidth"
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2) + 4,
        "vim 9.1: cursor lands at the first non-blank of the range's last line (oracle osh p=3 5)"
    );
}

// ------------------------------------------------------------- E-A-5 (P3)
// `:=` is missing: it prints the line number of the range's [last address]
// (ex_docmd.c ex_equal; no range = the last line's number, not the cursor
// line); the engine returns E492.
//
// oracle (oeq/oeq2): buf [a,b,c]:
//   `:=` → "3" (last line number); `:2=` → "2"; `:2,3=` → "3".
#[test]
fn equals_prints_range_last_line() {
    let mut f = Fixture::at("a\nb\nc\n", 1, 0);
    ex(&mut f, "=");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:=` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["3".to_owned()],
        "vim 9.1: bare `:=` prints the last line's number 3 (oracle oeq), not the cursor line 2"
    );
    ex(&mut f, "2=");
    assert_eq!(
        f.host.statuses.last(),
        Some(&"2".to_owned()),
        "vim 9.1: `:2=` prints the address's line number (oracle oeq2 m1=2)"
    );
}

// ------------------------------------------------------------- E-A-6 (P3)
// `:p`/`:print` is missing: it prints the range's lines to the message area;
// the engine returns E492.
//
// oracle (op): buf [a,b,c], `let m = execute('1,2p')` → "a\nb".
#[test]
fn print_command_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "1,2p");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2p` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["a".to_owned(), "b".to_owned()],
        "vim 9.1: `:1,2p` prints line by line (oracle op)"
    );
}

// ------------------------------------------------------------- E-A-7 (P3)
// `:l`/`:list` is missing: list-style printing (TAB as ^I, `$` at line end);
// the engine returns E492.
//
// oracle (ol): buf ["a","\tb","c"], `execute('1,2l')` → "a$\n^Ib$".
#[test]
fn list_command_missing() {
    let mut f = Fixture::new("a\n\tb\nc\n");
    ex(&mut f, "1,2l");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2l` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["a$".to_owned(), "^Ib$".to_owned()],
        "vim 9.1: `:1,2l` prints a$ and ^Ib$ (oracle ol)"
    );
}

// ------------------------------------------------------------- E-A-8 (P3)
// `:nu`/`:#`/`:number` is missing: printing with a line-number prefix. The
// format = a right-aligned line number of numberwidth-1 columns + space + line
// text (default nw=4 → "  1 a"); the engine returns E492.
//
// oracle (onu): buf [a,b], `execute('1,2nu')` → "  1 a\n  2 b".
#[test]
fn number_command_missing() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "1,2nu");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:1,2nu` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.host.statuses,
        vec!["  1 a".to_owned(), "  2 b".to_owned()],
        "vim 9.1: the `:nu` line-number prefix format (oracle onu)"
    );
}

// ------------------------------------------------------------- E-A-9 (P3)
// `:k{mark}` / `:ma[rk] {mark}` (the Ex commands that set a mark) is missing:
// after `:3ka`, `` `a `` should jump to line 3; the engine returns E492 for
// `:3ka` and leaves the mark unset.
//
// oracle (ok): buf [a,b,c], keys `:3ka<CR>` `` `a `` → p=3 1.
#[test]
fn k_mark_command_missing() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "3ka");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:3ka` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    f.feed(["`", "a"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: `:3ka` sets mark a, `` `a `` jumps to line 3 col 1 (oracle ok p=3 1)"
    );
}

// ------------------------------------------------------------- E-A-10 (P3)
// `:@{reg}` executes the register's content as Ex command lines (:h :@); the
// engine returns E492.
//
// oracle (oat): buf [a,b,c], after `@a="1,2d\r"`, `:@a` → buffer [c], no error.
#[test]
fn at_register_executes_ex() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.vim
        .registers
        .store_yank(Some('a'), "1,2d\n".into(), vimcore::registers::RegisterKind::Linewise);
    ex(&mut f, "@a");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:@a` is a legal command; actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.text(),
        "c\n",
        "vim 9.1: `:@a` executes the '1,2d' stored in register a (oracle oat)"
    );
}

// ------------------------------------------------------------- E-A-11 (P2)
// The whole cmdline cursor-editing family is missing: `<C-b>`/`<C-e>`/
// `<Left>`/`<Right>`/`<Home>`/`<End>` (c_CTRL-B/c_CTRL-E/c_<Left>/c_<Right>)
// move the cmdline cursor and allow editing from the middle; the engine's
// cmdline buffer has only an append/pop model, so these keys are silently
// swallowed and new keys always land at the line end.
//
// oracle (ocb, typeahead): keys `:set ts=4` `\x02` `x` `<CR>`:
//   cmdline 'xset ts=4' → E492: Not an editor command: xset ts=4.
// Engine: `<C-b>` is swallowed, 'x' appends → 'set ts=4x' → E521: Number
//   required after =: ts=4x.
#[test]
fn cmdline_ctrl_b_moves_cursor_to_start() {
    let mut f = Fixture::new("ab\n");
    f.feed([
        ":", "s", "e", "t", " ", "t", "s", "=", "4", "<C-b>", "x", "<CR>",
    ]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains("xset ts=4")),
        "vim 9.1: the x typed after <C-b> lands at the line start (oracle ocb: \
         E492: xset ts=4); actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-12 (P3)
// Cmdline `<C-k>` digraphs (c_CTRL-K): `:<C-k>ss` types 'ß' into the cmdline.
// Insert-mode <C-k> is implemented (audit H3); the cmdline side is the missing
// sibling shape.
//
// oracle (ock): keys `:` `\x0b` `ss` `<CR>` → the E492 message contains 'ß'.
// Engine: C-k is swallowed → E492: Not an editor command: ss.
#[test]
fn cmdline_ctrl_k_digraph() {
    let mut f = Fixture::new("x\n");
    f.feed([":", "<C-k>", "s", "s", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains('ß')),
        "vim 9.1: <C-k>ss produces ß on the cmdline (oracle ock's E492 message \
         contains ß); actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-13 (P3)
// `:y` does not book `'[`/`']`: op_yank sets b_op_start/b_op_end to the ends of
// the yanked text (Ex :y and plain yy share the same op_yank path). The
// engine's ex_yank_lines goes through yank_span and leaves the marks untouched
// (audit5 C-5 only fixed the normal-side paste path).
//
// oracle (oy): buf [a,b,c,d], after `:2,3y<CR>`:
//   `` `[ `` → p=2 1; `` `] `` → q=3 1.
#[test]
fn ex_yank_sets_marks() {
    let mut f = Fixture::at("a\nb\nc\nd\n", 0, 0);
    ex(&mut f, "2,3y");
    f.feed(["`", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "vim 9.1: after `:2,3y`, `[ = start of the yank's first line (oracle oy p=2 1)"
    );
    f.feed(["`", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(2),
        "vim 9.1: after `:2,3y`, ] = start of the yank's last line (oracle oy q=3 1)"
    );
}

// ------------------------------------------------------------- falsification pin (green)
// `'[`/`']` for `:d`: the engine resolves both marks to the deletion point
// (oracle od: `'[`=p=2 1, `']`=q=2 1; the engine probe already passes) —
// audit5 C-5/C-6's booking chain covers :d; pinned against regressions.
#[test]
fn ex_delete_marks_bracket() {
    let mut f = Fixture::at("a\nb\nc\nd\n", 0, 0);
    ex(&mut f, "2,3d");
    f.feed(["'", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "oracle od: `'[` = deletion point line 2 col 1 (engine passes; regression pin)"
    );
    f.feed(["'", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "oracle od: `']` = deletion point line 2 col 1"
    );
}

// ------------------------------------------------------------- E-A-14 (P3)
// `:pu` does not book `'[`/`']`: do_put sets both marks. The normal-side p/P
// is already booked in the engine (audit5 C-5); the Ex-side ex_put is not.
//
// oracle (opu): buf [a,b], after `yy` then `:pu<CR>`:
//   `'[` → p=2 1; `']` → q=2 1 (for a linewise put, ']' also sits at col 1 of
//   the pasted line).
// Note: the audit draft wrote p=3 1, a sampling slip that misread the cursor
// line by one; re-testing `:pu` shows the paste lands on line 2 (`yy` on line
// 1, `:pu` pastes on the line below it).
#[test]
fn ex_put_sets_marks() {
    let mut f = Fixture::at("a\nb\n", 0, 0);
    f.feed(["y", "y"]);
    ex(&mut f, "pu");
    f.feed(["'", "["]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "vim 9.1: after `:pu`, '[ = the pasted first line (oracle opu p=2 1)"
    );
    f.feed(["'", "]"]);
    assert_eq!(
        f.cursor(),
        f.buf.line_start(1),
        "vim 9.1: after `:pu`, '] is on the same pasted line (oracle opu q=2 1)"
    );
}

// ------------------------------------------------------------- E-A-15 (P3)
// The indent column of `:le {indent}` is not folded into TABs per
// 'expandtab'/'tabstop' (an audit4 open item): under noet+ts=4, `:le 8` folds
// into two TABs (the set_indent rule); the engine always uses 8 spaces.
//
// oracle (ole_out): buf ["中文"], `:set noet ts=4` + `:le 8` + `:w!`:
//   od = \t \t 中 文 \n.
#[test]
fn left_indent_folds_to_tabs_under_noexpandtab() {
    let mut f = Fixture::new("中文\n");
    ex(&mut f, "set noet");
    ex(&mut f, "set ts=4");
    ex(&mut f, "le 8");
    assert_eq!(
        f.text(),
        "\t\t中文\n",
        "vim 9.1: under noet+ts=4 the `:le 8` prefix folds into two TABs (oracle ole_out)"
    );
}

// ------------------------------------------------------------- E-A-16 (P3)
// The `:s` completion-report threshold is off: vim only reports
// "{n} substitutions on {m} lines" when the substitution count > 'report'
// (default 2, i.e. 3+) (ex_cmds.c do_sub_msg: sub_nsubs > p_report); the
// engine reports as soon as total > 1. With 2 substitutions vim is silent
// while the engine reports "2 substitutions on …".
//
// oracle (osr2/osr3/osr4): 2 substitutions ("foo bar foo" %s/foo/qux/g) → m='';
//   3 substitutions ("aaa") → "3 substitutions on 1 line"; [a,a,a] →
//   "3 substitutions on 3 lines".
#[test]
fn substitute_report_threshold_is_three() {
    let mut f = Fixture::new("foo bar foo\n");
    ex(&mut f, "%s/foo/qux/g");
    assert_eq!(
        f.text(),
        "qux bar qux\n",
        "the substitution itself agrees on both sides"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("substitutions")),
        "vim 9.1: 2 substitutions ≤ the report threshold, silent (oracle osr2 \
         m=''); actual statuses = {:?}",
        f.host.statuses
    );
    let mut f3 = Fixture::new("aaa\n");
    ex(&mut f3, "%s/a/b/g");
    assert_eq!(
        f3.host.statuses.last(),
        Some(&"3 substitutions on 1 line".to_owned()),
        "oracle osr3: only 3 substitutions get reported"
    );
}

// ------------------------------------------------------------- E-A-17 (P3)
// `:d` does not report the line count: after deleting lines vim reports
// "{n} fewer lines" via msgmore (misc1.c, threshold > 'report', i.e. 3+),
// where n is the [net line-count change] (deleting all 5 lines leaving 1 empty
// line = 4); the engine's ex_delete_lines is silent.
//
// oracle (odel/odel2): buf 5 lines, `execute('1,5d')` → "4 fewer lines";
//   `:1,2d` silent.
#[test]
fn delete_reports_fewer_lines() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    ex(&mut f, "1,5d");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("4 fewer lines")),
        "vim 9.1: `:1,5d` reports '4 fewer lines' (net change, oracle odel); \
         actual statuses = {:?}",
        f.host.statuses
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("5 fewer lines")),
        "the net line change is 4 (5 lines → 1 empty line), not the 5 deleted lines"
    );
}

// ------------------------------------------------------------- E-A-18 (P3)
// `:pu` does not report "N more lines": the same msgmore channel (threshold >
// 'report'); the engine is silent.
//
// oracle (opu2): after `@a="x\ny\nz\n"`, `:pu a` → "3 more lines".
#[test]
fn put_reports_more_lines() {
    let mut f = Fixture::new("a\n");
    f.vim
        .registers
        .store_yank(Some('a'), "x\ny\nz\n".into(), vimcore::registers::RegisterKind::Linewise);
    ex(&mut f, "pu a");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("3 more lines")),
        "vim 9.1: `:pu a` pasting 3 lines reports '3 more lines' (oracle opu2); \
         actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-19 (P3)
// `:y` does not report "N lines yanked": op_yank reports yanks of more than
// 'report' lines (threshold 3+); the engine's ex_yank_lines is silent.
//
// oracle (oy7a): buf [a,b,c,d], `execute('2y 7')` → "3 lines yanked"
//   (count 7 clamped from line 2 to the last line = lines 2-4, 3 lines).
#[test]
fn yank_reports_lines_yanked() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2y 7");
    let got = f.vim.registers.get('"').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("b\nc\nd\n"),
        "the yank range agrees on both sides (count clamped to the buffer end)"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("3 lines yanked")),
        "vim 9.1: `:2y 7` reports '3 lines yanked' (oracle oy7a); \
         actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-20 (P3)
// `::` (the second colon): a lone `:` is the empty command — silent no-op,
// cursor unmoved (oracle odc: v:errmsg empty, p=1); the engine reports
// E492: Not an editor command: :.
#[test]
fn double_colon_is_silent_noop() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, ":");
    assert!(
        f.host.statuses.is_empty(),
        "vim 9.1: `::` is a silent empty command (oracle odc e=''); \
         actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(line_of(&f), 0, "oracle odc: cursor unmoved (p=1)");
}

// ------------------------------------------------------------- E-A-21 (P3)
// `:5:` (address + a second colon): `:` empty command + bare-address parking —
// the cursor parks on line 5's first non-blank (oracle oac p=5 1); the engine
// reports E492 for the leftover `:`.
#[test]
fn address_then_colon_parks_like_bare_address() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "5:");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1: `:5:` produces no error (oracle oac e=''); actual statuses = {:?}",
        f.host.statuses
    );
    assert_eq!(
        f.cursor(),
        f.buf.line_start(4),
        "vim 9.1: `:5:` parks the cursor on line 5's first non-blank (oracle oac p=5 1)"
    );
}

// ------------------------------------------------------------- E-A-22 (P3)
// count=0 for `:y`: :yank carries EX_COUNT without EX_ZEROR (ex_cmds.h), so
// count 0 goes to the E939 "Positive count required" path (the errormsg path
// at ex_docmd.c:2415); the command does not run and registers stay put. The
// engine treats 0 as "no count" and yanks into "0 as usual.
//
// oracle (oyz): `:2,3y 0` → E939: Positive count required (the message does
//   [not] carry the command text; the string at errors.h:2459 is exactly that
//   one sentence), buffer [a,b,c,d] and "0 unchanged (r0=). The audit draft
//   wrongly recorded the message as carrying the whole command; corrected here
//   per the measurement.
#[test]
fn yank_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2,3y 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2,3y 0` refuses to run (oracle oyz)"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("Positive count required")),
        "vim 9.1: the E939 message (oracle oyz); actual statuses = {:?}",
        f.host.statuses
    );
    assert!(
        f.vim
            .registers
            .get('0')
            .map(|r| r.text.is_empty())
            .unwrap_or(true),
        "vim 9.1: \"0 is not written (oracle oyz r0=)"
    );
}

// ------------------------------------------------------------- E-A-23 (P3)
// `:d 0` is the same as E-A-22: E939 + no deletion.
//
// oracle (odz): `:2,3d 0` → E939: Positive count required, buffer [a,b,c,d]
//   unchanged. (The message does not carry the command text; see the E-A-22
//   correction.)
#[test]
fn delete_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2,3d 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2,3d 0` refuses to run (oracle odz)"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("Positive count required")),
        "oracle odz message; actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-24 (P3)
// count=0 for `:j 0`: :join likewise has EX_COUNT without ZEROR → E939; the
// engine treats 0 as no count and joins lines 2-3 as usual.
//
// oracle (ojz): `:2j 0` → E939: Positive count required, buffer unchanged.
//   (The message does not carry the command text; see the E-A-22 correction.)
#[test]
fn join_count_zero_is_e939() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "2j 0");
    assert_eq!(
        f.text(),
        "a\nb\nc\nd\n",
        "vim 9.1: `:2j 0` refuses to run (oracle ojz)"
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E939") && s.contains("Positive count required")),
        "oracle ojz message; actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-25 (P3)
// The `:reg` output lacks the header "Type Name Content" (ex_register.c
// show_registers' msg_puts_title; the data-row format already matches the
// engine).
//
// oracle (org): after yy, `execute('reg')`'s first line = "Type Name Content".
#[test]
fn registers_listing_has_header_line() {
    let mut f = Fixture::new("a\n");
    f.feed(["y", "y"]);
    ex(&mut f, "reg");
    assert_eq!(
        f.host.statuses.first().map(String::as_str),
        Some("Type Name Content"),
        "vim 9.1: `:reg`'s first line is the header (oracle org); actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-26 (P3)
// The `:marks` output lacks a header and the row format differs: vim's header
// = "mark line  col file/text", rows = `" %c %6ld %4d %s"` (mark.c ex_marks);
// the engine's rows = "a  line 1  col 0  a" with no header. (The 0-based byte
// col values already match — audit2 R35-2 — what differs is the header and the
// layout.)
//
// oracle (omk): after `ma`, `execute('marks')` =
//   ["mark line  col file/text", " '      1    0 a", " a      1    0 a", …].
#[test]
fn marks_listing_header_and_layout() {
    let mut f = Fixture::new("a\n");
    f.feed(["m", "a"]);
    ex(&mut f, "marks");
    assert_eq!(
        f.host.statuses.first().map(String::as_str),
        Some("mark line  col file/text"),
        "vim 9.1: `:marks`' first line is the header (oracle omk); actual statuses = {:?}",
        f.host.statuses
    );
    let want = format!(" a {:>6} {:>4} a", 1, 0);
    assert!(
        f.host.statuses.contains(&want),
        "vim 9.1: the mark a row = {:?} (the \" %c %6ld %4d %s\" format, oracle \
         omk); actual statuses = {:?}",
        want,
        f.host.statuses
    );
}

// ------------------------------------------------------------- E-A-27 (P3)
// The print shape of `:dl` is wrong in two places: vim's l flag =
// ex_may_print, which prints only the [cursor line after execution] once
// (ex_docmd.c ex_may_print), and the deletion goes through the same msgmore
// report ("3 fewer lines"); the engine list-prints every deleted line and
// emits no report.
//
// oracle (odl4): buf [a,b,c,d], `execute('1,3dl')` → m =
//   "3 fewer lines\nd$" (first the report, then the list form of the cursor
//   line after execution).
#[test]
fn dl_prints_surviving_cursor_line_once() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    ex(&mut f, "1,3dl");
    assert_eq!(f.text(), "d\n", "after deleting range 1-3 only [d] remains (both sides agree)");
    assert_eq!(
        f.host.statuses,
        vec!["3 fewer lines".to_owned(), "d$".to_owned()],
        "vim 9.1: `:1,3dl` = report '3 fewer lines' + print the post-execution \
         cursor line 'd$' (oracle odl4); actual statuses = {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------------- falsification pin
// `:g`/`:v` falling to E492 is a recorded trade-off in NOTES divergence #12.
// This pin only locks the shape: E492 + buffer unchanged; rewrite it as a
// positive assertion once :g is implemented.
#[test]
fn global_command_still_e492_recorded_divergence() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "g/b/d");
    assert_eq!(
        f.text(),
        "a\nb\nc\n",
        "divergence #12: :g unimplemented, buffer unchanged"
    );
}

// ------------------------------------------------------------- falsification pin
// `:?a?+1y b` (backward-search address + offset folding + command): the engine
// already agrees (oracle obs: register b = "b\n"), pinned against regressions.
#[test]
fn backward_search_address_with_offset() {
    let mut f = Fixture::at("a\nb\nc\n", 2, 0);
    ex(&mut f, "?a?+1y b");
    let got = f.vim.registers.get('b').map(|r| r.text.clone());
    assert_eq!(
        got.as_deref(),
        Some("b\n"),
        "oracle obs: search backward for a from line 3 (line 1) +1 = line 2"
    );
}

// ------------------------------------------------------------- falsification pin
// `:0d`: address 0 is legal for line commands (oracle od0 = [b,c], no error);
// the engine agrees.
#[test]
fn zero_address_delete_removes_first_line() {
    let mut f = Fixture::new("a\nb\nc\n");
    ex(&mut f, "0d");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "oracle od0: `:0d` produces no E16"
    );
    assert_eq!(f.text(), "b\nc\n", "oracle od0: `:0d` deletes the first line");
}

// ------------------------------------------------------------- falsification pin
// The cursor parks on the sorted range's first line after `:2,4sort`
// (oracle osc p=2 1); the engine agrees.
#[test]
fn sort_cursor_parks_on_range_first_line() {
    let mut f = Fixture::at("d\nb\na\nc\nz\n", 4, 0);
    ex(&mut f, "2,4sort");
    assert_eq!(f.text(), "d\na\nb\nc\nz\n");
    assert_eq!(line_of(&f), 1, "oracle osc: cursor line 2 col 1");
}

// ------------------------------------------------------------- falsification pin
// `:%s/zz//n` with zero matches reports E486 (oracle onf); the n flag does not
// change the failure path; the engine agrees.
#[test]
fn n_flag_zero_matches_reports_e486() {
    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "%s/zz//n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E486")),
        "oracle onf: E486: Pattern not found: zz"
    );
}

// ------------------------------------------------------------- falsification pin
// `<C-r>/` (pastes the last search into the cmdline): oracle ocr = E492: foo;
// the engine agrees.
#[test]
fn ctrl_r_slash_pastes_last_search() {
    let mut f = Fixture::new("foo\n");
    f.feed(["/", "f", "o", "o", "<CR>"]);
    f.feed([":", "<C-r>", "/", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.contains("E492") && s.contains("foo")),
        "oracle ocr: after <C-r>/ pastes 'foo', E492"
    );
}
