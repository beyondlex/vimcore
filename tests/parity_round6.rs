//! Round-6 review regressions. Every semantic case below was re-probed
//! against real vim 9.1 (headless `-es` scripts) before the fix; the probe
//! outputs are quoted in NOTES.md.

mod common;

use common::{edit, Fixture};

/// Type an Ex command line through the prompt (one `Key` per char).
fn ex(f: &mut Fixture, line: &str) {
    f.feed([":"]);
    let keys: Vec<String> = line.chars().map(String::from).collect();
    f.feed(keys);
    f.feed(["<CR>"]);
}

// ---- visual <Del> deletes the selection --------------------------------------

/// vim 9.1 probe (`viw<Del>` on "hello world" → "hello "): <Del> in visual
/// mode is `d`. The engine used to route it through the normal-mode char
/// delete, wiping one char at the cursor while the selection stayed alive.
#[test]
fn visual_delete_key_deletes_the_selection() {
    let f = edit("hello world", 0, 6, &["v", "i", "w", "<Del>"]);
    assert_eq!(f.text(), "hello ", "viw<Del> deletes the selected word");
    assert_eq!(f.vim.mode(), vimcore::mode::Mode::Normal);

    // a linewise selection too
    let f = edit("aaa\nbbb\nccc\n", 1, 0, &["V", "<Del>"]);
    assert_eq!(f.text(), "aaa\nccc\n");
}

// ---- count across the operator boundary --------------------------------------

/// vim 9.1 probe (`2d3w` on "w1 w2 w3 w4 w5 w6 w7 tail" → "w7 tail"): the
/// prefix count and the motion count MULTIPLY (2×3 = 6 words). The engine
/// used to let the digit runs concatenate (2 then 3 → 23), wiping across
/// following lines.
#[test]
fn operator_and_motion_counts_multiply() {
    let f = edit("aa bb cc dd ee ff gg\nnext\nline\n", 0, 0, &["2", "d", "3", "w"]);
    // 6 words: aa bb cc dd ee ff (each word's trailing blank goes with it)
    assert_eq!(f.text(), "gg\nnext\nline\n");

    // control: a single count on either side behaves as before
    let f = edit("aa bb cc dd\nnext\nline2\n", 0, 0, &["2", "d", "d"]);
    assert_eq!(f.text(), "line2\n", "2dd deletes two lines");
    let f = edit("aa bb cc dd\nnext\nline2\n", 0, 0, &["d", "2", "d"]);
    assert_eq!(f.text(), "line2\n", "d2d deletes two lines");
}

/// `2gUU` uppercases two lines (prefix count reaches the linewise doubling
/// through op_count).
#[test]
fn prefix_count_reaches_linewise_doubling() {
    let f = edit("ab\ncd\nef\n", 0, 0, &["2", "g", "U", "U"]);
    assert_eq!(f.text(), "AB\nCD\nef\n");
}

// ---- S with a count changes [count] lines -------------------------------------

/// vim 9.1 probe (`3S` on lines 2-4 of 5 with X typed → ['l1','X','l5'],
/// cursor 2:1): `S` deletes [count] lines and starts insert.
#[test]
fn substitute_line_applies_the_count() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\nl5\n", 1, 0);
    f.feed(["3", "S"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "l1\nX\nl5\n");
    assert_eq!(f.line(), 1);
}

// ---- linewise p cursor on a buffer without trailing newline --------------------

/// vim 9.1 probe (`yy p` on the single line "abc" → lines ['abc','abc'],
/// cursor_line=2): the cursor lands on the FIRST PASTED line. The engine
/// parked it on the original line because the separator `\n` swallowed the
/// insert offset.
#[test]
fn linewise_put_cursor_lands_on_pasted_line_without_trailing_newline() {
    let f = edit("abc", 0, 0, &["y", "y", "p"]);
    assert_eq!(f.text(), "abc\nabc");
    assert_eq!(f.line(), 1);

    // control: with a trailing newline the behavior was already right
    let f = edit("abc\ndef\n", 0, 0, &["y", "y", "p"]);
    assert_eq!(f.text(), "abc\nabc\ndef\n");
    assert_eq!(f.line(), 1);
}

// ---- :d / :y {count} anchors at the range's LAST line --------------------------

/// vim 9.1 probes (`:1,2d 3` on L1..L7 → ['L1','L5'..'L7'], `:1,2y 3`
/// yanks 'L2 L3 L4'): a trailing {count} starts at the range's last line.
/// The engine used to EXTEND the range instead (deleting L1-L4).
#[test]
fn ex_delete_count_starts_at_range_last_line() {
    let mut f = Fixture::new("L1\nL2\nL3\nL4\nL5\nL6\nL7\n");
    ex(&mut f, "1,2d 3");
    assert_eq!(f.text(), "L1\nL5\nL6\nL7\n");

    // one-line range: the documented `:2d 3` form keeps working
    let mut f = Fixture::new("L1\nL2\nL3\nL4\nL5\n");
    ex(&mut f, "2d 3");
    assert_eq!(f.text(), "L1\nL5\n");
}

#[test]
fn ex_yank_count_starts_at_range_last_line() {
    let mut f = Fixture::new("L1\nL2\nL3\nL4\nL5\n");
    ex(&mut f, "1,2y 3");
    f.feed(["G", "p"]);
    assert_eq!(f.text(), "L1\nL2\nL3\nL4\nL5\nL2\nL3\nL4\n");
}

// ---- :sort i u dedups case-insensitively ---------------------------------------

/// vim 9.1 probe (`%sort iu` on [foo, FOO, bar] → [bar, foo]): with `i` the
/// dedup folds case (the last of an equal run survives); plain `u` keeps
/// both ([FOO, bar, foo]).
#[test]
fn sort_unique_with_ignore_case_folds_case() {
    let mut f = Fixture::new("foo\nFOO\nbar\n");
    ex(&mut f, "%sort iu");
    assert_eq!(f.text(), "bar\nfoo\n");

    let mut f = Fixture::new("foo\nFOO\nbar\n");
    ex(&mut f, "%sort u");
    assert_eq!(f.text(), "FOO\nbar\nfoo\n");
}

// ---- :j cursor lands on the joined line's first non-blank ----------------------

/// vim 9.1 probes (`:1,2j` on ['    aaaa','bbbb'] → cursor 1:5; bare `:j`
/// → col 1 of the joined line): the ex join parks the cursor on the joined
/// line's first non-blank, NOT on the join seam (that's normal-mode `J`).
#[test]
fn ex_join_cursor_on_first_non_blank_of_joined_line() {
    let mut f = Fixture::new("    aaaa\nbbbb\ncccc\n");
    ex(&mut f, "1,2j");
    assert_eq!(f.text(), "    aaaa bbbb\ncccc\n");
    assert_eq!(f.cursor(), 4, "first non-blank of the joined line");

    // normal-mode J keeps the seam (control, unchanged behavior)
    let f = edit("    aaaa\nbbbb\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "    aaaa bbbb\n");
    assert_eq!(f.cursor(), 8);
}
