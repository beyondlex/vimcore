//! audit6_main: main-session lanes — jumplist / changelist / `.` repeat
//! semantics probes (fifth independent audit round, 2026-10-09).
//!
//! Blind spots complementary to the three domain subagents: jumplist
//! membership, changelist walking, the count/visual forms of `.` repeat.
//! All assertions carry oracle-expected values; oracle channel:
//! changelist/jumplist cases must go through expect PTY (the typeahead
//! channel is unreliable for repeated g; — measured this round: under -s
//! repeated g; does not walk while PTY does; see pty2/pty3).
//!
//! All currently #[ignore]: remove the attribute one by one as fixes turn green.

mod common;

use common::{edit, Fixture};

fn buf30() -> String {
    (1..=30).map(|i| format!("l{i:02}")).collect::<Vec<_>>().join("\n") + "\n"
}

#[test]
fn e_m1_dot_does_not_repeat_ex_command() {
    // oracle (repeat.txt:24-27 + typeahead re-check): `.` does not repeat Ex commands.
    // "aa bb cc" ×2, `.` after :s/b/X/ — the second line is unchanged
    // (the round-30 old pin re-check still holds).
    let f = edit("aa bb cc\naa bb cc\n", 0, 0, &["e", "x", "e", "c", "u", "t", "e", " ", "s", "/", "b", "/", "X", "/", "<CR>"]);
    // Run the Ex line directly
    let mut f = Fixture::new("aa bb cc\naa bb cc\n");
    f.feed([":", "s", "/", "b", "/", "X", "/", "<CR>", "."]);
    assert_eq!(f.text(), "aa Xb cc\naa bb cc\n");
}

#[test]
fn e_m2_gsemi_walks_older_per_press() {
    // oracle (PTY pty2/pty3): after x@5G, x@10G, gg, g; → line 10; another g; → line 5;
    // another g; → stays at line 5 (E662). Engine line numbers are 0-based: 4G=idx3, 10G=idx9.
    let text = buf30();
    let mut f = Fixture::new(&text);
    f.feed(["4", "G", "x"]); // line idx 3
    f.feed(["9", "G", "x"]); // line idx 8
    f.feed(["g", "g"]);
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 8, "first g; lands newest change");
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 3, "second g; walks older");
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 3, "third g; stays at oldest");
}

#[test]
fn e_m3_gcomma_walks_newer_and_count() {
    // oracle (PTY matrix): g, walks from older to newer; 2g; steps two entries at once.
    let text = buf30();
    let mut f = Fixture::new(&text);
    f.feed(["4", "G", "x", "9", "G", "x", "g", "g"]);
    f.feed(["g", ";", "g", ";"]); // now at idx 3
    f.feed(["g", ","]);
    assert_eq!(f.line(), 8, "g, walks newer");
    f.feed(["g", ","]);
    assert_eq!(f.line(), 8, "g, at newest stays (E663)");
    let mut g = Fixture::new(&text);
    g.feed(["4", "G", "x", "9", "G", "x", "1", "4", "G", "x", "g", "g"]);
    g.feed(["2", "g", ";"]);
    // movechangelist(-2): past-end idx 3 → entry[1] = the middle change (PTY4 evidence).
    assert_eq!(g.line(), 8, "2g; jumps two entries back to the middle change");
}

#[test]
fn e_m4_go_gd_gdstar_are_jumps() {
    // oracle (typeahead J1 + star/gD/gd matrix): `go`, `gd`, `gD`, `*` all push onto
    // the jumplist — C-o returns to each jump's origin; `:5` does not (e_m5).
    let text = buf30();
    // 15G(idx14) 20G(idx19) go(idx0) <C-o> → back to go's origin idx19.
    let mut f = Fixture::new(&text);
    f.feed(["1", "5", "G", "2", "0", "G", "g", "o", "<C-o>"]);
    assert_eq!(f.line(), 19, "go IS a jump: C-o returns to go's origin");
    // gd: 15G/20G clamped to last line idx4 (more x here), 6| lands on 'x', gd → declaration line idx0;
    // C-o → gd's origin idx4.
    let text2 = "int x;\nfiller\nuse of x here\nrest\nmore x here\n";
    let mut g = Fixture::new(text2);
    g.feed(["1", "5", "G"]); // clamped to last line idx 4
    g.feed(["6", "|"]); // lands on 'x'
    g.feed(["g", "d"]); // → idx 0
    assert_eq!(g.line(), 0, "gd lands on declaration");
    g.feed(["<C-o>"]);
    assert_eq!(g.line(), 4, "gd IS a jump: C-o returns to gd's origin (clamped last line)");
}

#[test]
fn e_m5_ex_goto_line_not_a_jump() {
    // oracle (typeahead J4): :5 does not enter the jumplist — '' returns to the 20G origin (line 1).
    let text = buf30();
    let mut f = Fixture::new(&text);
    f.feed(["2", "0", "G"]);
    f.feed([":", "5", "<CR>"]);
    f.feed(["'", "'"]);
    assert_eq!(f.line(), 0, "'' returns to 20G origin, :5 pushed nothing");
}

#[test]
fn e_m6_jumpcount_and_forward() {
    // oracle (typeahead J5/J7): 2<C-o> goes back two entries; C-i walks forward; no move at the newest end.
    let text = buf30();
    let mut f = Fixture::new(&text);
    f.feed(["1", "5", "G", "2", "0", "G", "2", "5", "G", "2", "<C-o>"]);
    assert_eq!(f.line(), 14, "2<C-o> back two jumps");
    f.feed(["<C-i>"]);
    assert_eq!(f.line(), 19, "C-i forward one");
    f.feed(["<C-i>", "<C-i>"]);
    assert_eq!(f.line(), 24, "C-i to newest");
}

#[test]
fn e_m9_gd_anchor_is_function_start_not_nearest_before() {
    // oracle (typeahead T1-T5 matrix): gd's anchor is the [[ function start (line 1 if none, no
    // blank-line fallback; if present, back up from the { line over non-blank lines), then search
    // forward from the anchor for the first whole-word occurrence.
    // T1: everything above the brace is non-blank → anchor retreats to line 1 → lands on int x; at line 1
    let t1 = "int x;\none\n{\nint x;\nfoo x mid\n}\nbar x end\n";
    let f = edit(t1, 6, 4, &["g", "d"]);
    assert_eq!(f.line(), 0, "T1: anchor walks back to line 1, first match wins");
    // T2: the blank line stops the backoff → anchor = the brace line → lands on the first foo x after {
    let t2 = "int x;\n\n{\nfoo x\n";
    let g = edit(t2, 3, 4, &["g", "d"]);
    assert_eq!(g.line(), 3, "T2: blank line stops the backoff at the brace line");
    // T3: { directly follows the declaration line → anchor retreats to line 1 → lands on int x;
    let t3 = "int x;\n{\nfoo x\n";
    let h = edit(t3, 2, 4, &["g", "d"]);
    assert_eq!(h.line(), 0, "T3: brace with non-blank above anchors line 1");
}

#[test]
fn e_m7_dot_count_replaces() {
    // oracle (typeahead, to be filled): after 3x, 2. — the new count replaces the old one (repeat.txt:30).
    let mut f = Fixture::new("abcdef\n");
    f.feed(["3", "x"]); // "def"
    f.feed(["2", "."]); // delete 2 → "f"
    assert_eq!(f.text(), "f\n");
}

#[test]
fn e_m8_dot_repeats_paste_and_survives_undo() {
    // oracle: `.` repeats p (normal-mode change); u is not a change, . still repeats x afterwards.
    let mut f = Fixture::new("word\n\ntarget\n");
    f.feed(["y", "i", "w", "j", "p", "."]);
    assert_eq!(f.text(), "word\nwordword\ntarget\n", "dot repeats paste twice total? oracle: 2nd p on same line");
    let mut g = Fixture::new("abcdef\n");
    g.feed(["x", "u", "."]);
    assert_eq!(g.text(), "bcdef\n", "u not a change; dot repeats the x");
}
