//! Round-29 probes (four): expanded-key boundaries of macros/replay × mappings.

mod common;

use common::Fixture;
use vimcore::keymap::ModeClass;

fn f_with_nmap(initial: &str, lhs: &str, rhs: &str) -> Fixture {
    let mut f = Fixture::at(initial, 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Normal, lhs, rhs, true);
    f
}

/// Probe BA: after deleting the mapping, `.` replays the original key (re-expanded through the mapping).
#[test]
fn probe_dot_after_mapped_delete() {
    let mut f = f_with_nmap("a1\na2\na3\n", "x", "dd");
    f.feed(["x"]);
    assert_eq!(f.text(), "a2\na3\n");
    f.feed(["."]);
    assert_eq!(f.text(), "a3\n", "`.` replays x, re-expanded through the mapping to delete another line");
}

/// Probe BB: count repeat `2.` expands the mapped key once per repetition.
#[test]
fn probe_count_dot_with_mapped_key() {
    let mut f = f_with_nmap("a1\na2\na3\na4\n", "x", "dd");
    f.feed(["x"]);
    f.feed(["2", "."]);
    assert_eq!(f.text(), "a4\n", "2. expands dd twice");
}

/// Probe BC: a macro containing a mapped key replayed with a count, `2@a`.
#[test]
fn probe_count_at_replay_with_mapped_key() {
    let mut f = f_with_nmap("a1\na2\na3\na4\na5\n", "x", "dd");
    f.feed(["q", "a", "x", "q"]); // record one dd execution
    assert_eq!(f.text(), "a2\na3\na4\na5\n");
    f.feed(["2", "@", "a"]);
    assert_eq!(f.text(), "a4\na5\n", "2@a expands dd twice");
}

/// Probe BD: changes triggered by a mapping form a normal undo group.
#[test]
fn probe_undo_after_mapped_delete() {
    let mut f = f_with_nmap("a1\na2\n", "x", "dd");
    f.feed(["x"]);
    f.feed(["u"]);
    assert_eq!(f.text(), "a1\na2\n", "u undoes the mapping-expanded dd");
}

/// Probe BE: insert mapping `z` → `<Esc>`: exits insert, `.` boundary stays clean.
#[test]
fn probe_insert_mapping_escape_flow() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Insert, "z", "<Esc>", true);
    f.feed(["i"]);
    f.type_text("X");
    f.feed(["z"]); // expands to Esc
    assert_eq!(f.vim.mode_indicator(), "");
    assert_eq!(f.text(), "Xab\ncd\n");
    // last_change covers only this insertion, not scrambled by the mapping expansion.
    f.feed(["G"]);
    f.feed(["."]);
    // G lands on the first non-blank of the last line, `.` replays the insertion there.
    assert_eq!(f.text(), "Xab\nXcd\n", "`.` replays the X insertion");
}

/// Probe BF: no spurious bell during mapping expansion — `nnoremap <F2> dd` with a Named key.
#[test]
fn probe_named_key_mapping_in_macro() {
    let mut f = Fixture::at("r1\nr2\nr3\n", 0, 0);
    f.vim
        .keymaps_mut()
        .map_str_noremap(ModeClass::Normal, "<F2>", "dd", true);
    f.feed(["q", "a", "<F2>", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "Named keys are also stored as raw keys");
    assert_eq!(f.text(), "r2\nr3\n");
    f.feed(["@", "a"]);
    assert_eq!(f.text(), "r3\n");
}

/// Probe BG: redo-register `"1P .` does not conflict with mappings (expansion never enters the redo path).
#[test]
fn probe_redo_register_unaffected_by_mappings() {
    let mut f = f_with_nmap("one\ntwo\nthree\n", "y", "yy"); // y is no longer a yank prefix!
    f.feed(["d", "d"]);
    f.feed(["d", "d"]);
    f.feed(["\"", "1", "P"]);
    f.feed(["."]);
    assert_eq!(f.text(), "one\ntwo\nthree\n", "redo-register restores two lines in a row");
    let _ = &mut f;
}

/// Probe BH: nested expansion (noremap pointing at a remap) has bounded recording depth and records the outermost key.
#[test]
fn probe_nested_expansion_bounded() {
    let mut f = Fixture::at("xy\n", 0, 0);
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "a", "b");
    f.vim.keymaps_mut().map_str_noremap(ModeClass::Normal, "b", "x", true);
    f.feed(["a"]); // a → b (remap) → x (noremap, literal delete)
    assert_eq!(f.text(), "y\n", "nested expansion stops at the noremap layer");
}

/// Probe BI: recording on the plain (unmapped) path is unchanged after the fix.
#[test]
fn probe_plain_typing_recording_unchanged() {
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    f.feed(["d", "d"]);
    f.feed(["j"]);
    f.feed(["."]);
    assert_eq!(f.text(), "", "`.` replay of a plain dd is unaffected");
}
