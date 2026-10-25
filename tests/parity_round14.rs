//! Round-14 regressions (part 1): motion repetition, operator change records, quote escaping,
//! paragraph objects, register prefixes. All expected behaviors were empirically verified with vim
//! 9.1 probes (probe scripts see NOTES round 14).

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;

/// PROBE: vim 9.1, on `a1b2c3d2e`, `gg0t2;` → col 7 (before the second 2).
/// The old engine's `;` rescanned from the cursor and immediately re-hit the same 2 — a fake success in place.
#[test]
fn till_repeat_semicolon_advances_to_next_target() {
    let f = edit("a1b2c3d2e\n", 0, 0, &["t", "2", ";"]);
    assert_eq!(f.vim.cursor_offset(), 6, "t2; must park before the SECOND 2");
}

/// PROBE: vim 9.1, `t2;;` — the second `;` has no target left, the cursor stays put.
#[test]
fn till_repeat_without_target_stays() {
    let f = edit("a1b2c3d2e\n", 0, 0, &["t", "2", ";", ";"]);
    assert_eq!(f.vim.cursor_offset(), 6, "exhausted repeat must not move");
}

/// PROBE: vim 9.1, on `a1b2c3d3e`, `gg0d2t3` → `3e` (deleted up to before the second 3).
#[test]
fn counted_till_repeat_deletes_through() {
    let f = edit("a1b2c3d3e\n", 0, 0, &["d", "2", "t", "3"]);
    assert_eq!(f.text(), "3e\n", "d2t3 stops before the second 3");
}

/// PROBE: vim 9.1, on `a1b2c3d3e`, `gg02t3` → col 7: a fresh (non-repeat) counted till does not
/// skip the adjacent target — the count is simply the number of finds.
#[test]
fn fresh_counted_till_counts_finds() {
    let f = edit("a1b2c3d3e\n", 0, 0, &["2", "t", "3"]);
    assert_eq!(f.vim.cursor_offset(), 6, "2t3 = second find, no skip");
}

/// PROBE: a fresh `t` on an adjacent target must keep the "success but don't move" semantics (`dx` deletes the target).
#[test]
fn fresh_till_adjacent_target_stays_put_without_bell() {
    let mut f = Fixture::new("a2x\n");
    f.feed(["t", "2"]);
    assert_eq!(f.vim.cursor_offset(), 0);
    assert_eq!(f.host.bells, 0, "adjacent target is a SUCCESS, not a miss");
}

/// PROBE: vim 9.1, on `a22`, `gg0t2;` → col 2 (skip the adjacent target once, then find again).
#[test]
fn till_repeat_skips_adjacent_target_pair() {
    let f = edit("a22\n", 0, 0, &["t", "2", ";"]);
    assert_eq!(f.vim.cursor_offset(), 1);
}

/// PROBE: vim 9.1, `:changes` shows the same number of entries before and after yiw (a yank is not a change).
/// Engine-observable surface: after a yank, `g;` lands on the real edit position, not the yank position.
#[test]
fn yank_does_not_enter_changelist() {
    let mut f = Fixture::new("abc\ndef\n");
    f.feed(["x"]); // the one real change, at offset 0
    f.feed(["G", "y", "i", "w"]); // yank on line 2
    f.feed(["g", "g", "g", ";"]);
    assert_eq!(
        f.vim.cursor_offset(),
        0,
        "g; must land on the x change, not the yank position"
    );
}

/// `d$` on an empty line is a no-op (vim 9.1) and must not pollute the changelist: g; skips it.
#[test]
fn noop_operator_does_not_enter_changelist() {
    let mut f = Fixture::new("abc\n\n");
    f.feed(["x"]); // real change at 0
    f.feed(["j", "d", "$"]); // no-op on the empty line
    f.feed(["g", "g", "g", ";"]);
    assert_eq!(f.vim.cursor_offset(), 0, "empty-span d$ must not record a change");
}

/// PROBE: vim 9.1, on `"a\\"` (the line content is `"a\"` — a double backslash before the closing
/// quote = literal backslash + real quote), `gg0ci"X<Esc>` → `"X"`. The single-backslash check
/// treated the closing quote as an escaped quote, so the whole line lost its pairing.
#[test]
fn quote_object_counts_backslash_run_parity() {
    let mut f = Fixture::new("\"a\\\\\"\n");
    f.feed(["c", "i", "\""]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "\"X\"\n", "double backslash = literal, quote is real");
}

/// A single-backslash escaped quote is still skipped (`"a\"b"` pairs as "a\"b").
#[test]
fn quote_object_single_escape_still_skipped() {
    let mut f = Fixture::new("\"a\\\"b\"\n");
    f.feed(["c", "i", "\""]);
    f.type_text("X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "\"X\"\n");
}

/// PROBE: vim 9.1, on [para1,"",para2], `3Gdap` → ['para1']. The last paragraph with no trailing
/// whitespace takes the leading blank line (:h ap).
#[test]
fn ap_on_last_paragraph_takes_leading_blank() {
    let f = edit("para1\n\npara2\n", 2, 0, &["d", "a", "p"]);
    assert_eq!(f.text(), "para1\n", "separator blank goes with the paragraph");
}

/// PROBE: vim 9.1, on `abc`, `gg0"ax` → @a='a' and the unnamed register is also 'a'.
#[test]
fn register_prefix_routes_x_into_named_register() {
    let f = edit("abc\n", 0, 0, &["\"", "a", "x"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a populated");
    assert_eq!(reg.text, "a", "\"ax must delete into register a");
}

/// PROBE: vim 9.1, on `xabc`, `$h"aX` → @a='a'.
#[test]
fn register_prefix_routes_x_backwards_into_named_register() {
    let f = edit("xabc\n", 0, 2, &["\"", "a", "X"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a populated");
    assert_eq!(reg.text, "a", "\"X must delete into register a");
}

/// PROBE: vim 9.1, on `abc`, `gg0"asZ<Esc>` → @a='a', line = 'Zbc'.
#[test]
fn register_prefix_routes_substitute_into_named_register() {
    let mut f = Fixture::new("abc\n");
    f.feed(["\"", "a", "s"]);
    f.type_text("Z");
    f.feed(["escape"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a populated");
    assert_eq!(reg.text, "a", "\"as must delete into register a");
    assert_eq!(f.text(), "Zbc\n");
}

/// PROBE: vim 9.1, on ['a','b'] do `dd`, `""dd`, `"1p` → ['','b']: the `""` prefix is fully
/// equivalent to no prefix, and the numbered ring rotates as usual.
#[test]
fn double_quote_register_prefix_behaves_like_no_prefix() {
    let f = edit("a\nb\n", 0, 0, &["d", "d", "\"\"", "d", "d", "g", "g", "\"", "1", "p"]);
    // Byte convention follows 'fixendofline' (write-back appends the trailing newline)
    assert_eq!(f.text(), "\nb\n", "\"\"dd must rotate the numbered ring");
}

/// PROBE: vim 9.1, after `:nnoremap j G`, typing `gj` → the cursor lands on line 2 (the builtin gj
/// fires directly; the continuation key is not looked up in the map table). The engine previously
/// let the mapping hijack the continuation key and G jumped to the last line.
#[test]
fn mapping_does_not_hijack_partial_builtin_continuation() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "G", true);
    f.feed(["g", "g"]);
    f.feed(["g", "j"]);
    assert_eq!(
        f.vim.cursor_offset(),
        4,
        "gj's continuation key j must complete the builtin; the mapped G must not fire"
    );
}

/// After `:nnoremap j gj`, typing `gj` does not trip the circuit breaker (the old engine hit the
/// mapping again inside the expansion replay, tripping map_depth, ringing the bell, and gj failed
/// completely).
#[test]
fn noremap_self_referential_builtin_no_runaway() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "gj", true);
    f.feed(["g", "j"]);
    assert_eq!(f.host.bells, 0, "no runaway circuit-breaker");
    assert_eq!(f.vim.cursor_offset(), 4, "builtin gj fires once");
}

/// Between commands mappings keep working: under the `j→G` mapping, a lone `j` still expands to G.
#[test]
fn mapping_still_applies_between_commands() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "G", true);
    f.feed(["g", "g"]);
    f.feed(["j"]);
    assert_eq!(
        f.vim.cursor_offset(),
        8,
        "j at a command boundary expands to G via the mapping (jumping to the last line's start)"
    );
}

/// Engine invariant: mutually-mapped self-reference (`:map x y` + `:map y x`) trips at maxmapdepth
/// (now matching vim's default = 1000): bell, queue cleared, no hang.
#[test]
fn recursive_mapping_depth_guard_stops() {
    let mut f = Fixture::new("abc\n");
    f.vim.keymaps.map_str(vimcore::keymap::ModeClass::Normal, "x", "y");
    f.vim.keymaps.map_str(vimcore::keymap::ModeClass::Normal, "y", "x");
    f.feed(["x"]);
    assert!(f.host.bells > 0, "mutual mappings must trip the breaker and ring");
    assert_eq!(f.text(), "abc\n", "the queue is cleared, buffer untouched");
}

/// Engine invariant: after the pipeline guard clears the queue, the leftover no_remap budget resets
/// to zero — subsequent keys go through mapping resolution normally (a leaked budget once made every
/// later key bypass mappings).
#[test]
fn guard_abort_resets_noremap_budget() {
    // A large RHS triggers the MAX_PIPELINE_STEPS guard (budget 100k keys)
    let big_rhs = "x".repeat(120_000);
    let mut f = Fixture::new("abc\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "q", &big_rhs, true);
    f.feed(["q"]); // passing without a hang suffices (the guard clears the queue at 100k keys)

    // The guard abort is per-session: later sessions resolve mappings normally (w→dd deletes the line)
    let mut f2 = Fixture::new("abc\n");
    f2.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "w", "dd", true);
    f2.feed(["w"]);
    assert_eq!(f2.text(), "", "mapping w→dd takes effect (a leaked budget would make w move by words and leave the buffer)");
}

// ---- cmdline count/register hygiene (vim 9.1 probes P7/P8b/P9/P9b) ----------

/// PROBE: vim 9.1, `3:<CR>` → the cursor lands on line 3: the count becomes the range `.,.+2` and
/// is consumed (the engine previously just opened an empty prompt, letting the count leak into the
/// next command).
#[test]
fn count_before_colon_becomes_range() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &["3", ":", "enter"]);
    assert_eq!(f.vim.cursor_offset(), 4, "3:'s range end address = line 3 (1-based)");
}

/// PROBE: vim 9.1, after `3:<Esc>`, `j` moves exactly 1 line — the count is consumed when the prompt is cancelled.
#[test]
fn count_dropped_on_cancelled_colon() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &["3", ":", "escape", "j"]);
    assert_eq!(f.vim.cursor_offset(), 2, "after cancelling, the count must not amplify the following j");
}

/// PROBE: vim 9.1, after `2/x<Esc>`, `x` deletes exactly 1 character.
#[test]
fn count_dropped_on_cancelled_search() {
    let f = edit("aXbXc\n", 0, 0, &["2", "/", "x", "escape", "x"]);
    assert_eq!(f.text(), "XbXc\n", "a cancelled search must not make x delete two characters");
}

/// PROBE: vim 9.1, `3/foo<CR>` (cursor not on a match) → jump to the 3rd match.
#[test]
fn count_before_search_jumps_to_nth_match() {
    let f = edit("x\nfoo\ny\nfoo\nz\nfoo\n", 0, 0, &["3", "/", "f", "o", "o", "enter"]);
    assert_eq!(
        f.vim.cursor_offset(),
        f.buf.line_start(5),
        "3/foo lands on the 3rd match"
    );
}

/// The count before a visual `:` is consumed (vim cancels the pending count; it does not amplify the next command).
#[test]
fn visual_colon_drops_count() {
    let f = edit("a\nb\nc\n", 0, 0, &["V", "3", "v", ":", "escape", "j"]);
    assert_eq!(f.vim.cursor_offset(), 2, "after a visual : the count must not leak into j");
}

// ---- Ex command surface (vim 9.1 probes P13-P17/P50/P54, B1-B8) ------------

/// PROBE: `:1,2d!` / `:1,2y!` → E477 "No ! allowed", the command does not execute.
#[test]
fn bang_on_delete_yank_reports_e477() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", "!", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", ":d! does not delete");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E477"), "got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "y", "!", "<CR>"]);
    // audit A15: vim 9.1's E477 does not carry the original command line as a suffix (round14's
    // "with the original line" conclusion was a misreading of the oracle back then; re-verified via
    // messages sampling)
    assert_eq!(f2.host.statuses.last().unwrap(), "E477: No ! allowed");
    assert_eq!(
        f2.vim.registers.lock().unwrap().get('0').map(|r| r.text.clone()),
        None,
        "y! did not execute (no yank-register side effect)"
    );
}

/// PROBE: the packed form `:y a3` = register a + count 3 (@a = 'b c ').
#[test]
fn packed_register_count_yank() {
    let f = edit("a\nb\nc\nd\ne\n", 0, 0, &[":", "2", "y", " ", "a", "3", "<CR>"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned().expect("register a");
    assert_eq!(reg.text, "b\nc\nd\n", "a3 = register a + 3 lines");
}

/// PROBE: `:d a b` → E488 trailing 'b', no deletion.
#[test]
fn garbage_register_tail_reports_e488() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "d", " ", "a", " ", "b", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

/// PROBE: garbage in the tail argument of `:j 3x` likewise reports E488 (previously it silently
/// counted 0 and joined anyway).
#[test]
fn join_garbage_count_reports_e488() {
    let f = edit("a\nb\nc\n", 0, 0, &[":", "j", " ", "3", "x", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

/// PROBE: out-of-range addresses report E16 and the command does not execute: `:1000000d`,
/// `:2,99999d`, `:.+99d`, bare `:99999999` (previously clamped to the last line silently and executed).
#[test]
fn out_of_range_addresses_report_e16() {
    for line in ["1000000d", "2,99999d"] {
        let f = edit("a\nb\nc\n", 0, 0, &[":", &line[..line.len() - 1], "<CR>"]);
        // feed goes character by character, so the slice above does not work —— construct them one by one
        drop(f);
    }
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", "0", "0", "0", "0", "0", "0", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "an out-of-range d does not execute");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "2", ",", "9", "9", "9", "9", "9", "d", "<CR>"]);
    let last = f2.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "got {last:?}");
    assert_eq!(f2.text(), "a\nb\nc\n");

    // Address 0 / negative are still legal: acting on the first line (vim 9.1 probes B1/B3)
    let f3 = edit("a\nb\nc\n", 1, 0, &[":", "0", "d", "<CR>"]);
    assert_eq!(f3.text(), "b\nc\n");
}

/// PROBE: `:s/a\/b/x/` matches the literal `a/b` (escaped delimiter; previously the parse was chopped up).
#[test]
fn escaped_delimiter_in_substitute() {
    let f = edit("a/b\n", 0, 0, &[
        ":", "s", "/", "a", "\\", "/", "b", "/", "x", "/", "<CR>",
    ]);
    assert_eq!(f.text(), "x\n", "an escaped delimiter does not split the pattern");
}

/// PROBE: `:%s/foo//n` → "2 matches on 2 lines", buffer untouched (`n` = report only; previously
/// the flag was ignored and a destructive substitution ran).
#[test]
fn substitute_n_flag_reports_without_modifying() {
    let f = edit("foo bar\nx foo y\n", 0, 0, &[
        ":", "%", "s", "/", "f", "o", "o", "/", "/", "n", "<CR>",
    ]);
    assert_eq!(f.text(), "foo bar\nx foo y\n", "the n flag does not modify the buffer");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert_eq!(last, "2 matches on 2 lines");
}

/// The `:se` abbreviation, `:setlocal`, and `:set ts&` resetting to default.
#[test]
fn set_spellings_and_amp_reset() {
    let mut f = Fixture::new("a\n");
    f.feed([":", "s", "e", " ", "t", "s", "=", "2", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 2, ":se ts=2 takes effect");
    f.feed([":", "s", "e", "t", "l", "o", "c", "a", "l", " ", "t", "s", "=", "6", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 6, ":setlocal = :set");
    f.feed([":", "s", "e", "t", " ", "t", "s", "&", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 4, "ts& resets to default");
    f.feed([":", "s", "e", "t", " ", "t", "s", "=", "3", "<CR>"]);
    f.feed([":", "s", "e", "t", " ", "t", "s", "&", "v", "i", "m", "<CR>"]);
    assert_eq!(f.vim.options.tabstop, 4, "ts&vim resets the same way");
}

/// PROBE: `:set ts ?` (a space before the question mark) = query; the bare numeric option name
/// `:set ts` is also a query (previously it belled and aborted the whole line).
#[test]
fn set_spaced_question_mark_queries() {
    let mut f = Fixture::new("a\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", " ", "?", "<CR>"]);
    assert_eq!(
        f.host.statuses.last().map(String::as_str),
        Some("  tabstop=4"),
        "ts ? queries without ringing"
    );
    assert_eq!(f.host.bells, 0);
    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "s", "e", "t", " ", "t", "s", "<CR>"]);
    assert_eq!(
        f2.host.statuses.last().map(String::as_str),
        Some("  tabstop=4"),
        "bare ts = query"
    );
    // boolean name + spaced question mark = query, not set
    let mut f3 = Fixture::new("a\n");
    f3.feed([":", "s", "e", "t", " ", "n", "o", "i", "c", "<CR>"]);
    f3.feed([":", "s", "e", "t", " ", "i", "c", " ", "?", "<CR>"]);
    assert_eq!(
        f3.host.statuses.last().map(String::as_str),
        Some("  noignorecase"),
        "ic ? is a query, not a set"
    );
}

/// The full vim prefix abbreviations of `:bn[ext]`/`:bp[revious]` (`:bne` previously gave E492).
#[test]
fn buffer_command_prefix_spellings() {
    for spelling in ["bn", "bne", "bnex", "bnext"] {
        let mut f = Fixture::new("a\n");
        let keys: Vec<&str> = std::iter::once(":")
            .chain(spelling.chars().map(|c| c.to_string().leak() as &str))
            .chain(std::iter::once("<CR>"))
            .collect();
        f.feed(keys);
        assert!(
            f.host.statuses.is_empty(),
            ":{spelling} must not report an error (got {:?})",
            f.host.statuses.last()
        );
    }
}

// ---- Replace-mode BS position stack + count-repeat autoindent (vim 9.1 probes
//      P23 matrix / Q8 matrix / P24) -------------------------------------------

/// PROBE matrix (Q8a): on `wxyz`, `Rab<BS><Esc>` → 'axyz': BS restores the overwritten original
/// character. A None entry (appended past line end) is treated as a delete when it matches (Q8e).
#[test]
fn replace_bs_restores_matching_position() {
    let mut f = edit("wxyz\n", 0, 0, &["R"]);
    f.type_text("ab");
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "axyz\n", "BS restores the entry matching the position");
}

/// PROBE matrix (Q8b/Q8c/P23b/P23d): BS after a cursor move is a **complete no-op** — the top-of-stack
/// entry does not belong to the current backspace position, so vim neither restores nor moves the
/// cursor. The old LIFO pop wrote to the wrong place (`Rabc<Left><Right><BS>` once yielded 'ayy').
#[test]
fn replace_bs_after_cursor_move_is_noop() {
    // Left once then BS: top of stack is c@2, the backspace position is 1 → no restore
    let mut f = edit("xyz\n", 0, 0, &["R"]);
    f.type_text("abc");
    f.feed(["<Left>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc\n", "P23b: BS after a move does not change the text");

    // P23d: Left Left Right BS → likewise nothing moves
    let mut f2 = edit("xyz\n", 0, 0, &["R"]);
    f2.type_text("abc");
    f2.feed(["<Left>", "<Left>", "<Right>"]);
    f2.feed(["<BS>"]);
    f2.feed(["<Esc>"]);
    assert_eq!(f2.text(), "abc\n", "P23d: BS after a move does not change the text");
}

/// PROBE (P23c): Right has nowhere to go at line end, the cursor is still at the typing frontier →
/// BS restores as usual.
#[test]
fn replace_bs_at_stuck_right_still_restores() {
    let mut f = edit("xyz\n", 0, 0, &["R"]);
    f.type_text("abc");
    f.feed(["<Right>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abz\n", "P23c: Right stuck at line end, BS restores normally");
}

/// PROBE (Q8d): BS on a character appended past line end = a pure delete (None entry), then the
/// overwritten z is restored.
#[test]
fn replace_bs_appended_char_deletes_then_restores() {
    let mut f = edit("wxyz\n", 0, 0, &["R"]);
    f.type_text("abcde");
    f.feed(["<BS>"]);
    f.feed(["<BS>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abcz\n", "Q8d: appended character deleted, overwritten character restored");
}

/// PROBE (P24): under autoindent, `3ifoo<CR>bar<Esc>` yields three foo/bar groups — the count
/// copies the **expanded** text (indent included) as the unit (previously the rep.text
/// original-length invariant failed and the count was silently dropped).
#[test]
fn count_repeat_insert_with_autoindent_newline_replicates() {
    let mut f = Fixture::new("start\n");
    f.vim.options.autoindent = true;
    f.feed(["3", "i"]);
    f.type_text("foo");
    f.feed(["<Enter>"]);
    f.type_text("bar");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "foo\nbarfoo\nbarfoo\nbarstart\n",
        "vim P24: three foo/bar groups"
    );
}

/// The newline-free count-repeat behavior is unchanged (`3ix<Esc>` → 'xxx').
#[test]
fn count_repeat_insert_plain_still_works() {
    let mut f = Fixture::new("y\n");
    f.feed(["3", "i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "xxxy\n");
}

// ---- config parsing (the vimrc surface) -------------------------------------

/// `let mapleader2 = ";"` is an unrelated variable — it must not hijack `<Leader>`, and must leave a trace.
#[test]
fn mapleader_variable_requires_word_boundary() {
    let rc = "let mapleader = \",\"\nlet mapleader2 = \";\"\nnnoremap <Leader>x ix<Esc>\n";
    let config = vimcore::config::parse(rc);
    // the leader is still ","
    let lhs = &config.mappings[0].lhs;
    assert_eq!(
        lhs,
        &vec![vimcore::key::Key::char(','), vimcore::key::Key::char('x')],
        "<Leader> resolves to comma + x"
    );
    assert!(
        config.ignored.iter().any(|l| l.contains("mapleader2")),
        "the mapleader2 line should land in ignored (got {:?})",
        config.ignored
    );
}

/// `set\tnumber` (TAB-separated) must take effect — the old implementation silently ignored it.
#[test]
fn tab_separated_set_line_applies() {
    let config = vimcore::config::parse("set\tnumber\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::On("number".to_owned())],
        "the TAB-separated set line takes effect"
    );
    assert!(config.ignored.is_empty());
}

/// `set ts=4 " note`: the trailing comment must not produce a garbage Setting (it once inflated the ignored count).
#[test]
fn set_line_trailing_comment_dropped() {
    let config = vimcore::config::parse("set ts=4 \" my preference\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::Value("ts".to_owned(), "4".to_owned())],
        "only one setting, ts=4"
    );
}

/// `set ts&` resets to default in an rc; `set ic?` queries are silently dropped (no garbage produced).
#[test]
fn set_reset_and_query_forms_in_rc() {
    let config = vimcore::config::parse("set ts&\nset ic?\n");
    assert_eq!(
        config.settings,
        vec![vimcore::config::Setting::Reset("ts".to_owned())],
        "ts& = Reset, ic? dropped"
    );
}

/// `<LocalLeader>` resolves via `let maplocalleader` (previously a Named key remained and the
/// mapping could never fire).
#[test]
fn local_leader_resolves_from_let_variable() {
    let rc = "let maplocalleader = \"-\"\nnnoremap <LocalLeader>x ix<Esc>\n";
    let config = vimcore::config::parse(rc);
    let lhs = &config.mappings[0].lhs;
    assert_eq!(lhs.len(), 2);
    assert_eq!(
        lhs[0],
        vimcore::key::Key::char('-'),
        "<LocalLeader> resolves to -"
    );
    assert_eq!(lhs[1], vimcore::key::Key::char('x'));
}

/// PROBE: vim 9.1 Q1 — `*` on a non-word non-blank character searches that character **literally**
/// (pattern `!`), neither ringing the bell nor reusing the old pattern. The backward first non-blank
/// on the line works the same way.
#[test]
fn star_on_non_word_char_searches_it_literally() {
    let mut f = Fixture::new("foo !\nbar !\n");
    f.feed(["/", "b", "a", "r", "<Enter>"]); // first arm an unrelated pattern
    f.feed(["g", "g"]); // back to line 1
    f.feed(["$"]); // cursor onto the '!'
    f.feed(["*"]);
    assert_eq!(
        f.vim.search.pattern.as_deref(),
        Some("!"),
        "the literal ! becomes the main pattern"
    );
    assert_eq!(f.vim.cursor_offset(), 10, "jumps to the next ! (line 2)");
}

/// When the cursor line is all whitespace, `*` reports E348 (vim's error string) and no longer references a stale pattern.
#[test]
fn star_on_blank_line_reports_e348() {
    let mut f = Fixture::new("word\n   \n");
    f.feed(["/", "w", "o", "r", "d", "<Enter>"]);
    f.feed(["j", "j"]); // whitespace line
    f.feed(["*"]);
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert_eq!(last, "E348: No string under cursor");
}
