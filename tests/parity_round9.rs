//! Round-9 review regression tests.
//!
//! Same workflow as the earlier rounds: read the code and list the suspects -> verify each with a
//! vim 9.1 probe (`-es` script) -> fix -> register it here. Every test states how it was verified.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

// ---- 1. gq no longer doubles blank / whitespace-only lines ---------------------

/// vim 9.1 probe (`ggVGgq` over ["","",""]): blank lines are kept verbatim -- the old behavior
/// flushed an empty paragraph for every blank line (one extra trailing \n), so three blank lines
/// doubled into seven.
#[test]
fn gq_keeps_blank_lines_one_to_one() {
    let mut f = Fixture::new("\n\n\n");
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n\n\n", "blank lines kept 1:1");
}

/// vim 9.1 probe (tw=10, ["hello world foo bar baz qux","","","tail"]): after the paragraph
/// reformat the two blank lines are still two; paragraph/wrap behaviour is unchanged.
#[test]
fn gq_mixed_paragraphs_and_blank_lines() {
    let mut f = Fixture::new("hello world foo bar baz qux\n\n\ntail\n");
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "hello\nworld foo\nbar baz\nqux\n\n\ntail\n"
    );
}

/// vim 9.1 probe (same shape with ["para","   ","tail"]): a whitespace-only line survives gq
/// **verbatim** (the three spaces stay three spaces); the old behavior collapsed it into a blank line.
#[test]
fn gq_keeps_whitespace_only_lines_verbatim() {
    let mut f = Fixture::new("hello world foo bar baz qux\n   \ntail\n");
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "hello\nworld foo\nbar baz\nqux\n   \ntail\n"
    );
}

// ---- 2. D/C line count ---------------------------------------------------------

/// vim 9.1 probe (['aaaa','bbbb','cccc','dddd'], cursor at (1,2), `3D`):
/// ['a','dddd'] -- deletes from the cursor through the end of the 3rd line below.
/// The old behavior ignored the count.
#[test]
fn d_with_count_deletes_through_count_lines() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd", 0, 1);
    f.feed(["3", "D"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "a\ndddd");
    assert_eq!(f.line(), 0);
}

/// vim 9.1 probe (same buffer, (1,2), `2Cnew<Esc>`): ['anew','cccc','dddd'] --
/// the text up to the cursor is kept, the 2 lines below are cleared, then insert begins.
/// The old behavior changed only the current line.
#[test]
fn c_with_count_changes_through_count_lines() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd", 0, 1);
    f.feed(["2", "C"]);
    assert!(f.vim.mode() == vimcore::Mode::Insert);
    f.type_text("new");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "anew\ncccc\ndddd");
}

/// a count running past the end of the buffer saturates (vim 9.1 probe: ['aaaa','bbbb'] (1,2)
/// `99D` → ['a'] -- the final newline is deleted as well).
#[test]
fn d_count_saturates_at_buffer_end() {
    let mut f = Fixture::at("aaaa\nbbbb", 0, 1);
    f.feed(["9", "9", "D"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "a");
}

// ---- 3. insert Tab aligns by display column ------------------------------------

/// vim 9.1 probe (expandtab ts=4, Tab at the end of the '中文' line): inserts **4** spaces --
/// alignment is by display column (the CJK char is 2 cells wide, display column 4 already is a
/// multiple of ts, and vim pads up to 8). The old behavior used the byte column (6 % 4 = 2) and
/// inserted only 2.
#[test]
fn insert_tab_aligns_by_display_column_after_wide_chars() {
    let mut f = Fixture::at("中文", 0, 6);
    f.feed(["A"]);
    f.vim.options.expandtab = true;
    f.vim.options.tabstop = 4;
    f.feed_raw(vimcore::key::Key::named("tab"));
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文    ");
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文    x");
}

// ---- 4. insert <C-w> line-start semantics (round 30 correction) + block session --

/// The round-30 typeahead re-probe overturned this round's original conclusion: vim 9.1's `i<C-w>`
/// at the start of a line with nothing typed is a **no-op** ("delete the word before the cursor" --
/// there is no character of this line before the cursor; the line join that `backspace=eol` allows
/// only happens when the deletion really crosses the line start).
/// Evidence: ['aaaa','bbbb'] with the cursor at (2,1), `i<C-w>x<Esc>` → ['aaaa','xbbbb']
/// (bs default and bs=start agree). The engine clamps at the typing start point and matches.
#[test]
fn insert_ctrl_w_at_line_start_with_no_typed_text_is_noop() {
    let mut f = Fixture::at("aaaa\nbbbb", 1, 0);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa\nxbbbb",
        "C-w with nothing typed at the line start does not join lines (vim 9.1 round 30 re-probe)"
    );
}

/// <C-w> in the middle of a line does not cross lines: with 'aa bb cc' and the cursor on 'c',
/// <C-w> deletes "bb " -- the same shape as vim (the word plus its preceding whitespace go together).
#[test]
fn insert_ctrl_w_mid_line_stays_within_line() {
    let mut f = Fixture::at("aa bb cc", 0, 6);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aa xcc");
}

/// <C-w> at the line start inside a block-insert session: with nothing typed it neither joins
/// lines nor rings the bell (since round 30 this shape is a no-op -- it deletes no existing text
/// and does not cross lines); hitting the join branch mid-line still refuses with a bell.
/// Typed text is copied to both lines as usual.
#[test]
fn insert_ctrl_w_never_joins_lines_mid_block_session() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    let bells_before = f.host.bells;
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    // nothing typed at the line start: C-w is a no-op (no deletion, no bell; the old behavior
    // took the join branch and rang)
    assert_eq!(f.host.bells, bells_before, "C-w with nothing typed at the line start is a silent no-op");
    f.type_text("X");
    f.feed(["<Esc>"]);
    // the line structure stays three lines (no join), typing still copied to both lines
    assert_eq!(f.buf.line_count(), 3);
    assert_eq!(f.buf.slice(0..f.buf.len()), "Xaaaa\nXbbbb\ncccc");
}

// ---- 5. Ex commands record changelist / `.` mark on the affected line ----------

/// vim: after `:4s` the `.` mark and the changelist point at the substituted line. With two
/// substitutions done, g; should go back to the **previous** (first) substituted line -- the old
/// behavior recorded the pre-command cursor (which stayed on line 0), so g; had nowhere to go.
#[test]
fn ex_substitute_records_changelist_on_substituted_line() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\nfive\n");
    f.feed(["g", "g"]);
    f.feed([":", "4", "s", "/", "o", "/", "0", "/", "<CR>"]);
    assert_eq!(f.line(), 3, "cursor lands on the last substituted line");
    assert_eq!(
        f.vim.marks.last_change.map(|o| f.buf.offset_to_line(o)),
        Some(3),
        "the `.` mark is recorded on the substituted line"
    );
    f.feed(["g", "g"]);
    f.feed([":", "2", "s", "/", "w", "/", "W", "/", "<CR>"]);
    assert_eq!(f.line(), 1);
    f.feed(["g", ";"]);
    // audit E8 (oracle w4): the first g; lands on the **most recent** change, i.e. line 1 of the
    // second substitution; only another press walks to the earlier line 3
    assert_eq!(f.line(), 1, "g; returns to the most recent substituted line");
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 3, "one more step reaches the earlier substituted line");
}

/// Same family for :d: the changelist / `.` mark records the line where the deletion happened (a
/// surviving line), not the pre-command cursor.
#[test]
fn ex_delete_records_changelist_on_surviving_line() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed(["g", "g", "j"]); // move the cursor to line 1 first so it differs from the post-delete landing
    f.feed([":", "1", ",", "2", "d", "<CR>"]);
    assert_eq!(
        f.vim.marks.last_change.map(|o| f.buf.offset_to_line(o)),
        Some(0),
        "the `.` mark is recorded on the surviving 'three' line (the old behavior stored the old cursor line 1)"
    );
}

// ---- 6. the changelist boundary messages depend on direction -------------------

/// vim: before the start of the changelist is E662 (At start), past the end is E663 (At end).
/// The old behavior reported E662 in both directions.
#[test]
fn newer_change_at_end_reports_e663() {
    let mut f = Fixture::new("one\n");
    f.feed(["i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    f.feed(["g", ","]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E663")),
        "running forward past the newest change reports E663, actual {:?}",
        f.host.statuses
    );
}

#[test]
fn older_change_first_press_is_silent() {
    // audit E8 (vim 9.1 oracle w6): the first g; lands on the most recent change and is **silent** --
    // a fresh cursor sits on the newest entry; the old "E662 on the first press" claim was
    // falsified by the oracle. Only the second g; really hits the bottom and reports E662.
    let mut f = Fixture::new("one\n");
    f.feed(["i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    f.feed(["g", ";"]);
    assert!(
        f.host.statuses.is_empty() && f.host.bells == 0,
        "the first g; succeeds silently, actual {:?}",
        f.host.statuses
    );
    f.feed(["g", ";"]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E662")),
        "running backward past the oldest change reports E662, actual {:?}",
        f.host.statuses
    );
}

// ---- 7. r<CR> lands the cursor at the start of the next line -------------------

/// vim 9.1 probe (['abc','def'] `r<CR>`): cursor (2,1) -- after replacing with a newline the cursor
/// lands on the first character of the new next line. The old behavior clamped back to the start of
/// the previous line.
#[test]
fn replace_with_newline_lands_cursor_on_next_line() {
    let mut f = Fixture::at("abc\ndef", 0, 0);
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::enter());
    assert_eq!(f.buf.slice(0..f.buf.len()), "\nbc\ndef");
    assert_eq!(f.line(), 1, "cursor lands on the next line");
    assert_eq!(f.cursor(), f.buf.line_start(1));
}

// ---- 8. :j with a count / the full `:substitute` spelling ----------------------

/// vim 9.1 probe (['aaaa'..'dddd'] `:2j 3`): joins 3 lines starting from the last line of the
/// range → ['aaaa','bbbb cccc dddd']. The old behavior ignored the count and joined only lines 2, 3.
#[test]
fn ex_join_with_count() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\ndddd\n");
    f.feed([":", "2", "j", " ", "3", "<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaa\nbbbb cccc dddd\n");
}

/// vim's full `:[range]s[ubstitute]` spelling: `:%substitute/a/b/` means the same as `:%s/a/b/`
/// (without the g flag only the first match of each line is substituted). The old behavior gave E492.
#[test]
fn substitute_full_spelling() {
    let mut f = Fixture::new("aaa\naba\n");
    f.feed([":"]);
    // the Ex command line is fed character by character (feed's domain is a single key; a bare
    // multi-character string is a Named key)
    for c in "%substitute/a/b/".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "baa\nbba\n");
    // vim 9.1 (oracle): 2 substitutions <= 'report'(2) → no count feedback; and the command is recognized
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "%substitute is a legal full spelling: {:?}",
        f.host.statuses
    );
}

// ---- 10. `:s/foo` with a missing replacement = empty; blockwise p leaves exhausted rows empty

/// vim 9.1 probe (`:s/foo` with no trailing delimiter): equivalent to `:s/foo//` -- deletes the
/// match; it neither rings the bell nor reuses the previous command's replacement.
#[test]
fn substitute_without_replacement_deletes_the_match() {
    let mut f = Fixture::new("foo\n");
    f.feed([":"]);
    for c in "s/foo".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n");
    assert_eq!(f.host.bells, 0, "no bell (the old behavior rang outright when the replacement was missing)");
}

/// vim 9.1 probe (a 2-line blockwise register put over a 3-line blockwise selection): the covered
/// range of the third row is **deleted outright** (empty string) -- no cycling and no repeat of the
/// last row. The old behavior pasted the register's last row there.
#[test]
fn block_put_with_short_register_leaves_exhausted_rows_empty() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\n");
    f.feed(["g", "g", "<C-v>", "j", "l", "y"]); // 2-row block "aa"/"bb"
    f.feed(["g", "g", "l", "l", "<C-v>", "j", "j", "l", "l", "p"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaa\nbbbb\ncc\n");
}

/// `<C-u>` with nothing typed at the line start is a **no-op** -- the round-30 typeahead re-probe
/// overturned this round's original join conclusion: in vim 9.1 there is no character of this line
/// before the line start to delete (bs default and bs=start agree: ['aaaa','bbbb'] (2,1)
/// `i<C-u>x<Esc>` → ['aaaa','xbbbb']). The engine's dead-end branch was removed along with it and
/// now clamps at the line start.
#[test]
fn insert_ctrl_u_at_line_start_with_no_typed_text_is_noop() {
    let mut f = Fixture::at("aaaa\nbbbb", 1, 0);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('u'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa\nxbbbb",
        "C-u with nothing typed at the line start does not join lines (vim 9.1 round 30 re-probe)"
    );
}

// ---- 12. insert_change_pos floored after a cross-line move (caught by fuzz round9)

/// Caught by fuzz round9 (seed 7): the first typing position of an insert session
/// (insert_change_pos) is a bare offset that the edit funnel never adjusts -- once a BS join inside
/// the session moved the text, the offset recorded into the changelist at exit fell in the middle of
/// a multi-byte character and the host panicked in offset_to_line while `:marks` listed the `.` line.
/// Fix: on exit, floor it to a character boundary of the current text.
#[test]
fn insert_change_pos_floored_after_mid_session_line_join() {
    // "中文\nab\n": the first typing position on line 1 = offset 5; after the BS join removes the
    // \n(4) the text becomes "中文zab\n" and offset 5 falls inside '文' (3..6) -- the old behavior
    // recorded it verbatim in the changelist and the host panicked in offset_to_line while `:marks`
    // listed the `.` line.
    let mut f = Fixture::at("中文\nab\n", 1, 0);
    f.feed(["i"]);
    f.type_text("z"); // insert_change_pos = 5
    f.feed(["<left>", "<BS>"]); // join: the text becomes "中文zab\n"
    f.feed(["<Esc>"]); // exit_insert records the changelist
    let text = f.text();
    assert_eq!(text, "中文zab\n");
    match f.vim.marks.last_change {
        Some(o) => assert!(
            o <= text.len() && (o == text.len() || text.is_char_boundary(o)),
            "the changelist position must sit on a character boundary (fuzz round9 caught it mid-char), got {o}"
        ),
        None => panic!("the changelist should have an entry"),
    }
    // `:marks` listing the `.` mark must not panic
    f.feed([":"]);
    for c in "marks".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文zab\n");
}

// ---- 11. :bfirst / :blast ---------------------------------------------------------

/// NOTES.md round 8 "unresolved": once the host trait gained `first_buffer`/`last_buffer`,
/// `:bfirst`/`:blast` were let through (vim aliases `:bf`/`:bl`/`:brewind`).
#[test]
fn bfirst_and_blast_reach_the_host() {
    let mut f = Fixture::new("one\n");
    f.feed([":", "b", "f", "i", "r", "s", "t", "<CR>"]);
    assert!(f.host.first_last_calls == vec![true]);
    f.feed([":", "b", "l", "a", "s", "t", "<CR>"]);
    assert!(f.host.first_last_calls == vec![true, false]);
    // empty buffer list (the host returns false) → bell
    f.host.first_last_result = false;
    let bells = f.host.bells;
    f.feed([":", "b", "f", "<CR>"]);
    assert!(f.host.bells > bells, "bell when the host refuses");
}
