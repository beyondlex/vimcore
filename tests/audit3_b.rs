//! audit3_b: independent audit of the Normal motion, operator-span and text-object domains (2026-10-08).
//!
//! One `#[test]` per finding; the assertions hold the **expected results empirically verified with
//! vim 9.1 (`-Nu NONE -N -i NONE -n -s` typeahead)**, so they should fail on the current working
//! tree. oracle channel: `python3 -c "open('b.txt','wb').write(<initial bytes>)"` +
//! `python3 -c "open('k.txt','wb').write(<keys>)"` +
//! `vim -Nu NONE -N -i NONE -n -s k.txt b.txt </dev/null`; the key file must end with `:wq\n`;
//! buffer sampling uses `:call writefile([...],"out")`. Note that vim 9.1 defaults
//! 'fixendofline' on: a modified noeol buffer gets its trailing newline re-added when written back
//! — all noeol-shape expectations below are counted as write-back bytes (i.e. vim's internal buffer text).
//!
//! Scope: src/motions.rs + src/word.rs + src/objects.rs + src/ops.rs's span_from_motion +
//! src/tables.rs command table + cmdline.rs's search motion.
//! Deduplicated against NOTES.md's 77 known divergences, the 50 round-31 audit items and the 59
//! round-32 audit items (audit2's A series just fixed TAB virtual columns/section jumps, the B
//! series just fixed the aw whitespace matrix/cw≡ce/object counts across lines — this round hunts
//! the shapes they did not cover).
//!
//! ---------------------------------------------------------------
//! Falsification list for this round (oracle proved engine agrees with vim 9.1; skip next round):
//! * `diw` on inter-word whitespace: vim likewise deletes the whitespace run ('a b' 2|diw → 'ab').
//! * void tag `<br>`: on '<p>a<br>b</p>', dit at a / b / the `<` of `<br>` all yield '<p></p>' — vim's unpaired open tags likewise do not participate in pairing; engine agrees.
//! * `dd` on a noeol last line: vim writes back "aaa\n" (fixendofline); engine text agrees.
//! * `dw` on single-line "word + trailing whitespace" buffers ('x  ' / '.  '): vim leaves one empty line ("\n"), the engine's single-w clamp agrees; for 'x  \nbar' the dw keeps the newline, agrees.
//! * `dgg`@line 1 / `dG`@last line: vim deletes whole lines, the engine's linewise span agrees.
//! * `d10|` on a short line: vim deletes only up to the clamped column ('abcde' → 'e'); engine agrees.
//! * Backward `d?pat` ending at col 1: exclusive rule (a) is byte-equivalent to a bare exclusive span ('foo pat\nnext' 2G0d?pat → 'foo \nnext'); engine agrees.
//! * `daw` on a line-end word: vim deletes only the word itself, keeping newlines/empty lines ('ab\ncd' 1|daw → '\ncd'); engine agrees (merges_following_line_break only governs whitespace starts).
//! * `}` at buffer end lands on the very last character (getcurpos col=3 on 'bbb'); the d} col-0 shape matches the engine's phantom promotion (noeol 'aaa\nbbb' 1|d} → empty).
//! ---------------------------------------------------------------

mod common;

use common::{edit, Fixture};

// ------------------------------------------------ B1 (P1) w-family span on the last noeol word is one char short

/// **B1 (P1)**: on a noeol buffer `w` stops at the last character (byte model), but vim's internal
/// buffer always has a trailing newline — `w` lands on the phantom empty line at buffer end, so
/// `dw`/`cw` cover the entire last word. The engine deletes only up to one char before the last,
/// **silently leaving the final character** (a data-corruption shape on noeol files).
/// oracle: printf 'abc def' (no trailing newline) + `5|dw` + :wq → file bytes "abc \n";
/// printf 'abc' + `dw` → "\n"; `5|cwXY<Esc>` → "abc XY\n".
/// Engine: dw yields "abc f" / "c", cw yields "abc XYf".
#[test]
fn b1_noeol_last_word_dw_cw_one_char_short() {
    // Assertion convention: the oracle samples file write-back bytes, and vim's fixendofline adds
    // the trailing newline on write-back; the engine buffer has no file model, so assertions use
    // internal text (consistent with the audit2 C series' noeol-shape convention). Semantic point:
    // `w` lands on the phantom line at buffer end, dw/cw cover the whole last word.
    let f = edit("abc def", 0, 4, &["d", "w"]);
    assert_eq!(f.text(), "abc ", "vim: on a noeol last word dw deletes all of def (w lands on the phantom line)");

    let f = edit("abc", 0, 0, &["d", "w"]);
    assert_eq!(f.text(), "", "vim: dw on a single-word noeol buffer deletes everything");

    let mut f = edit("abc def", 0, 4, &["c", "w"]);
    f.type_text("XY");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc XY", "vim: on a noeol last word cw ≡ ce covers all of def, f must not survive");
}

// ------------------------------------------------ B2 (P2) line-end `dl`/`cl`/`d<Space>` should behave like x

/// **B2 (P2)**: when `l`/`<Space>` cannot move at line end, vim's operator semantics **include the
/// cursor character** (`dl` ≡ `x`, `:h x` states the synonymy explicitly). The engine's
/// Motion::Right returns stuck → the operator rings and cancels, and the following keys leak out as
/// plain commands.
/// oracle: 'ab' + `$dl` → "a"; `$d ` (d+Space) → "a"; `$clXY<Esc>` → "aXY".
/// Engine: all three shapes do nothing (cl even turns X into an X-backspace delete).
#[test]
fn b2_exclusive_l_under_operator_includes_char_at_line_end() {
    let f = edit("ab\n", 0, 1, &["d", "l"]);
    assert_eq!(f.text(), "a\n", "vim: dl at line end ≡ x, deletes the last character");

    let f = edit("ab\n", 0, 1, &["d", " "]);
    assert_eq!(f.text(), "a\n", "vim: d<Space> at line end same as dl (Space is an alias of l)");

    let mut f = edit("ab\n", 0, 1, &["c", "l"]);
    f.type_text("XY");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "aXY\n", "vim: cl at line end rewrites the last character");
}

// ------------------------------------------------ B3 (P2) the `!` filter operator is missing

/// **B3 (P2)**: the `!{motion}{cmd}` filter operator is entirely unbound (the Operator enum has no
/// Filter, tables.rs has no `!` row). vim 9.1: on 'hello', `!!echo XY<CR>` replaces the current
/// line with the shell command's output "XY".
/// oracle: printf 'hello\n'; keys `!!echo XY\r:wq\n` → file "XY\n".
/// Engine: `!` rings and is dropped, the following keys become garbage normal-mode keys (e/c/h/o
/// moves, X deletes a character, Y enters insert), corrupting the buffer.
#[test]
fn b3_filter_operator_unbound() {
    let f = edit("hello\n", 0, 0, &["!", "!", "e", "c", "h", "o", " ", "X", "Y", "<CR>"]);
    assert_eq!(f.text(), "XY\n", "vim: !!echo XY<CR> replaces the current line with the command output");
}

// ------------------------------------------------ B4 (P2) the `=` indent operator is missing

/// **B4 (P2)**: the `={motion}` reindent operator is entirely unbound (no Indent operator row;
/// `==`/`={motion}`/visual `=` all ring the bell). vim 9.1 reindents with its built-in rules:
/// without a filetype, 'lisp' (a built-in option) gives a deterministic oracle —
/// oracle: printf '(defun foo\nbar)\n'; `:set lisp\r2gg==:wq\n` → line 2 becomes " bar)" (lisp
/// indent alignment). Engine: both the 'lisp' option and the `=` operator are missing, buffer
/// unchanged (plus E518 noise afterwards).
#[test]
fn b4_indent_operator_unbound() {
    let mut f = Fixture::at("(defun foo\nbar)\n", 1, 0);
    f.feed([":"]);
    f.type_text("set lisp");
    f.feed(["<CR>", "2", "g", "g", "=", "="]);
    assert_eq!(f.text(), "(defun foo\n bar)\n", "vim: == reindents the current line by the 'lisp' rules");
}

// ------------------------------------------------ B5 (P3) the `gm` screen-line-middle jump is missing

/// **B5 (P3)**: `gm` (move the cursor to the middle of the current screen line) is unbound. In the
/// engine, after the `g` trie miss is dropped, `m` becomes MarkSet waiting for a parameter and the
/// cursor stays put.
/// oracle: a 200-character single line (wrapped in an 80-column terminal) `1|gm` → getcurpos col 41
/// (0-based 40, middle of the first screen line). Engine: cursor stays at 0.
#[test]
fn b5_gm_middle_of_screenline_unbound() {
    let line = "x".repeat(200);
    let f = edit(&format!("{line}\n"), 0, 0, &["g", "m"]);
    assert_eq!(f.cursor(), 40, "vim: gm lands at the middle of the screen line (column 41)");
}

// ------------------------------------------------ B6 (P2) `gj`/`gk` do not move by screen lines

/// **B6 (P2)**: the engine binds `gj`/`gk` straight to `j`/`k` (the tables.rs comment says "no soft
/// wrap in v1"). Under 'wrap' (on by default) vim moves by **screen lines**: on a 200-character
/// single line, `gj` stays in the same buffer line and moves to the start of the next displayed
/// line (col 81, 1-based).
/// oracle: 200 'x' + `\nnext\n`; `1|gj` → getcurpos [0,1,81,…].
/// Engine: j jumps to buffer line 2.
#[test]
fn b6_gj_gk_move_screen_lines_not_buffer_lines() {
    let line = "x".repeat(200);
    let f = edit(&format!("{line}\nnext\n"), 0, 0, &["g", "j"]);
    assert_eq!(f.line(), 0, "vim: gj stays on the same buffer line (moving down one screen line within the soft wrap)");
    assert_eq!(f.cursor(), 80, "vim: gj lands at the start of the second displayed line (0-based column 80)");
}

// ------------------------------------------------ B7 (P2) search-operator column-1 landing does not promote to linewise

/// **B7 (P2)**: `d/pat`'s span hand-builds `OpSpan { linewise: false }` in
/// cmdline.rs::search_motion_operator, **bypassing** ops::span_from_motion's column-1
/// exclusive→linewise promotion rule (`:h exclusive`). In vim this shape is a LINEWISE delete: the
/// register is accounted linewise and `p` puts whole lines.
/// oracle: on 'aaa\nxyz\n', `1|d/xyz<CR>p` → file "xyz\naaa\n" (the whole aaa line is put below
/// xyz). Engine: charwise delete + charwise put → "xaaayz\n".
#[test]
fn b7_search_motion_column1_missing_linewise_promotion() {
    let f = edit("aaa\nxyz\n", 0, 0, &["d", "/", "x", "y", "z", "<CR>", "p"]);
    assert_eq!(f.text(), "xyz\naaa\n", "vim: d/pat landing at column 1 promotes to linewise, p puts whole lines");
}

// ------------------------------------------------ B8 (P2) a bare CR is treated as whitespace

/// **B8 (P2)**: word.rs::char_class uses `char::is_whitespace` — a bare `\r` (a ^M in the file,
/// not a newline) classifies as Blank, so `w`/`b` step over it freely. vim's utf_class records only
/// ' ' and '\t' as whitespace; \r is class 1 (punctuation) — `w` must stop on the ^M.
/// oracle: printf 'ab\rcd\n'; `1|w` → getcurpos col 3 (0-based 2, exactly on the ^M).
/// Engine: w lands on the 'c' (0-based 3).
#[test]
fn b8_bare_cr_is_punct_for_word_motions() {
    let f = edit("ab\rcd\n", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 2, "vim: w stops on the bare ^M (punctuation class), does not step over");
}

// ------------------------------------------------ B9 (P2) the `go` byte jump is missing and its degradation is dangerous

/// **B9 (P2)**: `go` (`:h go`, move the cursor to buffer byte [count]) is unbound. After the engine's
/// trie miss drops the `g`, the `o` executes with the prefix count — **5go opens 5 new lines and
/// enters insert mode** (the count is discarded in the miss retry; measured: opens 1 line) — the
/// same shape as audit2 F1's gd, but this one corrupts data.
/// oracle: on 'abcdefgh\n', `5go` → getcurpos col 5 (0-based 4, byte 5), buffer unchanged.
/// Engine: opens 5 lines + Insert.
#[test]
fn b9_go_byte_motion_unbound_degrades_to_count_open_line() {
    let f = edit("abcdefgh\n", 0, 0, &["5", "g", "o"]);
    assert_eq!(f.text(), "abcdefgh\n", "vim: go only jumps bytes, never opens lines");
    assert_eq!(f.cursor(), 4, "vim: 5go lands on the 5th byte ('e')");
}

// ------------------------------------------------ B10 (P2) z<CR>/z./z- missing and z. degrades to `.`

/// **B10 (P2)**: the scroll family `z<CR>` (scroll current line to window top), `z.` (scroll to
/// window middle), `z-` (scroll to window bottom) is unbound (the engine only has zz/zt/zb). `z.`'s
/// degradation is the most dangerous: after the `z` is dropped, `.` executes **RepeatChange —
/// repeating the last change**.
/// oracle: on 'hello', `xz.` → file "ello" (z. only scrolls, does not repeat the x); on 'a\nb\nc\n'
/// line 2, `z<CR>` → buffer/cursor line both unchanged (getcurpos line 2).
/// Engine: z. deletes another character from the x ("llo"); z<CR> triggers a CR motion moving down one line.
#[test]
fn b10_z_enter_zdot_zminus_unbound_zdot_repeats_change() {
    let f = edit("hello\n", 0, 0, &["x", "z", "."]);
    assert_eq!(f.text(), "ello\n", "vim: z. is a scroll command and must never repeat the last change");

    let f = edit("a\nb\nc\n", 1, 0, &["z", "<CR>"]);
    assert_eq!(f.line(), 1, "vim: z<CR> only scrolls the view, the cursor does not move");
}

// ------------------------------------------------ B11 (P2) `d]]` is an exclusive motion

/// **B11 (P2)**: vim's `]]` is an **exclusive** motion (`:h ]]`, which also notes exclusive-linewise
/// applies) — the operator span reaches the section header line's start, and with the endpoint at
/// column 1 rule (a) folds back to the previous line's end and turns inclusive: the start line's
/// **line head is preserved**. The engine models `]]` as Linewise (motions.rs, the Section arm's
/// non-method branch), deleting line after line wholesale.
/// oracle: on 'aaa\nbbb\n{\nccc\n' cursor (0,1), `d]]` → file "a\n{\nccc\n" (both the "a" head and
/// the `{` line survive). Engine: linewise deletion of lines 1-3 → "ccc\n".
#[test]
fn b11_section_forward_operator_is_exclusive() {
    let f = edit("aaa\nbbb\n{\nccc\n", 0, 1, &["d", "]", "]"]);
    assert_eq!(f.text(), "a\n{\nccc\n", "vim: d]] exclusive rule (a) keeps the line head and the section line");
}

// ------------------------------------------------ B12 (P2) `]m`/`]M` do not search the cursor line

/// **B12 (P2)**: motions.rs's Section arm does `line += step` first — **the cursor's own line never
/// participates in the search**. Method parentheses are by definition not at column 1, so `]m`/`]M`'s
/// most common target (the method paren after the cursor on the same line) can never be found: bell,
/// cursor stays put. vim's `]m` searches forward from the cursor position and hits the current line.
/// oracle: on 'foo x {\nbar\n', `1|]m` → getcurpos [0,1,7,…] (0-based 6, the `{` on this line).
/// Engine: both shapes ring, cursor unmoved.
///
/// Convention note (oracle re-verified twice, 2026-10-08): the method family matches parens at
/// **any column** and scans the cursor line. `]M`'s landing rule (reverse-engineered from the source
/// nv_bracket_block PHASE1/2 + the oracle matrix): the first paren b1 — if b1 is a close paren → land
/// on b1; if b1 is an open paren → if its pair exists and another paren follows the pair → land on
/// **the pair**, otherwise land on b1 (an unterminated method lands on the open paren). `if (a) {\nx\n} y\n`:
/// b1 = `{`, the pair `}` exists but no paren follows it → land on `{` (offset 7, oracle 1|8 re-verified
/// twice). `]m` = the first paren.
#[test]
fn b12_method_motions_never_search_the_cursor_line() {
    let f = edit("foo x {\nbar\n", 0, 0, &["]", "m"]);
    assert_eq!(f.cursor(), 6, "vim: ]m hits the method open paren on the cursor line");

    let f = edit("if (a) {\nx\n} y\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 7, "oracle 1|8: the open paren of an unterminated method");

    // Pair exists and another paren follows it → land on the pair (oracle 3|1)
    let f = edit("if (a) {\nx\n} y\nz }\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 11, "oracle 3|1: land on the method's closing pair");

    // A pure close paren first → land on it (oracle 1|9)
    let f = edit("abc def }\nmore\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 8, "oracle 1|9: b1 is already the close paren");
}

// ------------------------------------------------ B14 (P2) `d]m` swallows the `{`

/// **B14 (P2)**: vim's `]m` is **exclusive** (`:h ]m`) — the operator deletes up to before the `{`,
/// and the `{` itself is preserved (a plain jump still parks the cursor on the `{`). The engine
/// models the method arm as Inclusive (motions.rs: `Some(o) if method => …Inclusive`), so `d]m`
/// deletes the `{` along with everything else.
/// oracle: on 'foo\nbar {\nbaz\n', `1|d]m` → file "{\nbaz\n" (deletes "foo\nbar ").
/// Engine: inclusive span → deletes down to "\nbaz\n" (the `{` swallowed).
#[test]
fn b14_method_forward_operator_is_exclusive() {
    let f = edit("foo\nbar {\nbaz\n", 0, 0, &["d", "]", "m"]);
    assert_eq!(f.text(), "{\nbaz\n", "vim: d]m deletes up to before the open brace, which survives");
}

// ------------------------------------------------ B13 (P1) the `d}` phantom landing from mid-line joins lines

/// **B13 (P1)**: when `}` runs off the end of the buffer it lands on the phantom line after the
/// last line (column 1). Starting mid-line (neither first nor non-blank), vim applies exclusive rule
/// (a): the endpoint folds back to the **last line's end** and turns inclusive — the line structure
/// is preserved ("a\n"). The engine's phantom branch uses `clamp_to_line_end` to pull the landing
/// back to the buffer's **last line** and then computes `line_end(target_line-1)`, yielding the end
/// of the **cursor's own line** — the cross-line newline gets deleted and the two lines **merge**
/// (a data-corruption shape).
/// oracle: on 'aaa\nbbb\n' cursor (0,1), `d}` → file "a\n"; noeol 'aaa\nbbb' with the same keys →
/// "a\n" (re-added by fixendofline). Engine: both shapes yield "abbb\n"/"abbb".
#[test]
fn b13_brace_phantom_midline_start_joins_lines() {
    let f = edit("aaa\nbbb\n", 0, 1, &["d", "}"]);
    assert_eq!(f.text(), "a\n", "vim: the paragraph operator running off buffer end from mid-line keeps the line structure");

    let f = edit("aaa\nbbb", 0, 1, &["d", "}"]);
    // Internal-text convention (fixendofline is write-back behavior, see B1's note)
    assert_eq!(f.text(), "a", "vim: same shape for noeol (line structure kept, no merging)");
}
