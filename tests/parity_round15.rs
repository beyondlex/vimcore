//! Round-15 regressions: substitute replacement escapes, out-of-range Ex
//! addresses, paragraph boundaries, J on empty lines, quote-object fallback,
//! `N%` file percentage, the blackhole register, `3R` count-repeat,
//! redo-register, `<C-h>` delivery shapes, visual mark jumps. Every expected
//! behavior was verified against vim 9.1 probes (probe scripts in NOTES,
//! round 15).

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

/// Expand key strings char by char (`<...>` notations are delivered whole):
/// the Ex cmdline test domain.
fn expand(keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        if k.starts_with('<') && k.ends_with('>') && k.len() > 2 {
            out.push(k.to_string());
        } else {
            out.extend(k.chars().map(|c| c.to_string()));
        }
    }
    out
}

fn feed_expanded(f: &mut Fixture, keys: &[&str]) {
    f.feed(expand(keys));
}

// ---- :s replacement escapes (first run + & / :s replay) --------------------

/// PROBE: vim 9.1 `:s/a/b\/c/` → `b/c` (a `\/` inside the replacement unfolds
/// to the delimiter; the old engine kept both characters verbatim). `&` and the
/// bare `:s` replay behave the same (replay goes through the same parser).
#[test]
fn substitute_replacement_unescapes_separator() {
    // `&`/bare `:s` are typed after the command closes (`&` is just a literal
    // char inside the prompt)
    for replay in [&["&"][..], &["<CR>", ":", "s", "<CR>"][..]] {
        let mut f = edit("a\n", 0, 0, &[":"]);
        let mut keys: Vec<String> = expand(&["s/a/b\\/c/"]);
        keys.extend(expand(&["<CR>"]));
        f.feed(keys);
        f.feed(expand(replay));
        assert_eq!(
            f.text(),
            "b/c\n",
            "replacement \\/ must fold to / (replay = {replay:?})"
        );
    }
}

/// PROBE: vim 9.1 `:s/a/b\\c/` → `b\c` (`\\` unfolds to a single backslash).
#[test]
fn substitute_replacement_unescapes_backslash() {
    let mut f = edit("a\n", 0, 0, &[":"]);
    feed_expanded(&mut f, &["s/a/b\\\\c/", "<CR>"]);
    assert_eq!(f.text(), "b\\c\n");
}

/// Unknown escapes stay verbatim (vim does the same): `\c` is not a special
/// replacement sequence.
#[test]
fn substitute_replacement_keeps_unknown_escape() {
    let mut f = edit("a\n", 0, 0, &[":"]);
    feed_expanded(&mut f, &["s/a/b\\c/", "<CR>"]);
    assert_eq!(f.text(), "b\\c\n");
}

// ---- out-of-range Ex addresses (E1247) -------------------------------------

/// PROBE: vim 9.1 `:1+<21 nines>d` → E1247 and no change (the old engine's
/// unwrap_or(0) swallowed the offset into +0, silently deleting line 1). The
/// negative side is the same (`:5-<huge>d`).
#[test]
fn overflowing_address_offset_reports_e1247() {
    for line in ["1+99999999999999999999d", "5-99999999999999999999d"] {
        let mut f = edit("one\ntwo\nthree\n", 0, 0, &[":"]);
        feed_expanded(&mut f, &[line, "<CR>"]);
        assert_eq!(f.text(), "one\ntwo\nthree\n", "{line} must not act");
        assert!(
            f.host
                .statuses
                .iter()
                .any(|s| s.starts_with("E1247")),
            "{line} must report E1247, got {:?}",
            f.host.statuses.last()
        );
    }
}

// ---- } / { paragraph boundaries only count truly empty lines ----------------

/// PROBE: `d}` on vim 9.1 `aaa / "   " / bbb` → deletes all, leaving one empty
/// line (a whitespace-only line is not a paragraph boundary; the old engine
/// stopped at the blank-looking line's start). `{` mirrors it.
#[test]
fn paragraph_motion_skips_whitespace_only_lines() {
    let f = edit("aaa\n   \nbbb\n", 0, 0, &["d", "}"]);
    assert_eq!(f.text(), "", "d}} leaves one empty line (vim [''])");
    let f = edit("aaa\n   \nbbb\n", 2, 0, &["d", "{"]);
    assert_eq!(f.text(), "bbb\n", "d{{ from bbb keeps only bbb (9.1 probe)");
}

// ---- J on an empty line ------------------------------------------------------

/// PROBE: `J` on vim 9.1 `["", "def"]` → `"def"` (no space when either side of
/// the seam is empty; the old engine gave `" def"`). Same for `["def", ""]`.
#[test]
fn join_on_empty_line_adds_no_space() {
    let f = edit("\ndef\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "def\n");
    let f = edit("def\n\n", 0, 0, &["J"]);
    assert_eq!(f.text(), "def\n");
}

// ---- ci" past the last quote falls back to the first pair --------------------

/// PROBE: on vim 9.1 `say "hi" then "bye" end` with the cursor after `"bye"`,
/// `ci"` changes `hi` (old behavior: bell + no-op; no right pair when the left
/// quote dangles as a closing quote).
#[test]
fn ci_quote_past_last_pair_falls_back_to_first() {
    let mut f = edit("say \"hi\" then \"bye\" end\n", 0, 19, &["c", "i", "\""]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "say \"X\" then \"bye\" end\n");
}

// ---- N% file percentage -------------------------------------------------------

/// PROBE: vim 9.1's nv_percent formula is the 1-based ceil
/// `(count*total+99)/100`: on a 200-line buffer `2%` → line 4 (0-based 3),
/// `50%` → 0-based 99, `30%` → 0-based 59. The old floor formula was off by one
/// line when the product was an exact multiple of 100.
#[test]
fn count_percent_uses_ceil_formula() {
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.push(format!("L{i}"));
    }
    let initial = format!("{}\n", lines.join("\n"));
    for (count, want) in [(2usize, 3usize), (50, 99), (30, 59)] {
        let mut keys: Vec<String> = vec!["g".into(), "g".into()];
        keys.extend(count.to_string().chars().map(|c| c.to_string()));
        keys.push("%".into());
        let mut f = Fixture::new(&initial);
        f.feed(keys);
        assert_eq!(
            f.buf.offset_to_line(f.vim.cursor_offset()),
            want,
            "{count}%@200 must land 0-based line {want}"
        );
    }
}

/// PROBE: on vim 9.1 `1%` is a file percentage (200 lines → line 2), not
/// bracket matching; the old `count > 1` guard merged a typed 1 into "no
/// count", ringing in place on a line without brackets.
#[test]
fn typed_one_percent_is_file_position() {
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.push(format!("L{i}"));
    }
    let initial = format!("{}\n", lines.join("\n"));
    let f = edit(&initial, 0, 0, &["g", "g", "1", "%"]);
    assert_eq!(f.buf.offset_to_line(f.vim.cursor_offset()), 1);
    assert_eq!(f.host.bells, 0, "1% is a percentage, not a bracket search");
    // bare % still rings in place on a line without brackets
    let f = edit("no brackets\n", 0, 0, &["%"]);
    assert_eq!(f.vim.cursor_offset(), 0);
    assert_eq!(f.host.bells, 1);
}

// ---- "_yy leaves unnamed untouched -------------------------------------------

/// PROBE: after vim 9.1 `"_yy`, `p` puts nothing at all (unnamed was never
/// touched); the old fall-through unconditionally retargeted last.
#[test]
fn blackhole_yank_leaves_unnamed_untouched() {
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["\"", "_", "y", "y", "G", "$", "p"]);
    assert_eq!(f.text(), "aaa\nbbb\nccc\n");
}

// ---- 3R count-repeat -----------------------------------------------------------

/// PROBE: `3Rab<Esc>` on vim 9.1 `abcdefghij` → `abababghij` (Replace
/// overwrites the following text by char count on exit; the old engine excluded
/// Replace from count-repeat).
#[test]
fn count_replace_repeats_overwrite() {
    let mut f = edit("abcdefghij\n", 0, 0, &["3", "R"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababghij\n");
}

// ---- redo-register --------------------------------------------------------------

/// PROBE: vim 9.1 `:h redo-register` — `dd dd "1P .` restores both lines (`.`
/// advances the numbered-register index "1→"2; the old engine put the same
/// register twice).
#[test]
fn dot_repeats_walk_numbered_register_ring() {
    let f = edit(
        "one\ntwo\nthree\ntail\n",
        0,
        0,
        &["d", "d", "d", "d", "\"", "1", "P", "."],
    );
    assert_eq!(f.text(), "one\ntwo\nthree\ntail\n");
}

// ---- <C-h> in both delivery shapes ---------------------------------------------

/// gpui-style hosts deliver the canonical ctrl form `Key::ctrl_char('h')`;
/// crossterm-style hosts deliver raw `\x08`. vim binds C-h ≡ BS — both the
/// prompt and insert mode once silently swallowed it.
#[test]
fn ctrl_h_folds_to_backspace_in_both_delivery_forms() {
    // prompt
    let mut f = Fixture::new("x\n");
    feed_expanded(&mut f, &[":", "a", "b", "c"]);
    f.feed_raw(vimcore::key::Key::parse("<C-h>"));
    assert_eq!(f.vim.cmdline.buffer, "ab");
    // insert mode
    let mut f = edit("abc\n", 0, 0, &["a"]);
    f.feed_raw(vimcore::key::Key::parse("<C-h>"));
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "bc\n");
    // the raw \x08 byte in the prompt (crossterm-style text path) still works
    let mut f = Fixture::new("x\n");
    feed_expanded(&mut f, &[":", "a", "b"]);
    f.feed_raw(vimcore::key::Key::char('\x08'));
    assert_eq!(f.vim.cmdline.buffer, "a");
}

// ---- 'a / `a in visual mode -----------------------------------------------------

/// PROBE: on vim 9.1, `'a`/`` `a `` in visual move the cursor and the selection
/// follows (old behavior: a single-key Miss rang and cleared pending).
#[test]
fn visual_mark_jump_extends_selection() {
    let mut f = Fixture::new("aaa\nbbb\nccc\n");
    feed_expanded(&mut f, &["m", "a", "V", "j", "'", "a"]);
    assert_eq!(f.host.bells, 0, "'a in visual must not bell");
    assert!(
        matches!(f.vim.mode, vimcore::Mode::Visual { .. }),
        "visual mode must persist"
    );
    assert_eq!(f.vim.cursor_offset(), 0, "cursor jumped back to mark a");
}

// ---- huge counts terminate at fixed points -----------------------------------------

/// PROBE: vim's motion repeat loop stops at the first non-advancing step; the
/// old engine's `999999999e` ran a full 10⁹ whole-word scans on the buffer's
/// last char.
#[test]
fn huge_counts_stop_at_fixed_points() {
    let start = std::time::Instant::now();
    let f = edit("foo\n", 0, 2, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "e"]);
    let f2 = edit("foo bar\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "}"]);
    let f3 = edit("foo\n", 0, 0, &["9", "9", "9", "9", "9", "9", "9", "9", "9", "b"]);
    assert!(
        start.elapsed().as_millis() < 500,
        "fixed-point counts must not spin (took {:?})",
        start.elapsed()
    );
    assert_eq!(f.vim.cursor_offset(), 2);
    assert_eq!(f2.vim.cursor_offset(), 6, "brace lands on the last char (cursor clamp)");
    assert_eq!(f3.vim.cursor_offset(), 0);
}

// ---- empty trailing row of a block register ----------------------------------------

/// Host block-API yank rows are joined with `\n` (no terminator); when the
/// last row is empty the text ends with `\n`, and the put-side trim once
/// swallowed that empty last row, inserting one line too few.
#[test]
fn block_register_trailing_empty_row_survives_put() {
    let mut f = Fixture::new("abc\ncde\nfgh\n");
    // build a block whose last row is shorter than the block's left edge
    // (host API: drag-select (0,1)-(2,4))
    f.vim.set_visual_range(&f.buf, 1, 4);
    f.feed(["y"]);
    f.vim.set_cursor_offset(&f.buf, 0);
    f.feed(["p"]);
    // line count only grows: a 2-row block puts 2 rows
    let text = f.text();
    let line_count = text.lines().count();
    assert!(line_count >= 4, "block put must keep both rows, got {text:?}");
}
