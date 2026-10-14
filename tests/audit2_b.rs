//! audit2_b: the second-round operator / register / paste audit (2026-10-07).
//! One `#[test]` per finding, asserting the expected result from vim 9.1
//! (`-Nu NONE -N -s` typeahead oracle, verified on this machine), so all of them
//! fail right now.
//!
//! Findings list (severity / numbering match the reply body):
//! 1. an empty linewise register put with p/P pastes one line short on a line
//!    without a trailing newline (P1)
//! 2. on a last line without a trailing newline, `p` swallows the trailing `\n`
//!    the register itself carries (P1)
//! 3. on a last line without a trailing newline, `]p` merges the pasted content
//!    into the current line (P1)
//! 4. repeating a linewise register by a count does not add line separators (P1)
//! 5. the case operators use the full Unicode mapping (ﬁ→FI, ǰ→J̌, İ→i̇) while vim
//!    uses the simple mapping (P2)
//! 6. the `gP` linewise paste cursor lands on the first line of the pasted block;
//!    vim lands it on the line after the block (P2)
//! 7. the `"{reg}` prefix is accepted in operator-pending state (vim cancels the
//!    operator and rings the bell) (P2)
//! 8. read-only registers (`"%` `":` `".` `"/`) can be written through the
//!    `"{reg}` prefix (vim refuses) (P2)
//! 9. after `"Add` the unnamed register points at the merged result; vim points it
//!    at the newly appended piece (P3)
//! 10. the `"=` expression register is missing (vim evaluates it and pastes the
//!     result) (P2)

mod common;

use common::edit;

/// [1] (P1) Putting an empty linewise register loses a line.
/// oracle: on an empty file (1 empty line) `yyp` -> line("$")==2; `yy3p` -> 4;
/// `yyP` -> 2.
/// engine: the `p`/`P` branch consumes the `\n` the register carries as the
/// "separator from the current line", so once the empty register is forced to
/// `"\n"` the strip_suffix call zeroes it and not a single line lands in the
/// buffer.
#[test]
fn f1_empty_linewise_put_on_empty_buffer_loses_line() {
    // yy + p: vim gives 2 lines ("\n\n"), the engine still has 1 ("\n")
    let f = edit("", 0, 0, &["y", "y", "p"]);
    assert_eq!(f.text(), "\n\n", "yy p on an empty buffer should give two empty lines");

    // yy + P: vim gives 2 lines
    let f = edit("", 0, 0, &["y", "y", "P"]);
    assert_eq!(f.text(), "\n\n", "yy P on an empty buffer should give two empty lines");

    // yy + 3p: vim gives 1+3=4 lines
    let f = edit("", 0, 0, &["y", "y", "3", "p"]);
    assert_eq!(f.text(), "\n\n\n\n", "yy 3p on an empty buffer should give four empty lines");
}

/// [2] (P1) When `p` pastes below a last line without a trailing newline, it
/// strips the trailing `\n` of a newline-terminated register text and uses it as
/// the separator, so the last pasted line loses the file-ending newline.
/// oracle: on a file "a\n", `ddp` + `:wq` -> file bytes `\na\n`; `dd3p` ->
/// `\na\na\na\n`.
/// engine: `"\na"` / `"\na\na\na"` (last line noeol, bytes dropped silently).
#[test]
fn f2_put_below_noel_line_drops_final_newline() {
    let f = edit("a\n", 0, 0, &["d", "d", "p"]);
    assert_eq!(f.text(), "\na\n", "dd p should give [\"\"] and [\"a\"] (last line with eol)");

    let f = edit("a\n", 0, 0, &["d", "d", "3", "p"]);
    assert_eq!(f.text(), "\na\na\na\n", "dd 3p should give [\"\",a,a,a]");
}

/// [3] (P1) The `]p` put position is `line_range(cur).end.min(len)` -- when the
/// last line has no trailing newline that point is the very end of the buffer,
/// so the pasted text is appended inside the line.
/// oracle: on a noeol file [ab, cd], `yy j ]p` -> [ab, cd, ab].
/// engine: "ab\ncdab\n" ("ab" merged into the "cd" line).
#[test]
fn f3_bracket_p_below_noel_line_glues() {
    let f = edit("ab\ncd", 0, 0, &["y", "y", "j", "]", "p"]);
    // byte-level oracle cross-check (od -c): the file vim writes back is
    // "ab\ncd\nab\n" -- 'fixendofline' (on by default) restores the trailing
    // newline on save; the gluing "ab\ncdab" is the actual bug here, so the line
    // structure [ab,cd,ab] is what we assert on
    assert_eq!(f.text(), "ab\ncd\nab\n", "]p should open a new line below the last line and paste ab there");
}

/// [4] (P1) Repeating a linewise register by a count uses `text.repeat(count)` --
/// linewise text yanked from a noeol last line carries no `\n` of its own, so
/// after the repetition the lines glue together horizontally.
/// oracle: on a noeol file "abc", `yy3p` -> [abc,abc,abc,abc].
/// engine: "abc\nabcabcabc".
#[test]
fn f4_linewise_count_repeat_glues_without_separators() {
    let f = edit("abc", 0, 0, &["y", "y", "3", "p"]);
    // byte-level oracle cross-check (od -c): vim writes back
    // "abc\nabc\nabc\nabc\n" (fixendofline adds the trailing newline); the four
    // independent lines with no horizontal gluing are the assertion body
    assert_eq!(f.text(), "abc\nabc\nabc\nabc\n", "3p should paste three separate lines");
}

/// [5] (P2) The case operators go through Rust's full Unicode mapping; vim 9.1
/// uses the simple mapping and leaves characters without a simple mapping
/// unchanged:
///   `~` on ﬁ (U+FB01) -> vim keeps "ﬁx" unchanged, engine gives "FIx";
///   `gul` on İ (U+0130) -> vim "ix" (a plain i), engine "i\u{307}x";
///   `~` on ǰ (U+01F0) -> vim keeps "ǰx" unchanged, engine "J\u{30c}x".
/// (oracle: all three verified one by one with getline(1). ß→ẞ already has its
/// own special case; this item is the rest of the same family.)
#[test]
fn f5_case_operators_full_vs_simple_unicode() {
    let f = edit("\u{fb01}x", 0, 0, &["~"]);
    assert_eq!(f.text(), "\u{fb01}x", "~ must not expand the ligature ﬁ");

    let f = edit("\u{0130}x", 0, 0, &["g", "u", "l"]);
    assert_eq!(f.text(), "ix", "gu on İ should give a plain i, with no combining dot");

    let f = edit("\u{01f0}x", 0, 0, &["~"]);
    assert_eq!(f.text(), "\u{01f0}x", "~ must not split ǰ into J + a combining mark");
}

/// [6] (P2) the `gP` linewise paste cursor: vim puts it "after the pasted text",
/// i.e. on the line following the pasted block (oracle: on [aaa,bbb,ccc],
/// `jyyGgP` -> cursor line 4; a 2-line block -> line 5).
/// The engine puts it on the first line of the pasted block (line 3, 1-based).
#[test]
fn f6_gp_linewise_cursor_after_block() {
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &["y", "y", "G", "g", "P"]);
    // after the paste the buffer is [aaa,bbb,bbb,ccc]; vim's cursor is on the
    // 1-based line 4 (after the block)
    assert_eq!(f.line(), 3, "the gP cursor should land on the line after the pasted block (0-based 3)");
}

/// [7] (P2) the `"{reg}` prefix is not accepted in operator-pending state.
/// oracle: on "foo bar", `d"aw<Esc>` -> the operator is cancelled by `"` (bell),
/// then `a` enters insert mode and `w` is typed -> buffer "fwoo bar", register a
/// stays empty.
/// Engine: deletes "foo " into register a, turning the buffer into "bar".
#[test]
fn f7_register_prefix_rejected_under_operator() {
    // oracle evidence: vim rings the bell and cancels the operator when it meets
    // `"` in operator-pending state, then `a` enters insert and `w` lands as
    // typed text ("fwoo bar"). This fixture's feed does not simulate typing in
    // insert mode (that is handed to the host per the contract), so the
    // assertion pins the observable essence of the oracle: the buffer is not
    // deleted, register a is not written (so "ap cannot paste any deleted
    // content), and the bell rings.
    let f = edit("foo bar", 0, 0, &["d", "\"", "a", "w", "<Esc>"]);
    assert_eq!(f.text(), "foo bar", "d\"aw must not be executed as a delete");
    assert!(f.host.bells >= 1, "meeting \" in operator-pending state should ring the bell: {:?}", f.host.bells);
    let mut f2 = edit("foo bar", 0, 0, &["d", "\"", "a", "w", "<Esc>"]);
    f2.feed(["\"", "a", "p"]);
    assert_eq!(f2.text(), "foo bar", "register a must not be written (\"ap pastes no deleted content)");
}

/// [8] (P2) read-only registers must not be written through the `"{reg}` prefix.
/// oracle (verified one by one):
///   `"%dd` -> refused, buffer stays "abc" (1 line);
///   `":dd`, `".dd`, `"/dd` -> refused the same way (after `.dd`, getreg('.') is
///   still the old inserted text).
/// Engine: `"%dd` deletes as usual (the buffer becomes empty) and even stuffs
/// the deleted content into a slot named %.
#[test]
fn f8_readonly_registers_reject_prefix_write() {
    let f = edit("abc\n", 0, 0, &["\"", "%", "d", "d"]);
    assert_eq!(f.text(), "abc\n", "\"%dd must be refused, buffer unchanged");
}

/// [9] (P3) the content of the unnamed register after `"Add`.
/// oracle: on [ab,cd,ef], `"ayy` `j` `"Add` -> getreg('"') == "ef\n" (the newly
/// deleted piece), register a == "ab\ncd\n" (merged). The engine points the
/// unnamed register at the whole merged result, so `p` pastes two lines.
#[test]
fn f9_uppercase_append_unnamed_gets_new_piece() {
    let f = edit("ab\ncd\nef\n", 1, 0, &[
        "\"", "a", "y", "y", "j", "\"", "A", "d", "d", "p",
    ]);
    assert_eq!(
        f.text(),
        "ab\ncd\nef\n",
        "p should paste only the newly appended ef line (unnamed = the new piece, not the merged body)"
    );
}

/// [10] (P2) the `"=` expression register is missing.
/// oracle: on "ab", `"=2+3<CR>p` -> vim pops the `=` prompt line, evaluates 2+3,
/// and p pastes "5" -> buffer "a5b". The engine has no expression register, so
/// `p` lands on nothing (bell) and the buffer is unchanged.
#[test]
fn f10_expression_register_put() {
    let f = edit("ab", 0, 0, &["\"", "=", "2", "+", "3", "<CR>", "p"]);
    assert_eq!(f.text(), "a5b", "\"=2+3<CR>p should paste the evaluation result 5");
}
