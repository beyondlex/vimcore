//! Scratch probes for audit round B — exploratory prints, deleted before delivery.

mod common;

use common::edit;

fn show(label: &str, text: String, cur: usize, extra: String) {
    println!("### {label}\n  text={text:?}\n  cursor={cur}\n  {extra}");
}

#[test]
fn scratch_probe() {
    // P1: linewise put count from a noeol register
    let f = edit("abc", 0, 0, &["y", "y", "3", "p"]);
    show("P1 yy3p noeol", f.text(), f.cursor(), format!("lines={:?}", f.text().split('\n').count()));

    // P1c: 2P
    let f = edit("abc", 0, 0, &["y", "y", "2", "P"]);
    show("P1c yy2P noeol", f.text(), f.cursor(), String::new());

    // P1d: dd then 2p
    let f = edit("abc", 0, 0, &["d", "d", "2", "p"]);
    show("P1d dd2p noeol", f.text(), f.cursor(), String::new());

    // C7: yy p on EMPTY buffer
    let f = edit("", 0, 0, &["y", "y", "p"]);
    show("C7 yy p empty", f.text(), f.cursor(), String::new());

    // J on last line: bell?
    let mut f = edit("ab\n", 0, 0, &["j"]);
    f.feed(["J"]);
    show("J last line", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // gP linewise cursor: yank line 1, P above line 2, check cursor line
    let f = edit("l1\nl2\nl3\n", 1, 0, &["y", "y", "k", "g", "P"]);
    show("gP linewise cursor", f.text(), f.cursor(), format!("line={}", f.line()));

    // >> on whitespace-only line
    let f = edit("   \nab\n", 0, 0, &[">", ">"]);
    show(">> whitespace-only", f.text(), f.cursor(), String::new());

    // d$ on empty line: bell?
    let mut f = edit("\nab\n", 0, 0, &["d", "$"]);
    show("d$ empty line", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // X with count beyond line start: bell?
    let mut f = edit("ab\n", 0, 1, &["5", "X"]);
    show("5X col1", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // J on 3 lines with middle empty
    let f = edit("a\n\nb\nc\n", 0, 0, &["3", "J"]);
    show("3J a,'',b", f.text(), f.cursor(), String::new());

    // r on CJK with ascii replacement
    let f = edit("中文\n", 0, 0, &["r", "-"]);
    show("r- on CJK", f.text(), f.cursor(), String::new());

    // 3r- crossing EOL: bell, no change
    let mut f = edit("ab\n", 0, 0, &["3", "r", "-"]);
    show("3r- short line", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // ~ count beyond EOL
    let f = edit("ab\n", 0, 0, &["5", "~"]);
    show("5~ short line", f.text(), f.cursor(), String::new());

    // cc on empty buffer
    let mut f = edit("", 0, 0, &["c", "c"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    show("cc empty buffer", f.text(), f.cursor(), String::new());

    // "1p .. "3p ring after 3 dd
    let f = edit("l1\nl2\nl3\nl4\n", 0, 0, &["d", "d", "d", "d", "d", "d", "\"", "1", "p"]);
    show("dd*3 \"1p", f.text(), f.cursor(), String::new());
    let f = edit("l1\nl2\nl3\nl4\n", 0, 0, &["d", "d", "d", "d", "d", "d", "\"", "3", "p"]);
    show("dd*3 \"3p", f.text(), f.cursor(), String::new());

    // gp charwise at line end
    let f = edit("ab\n", 0, 1, &["x", "g", "p"]);
    show("x gp line-end", f.text(), f.cursor(), String::new());

    // gp linewise cursor: last pasted line
    let f = edit("l1\nl2\nl3\n", 0, 0, &["y", "y", "2", "g", "p"]);
    show("yy 2gp linewise", f.text(), f.cursor(), format!("line={}", f.line()));

    // 2gp charwise count
    let f = edit("ab cd\n", 0, 0, &["y", "w", "3", "g", "p"]);
    show("yw 3gp", f.text(), f.cursor(), String::new());

    // x on empty buffer bell (known good) / d$ on empty line
    // guw on CJK
    let f = edit("中文 abc\n", 0, 0, &["g", "u", "w"]);
    show("guw CJK", f.text(), f.cursor(), String::new());

    // gUU on line with symbols
    let f = edit("a-b! c\n", 0, 0, &["g", "U", "U"]);
    show("gUU symbols", f.text(), f.cursor(), String::new());

    // "~
    let f = edit("aBc1-\n", 0, 0, &["g", "~", "~"]);
    show("g~~ mixed", f.text(), f.cursor(), String::new());

    // 3>> count shift
    let f = edit("a\nb\nc\n", 0, 0, &["3", ">", ">"]);
    show("3>>", f.text(), f.cursor(), String::new());

    // << at col 0 with under-indent
    let mut f = edit("  ab\n", 0, 0, &["<", "<"]);
    show("<< 2sp", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // p with linewise count on MIDDLE line (control)
    let f = edit("l1\nl2\nl3\n", 0, 0, &["j", "y", "y", "k", "3", "p"]);
    show("mid yy 3p", f.text(), f.cursor(), String::new());

    // visual v y zero-width on empty buffer, then p
    let mut f = edit("", 0, 0, &["v", "y", "p"]);
    show("v y p empty", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // "ayy then "Ayy then "ap
    let f = edit("l1\nl2\n", 0, 0, &["\"", "a", "y", "y", "j", "\"", "A", "y", "y", "\"", "a", "p"]);
    show("A-append", f.text(), f.cursor(), String::new());

    // "Add charwise to linewise register
    let f = edit("l1\nl2\nl3\n", 0, 0, &["\"", "a", "y", "y", "j", "l", "\"", "A", "d", "l", "\"", "a", "p"]);
    show("A-append charwise to linewise", f.text(), f.cursor(), String::new());

    // "0 after yy then dd then "0p
    let f = edit("l1\nl2\nl3\n", 0, 0, &["y", "y", "j", "d", "d", "\"", "0", "p"]);
    show("yy dd \"0p", f.text(), f.cursor(), String::new());

    // "- after charwise then dd: "- still charwise delete
    let f = edit("abc\ndef\n", 0, 0, &["x", "d", "d", "\"", "-", "p"]);
    show("x dd \"-p", f.text(), f.cursor(), String::new());

    // 3p charwise count at buffer start
    let f = edit("ab\ncd\n", 0, 0, &["y", "l", "3", "p"]);
    show("yl 3p", f.text(), f.cursor(), String::new());

    // 2J on ["a", "b", "c"]: joins 2
    let f = edit("a\nb\nc\n", 0, 0, &["2", "J"]);
    show("2J", f.text(), f.cursor(), String::new());

    // 4J short buffer
    let mut f = edit("a\nb\n", 0, 0, &["9", "J"]);
    show("9J short", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // D count register shape count=1: "-?
    let f = edit("abc\ndef\n", 0, 1, &["D", "\"", "-", "p"]);
    show("D then \"-p", f.text(), f.cursor(), String::new());

    // x count > line: register content = whole rest?
    let f = edit("abc\n", 0, 1, &["9", "x", "\"", "-", "p"]);
    show("9x then \"-p", f.text(), f.cursor(), String::new());

    // r<CR> register: "-?
    let f = edit("abcdef\n", 0, 1, &["r", "<CR>", "\"", "-", "p"]);
    show("r<CR> \"-p", f.text(), f.cursor(), String::new());

    // J bell count
    let mut f = edit("ab\n", 0, 0, &["3", "J"]);
    show("3J last line", f.text(), f.cursor(), format!("bells={}", f.host.bells));

    // S on last line count clamp
    let mut f = edit("a\nb\n", 1, 0, &["9", "S"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    show("9S clamp", f.text(), f.cursor(), String::new());

    // yank on whitespace-only line then p
    let f = edit("   \nab\n", 0, 0, &["y", "y", "j", "p"]);
    show("yy ws-line p", f.text(), f.cursor(), String::new());
}
