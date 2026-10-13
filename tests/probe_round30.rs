//! Round-30 probes (part 1): Replace mode R, Ex range boundaries, invalid
//! regex, low-frequency multi-byte paths.
//!
//! This round of probes takes the "unexpected operation x unusual data" route:
//! every assertion is first written from vim 9.1 semantics, the ones that fail
//! are checked against the vim oracle one by one (or disproved as a known
//! divergence), and only then do we decide to fix or to record.

mod common;

use common::{edit, Fixture};

// ---------------------------------------------------------------- the R replace mode

/// Probe A: with `R`, keep typing past the end of the line -- vim appends
/// (there is no character left to overwrite).
#[test]
fn r_mode_appends_past_eol() {
    let mut f = edit("ab\ncd\n", 0, 0, &["R"]);
    f.type_text("XYZW"); // overwrites ab, then appends past the end of line
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "XYZW\ncd\n");
}

/// Probe B: a `R` session where BS walks back restores the overwritten
/// characters. vim 9.1 typeahead evidence: `RXY<BS><BS>Q<Esc>` -> `Qbcdef`
/// (BS restores the overwritten characters, Q then overwrites a again).
#[test]
fn r_mode_backspace_restores() {
    let mut f = edit("abcdef\n", 0, 0, &["R"]);
    f.type_text("XY");
    f.feed(["<BS>", "<BS>"]);
    f.type_text("Q");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Qbcdef\n");
}

/// Probe C: `R` overwriting a multi-byte character -- '中' is 3 bytes, and a
/// single-byte overwrite must not tear it apart.
#[test]
fn r_mode_overwrites_multibyte_cleanly() {
    let mut f = edit("中文\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab\n", "ab overwrites all of '中' (3 bytes)");
}

/// Probe D: the repeat-overwrite semantics of `3Rab<Esc>` (vim: ababab
/// overwrites 6 characters).
#[test]
fn r_mode_count_repeat_overwrites() {
    let mut f = edit("abcdefghij\n", 0, 0, &["3", "R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababghij\n");
}

/// Probe E: `<Esc>` straight after `R` (zero input) -- must not change anything.
#[test]
fn r_mode_immediate_escape_is_noop() {
    let mut f = edit("ab\ncd\n", 0, 0, &["R", "<Esc>"]);
    assert_eq!(f.text(), "ab\ncd\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "ab\ncd\n", "an empty R session should not leave an undoable group either");
}

/// Probe F: `<C-w>` (delete word) inside a replace session -- vim 9.1
/// typeahead evidence: C-w deletes the typed word **and restores the
/// overwritten characters** (`RXX<C-w>` -> `one two`, the cursor lands at the
/// typing start point; `RXX<C-w>X` -> `Xne two`).
#[test]
fn r_mode_ctrl_w_restores_overwritten() {
    let mut f = edit("one two\n", 0, 0, &["R"]);
    f.type_text("XX");
    f.feed(["<C-w>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "one two\n", "C-w restores the overwritten on");
    // after the restore the cursor sits at the typing start point, so typing on
    // overwrites the original first character
    let mut f = edit("one two\n", 0, 0, &["R"]);
    f.type_text("XX");
    f.feed(["<C-w>"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Xne two\n");
}

// ---------------------------------------------------------------- Ex range boundaries

/// Probe G: the reversed range `:5,1d` -- vim asks interactively whether to
/// swap, and refuses in script mode (9.1 evidence: no line is deleted). After
/// the engine fix it reports E16 and leaves the buffer alone.
#[test]
fn ex_reversed_range_delete_reports_e16() {
    let f = edit("l1\nl2\nl3\nl4\nl5\n", 0, 0, &[":", "5", ",", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl3\nl4\nl5\n", "a reversed range deletes no line");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "should report E16, actual: {:?}",
        f.host.statuses
    );
}

/// Probe H: `:0d` -- vim 9.1 evidence deletes the first line (address 0 is
/// legal and resolves onto the first line).
#[test]
fn ex_zero_address_delete() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "0", "d", "<CR>"]);
    assert_eq!(f.text(), "l2\nl3\n", "0 points at the first line, so line 1 is deleted");
}

/// Probe I: the single-line range `:2,2d`.
#[test]
fn ex_same_address_delete() {
    let f = edit("l1\nl2\nl3\n", 0, 0, &[":", "2", ",", "2", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl3\n");
}

/// Probe J: a range past the last line, `:99d` -- vim reports
/// E16: Invalid range.
#[test]
fn ex_out_of_range_address() {
    let f = edit("l1\nl2\n", 0, 0, &[":", "9", "9", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\n", "an out-of-range address deletes no line");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "should report E16, actual: {:?}",
        f.host.statuses
    );
}

/// Probe K: the relative range `:.,+2d` -- +2 means "2 more lines further
/// down", so 3 lines are deleted in total.
#[test]
fn ex_relative_range_delete() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\n", 1, 0);
    f.feed([":", ".", ",", "+", "2", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\n");
}

// ---------------------------------------------------------------- invalid regex

/// Probe L: an unterminated character class `/[abc` -- must not panic, and
/// should ring the bell and report an error.
#[test]
fn search_invalid_regex_no_panic() {
    let mut f = edit("abc [abc\n", 0, 0, &["/"]);
    f.type_text("[abc");
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "abc [abc\n");
    // either an E383/E54-ish error message or a bell -- at least one of the two
    assert!(
        f.host.bells > 0 || !f.host.statuses.is_empty(),
        "an invalid regex needs feedback"
    );
}

/// Probe M: `:s/[abc/X` must not panic either.
#[test]
fn substitute_invalid_regex_no_panic() {
    let mut f = edit("abc\n", 0, 0, &[":", "%", "s", "/"]);
    f.type_text("[abc/X");
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "abc\n", "the substitution fails, buffer untouched");
}

/// Probe N: the replacement side is now vim semantics (after the round-31 audit
/// C1 fix) -- `$` is an ordinary character and the backreference is `\1` (same
/// as vim); `&` for the whole match and the `\u` upper-case modifier both work.
#[test]
fn substitute_replacement_vim_semantics() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed([":", "%", "s", "/", "a", "b", "c", "/", "X", "$", "1", "y", "/", "<CR>"]);
    assert_eq!(f.text(), "X$1y\n", "a literal $1 ($ is no longer a backreference in this engine's dialect)");
    // note: the engine's regex dialect (NOTES divergence #1) uses bare
    // parentheses for groups (Rust regex) and `\(` is a literal paren -- only the
    // replacement side's `\1` matches vim.
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed([
        ":", "%", "s", "/", "(", "a", "b", "c", ")", "/", "X", "\\", "1", "y", "/", "<CR>",
    ]);
    assert_eq!(f.text(), "Xabcy\n", "\\1 = the first capture group (the vim backreference syntax)");
}

// ---------------------------------------------------------------- C-w/C-u typing scope (round-30 fix)

/// Probe T1: in insert, `C-u` only deletes the characters typed in this
/// session; pre-existing text survives (vim 9.1 typeahead evidence P4: at
/// column 2 of `hello world`, typing `ab` + C-u -> `hello world`).
#[test]
fn insert_ctrl_u_scoped_to_typed_text() {
    let mut f = edit("hello world\n", 0, 2, &["i"]);
    f.type_text("ab");
    f.feed(["<C-u>", "<Esc>"]);
    assert_eq!(f.text(), "hello world\n", "C-u does not eat text before the insert start point");
}

/// Probe T2: in insert, `C-w` is clamped to the typing start point as well (P5:
/// typing X mid-word + C-w deletes only X); the second C-w, with the cursor
/// already at the typing start point, is a no-op (D2) and does not walk into the
/// pre-existing word.
#[test]
fn insert_ctrl_w_scoped_to_typed_text() {
    let mut f = edit("abcd\n", 0, 2, &["i"]);
    f.type_text("X");
    f.feed(["<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "abcd\n");
    let mut f = edit("abcd\n", 0, 2, &["i"]);
    f.type_text("X");
    f.feed(["<C-w>", "<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "abcd\n", "the second C-w does not eat ab");
}

/// Probe T3: in a session spanning lines, C-u acts per line (it only deletes
/// what was typed on the current line).
#[test]
fn insert_ctrl_u_per_line() {
    let mut f = edit("aa\nbb\n", 1, 0, &["i"]);
    f.type_text("cd");
    f.feed(["<C-u>", "<Esc>"]);
    assert_eq!(f.text(), "aa\nbb\n", "the C-u on the second line leaves the first line alone");
}

/// Probe T4: in a Replace session C-u restores everything (vim 9.1 evidence:
/// `Rab cd<C-u>Z` -> `Zbcdefg` -- after the restore, Z overwrites a).
#[test]
fn replace_ctrl_u_restores_overwritten() {
    let mut f = edit("abcdefg\n", 0, 0, &["R"]);
    f.type_text("ab cd");
    f.feed(["<C-u>"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "Zbcdefg\n");
}

/// Probe T5: the C-w word boundary also works in Replace mode and restores the
/// characters inside the deleted span (vim 9.1 evidence: `Rab cd<C-w>Z` ->
/// `ab Zefg` -- only the word cd is deleted, d and e are restored).
#[test]
fn replace_ctrl_w_word_scoped_restore() {
    let mut f = edit("abcdefg\n", 0, 0, &["R"]);
    f.type_text("ab cd");
    f.feed(["<C-w>"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ab Zefg\n");
}

/// Probe T6: multi-byte restore -- after overwriting '中', C-w restores the
/// whole character without tearing its bytes.
#[test]
fn replace_ctrl_w_restores_multibyte() {
    let mut f = edit("中文x\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<C-w>", "<Esc>"]);
    assert_eq!(f.text(), "中文x\n");
}

// ---------------------------------------------------------------- the :& and :~ replays

/// Probe V1: `:&` does not carry the flags of the previous `:s` (vim 9.1
/// evidence: after `:1,1s/a/B/g`, `:2,2&` replaces only the first match on
/// line 2).
#[test]
fn ex_ampersand_repeats_without_flags() {
    let f = edit(
        "xaxax\nxaxax\nxaxax\n",
        0,
        0,
        &[
            ":", "1", ",", "1", "s", "/", "a", "/", "B", "/", "g", "<CR>",
            ":", "2", ",", "2", "&", "<CR>",
        ],
    );
    assert_eq!(f.text(), "xBxBx\nxBxax\nxaxax\n");
}

/// Probe V2: `:~` uses the **current search pattern** plus the original
/// replacement string, and likewise carries no flags (vim 9.1 evidence: after
/// repointing with `/x`, `:3,3~` turns x into Q -- the replacement string of the
/// first `:s`).
#[test]
fn ex_tilde_uses_last_search_pattern() {
    let f = edit(
        "axa\nxyx\nxax\n",
        0,
        0,
        &[
            ":", "1", ",", "1", "s", "/", "a", "/", "Q", "/", "<CR>",
            "/", "x", "<CR>",
            ":", "3", ",", "3", "~", "<CR>",
        ],
    );
    assert_eq!(f.text(), "Qxa\nxyx\nQax\n");
}

/// Probe V3: with no previous substitution, `:&`/`:~` report E33 and ring the
/// bell (the same as a bare `:s`).
#[test]
fn ex_ampersand_tilde_without_previous_report_e33() {
    let mut f = Fixture::new("abc\n");
    f.feed([":", "&", "<CR>"]);
    assert_eq!(f.text(), "abc\n");
    assert!(f.host.statuses.iter().any(|s| s.contains("E33")));
    assert!(f.host.bells > 0);
    let mut f = Fixture::new("abc\n");
    f.feed([":", "~", "<CR>"]);
    assert!(f.host.statuses.iter().any(|s| s.contains("E33")));
}

/// Probe V4: when the search pattern contains the separator, `:~` keeps the
/// escaping alive (after `/a\/b`, the `s/a\/b/Q/` it rebuilds does not fall
/// apart -- the `/` inside the pattern must be re-escaped).
#[test]
fn ex_tilde_escapes_separator_in_pattern() {
    let f = edit(
        "a/b x\n",
        0,
        0,
        &[
            ":", "s", "/", "x", "/", "Q", "/", "<CR>", // a/b Q (pattern x)
            "/", "a", "\\", "/", "b", "<CR>", // the search pattern "a/b"
            ":", "~", "<CR>", // s/a\/b/Q/ -> "Q Q"
        ],
    );
    assert_eq!(f.text(), "Q Q\n");
}

// ---------------------------------------------------------------- low-frequency multi-byte paths

/// Probe O: `3r中` -- a count x a multi-byte replace.
#[test]
fn r_cmd_count_multibyte() {
    let f = edit("abcdef\n", 0, 0, &["3", "r", "中"]);
    assert_eq!(f.text(), "中中中def\n");
}

/// Probe P: `~` on the last character of a line does not cross the line.
#[test]
fn tilde_at_line_end_stays() {
    let f = edit("ab\ncd\n", 0, 1, &["~"]);
    assert_eq!(f.text(), "aB\ncd\n");
    assert_eq!(f.line(), 0, "must not cross the line");
}

/// Probe J2: `J` joining multi-byte lines -- vim 9.1 evidence shows a CJK line
/// end also gets one inserted space (J omits the space only when the line end
/// already has whitespace / the line is empty, independent of character width).
#[test]
fn j_multibyte_inserts_separator_space() {
    let f = edit("中文\n第二行\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "中文 第二行\n");
}

/// Probe Q: `x` on a wide character (a CJK char occupying two columns) deletes
/// the whole character at once.
#[test]
fn x_on_wide_char() {
    let f = edit("中a\n", 0, 0, &["x"]);
    assert_eq!(f.text(), "a\n");
}

/// Probe R2: `f中` searching for a multi-byte character target.
#[test]
fn f_to_multibyte_target() {
    let f = edit("a中b\n", 0, 0, &["f", "中"]);
    assert_eq!(f.cursor(), "a".len(), "f lands on the byte start of the 中 character");
}

/// Probe S: meaningless operations on an empty buffer must not panic or leave
/// dirty state behind.
#[test]
fn empty_buffer_hostile_ops() {
    let mut f = Fixture::new("");
    for keys in [
        vec!["d", "d"],
        vec!["y", "y"],
        vec!["c", "c"],
        vec!["g", "v"],
        vec!["p"],
        vec!["P"],
        vec!["J"],
        vec!["~"],
        vec!["D"],
        vec!["C"],
        vec!["S"],
        vec!["X"],
        vec!["<C-a>"],
        vec!["g", ";"],
        vec!["g", ","],
        vec!["'", "'"],
        vec!["`", "`"],
    ] {
        f.feed(keys);
    }
    assert_eq!(f.text(), "", "nothing may be created out of an empty buffer");
    f.feed(["i"]);
    f.type_text("ok");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "ok", "later normal editing is not polluted");
}
