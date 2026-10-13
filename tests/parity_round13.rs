//! Round-13 regression: locks in every fix surfaced by the read-the-code probes, the fuzz
//! bombardment and the three-way review.
//! All expected behaviors were verified with probes against vim 9.1 (`/usr/bin/vim`, patches 1-1752).

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;
use vimcore::keymap::ModeClass;
use vimcore::registers::{RegisterKind, Registers};
use vimcore::state::KeyResult;

/// PROBE: vim 9.1 — clicking (mouse) inside visual mode moves the cursor and
/// EXTENDS the selection from the original anchor (`:h visual-use`: a mouse
/// click while visual moves the cursor; the anchor stays). If the engine's
/// `set_cursor_offset` overwrites `visual_anchor` with the click point, the
/// selection collapses to zero width.
#[test]
fn probe_click_in_visual_keeps_anchor() {
    let mut f = Fixture::new("abcdef");
    f.feed(["v"]);
    f.vim.set_cursor_offset(&f.buf, 4); // click on 'e'
    let sel = f.vim.visual_selection();
    assert_eq!(
        sel,
        Some((0, 4, vimcore::mode::VisualKind::Char)),
        "click in visual must keep the anchor at 0 (vim: selection follows the cursor)"
    );
}

/// PROBE: `mA` — uppercase marks are global-file marks in vim; a
/// single-buffer engine may treat them as local, but set+jump must work.
#[test]
fn probe_uppercase_mark_set_and_jump() {
    let mut f = edit("one\ntwo\nthree\n", 0, 0, &["m", "A"]);
    assert_eq!(f.vim.marks.get('A'), Some(0), "mA must store a mark");
    f.feed(["G", "`", "A"]);
    assert_eq!(f.vim.cursor_offset(), 0);
}

/// PROBE: `q1` — vim only accepts a-zA-Z0-9 as macro registers; a random
/// char (e.g. `q/`) must be rejected, not silently recorded.
#[test]
fn probe_macro_register_charset() {
    let mut f = Fixture::new("abc\n");
    f.feed(["q", "/"]);
    assert!(f.vim.macro_recording().is_none(), "q/ must not start a recording");
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "qa…q records into a");
}

/// PROBE: `""p` — an empty register spelling pastes the unnamed register.
#[test]
fn probe_double_quote_register() {
    let f = edit("ab\ncd\n", 0, 0, &["yy", "j", "\"\"", "p"]);
    assert!(
        f.text().contains("ab\n"),
        "empty register spec = unnamed; got {:?}",
        f.text()
    );
}

/// PROBE: `g;`/`g,` with an empty changelist and huge counts must not panic.
#[test]
fn probe_changelist_edge() {
    let mut f = Fixture::new("x\n");
    f.feed(["g;", "g,", "g;", "10g;", "10g,"]);
    assert_eq!(f.vim.cursor_offset(), 0);
}

/// PROBE: C-o/C-i walking with an empty jumplist, then real jumps.
#[test]
fn probe_jumplist_walk() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
    f.feed(["<C-o>", "<C-i>", "G", "<C-o>", "<C-o>", "<C-i>"]);
    assert!(f.vim.cursor_offset() < f.buf.len());
}

/// PROBE: octal increment growing past its width (`0777` + C-a → `01000`).
#[test]
fn probe_octal_increment_overflow() {
    let f = edit("0777\n", 0, 0, &["<C-a>"]);
    assert_eq!(
        f.text(),
        "01000\n",
        "octal must grow past its width, got {:?}",
        f.text()
    );
}

/// PROBE: `:set`-describe with garbage names must return None, not panic.
/// (The `?` suffix is stripped by the cmdline caller, not by `describe`.)
#[test]
fn probe_describe_garbage() {
    let o = vimcore::options::Options::default();
    // `no{bool}` is a legal spelling of `{bool}?`, and the prefix is stripped for **value options**
    // just the same: vim 9.1's `execute('set notabstop?')` (with `ts=6`) returns
    // `  tabstop=6`, not unknown option (proven by audit8 A-6; the old pin is void)
    assert_eq!(o.describe("notabstop").as_deref(), Some("  tabstop=4"));
    assert_eq!(o.describe("nosuchopt"), None);
    assert_eq!(o.describe("nu").as_deref(), Some("  nonumber"));
}

/// PROBE: register semantics — uppercase append merges linewise; blackhole
/// delete leaves the unnamed register untouched.
#[test]
fn probe_register_semantics() {
    let mut r = Registers::default();
    r.store_yank(Some('a'), "one\n".into(), RegisterKind::Linewise);
    r.store_delete(Some('A'), "two\n".into(), RegisterKind::Linewise, false);
    let reg = r.get('a').unwrap();
    assert_eq!(reg.text, "one\ntwo\n", "uppercase append merges, got {:?}", reg);
    assert_eq!(reg.kind, RegisterKind::Linewise);

    let mut r2 = Registers::default();
    r2.store_yank(None, "keep\n".into(), RegisterKind::Linewise);
    r2.store_delete(Some('_'), "gone\n".into(), RegisterKind::Linewise, false);
    assert_eq!(r2.get('"').unwrap().text, "keep\n");
}

/// PROBE (round-19 correction): the old pin here claimed `3D` at col 0
/// "empties line 1" — that was REASONING, never a real probe. Byte-level
/// vim 9.1 PTY probes (round 19): `count >= 2` from the line start deletes
/// the covered lines WHOLE (`3D` → "dddd\n", `2D` on a/b/c → "c\n",
/// `99D` on two lines → empty buffer), while count=1 empties the cursor
/// line (see review_regressions). With the cursor ON a char (col 1) the
/// round-9 probe shape holds (`99D` → "a").
#[test]
fn probe_count_d_shapes() {
    let f = edit("aaaa\nbbbb\ncccc\ndddd\n", 0, 0, &["3", "D"]);
    assert_eq!(f.text(), "dddd\n", "3D from col 0 deletes covered lines whole, got {:?}", f.text());

    let f2 = edit("aaaa\nbbbb\n", 0, 0, &["9", "9", "D"]);
    assert_eq!(f2.text(), "", "99D from (0,0) deletes the whole buffer, got {:?}", f2.text());

    let f3 = edit("aaaa\nbbbb\n", 0, 1, &["9", "9", "D"]);
    // round-19 correction: the old pin "a" claimed the final newline goes
    // too — that was reasoning, not a probe. Byte-level vim 9.1 PTY probe
    // (`ggl2D` on "aaaa\nbbbb\n" then :wq → file "a\n"): the file's final
    // newline SURVIVES a from-mid-line count-D.
    assert_eq!(f3.text(), "a\n", "99D from col 1 keeps the text before the cursor AND the file eol");
}

/// PROBE: a self-expanding mapping pair must trip the depth guard, not hang
/// or corrupt the buffer.
#[test]
fn probe_mapping_ping_pong_terminates() {
    let mut f = Fixture::new("hello\n");
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "x", "y");
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "y", "x");
    let r = f.feed_raw(vimcore::key::Key::char('x'));
    assert_eq!(r, KeyResult::Consumed);
    assert_eq!(f.text(), "hello\n", "ping-pong mapping aborts without editing");
}

/// PROBE: `0` typed while a count is pending is a count digit (`10j`).
/// `0` alone must stay the line-start motion (documented, but guard the
/// boundary: `100` then escape then `0`).
#[test]
fn probe_zero_after_escape_is_motion() {
    let f = edit("abc\ndef\n", 0, 2, &["10", "escape", "0"]);
    assert_eq!(f.vim.cursor_offset(), 0, "0 after cancel is the line-start motion");
}

/// PROBE: `.` after a no-op command (`x` on an empty line) must not replay
/// a stale change (vim: `.` with no last change beeps).
#[test]
fn probe_dot_with_no_change_bells() {
    let mut f = Fixture::new("\nfoo\n");
    f.feed(["x"]);
    let bells_before = f.host.bells;
    f.feed(["."]);
    assert!(
        f.host.bells > bells_before,
        "`.` with no recorded change must bell (vim E30-ish)"
    );
    assert_eq!(f.text(), "\nfoo\n");
}

/// PROBE: huge count on `dd` stays bounded and does not panic.
#[test]
fn probe_huge_count_dd() {
    let digits = ["9"; 20];
    let mut keys: Vec<&str> = digits.to_vec();
    keys.extend(["d", "d"]);
    let f = edit("a\nb\n", 0, 0, &keys);
    assert_eq!(f.text(), "", "count saturates, both lines die");
}

// ---- data-loss surface -----------------------------------------------------

/// `~` consumes the whole grapheme cluster but maps only the base character: combining marks and the
/// tail of a ZWJ family must survive verbatim (vim 9.1: after `~`, `E`+U+0301 is still 3 characters;
/// the emoji family is completely unchanged).
#[test]
fn tilde_preserves_grapheme_tail() {
    // decomposed é (e + U+0301) followed by x: ~ toggles the case of the e only
    let f = edit("e\u{0301}x\n", 0, 0, &["~"]);
    assert_eq!(f.text(), "E\u{0301}x\n", "the combining-mark tail must survive");

    // ZWJ emoji family: ~ acts on the base character, the family keeps its 9 units untouched and the
    // cursor moves right
    // ZWJ emoji family: ~ consumes the whole family but writes every unit back verbatim (the base
    // character has no case), not one member may go missing (vim 9.1 probe: family fully unchanged)
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F466}";
    let f2 = edit(&format!("{family} b\n"), 0, 0, &["~"]);
    assert_eq!(f2.text(), format!("{family} b\n"), "not a single family member may be missing");
}

/// The trailing empty line of a linewise register survives p/P (vim 9.1: a register holding
/// ["a",""] pastes two lines).
#[test]
fn linewise_put_keeps_trailing_empty_lines() {
    use vimcore::registers::RegisterKind;
    // P above: 5 lines
    let mut f = Fixture::new("a\n\nz\n");
    f.feed(["g", "g", "V", "j", "y"]); // register = "a\n\n"
    f.feed(["G", "P"]);
    assert_eq!(f.text(), "a\n\na\n\nz\n", "P keeps the register's empty line");

    // p past the end of the buffer (last line without a newline): ["a",""]
    let mut f2 = Fixture::new("z");
    f2.vim
        .registers
        .store_yank(None, "a\n\n".into(), RegisterKind::Linewise);
    f2.feed(["p"]);
    // the byte text of ["z","a",""] gets a final newline appended per 'fixendofline': "z\na\n\n"
    // (vim 9.1 od evidence: writing back a noeol file after yyp carries the terminator)
    assert_eq!(f2.text(), "z\na\n\n", "p keeps the register's empty line");
}

// ---- block-session surface --------------------------------------------------

/// When a block session exits and the typing-row delta differs from the copy length (a diverged
/// session), the replication is skipped instead of writing misaligned copies. This reproduces
/// deterministically the mid-char insert caught by fuzz: the count-repeat left over from 3o once
/// inserted the text first at block-session exit, pushing the copy offset into a multi-byte character.
#[test]
fn block_session_replication_divergence_safe() {
    // delete the typed text with <Del> in the middle of a block I session (vim probe: no row is reproduced)
    let mut f = Fixture::new("ab中\ncd中\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    f.feed(["<Left>", "<Del>", "escape"]);
    assert_eq!(f.text(), "ab中\ncd中\n", "a diverged session reproduces nothing (vim 9.1 evidence)");
}

/// With the per-edit hlsearch rescan turned off, the host's refresh_highlights must really refresh the
/// match cache and the highlights (it used to be short-circuited by the hlsearch_live_update guard
/// into a permanent no-op).
#[test]
fn refresh_highlights_works_with_live_update_off() {
    let mut f = Fixture::new("fox dog fox\n");
    f.vim.set_hlsearch_live_update(false);
    f.feed(["/", "f", "o", "x", "<CR>"]);
    f.feed(["x"]); // the edit removes one o: with live update off, the cache may be stale
    let mut ctx = vimcore::state::Ctx {
        buf: &mut f.buf,
        host: &mut f.host,
    };
    f.vim.refresh_highlights(&mut ctx);
    // cache and highlights both refresh against the current text (1 fox left: the first lost an o to x)
    assert_eq!(f.vim.search.last_matches.len(), 1, "the explicit refresh updates the match cache");
    assert_eq!(f.host.highlights.len(), 1, "the explicit refresh publishes the new highlights");
}

// ---- visual linewise semantics (vim probes: v_D/v_X delete whole lines, v_Y yanks
// lines, v_C=v_S changes lines) -----------------------------------------------

#[test]
fn visual_linewise_spellings() {
    // v_D deletes the covered lines
    let f = edit("abcd\nefgh\nijkl\n", 0, 0, &["v", "l", "D"]);
    assert_eq!(f.text(), "efgh\nijkl\n", "v_D is a linewise delete");

    // v_Y yanks whole lines (p opens a new line)
    let f2 = edit("abcd\nefgh\n", 0, 0, &["v", "l", "Y", "p"]);
    assert_eq!(f2.text(), "abcd\nabcd\nefgh\n", "v_Y is a linewise yank");

    // v_C changes whole lines (keeps the line, empties it, enters insert)
    let mut f3 = edit("abcd\nefgh\nijkl\n", 0, 0, &["v", "j", "l", "C"]);
    f3.type_text("x");
    f3.feed(["escape"]);
    assert_eq!(f3.text(), "x\nijkl\n", "v_C is a linewise change");

    // v_x keeps the charwise semantics
    let f4 = edit("abcd\nefgh\n", 0, 0, &["v", "l", "x"]);
    assert_eq!(f4.text(), "cd\nefgh\n", "v_x is a charwise delete");
}

// ---- the ap paragraph span (directional) ----------------------------------

#[test]
fn ap_paragraph_span_directional() {
    // text line: swallows every following blank line (vim: [aaa,bbb,"","",ccc]
    // with 2Gdap -> [ccc])
    let f = edit("aaa\nbbb\n\n\nccc\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f.text(), "ccc\n", "all trailing blank lines go with it");

    // blank line: the blank block plus the whole next paragraph
    // (not that paragraph's own trailing blanks)
    let f2 = edit("aaa\n\n\nbbb\nccc\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f2.text(), "aaa\n", "from a blank line the session swallows the whole next paragraph");

    let f3 = edit("aaa\n\nbbb\n\nx\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f3.text(), "aaa\n\nx\n", "the next paragraph's own trailing blank lines survive");
}

// ---- blockwise A pads the gap (vim probe: "123456"/"12" -> "123 X456"/"12  X")

#[test]
fn block_append_pads_to_append_column() {
    // note: the engine's blockwise visual `l` does not go past the end of a
    // line (vim lets a block enter virtual columns -- recorded as NOTES
    // divergence #35), so the short line is placed in the middle of the block
    // to build the shape
    let mut f = Fixture::new("123456\n1\n123456\n");
    f.feed(["<C-v>", "j", "j", "l", "A"]);
    f.type_text("_X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "12_X3456\n1 _X\n12_X3456\n", "the short line is padded up to the append column");
}

// ---- Ex command surface ---------------------------------------------------

#[test]
fn ex_bang_forms_and_abbreviations() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "w", "!", "<CR>"]);
    assert_eq!(f.host.saved, 1, ":w! saves (it used to be E492)");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "2", "d", "e", "<CR>"]);
    assert_eq!(f2.text(), "a\nc\n", ":2de is the abbreviated delete");

    let f3 = edit("ab\n", 0, 0, &[":", "s", "u", "/", "a", "/", "b", "/", "<CR>"]);
    assert_eq!(f3.text(), "bb\n", ":su is an abbreviation of :substitute");
}

#[test]
fn ex_garbage_count_reports_e488_and_deletes_nothing() {
    // [Round-14 correction] vim 9.1 probe P13: `:1,2d 3x` reports E488 and
    // **deletes nothing**. Round 13 treated the garbage count as 0 (= no
    // count) and still deleted the whole typed range -- the adjacent fix had
    // the right direction but the wrong endpoint; round 14's parse_reg_count
    // now reports an error.
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", " ", "3", "x", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "garbage trailing argument = E488, buffer untouched");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

#[test]
fn ex_address_whitespace_and_midrange_percent() {
    // [Round-14 correction] skipping whitespace inside an address is proven
    // with an **in-range** example (vim 9.1): `:2 +1d` deletes line 3. The
    // out-of-range address `:5 +2` now reports E16 per probe P15 (the "clamp
    // to $" in the round-12 comment was what that implementation expected,
    // not vim behaviour).
    let f = edit("a\nb\nc\nd\n", 0, 0, &[":", "2", " ", "+", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nd\n", "whitespace inside the address is skipped");

    let f3 = edit("a\nb\nc\n", 0, 0, &[":", "5", " ", "+", "2", "d", "<CR>"]);
    let last = f3.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "out-of-range address = E16, got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "%", "d", "<CR>"]);
    assert_eq!(f2.text(), "", "mid-range % = 1,$ (it used to be the cursor line)");
}

#[test]
fn ex_comment_lines() {
    // a leading " makes the whole line a comment, skipped silently
    // (it used to be E492)
    let mut f = Fixture::new("a\n");
    f.feed([":", " ", "\"", " ", "s", "c", "r", "a", "t", "c", "h", "<CR>"]);
    assert_eq!(f.text(), "a\n");
    assert!(f.host.bells == 0, "a comment line does not ring the bell");

    // a trailing comment on :set
    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "s", "e", "t", " ", "t", "s", "=", "8", " ", "\"", " ", "x", "<CR>"]);
    assert!(f2.host.bells == 0, ":set ts=8 \" x takes effect silently");
}

#[test]
fn cmdline_editing_chords() {
    // C-u clears the line, C-w deletes a word
    let mut f = Fixture::new("a\n");
    f.feed([":", "f", "o", "o"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('u'));
    f.feed(["w"]);
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "a\n", "C-u clears foo, leaving only :w");
    assert_eq!(f.host.saved, 1);

    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "f", "o", "o", " ", "b", "a"]);
    f2.feed_raw(vimcore::key::Key::ctrl_char('w'));
    assert_eq!(f2.vim.cmdline.buffer, "foo ", "C-w deletes the last word");
}

#[test]
fn cmdline_history_dedups_nonconsecutive() {
    use vimcore::key::Key;
    let mut f = Fixture::new("a\n");
    for line in ["/foo", "/bar", "/foo"] {
        let mut chars = line.chars();
        f.feed_raw(Key::char(chars.next().unwrap())); // the prompt char
        for c in chars {
            f.feed_raw(Key::char(c));
        }
        f.feed_raw(Key::enter());
    }
    let history = f.vim.cmdline.history.get(&'/').unwrap();
    assert_eq!(history, &vec!["bar".to_owned(), "foo".to_owned()], "the older duplicate is moved away");
}

// ---- macro and mapping surface --------------------------------------------

#[test]
fn macro_register_rules() {
    use vimcore::keymap::ModeClass;
    // qA appends to the recording of a
    let mut f = Fixture::new("abc\n");
    f.feed(["q", "a", "x", "q"]);
    f.feed(["j"]);
    f.feed(["q", "A", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 2, "qA appends to a");
    assert_eq!(f.vim.macro_len('A'), 0, "there is no separate A slot");

    // a count before q does not leak
    let mut f2 = Fixture::new("abc\n");
    f2.feed(["2", "q", "a", "q"]);
    f2.feed(["x"]);
    assert_eq!(f2.text(), "bc\n", "the count 2 does not multiply x after q");
    let _ = ModeClass::Normal;
}

/// Several declined printable keys held back by a mapping prefix must be
/// handed to the host in order (the single-slot IOU used to drop j -- with
/// `:imap jk <Esc>`, typing `jx` produced `xXabc`).
#[test]
fn declined_printables_queue_in_order() {
    use vimcore::keymap::ModeClass;
    let mut f = Fixture::new("abc\n");
    f.vim.keymaps_mut().map_str(ModeClass::Insert, "jk", "<Esc>");
    f.feed(["i"]);
    f.feed(["j"]); // mapping prefix: the engine answers Consumed, owing the host a j
    f.feed(["x"]); // x breaks the prefix: both j and x should queue for the host to place
    let pending = f.vim.take_pending_unknown_chars();
    assert_eq!(pending, vec!['j', 'x'], "both declined characters are owed, in order");
}

// ---- configuration surface ------------------------------------------------

#[test]
fn config_g_mapleader_and_tab_separator() {
    use vimcore::config;
    let cfg = config::parse("let g:mapleader = \",\"\nnnoremap <Leader>w :w<CR>\n");
    assert!(
        !cfg.mappings.is_empty(),
        "g:mapleader takes effect, <Leader>w resolves to ,w"
    );
    assert_eq!(cfg.mappings[0].lhs.len(), 2, "two keys: leader + w");

    let cfg2 = config::parse("nnoremap\t<Leader>x ix<CR>");
    assert!(!cfg2.ignored.iter().any(|l| l.contains("nnoremap\t")), "a TAB separator does not land in ignored");
    assert_eq!(cfg2.mappings.len(), 1, "a TAB-separated mapping parses correctly");
}