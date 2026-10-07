//! 第四轮独立审计（搜索 / 显示列 / 敌意宿主数据域）。
//!
//! 每个 `#[test]` 对应一个新发现；断言写的是 vim 9.1（`-Nu NONE -N -s`
//! typeahead oracle）实证过的期望值，因此在当前引擎上**必须失败**。
//! 各测试开头注释给出：严重度、复现、vim 期望、引擎现状。

mod common;

use common::Fixture;

// ---------------------------------------------------------------------------
// F1（P2）`gd`/`gD` 未绑定，且 `gd` 静默武装 delete 算子
//
// 复现：buffer "int x;\nuse x here\n"，光标 (line 1, col 4)（"use x" 的 x），
//       按键 g d x。
// vim 9.1（oracle：gd 落 (1,5) 即 "int x;" 的 x；随后 x 删它）：文本
//       "int ;\nuse x here\n"，光标 offset 4。
// 引擎：`gd` 走 trie miss → 丢弃 g、重喂 d → Delete 算子被静默武装；随后的
//       x 是非法 motion，响铃放弃。文本原样、光标原地（11）。
#[test]
fn f1_gd_unbound_arms_delete_operator() {
    let mut f = Fixture::at("int x;\nuse x here\n", 1, 4);
    f.feed(["g", "d", "x"]);
    assert_eq!(f.cursor(), 4, "gd must jump to the declaration (line0 col4)");
    assert_eq!(f.text(), "int ;\nuse x here\n", "x then deletes the declared 'x'");
}

// ---------------------------------------------------------------------------
// F2（P2）`g*`/`g#` 缺失，退化为整词 `*`/`#`（外加一声多余响铃）
//
// 复现 A：buffer "foo bar\nfoofoo\nfoo\n"，(0,0)，g *。
//   vim：g* = 无 \<\> 边界的子串搜索 → 落 line2 col1（foofoo 开头，offset 8）。
//   引擎：trie miss 响铃后把 `*` 按整词搜索重放 → 落 line3 col0（offset 15）。
// 复现 B：buffer "foofoo\nfoo bar\n"，(1,0)，g #。
//   vim：子串向前（上）找 → line1 内第二个 "foo"，offset 3。
//   引擎：整词 `\bfoo\b` 向上无果 → 回绕停在自身（offset 7）。
#[test]
fn f2_gstar_ghash_missing_degrade_to_whole_word() {
    let mut f = Fixture::at("foo bar\nfoofoo\nfoo\n", 0, 0);
    f.feed(["g", "*"]);
    assert_eq!(f.cursor(), 8, "g* searches the substring 'foo' (no word bounds)");
    assert_eq!(f.host.bells, 0, "g* is a real command, not a trie miss");

    let mut g = Fixture::at("foofoo\nfoo bar\n", 1, 0);
    g.feed(["g", "#"]);
    assert_eq!(g.cursor(), 3, "g# finds the earlier in-word 'foo'");
}

// ---------------------------------------------------------------------------
// F3（P2）`n`/`N` 不重放 search offset（且 `/pat/e` 首跳锚错匹配）
//
// 复现：buffer "foo x\nfoo y\n"，(0,0)：/ f o o / e <CR> → n → n。
// vim 9.1（oracle 逐步采点）：首跳落 (1,3)=offset 2（光标所在匹配的词尾），
//   第一个 n 落 (2,3)=offset 8，第二个 n 回绕落 (1,3)=offset 2 —— 每次 n
//   都重放 `e` 锚点（:h search-offset）。
// 引擎：首跳跳过光标处匹配落 offset 8；之后每个 n 只落匹配**起点**
//   （0），offset 完全丢失。search.offset 的文档注释自称 "reapplied by
//   n/N like vim"，motions.rs 的 SearchNext 臂从未调用 apply_search_offset。
#[test]
fn f3_n_does_not_reapply_search_offset() {
    let mut f = Fixture::at("foo x\nfoo y\n", 0, 0);
    f.feed(["/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(f.cursor(), 2, "first /foo/e anchors the cursor match's end");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 8, "n re-applies the e offset to the next match");
    f.feed(["n"]);
    assert_eq!(f.cursor(), 2, "wrapping n re-applies the e offset again");
}

// ---------------------------------------------------------------------------
// F4（P2）算子/可视搜索提示完全丢弃 search offset（d/v + /pat/e）
//
// 复现 A：buffer "x foo y\n"，(0,0)：d / f o o / e <CR>。
//   vim（oracle）：删到匹配词尾 → " y"。
//   引擎：cmdline.rs 只在 motion==None 时 apply_search_offset → 算子跨度
//   停在匹配起点 → "foo y\n"（少删 "foo"）。
// 复现 B：同 buffer：v / f o o / e <CR>。
//   vim：选区延伸到匹配词尾（光标 offset 4）。
//   引擎：search_motion_visual 跳匹配起点（光标 offset 2）。
#[test]
fn f4_operator_visual_search_ignore_offsets() {
    let mut f = Fixture::at("x foo y\n", 0, 0);
    f.feed(["d", "/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(f.text(), " y\n", "d/pat/e deletes through the match end");

    let mut v = Fixture::at("x foo y\n", 0, 0);
    v.feed(["v", "/", "f", "o", "o", "/", "e", "<CR>"]);
    assert_eq!(v.cursor(), 4, "v/pat/e extends the selection to the match end");
}

// ---------------------------------------------------------------------------
// F5（P1）带数字的 s/b/e offset 尾巴把数字当「行位移」；vim 是「字符位移」
//
// 复现：buffer "aaaaaaaaaa\nbbbbbbbbbb\nccc foo\ndddddddddd\neeeeeeeeee\n"，
//       (0,0)，行首偏移 L1=0 L2=11 L3=22 L4=30 L5=41，匹配 "foo" 在 (3,5..8)。
// vim 9.1（oracle 逐一实证）：s/b/e 后的 [±N] 是**字符**位移——
//   /foo/b2  → (3,7)  = offset 28（起点 +2 字符）
//   /foo/e2  → (4,2)  = offset 31（词尾 +2 字符，跨行）
//   /foo/s-2 → (3,3)  = offset 24（起点 -2 字符）
// 引擎：parse_search_offset 把 N 当 line_shift、列固定在锚点 ——
//   b2 → 45、e2 → 47、s-2 → 4，落点整行错开。
#[test]
fn f5_offset_sbe_count_is_chars_not_lines() {
    let buf = "aaaaaaaaaa\nbbbbbbbbbb\nccc foo\ndddddddddd\neeeeeeeeee\n";

    let mut f = Fixture::at(buf, 0, 0);
    f.feed(["/", "f", "o", "o", "/", "b", "2", "<CR>"]);
    assert_eq!(f.cursor(), 28, "/foo/b2 = start + 2 chars (line3 col7)");

    let mut g = Fixture::at(buf, 0, 0);
    g.feed(["/", "f", "o", "o", "/", "e", "2", "<CR>"]);
    assert_eq!(g.cursor(), 31, "/foo/e2 = end + 2 chars (line4 col2)");

    let mut h = Fixture::at(buf, 0, 0);
    h.feed(["/", "f", "o", "o", "/", "s", "-", "2", "<CR>"]);
    assert_eq!(h.cursor(), 24, "/foo/s-2 = start - 2 chars (line3 col3)");
}

// ---------------------------------------------------------------------------
// F6（P2）纯行位移 offset（[N]/+N/-N）保留匹配列；vim 落第 1 列
//
// 复现：buffer "x foo y\n    ind\nzzz\n"，(0,0)：/ f o o / + 1 <CR>。
// vim 9.1（oracle）：匹配行 1 +1 行 → line2 **col 1**（"    ind" 的第一个
//   空格；列 1，非首非空白）→ offset 8。
// 引擎：apply_search_offset 保留匹配起点列（display col 2）→ offset 10（'i'）。
#[test]
fn f6_line_offset_lands_column1() {
    let mut f = Fixture::at("x foo y\n    ind\nzzz\n", 0, 0);
    f.feed(["/", "f", "o", "o", "/", "+", "1", "<CR>"]);
    assert_eq!(f.cursor(), 8, "/pat/+1 lands in column 1 of the shifted line");
}

// ---------------------------------------------------------------------------
// F7（P1）显示列模型把 TAB 记 1 格；vim 按 tabstop 展开（`|`、j/k、C-d/u、
//         search-offset 列保持全受累）
//
// 复现：buffer "a\tb\nefg\n"，(0,0)：4 | x。
// vim 9.1（oracle）：tab 覆盖虚拟列 2..9；`4|` 落在 tab 上，`x` 删除整个
//   tab → "ab\nefg\n"，光标 offset 1（'b'）。
// 引擎：char_display_width('\t')=1 → `4|` 钳到行尾字符 'b'，x 删掉的是
//   'b' → "a\t\nefg\n"（删错字符、tab 留存）。
#[test]
fn f7_tab_counts_one_cell_in_column_math() {
    let mut f = Fixture::at("a\tb\nefg\n", 0, 0);
    f.feed(["4", "|", "x"]);
    assert_eq!(f.text(), "ab\nefg\n", "4| parks on the tab; x deletes the whole tab");
    assert_eq!(f.cursor(), 1, "cursor rests on 'b' after deleting the tab");
}

// ---------------------------------------------------------------------------
// F8（P3）`?` 方向的 incsearch 预览把「向前最近」匹配标成 current
//
// 复现：buffer "foo\nbar\nfoo bar\n"，光标 (2,5)（offset 13）：? o（不回车）。
// vim：incsearch 预览标出回车后会跳到的匹配 —— 向后最近 = line3 第二个
//   'o'（offset 10..11；oracle 由 `?o<CR>` 落点 (3,3) 反证）。
// 引擎：publish_incsearch 不看方向，恒取 cursor 之后第一个匹配并回绕 →
//   current_highlight = 1..2（line1 的 o）。
#[test]
fn f8_incsearch_backwards_preview_marks_forward_match() {
    let mut f = Fixture::at("foo\nbar\nfoo bar\n", 2, 5);
    f.feed(["?", "o"]);
    assert_eq!(
        f.host.current_highlight,
        Some(10..11),
        "?-incsearch preview marks the match Enter would land on"
    );
}

// ---------------------------------------------------------------------------
// F9（P2）Ex 范围地址不接受 offset 尾巴（:/foo/+1）
//
// 复现：buffer "foo\nbar\nbaz foo\n"，(0,0)：: / f o o / + 1 <CR>。
// vim 9.1（oracle）：地址 = 光标行后第一个 foo 匹配行（line3）+1 → 钳到
//   line3 col1 → offset 8，无任何报错。
// 引擎：parse_range 的 /pat/ 分支只剥结尾终结符，"foo/+1" 被当模式 →
//   E16: Invalid range，光标不动（offset 0）。
#[test]
fn f9_ex_range_address_rejects_offset_tail() {
    let mut f = Fixture::at("foo\nbar\nbaz foo\n", 0, 0);
    f.feed([":", "/", "f", "o", "o", "/", "+", "1", "<CR>"]);
    assert_eq!(f.cursor(), 8, ":/foo/+1 moves to the offset address line");
    assert!(
        f.host.statuses.is_empty(),
        "address movement is silent; engine says {:?}",
        f.host.statuses
    );
}

// ---------------------------------------------------------------------------
// F10（P1）linewise put 进被删空的缓冲丢文件末换行 / P 形丢一行
//
// 复现 A：buffer "abc\n"，(0,0)：d d p。
//   vim（oracle）：行 = ["", "abc"] 且保留末 EOL → 文本 "\nabc\n"。
//   引擎：寄存器尾部的 \n 被当作行分隔符消耗 → "\nabc"（末 EOL 丢失，
//   写盘即文件截短一字节）。
// 复现 B：buffer "abc\n"，(0,0)：d d P。
//   vim（oracle）：行 = ["abc", ""] → 文本 "abc\n\n"。
//   引擎：插出 "abc\n"（1 行，比 vim 少一个空行）。
#[test]
fn f10_linewise_put_into_emptied_buffer_loses_final_newline() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["d", "d", "p"]);
    assert_eq!(f.text(), "\nabc\n", "ddp on a single-line buffer keeps the EOL");

    let mut g = Fixture::at("abc\n", 0, 0);
    g.feed(["d", "d", "P"]);
    assert_eq!(g.text(), "abc\n\n", "ddP keeps the emptied line below");
}

// ---------------------------------------------------------------------------
// F11（P2）`[count]<C-f>`（同族 C-b/C-d/C-u）忽略 count
//
// 复现：buffer 200 行 "line000\n".."line199\n"（每行 8 字节），宿主视口
//       (0,9)（等价 vim `:set lines=12` 的 10 行窗口），(0,0)：2 <C-f>。
// vim 9.1（oracle，lines=12）：1<C-f> 落 line idx 9；**2<C-f> 落 idx 18**
//   （w0=19，每页乘上 count）→ offset 18*8 = 144。
// 引擎：motions.rs PageDown 的 step 恒为 visible，count 不参与 → idx 9
//   （offset 72），与 1<C-f> 无异。
#[test]
fn f11_page_scroll_ignores_count() {
    let mut buf = String::new();
    for i in 0..200 {
        buf.push_str(&format!("line{i:03}\n"));
    }
    let mut f = Fixture::at(&buf, 0, 0);
    f.host.viewport = (0, 9);
    f.feed(["2", "<C-f>"]);
    assert_eq!(f.cursor(), 18 * 8, "2<C-f> scrolls two pages (line idx 18)");
}
