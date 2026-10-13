//! audit3_a: independent audit of the Ex commands / command-line editing surface / `:set` options
//! (2026-10-08).
//!
//! One `#[test]` per finding; the assertions hold the **expected results of the vim 9.1 oracle**, so
//! they should fail on the current working tree (becoming regressions once fixed). oracle channel:
//! `vim -Nu NONE -N -i NONE -n -s keys buf` (typeahead; the key-sequence file ends with Esc/:wq;
//! message-class results are sampled via v:errmsg / `execute("messages")`). Option shapes are
//! aligned per the task's iron rule (vim `-Nu NONE` ships noet/ts=8/sw=8 while the engine defaults
//! to et/ts=4/sw=4 — wherever alignment matters the test explicitly `:set`s and notes it).
//!
//! ## Falsified (oracle proved engine and vim agree; skip next round)
//! - Falsified: `:/foo/` search address when there is a following match on the cursor's own line —
//!   vim also jumps to the next line (with the cursor at (0,0) on `a foo`, `:/foo/d` deletes the
//!   second line); the engine's `line > cursor_line` check agrees.
//! - Falsified: `:2,2j 3` (equal two-address range + count) — vim also joins the three lines 2-4;
//!   the engine joins the same after reanchoring.
//! - Falsified: the cursor landing of `:sort`/`:retab` — vim likewise does not move the cursor line
//!   (the engine's `:sort` landing on the range's first line agrees with vim; `:retab` leaves it
//!   alone on both sides).
//! - Falsified: indent stripping of `:ce 10` on an over-wide line — vim also turns
//!   `    averylongline` into `averylongline` (no padding re-added after trimming); engine agrees.
//! - Falsified: the missing `"/` search register — the engine has `Registers::store_search`, and the
//!   cmdline `<C-r>/` behavior agrees with vim.
//! - Falsified: `:reg !` garbage argument — the vim side is also silent (v:errmsg empty); engine agrees.
//! - Falsified: the `&` flag of `:s/a/Z/&` — vim silently accepts it (the flag-effect side is known
//!   divergence #73); the engine's acceptance behavior agrees.
//! - Falsified: `:d #` — vim also accepts `#` as a register (the buffer likewise deletes the line,
//!   no error); engine agrees (the divergence narrows to `%`/`.`/`:`, see A6).

mod common;

use common::Fixture;
use vimcore::key::Key;

/// Feed one Ex command line character by character into the prompt (each item of `f.feed` is "one key").
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

// ---------------------------------------------------------------- A1 (P1)
// `:retab` measures every whitespace run's width starting from display column 0, ignoring the run's start column.
//
// Repro: buffer `ab\tcd` (TAB at display column 2), `:set et ts=4` then `:retab 4`.
// vim 9.1: the TAB occupies only 2 cells (column 2 → the next ts boundary 4), expanded under et
//     into 2 spaces → "ab  cd".
// Engine: retab_run treats the run as starting from column 0 and measures 4 cells → 4 spaces
//     → "ab    cd" (silently corrupting the width).
// oracle: printf 'ab\tcd\n' > b; python3 -c "open('k','wb').write(
//   b':set et ts=4\r:retab 4\r:call writefile([getline(1)],\"out\")\r:wq\r')"
//   && vim -Nu NONE -N -i NONE -n -s k b; od -c out
//   → "a b     c d" (exactly 2 spaces).
#[test]
fn retab_measures_runs_from_their_start_column() {
    let mut f = Fixture::new("ab\tcd\n");
    // Alignment: the oracle runs under -Nu NONE (shipping noet ts=8) and first does :set et ts=4;
    // the engine defaults to et/ts=4, but set it explicitly anyway to keep both sides the same shape.
    ex(&mut f, "set et ts=4");
    ex(&mut f, "retab 4");
    assert_eq!(
        f.text(),
        "ab  cd\n",
        "vim 9.1: :retab measures each run from its own start column (TAB@col2, ts=4 → 2 spaces)"
    );
}

// ---------------------------------------------------------------- A2 (P1)
// `:%+1d`: when `%` carries an offset it gets collapsed to the last address and the offset is dropped entirely.
//
// Repro: a 5-line buffer, `:%+1d`.
// vim 9.1: `%` = 1,$ (+1 absorbed/ignored) → deletes the whole buffer (writefile samples empty).
// Engine: parse_range takes the `%+1` base `%` directly as last (the comment itself admits "collapsed
//     to its LAST line"), the offset is dropped → range (5,5) → deletes only line 5.
// oracle: printf 'l1\nl2\nl3\nl4\nl5\n' > b; keys b':%%+1d\r:call
//   writefile(getline(1,"$"),"out")\r:wq\r' → out is empty (buffer deleted entirely).
#[test]
fn percent_range_with_offset_keeps_whole_file_semantics() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    ex(&mut f, "%+1d");
    assert_eq!(
        f.text(),
        "",
        "vim 9.1: :%+1d empties the whole file (% = 1,$; the offset does not collapse % to the last address)"
    );
}

// ---------------------------------------------------------------- A3 (P2)
// The numeric argument of `:pu {N}` is a register name (:put has no count syntax); the engine treats it as a discarded count.
//
// Repro: after `dd dd dd` ("1=c, "2=b, "3=a, buffer left with "d"), `:pu 3`.
// vim 9.1: puts register 3 → "d\na".
// Engine: ex_put uses parse_reg_count; the digit head → count 3 which is dropped by
//     `let (_, _count)`, the register falls back to None → puts the unnamed register ("c) → "d\nc".
// oracle: printf 'a\nb\nc\nd\n' > b; keys b'ddddddd\x1b:pu 3\r:call
//   writefile(getline(1,"$"),"out")\r:wq\r' → out = "d\na\n"
//   (the preceding probe proved "3 is exactly "a" after three dd).
#[test]
fn put_digit_argument_names_a_register() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["d", "d", "d", "d", "d", "d"]);
    ex(&mut f, "pu 3");
    assert_eq!(
        f.text(),
        "d\na\n",
        "vim 9.1: :pu 3 puts register 3 (:put has no count syntax), it does not treat 3 as a line count"
    );
}

// ---------------------------------------------------------------- A4 (P2)
// The default width of bare `:ce`/`:ri` after `:set tw=0`: vim uses 80, the engine uses `tw.max(1)`=1.
//
// Repro: after `:set tw=0`, `:ce` on "ab".
// vim 9.1: `:h :ce` — when width is absent or 0, use 'textwidth'; with tw=0 → 80 →
//     39 columns of padding + "ab" (under the shipped noet ts=8 the padding = 4 TAB + 7 spaces).
// Engine: ex_align's `width = textwidth.max(1)` → 1; every line width ≥ 1 exits early →
//     the whole command silently no-ops.
// oracle: printf 'ab\n' > b; keys b':set tw=0\r:ce\r:call writefile(
//   [getline(1)],"out")\r:wq\r' → od -c out = \t \t \t \t + 7 spaces + "ab".
#[test]
fn center_with_zero_textwidth_uses_80_columns() {
    let mut f = Fixture::new("ab\n");
    // Alignment: the oracle is -Nu NONE shipping noet+ts=8; pin the engine side to the same shape explicitly.
    ex(&mut f, "set tw=0 noet ts=8");
    ex(&mut f, "ce");
    assert_eq!(
        f.text(),
        "\t\t\t\t       ab\n",
        "vim 9.1: with tw=0, :ce centers to 80 columns (4 TAB + 7 spaces + ab)"
    );
}

// ---------------------------------------------------------------- A5 (P2)
// Command-line `<C-p>`/`<C-n>` (history recall) are completely unbound — the keys are silently swallowed by the prompt.
//
// Repro: after `:s/a/X/<CR>`, type `j` `:` `<C-p>` `<CR>`.
// vim 9.1: c_CTRL-P recalls the previous command line and executes it → line 2 also gets replaced → "Xa|Xa".
// Engine: cmdline_key falls through to the catch-all arm for a Char with the control modifier,
//     silently Consumed → Enter on an empty command line is a no-op → "Xa|aa".
// oracle: printf 'aa\naa\n' > b; keys b':s/a/X/\rj:\x10\r:call writefile(
//   getline(1,"$"),"out")\r:wq\r' → out = "Xa\nXa\n".
#[test]
fn cmdline_ctrl_p_recalls_history() {
    let mut f = Fixture::at("aa\naa\n", 0, 0);
    ex(&mut f, "s/a/X/");
    f.feed(["j", ":"]);
    f.feed_raw(Key::ctrl_char('p'));
    f.feed(["<CR>"]);
    assert_eq!(
        f.text(),
        "Xa\nXa\n",
        "vim 9.1: <C-p> on the command line recalls and executes the previous history entry"
    );
}

// ---------------------------------------------------------------- A6 (P2)
// The Ex side `:d`/`:y` accepting the read-only register characters `%`/`.`/`:` as targets: vim
// reports E488 Trailing characters and leaves the buffer alone; the engine executes anyway and
// pollutes the register slots.
//
// Repro a: `:1d %`. vim: E488, buffer unchanged; engine deletes the line + named['%'] gets written.
// Repro b: after `:1d .`, `".p`. vim: E488, `".` is still empty (p has nothing to paste);
//     engine: named['.'] stores the deleted line → `".p` pastes the just-deleted line back
//     (the `".` "last inserted" semantics gets overwritten; the read side get(LAST_INSERT) reads that slot directly).
// oracle: keys b':1d %\r:call writefile(["em=".v:errmsg],"out")\r:wq\r'
//   → "em=E488: Trailing characters: %", buffer stays "a\nb\n";
//   `:1d .` → "E488: Trailing characters: ."; `:1y %` gives the same message.
#[test]
fn delete_percent_colon_dot_targets_are_rejected() {
    // a) vim rejects: buffer unchanged + E488 message
    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1d %");
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: :1d % reports E488, buffer unchanged");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E488: Trailing characters: %")),
        "vim 9.1 message E488: Trailing characters: % (the current engine deletes silently with no message)"
    );

    // b) the `.` pollution shape: the deleted line must not come back to life via `".`
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    ex(&mut f, "1d .");
    f.feed(["\"", ".", "p"]);
    assert_eq!(
        f.text(),
        "ab\ncd\n",
        "vim 9.1: after :1d . is rejected, `\".p` has nothing to paste (the engine wrote the deleted line into named['.'])"
    );
}

// ---------------------------------------------------------------- A7 (P2)
// `:{n}j 1`: an explicit join with count 1 should be a no-op, but the engine joins two lines anyway.
//
// Repro: a 6-line buffer, `:5j 1`.
// vim 9.1: count = the number of lines to join; with 1 line there is nothing to join → buffer unchanged.
// Engine: after reanchor_range(count=1), addr_count==1 does not trigger the `:2,2j` equal-range
//     early exit; lines==1 still yields one seam → "l5 l6".
// oracle: keys b':5j 1\r:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → out = "l1\nl2\nl3\nl4\nl5\nl6\n" (all 6 lines unchanged).
#[test]
fn join_with_explicit_count_one_is_a_noop() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\nl6\n");
    ex(&mut f, "5j 1");
    assert_eq!(
        f.text(),
        "l1\nl2\nl3\nl4\nl5\nl6\n",
        "vim 9.1: count=1 in :5j 1 means join 1 line = a no-op"
    );
}

// ---------------------------------------------------------------- A8 (P3)
// When both `i`/`I` flags appear on `:s`, vim processes it as "the later one wins", but the engine
// checks `contains('i')` first — case-insensitive regardless of order.
//
// Repro: on `FOO\nfoo`, `:%s/foo/B/iI` (i first, I second).
// vim 9.1: the last-position I wins → case-sensitive → only the "foo" line is replaced → "FOO|B".
//     (Contrast `Ii` → "B|B", the last-position i wins.)
// Engine: `if flags.contains('i') { insensitive } else if contains('I')` → both orders give "B|B".
// oracle: keys b':%s/foo/B/iI\r:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → out = "FOO\nB\n"; the Ii order → "B\nB\n".
#[test]
fn substitute_case_flags_last_one_wins() {
    let mut f = Fixture::at("FOO\nfoo\n", 0, 0);
    ex(&mut f, "%s/foo/B/iI");
    assert_eq!(
        f.text(),
        "FOO\nB\n",
        "vim 9.1: with iI both present the last-position I wins (case-sensitive, the FOO line is not replaced)"
    );
}

// ---------------------------------------------------------------- A9 (P3)
// `:ce`/`:ri`/`:le` move the cursor to the range's first line; vim does not move the cursor line at all.
//
// Repro: a 4-line buffer, cursor `G` (line 4) then `:1,3le`.
// vim 9.1: cursor still on line 4 (line(".") == 4).
// Engine: ex_align's epilogue `cursor.offset = clamp_to_line_end(line_start(first))` →
//     jumps to line 1.
// oracle: keys b'G:1,3le\r:call writefile(["le=".line(".")],"out")\r:wq\r'
//   → out = "le=4".
#[test]
fn align_commands_leave_the_cursor_line_alone() {
    let mut f = Fixture::at("eee\n ddd\n  ccc\n   bbb\n", 3, 4);
    ex(&mut f, "1,3le");
    assert_eq!(
        f.line(),
        3,
        "vim 9.1: :1,3le does not move the cursor line (the engine moved it to the range's first line)"
    );
}

// --------------------------------------------------------------- A10 (P3)
// A 4th garbage field on `:s` (`:s/a/b/c/d`) only rings a mute bell in the engine; vim reports E488.
//
// Repro: on `ab`, `:1s/a/b/c/d`.
// vim 9.1: v:errmsg = "E488: Trailing characters: /d", buffer unchanged.
// Engine: after splitting out the 4th part, `parts.next().is_some()` → bare bell, no message at all.
// oracle: keys b':1s/a/b/c/d\r:call writefile(["em=".v:errmsg],"out")\r:wq\r'
//   → out = "em=E488: Trailing characters: /d".
#[test]
fn substitute_extra_field_reports_e488() {
    let mut f = Fixture::at("ab\n", 0, 0);
    ex(&mut f, "1s/a/b/c/d");
    assert_eq!(f.text(), "ab\n", "neither side performs the substitution");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E488: Trailing characters: /d"),
        "vim 9.1: garbage in the 4th field reports E488: Trailing characters: /d"
    );
}

// --------------------------------------------------------------- A11 (P3)
// Two argument-error sides of `:delmarks` are silent in the engine: bare `:delm` (vim E471 Argument
// required) and a reversed range `:delm z-a` (vim E475 Invalid argument: z-a; the engine silently skips).
//
// oracle: keys b':delm\r:call writefile(["em=".v:errmsg],"out")\r:qa!\r'
//   → "em=E471: Argument required";
//   keys b'mz:delm z-a\r:call writefile(["em=".v:errmsg],"out")\r:qa!\r'
//   → "em=E475: Invalid argument: z-a".
#[test]
fn delmarks_argument_errors_are_reported() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "delm");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E471: Argument required"),
        "vim 9.1: bare :delm reports E471 (the engine silently no-ops)"
    );

    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["m", "z"]);
    ex(&mut f, "delm z-a");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E475: Invalid argument: z-a"),
        "vim 9.1: a reversed range reports E475 (the engine silently skips)"
    );
}

// --------------------------------------------------------------- A12 (P3)
// `:set all`: vim lists every option; the engine gives E518: Unknown option: all.
//
// oracle: printf ':set all\n:qa!\n' | vim -es -Nu NONE -N -n /dev/null
//   → stdout prints the full option table (tabstop=8, shiftwidth=8, textwidth=0 …).
#[test]
fn set_all_lists_every_option() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "set all");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop")),
        "vim 9.1: :set all lists every option (including tabstop); the engine reports E518"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("E518: Unknown option: all")),
        "all is not an unknown option name"
    );
}

// --------------------------------------------------------------- A13 (P3)
// `:setglobal` (and the `:setg…` prefix family) does not exist — E492. vim accepts it (the engine is
// a single-buffer global model, accepting the spelling suffices); config.rs's rc side recognizes
// `setglobal` while the runtime ex_set does not — the two sides are split.
//
// oracle: printf 'x\n' > b; keys b':setglobal ts=2\r:call writefile(
//   ["em=".v:errmsg],"out")\r:qa!\r' → out = "em=" (no error at all).
#[test]
fn setglobal_spelling_is_accepted() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "setglobal ts=2");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1 accepts :setglobal (the engine gives E492: Not an editor command: setglobal ts=2)"
    );
}

// --------------------------------------------------------------- A14 (P3)
// With no previous substitution, the `~` in the replacement should expand to empty; the engine writes a literal `~`.
//
// Repro: in a fresh session, `:s/a/~` on `ab`.
// vim 9.1: the previous replacement is empty → result "b".
// Engine: `last_replacement.unwrap_or("~")` → literal "~" → result "~b".
// oracle: printf 'ab\n' > b; keys b':s/a/~\r:call writefile([getline(1)],
//   "out")\r:wq\r' → out = "b".
#[test]
fn fresh_tilde_replacement_expands_to_empty() {
    let mut f = Fixture::at("ab\n", 0, 0);
    ex(&mut f, "s/a/~");
    assert_eq!(
        f.text(),
        "b\n",
        "vim 9.1: with no previous substitution ~ expands to the empty string (the engine emits a literal ~)"
    );
}

// --------------------------------------------------------------- A15 (P3)
// The E488/E477 messages carry a suffix of the original command line; vim 9.1 only reports the tail
// argument itself / the short wording (round14's "message carries the full original line" was a
// misreading of the oracle back then — when fixing, the E477 text pinned by parity_round14 and the
// existing E488 assertions must be updated in sync).
//
// Repro a: `:1d 3x`. vim shows "E488: Trailing characters: x"
//     (vim also swallows the 3 as a count, leaving only x in the tail; the engine's token is "3x"
//     and appends the ": d 3x" suffix).
// Repro b: `:1,2y!`. vim shows "E477: No ! allowed"; the engine appends ": 1,2y!".
// oracle: keys b':1d 3x\r:let m=execute("messages")\r:call
//   writefile(split(m,"\n"),"out")\r:wq\r' → last line "E488: Trailing
//   characters: x" (v:errmsg likewise); `:1,2y!` through the same channel → "E477: No ! allowed".
#[test]
fn e488_e477_messages_quote_only_the_tail() {
    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1d 3x");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E488: Trailing characters:")),
        "should report E488"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.ends_with(": 1d 3x") || s.contains(": d 3x")),
        "vim 9.1's E488 does not carry the original command line as a suffix (the engine reports \"…: 3x: d 3x\")"
    );

    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1,2y!");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E477: No ! allowed"),
        "vim 9.1's E477 wording has no original-line suffix (the engine reports \"E477: No ! allowed: 1,2y!\")"
    );
}
