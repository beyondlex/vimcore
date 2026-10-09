//! audit6_main：主会话自有赛道——jumplist / changelist / `.` 重复 语义探针
//! （第五轮独立审计，2026-10-09）。
//!
//! 与三个分域智能体互补的死角：跳转列表成员资格、changelist 行走、
//! `.` 重复的计数/可视形态。所有断言写 oracle 期望值；oracle 通道：
//! changelist/jumplist 类必须 expect PTY（typeahead 通道对连续 g; 有
//! 假象——本轮实测 -s 下重复 g; 不行走而 PTY 行走，见 pty2/pty3）。
//!
//! 目前全部 #[ignore]：修复转绿后逐个摘除。

mod common;

use common::{edit, Fixture};

fn buf30() -> String {
    (1..=30).map(|i| format!("l{i:02}")).collect::<Vec<_>>().join("\n") + "\n"
}

#[test]
fn e_m1_dot_does_not_repeat_ex_command() {
    // oracle（repeat.txt:24-27 + typeahead 复核）: `.` 不重复 Ex 命令。
    // "aa bb cc" ×2，:s/b/X/ 后 `.`——第二行不变（round-30 旧钉复核仍成立）。
    let f = edit("aa bb cc\naa bb cc\n", 0, 0, &["e", "x", "e", "c", "u", "t", "e", " ", "s", "/", "b", "/", "X", "/", "<CR>"]);
    // 直接走 Ex 行
    let mut f = Fixture::new("aa bb cc\naa bb cc\n");
    f.feed([":", "s", "/", "b", "/", "X", "/", "<CR>", "."]);
    assert_eq!(f.text(), "aa Xb cc\naa bb cc\n");
}

#[test]
fn e_m2_gsemi_walks_older_per_press() {
    // oracle（PTY pty2/pty3）: x@5G, x@10G, gg 后 g; → 行10；再 g; → 行5；
    // 再 g; → 停在行5（E662）。引擎行号 0 基：4G=idx3，10G=idx9。
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
    // oracle（PTY 矩阵）: g, 从旧往新走；2g; 一步跨两级。
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
    // movechangelist(-2)：past-end idx 3 → entry[1] = 中间改动（PTY4 实证）。
    assert_eq!(g.line(), 8, "2g; jumps two entries back to the middle change");
}

#[test]
fn e_m4_go_gd_gdstar_are_jumps() {
    // oracle（typeahead J1 + star/gD/gd 矩阵）: `go`、`gd`、`gD`、`*` 全部
    // 入 jumplist——C-o 回到各自跳转的源头；`:5` 不入（e_m5）。
    let text = buf30();
    // 15G(idx14) 20G(idx19) go(idx0) <C-o> → 回 go 源头 idx19。
    let mut f = Fixture::new(&text);
    f.feed(["1", "5", "G", "2", "0", "G", "g", "o", "<C-o>"]);
    assert_eq!(f.line(), 19, "go IS a jump: C-o returns to go's origin");
    // gd: 15G/20G 钳到末行 idx4（more x here），6| 落 'x'，gd → 声明行 idx0；
    // C-o → gd 源头 idx4。
    let text2 = "int x;\nfiller\nuse of x here\nrest\nmore x here\n";
    let mut g = Fixture::new(text2);
    g.feed(["1", "5", "G"]); // 钳到末行 idx 4
    g.feed(["6", "|"]); // 落在 'x'
    g.feed(["g", "d"]); // → idx 0
    assert_eq!(g.line(), 0, "gd lands on declaration");
    g.feed(["<C-o>"]);
    assert_eq!(g.line(), 4, "gd IS a jump: C-o returns to gd's origin (clamped last line)");
}

#[test]
fn e_m5_ex_goto_line_not_a_jump() {
    // oracle（typeahead J4）: :5 不入 jumplist——'' 回 20G 源头（行1）。
    let text = buf30();
    let mut f = Fixture::new(&text);
    f.feed(["2", "0", "G"]);
    f.feed([":", "5", "<CR>"]);
    f.feed(["'", "'"]);
    assert_eq!(f.line(), 0, "'' returns to 20G origin, :5 pushed nothing");
}

#[test]
fn e_m6_jumpcount_and_forward() {
    // oracle（typeahead J5/J7）: 2<C-o> 跨两级；C-i 前行；到新端不动。
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
    // oracle（typeahead T1-T5 矩阵）: gd 的锚是 [[ 函数起点（无则行1，无
    // 空行回退；有则从 { 行向上越过非空行），从锚点向前搜全词第一出现。
    // T1: brace 上方全非空 → 锚退到行1 → 落行1 的 int x;
    let t1 = "int x;\none\n{\nint x;\nfoo x mid\n}\nbar x end\n";
    let f = edit(t1, 6, 4, &["g", "d"]);
    assert_eq!(f.line(), 0, "T1: anchor walks back to line 1, first match wins");
    // T2: 空行截断回退 → 锚 = { 行 → 落 { 后首个 foo x
    let t2 = "int x;\n\n{\nfoo x\n";
    let g = edit(t2, 3, 4, &["g", "d"]);
    assert_eq!(g.line(), 3, "T2: blank line stops the backoff at the brace line");
    // T3: { 紧跟声明行 → 锚退到行1 → 落 int x;
    let t3 = "int x;\n{\nfoo x\n";
    let h = edit(t3, 2, 4, &["g", "d"]);
    assert_eq!(h.line(), 0, "T3: brace with non-blank above anchors line 1");
}

#[test]
fn e_m7_dot_count_replaces() {
    // oracle（typeahead 待补）: 3x 后 2. —— 新计数替换旧计数（repeat.txt:30）。
    let mut f = Fixture::new("abcdef\n");
    f.feed(["3", "x"]); // "def"
    f.feed(["2", "."]); // 删 2 个 → "f"
    assert_eq!(f.text(), "f\n");
}

#[test]
fn e_m8_dot_repeats_paste_and_survives_undo() {
    // oracle: `.` 重复 p（normal-mode change）；u 不是 change，. 后仍重复 x。
    let mut f = Fixture::new("word\n\ntarget\n");
    f.feed(["y", "i", "w", "j", "p", "."]);
    assert_eq!(f.text(), "word\nwordword\ntarget\n", "dot repeats paste twice total? oracle: 2nd p on same line");
    let mut g = Fixture::new("abcdef\n");
    g.feed(["x", "u", "."]);
    assert_eq!(g.text(), "bcdef\n", "u not a change; dot repeats the x");
}
