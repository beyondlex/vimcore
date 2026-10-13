//! Second independent audit, group D (Ex commands / range addresses / command-line editing keys /
//! `:set` options).
//!
//! 15 new findings, one `#[test]` each, with assertions written from the results verified against
//! the vim 9.1 (`-Nu NONE -N`) oracle -- the current engine satisfies none of them, so a red run is
//! the reproduction. The evidence annotations live in each test comment and in the commit report.

mod common;

use common::{edit, Fixture};

// ------------------------------------------------------- the range address of :pu

/// D1 (P1) the range of `:{range}pu` is dropped entirely: `:0pu` should paste at the **top of the
/// file** (above line 1) and `:5pu` below line 5; the engine always pastes beside the **cursor line**.
/// oracle (vim 9.1): five-line buffer, `:1y a` + `:0pu a` → `a|a|b|c|d|e`, cursor on line 1.
#[test]
fn put_range_address_moves_insertion_point() {
    // cursor parked mid-buffer: vim pastes at the top of the file, the engine (range dropped) would
    // paste beside the cursor line
    let mut f = edit("a\nb\nc\nd\ne\n", 2, 0, &[":", "1", "y", " ", "a", "<CR>"]);
    f.feed([":", "0", "p", "u", " ", "a", "<CR>"]);
    assert_eq!(f.text(), "a\na\nb\nc\nd\ne\n", ":0pu a must paste above line 1");
    assert_eq!(f.line(), 0, "the cursor should land on the newly pasted line 1");
}

/// D2 (P2) the cursor of `:put` should land on the first non-blank of the **last** pasted line
/// (`:h :put`); the engine reuses the ordinary `p` path and lands on the **first** pasted line.
/// oracle: after a three-line register `:$pu`, `line(".")=8` (lines 6-8 pasted, cursor on line 8).
#[test]
fn put_cursor_on_last_pasted_line() {
    let mut f = edit("one\ntwo\nthree\n", 2, 0, &[":", "1", ",", "3", "y", " ", "b", "<CR>"]);
    f.feed([":", "p", "u", " ", "b", "<CR>"]);
    // pasted below line 3: new lines 4/5/6, vim puts the cursor on line 6 (0-based 5)
    assert_eq!(f.line(), 5, ":pu with a multi-line register should leave the cursor on the last pasted line");
}

// ------------------------------------------------------- command-line editing keys

/// D3 (P1) `<Up>` on the command line should filter the history by the **prefix typed so far**
/// (vim 9.1 cmdline.txt c_<Up>: "recall older command-line from history, whose beginning matches
/// the current command-line"); the engine ignores the prefix and replays the newest entry, so a CR
/// executes a command vim would never have selected.
/// Probe design: history `:1d`/`:2d`/`:3d` (3d newest); once the buffer is down to `m2/m4`, type
/// `:2` + `<Up>` + CR: vim replays `:2d` and deletes m4; the engine replays `:3d`, which reports E16
/// on a two-line buffer and deletes nothing.
#[test]
fn cmdline_up_recalls_prefix_matching_history() {
    let mut f = edit(
        "m1\nm2\nm3\nm4\nm5\n",
        0,
        0,
        &[
            ":", "1", "d", "<CR>", ":", "2", "d", "<CR>", ":", "3", "d", "<CR>",
        ],
    );
    assert_eq!(f.text(), "m2\nm4\n", "the three setup :d commands agree on both sides");
    f.feed([":", "2", "<Up>", "<CR>"]);
    assert_eq!(f.text(), "m2\n", "with the prefix 2 it should replay :2d (deleting m4), not the newest :3d");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "it must not replay :3d and report E16 on a two-line buffer: {:?}",
        f.host.statuses
    );
}

/// D4 (P2) `:delm A-Z` deletes only the **uppercase** file marks; the engine runs both range ends
/// through `to_ascii_lowercase` and deletes a-z -- the lowercase mark `a` is erased by mistake
/// (data loss).
/// oracle: after `ma` + `:delm A-Z`, `:marks` still lists mark a.
#[test]
fn delmarks_uppercase_range_spares_lowercase() {
    let mut f = edit("a\nb\n", 0, 0, &["m", "a"]);
    f.feed([":", "d", "e", "l", "m", " ", "A", "-", "Z", "<CR>"]);
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E20")),
        "the lowercase mark a is outside what :delm A-Z deletes, so no E20: {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- :set options

/// D5 (P3) `:set inv{name}` is vim's boolean-inversion spelling (oracle: `:set invic` reports no
/// error and flips ic from its default off to on); the engine gives E518 "Unknown option: invic".
#[test]
fn set_inv_spelling_toggles_boolean() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "i", "n", "v", "i", "c", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        ":set invic must not report an error: {:?}",
        f.host.statuses
    );
}

/// D6 (P3) `:set {bool}&` resets a boolean option to its default (oracle: `:set ic&` reports no
/// error and ic goes back to its default off); the engine implements `&` only for numeric options and
/// gives E518 for the boolean form.
#[test]
fn set_ampersand_resets_boolean_option() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "i", "c", "&", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        ":set ic& must not report an error: {:?}",
        f.host.statuses
    );
}

/// D7 (P3) for `:set foo?` (the query form of an unknown name) vim reports
/// "E518: Unknown option: foo?" (oracle); the engine only rings the bell and emits no status message.
#[test]
fn set_unknown_name_query_reports_e518() {
    // (an earlier draft mistakenly fed an `m` first, which swallowed the `:` into the mark wait --
    // the engine already refused correctly; the probe itself removed that noise)
    let mut f = edit("a\n", 0, 0, &[]);
    f.feed([":", "s", "e", "t", " ", "f", "o", "o", "?", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E518: Unknown option: foo?"),
        "the query form of an unknown option must report E518: {:?}",
        f.host.statuses
    );
}

/// D8 (P3) the vim output of `:set ts?` carries **two leading spaces** ("  tabstop=8", verified with
/// the oracle through redir; a boolean on reads the same, "  ignorecase"); the engine emits a bare
/// "tabstop=4".
#[test]
fn set_query_output_two_space_padding() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s == "  tabstop=4"),
        "the query output should be vim's '  tabstop=4' (two leading spaces): {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- :le / :ce alignment

/// D9 (P1) the indent argument of `:le {indent}` is ignored completely (`let _ = indent;` in
/// ex_align of cmdline.rs): vim's `:le 4` on "ab" → "    ab" (oracle LE1), and `:1,2left 2` →
/// both lines get 2 spaces (oracle LE3); the engine only does the indent-stripping left alignment.
#[test]
fn left_align_honors_indent_argument() {
    let f = edit("ab\n", 0, 0, &[":", "l", "e", " ", "4", "<CR>"]);
    assert_eq!(f.text(), "    ab\n", ":le 4 should prefix 4 spaces");

    let f = edit("ab\ncd\n", 0, 0, &[":", "1", ",", "2", "l", "e", " ", "2", "<CR>"]);
    assert_eq!(f.text(), "  ab\n  cd\n", ":le 2 with a range adds 2 spaces to every line");
}

/// D10 (P1) under `noet` the padding of `:ce`/`:ri` follows 'tabstop': tabs first, then spaces
/// (oracle od bytes: `ab` + `:ce 40` with noet ts=8 → `\t\t   ab`, i.e. 2 tabs + 3 spaces; only with
/// et is it spaces alone); the engine always uses a plain `" ".repeat`.
#[test]
fn center_padding_composes_tabs_under_noet() {
    // vim used its default ts=8 when the oracle was sampled -- the engine defaults to ts=4 (a
    // deliberate NOTES divergence), so pin ts to 8 first and the oracle's byte expectation holds
    let mut f = edit("ab\n", 0, 0, &[":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>"]);
    f.feed([":", "s", "e", "t", " ", "n", "o", "e", "t", "<CR>"]);
    f.feed([":", "c", "e", " ", "4", "0", "<CR>"]);
    assert_eq!(
        f.text(), "\t\t   ab\n",
        "the 19-column padding of :ce 40 under noet = 2 tabs + 3 spaces"
    );
}

// ------------------------------------------------------- :retab

/// D11 (P3) `:retab 0` / `:retab! 0` are legal in vim (0 = reformat using the current 'tabstop';
/// oracle: on an 8-space line under noet, `:retab! 0` → `\tab` with no error); the engine reports
/// E488 Trailing characters for 0 and leaves the buffer alone.
#[test]
fn retab_zero_uses_current_tabstop() {
    // as above: pin the oracle's ts=8 premise first
    let mut f = edit("        ab\n", 0, 0, &[":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>"]);
    f.feed([":", "s", "e", "t", " ", "n", "o", "e", "t", "<CR>"]);
    f.feed([":", "r", "e", "t", "a", "b", "!", " ", "0", "<CR>"]);
    assert_eq!(f.text(), "\tab\n", ":retab! 0 should fold the 8 spaces into 1 tab at ts=8");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E488")),
        "retab 0 is legal and must not report E488: {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- the quit/list command family

/// D12 (P2) `:wqa` (together with spellings such as `:qa`/`:exit`/`:wqall`/`:xa`) saves and quits in
/// vim (oracle: the file on disk was rewritten); the engine reports E492 and the host never receives
/// a close request.
#[test]
fn quit_all_spellings_wqa_exit_exist() {
    let f = edit("a\n", 0, 0, &[":", "w", "q", "a", "<CR>"]);
    assert!(
        f.host.close_requested,
        ":wqa should request closing the buffer (vim saves and quits)"
    );
    assert!(
        f.host.statuses.is_empty(),
        ":wqa is not an unknown command: {:?}",
        f.host.statuses
    );

    let f = edit("a\n", 0, 0, &[":", "e", "x", "i", "t", "<CR>"]);
    assert!(f.host.close_requested, ":exit is a legal spelling of :x");
    assert!(
        f.host.statuses.is_empty(),
        ":exit is not an unknown command: {:?}",
        f.host.statuses
    );
}

/// D13 (P3) `:marks {name}` lists marks by name (oracle: `:marks a` prints the header plus one line
/// ` a      1    0 aa`); the engine only accepts a bare `:marks` and gives E492 with an argument.
#[test]
fn marks_accepts_name_arguments() {
    let mut f = edit("aa\n", 0, 0, &["m", "a"]);
    f.feed([":", "m", "a", "r", "k", "s", " ", "a", "<CR>"]);
    // vim 9.1 (oracle redir): the header plus one row rendered as ` %c %6ld %4d %s`
    //   mark line  col file/text
    //    a      1    0 aa
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "mark line  col file/text"),
        ":marks a prints the header first: {:?}",
        f.host.statuses
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with(" a") && s.contains("aa")),
        ":marks a should list the mark a: {:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        ":marks {{name}} must not give E492: {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- special command-line keys

/// D14 (P3) c_CTRL-V inserts the next key **literally** into the command line (oracle: `:` + C-v +
/// Esc + CR → the command line holds a literal ^[ and executing it reports "E492: Not an editor
/// command: ^["); the engine swallows C-v, Esc cancels the whole prompt, and the following CR lands
/// back in Normal mode.
#[test]
fn cmdline_ctrl_v_quotes_next_key() {
    let mut f = edit("a\n", 0, 0, &[]);
    f.feed([":", "<C-v>", "<Esc>", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E492")),
        "the Esc quoted by C-v should enter the command line as a literal ^[ and get executed (E492): {:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- range addresses

/// D15 (P2) the spaces between `+` and the digit inside an address offset: vim parses `:5 + 2` as 8
/// (oracle: the bare address puts the cursor on line 8; `:5 + 1d` deletes line 7; `:5 + 2d 2` deletes
/// lines 8-9 -- three probes agree); the engine treats `+` as a bare offset (+1) and truncates early,
/// so `:5 + 2d` deletes **line 6** -- off by two lines.
#[test]
fn address_offset_accepts_space_before_count() {
    let text = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
    let f = edit(text, 0, 0, &[":", "5", " ", "+", " ", "2", "d", "<CR>"]);
    assert_eq!(
        f.text(),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl9\nl10\n",
        "vim's :5 + 2d deletes line 8 (l8)"
    );
}

// ------------------------------------------------------- kept: Fixture stays referenced

/// Placeholder assertion (keeps the unused-import warning from drifting): the Fixture type is used
/// only indirectly through the return value of `edit(..)` in D14, so pin it explicitly once here.
#[test]
fn fixture_type_is_referenced() {
    let f = Fixture::new("x\n");
    assert_eq!(f.text(), "x\n");
}
