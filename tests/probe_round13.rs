//! Round 13 review probes: candidate bugs found while reading state.rs.
//! Each test documents expected vim 9.1 behavior; failures are real findings.

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
    assert_eq!(o.describe("notabstop"), None);
    assert_eq!(o.describe("nosuchopt"), None);
    assert_eq!(o.describe("nu").as_deref(), Some("nonumber"));
}

/// PROBE: register semantics — uppercase append merges linewise; blackhole
/// delete leaves the unnamed register untouched.
#[test]
fn probe_register_semantics() {
    let mut r = Registers::default();
    r.store_yank(Some('a'), "one\n".into(), RegisterKind::Linewise);
    r.store_delete(Some('A'), "two\n".into(), RegisterKind::Linewise);
    let reg = r.get('a').unwrap();
    assert_eq!(reg.text, "one\ntwo\n", "uppercase append merges, got {:?}", reg);
    assert_eq!(reg.kind, RegisterKind::Linewise);

    let mut r2 = Registers::default();
    r2.store_yank(None, "keep\n".into(), RegisterKind::Linewise);
    r2.store_delete(Some('_'), "gone\n".into(), RegisterKind::Linewise);
    assert_eq!(r2.get('"').unwrap().text, "keep\n");
}

/// PROBE: `3D` on 4 lines from col 0: D deletes from the CURSOR onward, so
/// the first line becomes empty and line 4 survives; `99D` past the buffer
/// end also takes the final newline. With the cursor ON a char (col 1) the
/// engine's documented probe shape holds (`99D` → ['a']).
#[test]
fn probe_count_D_shapes() {
    let f = edit("aaaa\nbbbb\ncccc\ndddd\n", 0, 0, &["3", "D"]);
    assert_eq!(f.text(), "\ndddd\n", "3D empties line 1, keeps line 4, got {:?}", f.text());

    let f2 = edit("aaaa\nbbbb\n", 0, 0, &["9", "9", "D"]);
    assert_eq!(f2.text(), "", "99D from (0,0) deletes the whole buffer, got {:?}", f2.text());

    let f3 = edit("aaaa\nbbbb\n", 0, 1, &["9", "9", "D"]);
    assert_eq!(f3.text(), "a", "99D from col 1 keeps the text before the cursor (documented probe)");
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
