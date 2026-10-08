//! 第四轮审计伴随探针（fuzz round35 抓取的 bug 回归，2026-10-09）。
//!
//! 这些断言写的是 vim 9.1 oracle 的期望结果，修复前应当失败。
//! oracle 通道：`vim -Nu NONE -N -i NONE -n -s/-es`（2026-10-09 采样）。

mod common;

use common::Fixture;

// ---- R35-1（P0）：`vl<Esc>:marks` 于 CJK 缓冲 panic ------------------------
// `list_marks` 渲染 `>` 行用 `hi.saturating_sub(1)` 裸减 1 字节，能落进
// 多字节字符内部，宿主 `offset_to_line` 切片 panic。
//
// oracle：`中文缓冲` 上 `vll<Esc>` 后 `:marks` 正常输出 `>` 行
// `>      1    6 中文缓冲`（fuzz round35 只抓 panic 面；col 语义见 R35-2）。
#[test]
fn r35_1_marks_gt_row_on_cjk_selection_does_not_panic() {
    let mut f = Fixture::new("中文缓冲\n");
    f.feed(["v", "l", "l", "<Esc>"]);
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("marks".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    // `>` 行落在选区末字符（缓）上，col = 0 基字节列 6
    let gt = f
        .host
        .statuses
        .iter()
        .find(|s| s.starts_with('>') || s.starts_with(" >"))
        .or_else(|| {
            f.host
                .statuses
                .iter()
                .find(|s| s.contains("line 1") && s.contains("缓冲"))
        })
        .cloned();
    let gt = gt.unwrap_or_else(|| format!("no > row in {:?}", f.host.statuses));
    assert!(
        gt.contains("col 6"),
        "oracle: `>` 行 col 6（选区末字符「缓」的 0 基字节列）：{gt:?}"
    );
}

// ---- R35-2（P2）：`:marks` 的 col 列是 0 基字节列，不是显示列+1 ------------
//
// oracle（-es 通道，2026-10-09）：`ab\tcd` 上 `fc` `md` 后 `:marks d` →
// `d      1    3 ab^Icd`。d 的 0 基**字节**列是 3（显示列是 9，ts=8）；
// 行内容原样列出（不 trim 缩进）、TAB 转 `^I`。
#[test]
fn r35_2_marks_col_is_zero_based_byte_column() {
    let mut f = Fixture::new("ab\tcd\nsecond\n");
    f.feed(["f", "c", "m", "d"]);
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("marks d".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(
        f.host.statuses.last().map(String::as_str),
        Some("d  line 1  col 3  ab^Icd"),
        "oracle: col 是 0 基字节列 3，行内容不 trim、TAB 转 ^I：{:?}",
        f.host.statuses
    );
}

// ---- R35-3（P1）：`:retab` 后光标留在原字符上 ------------------------------
//
// 旧行为：整段 `edit_replace` 后光标保持原始字节偏移——空白 run 变长后
// 光标指到别的字符、甚至陷进多字节字符内部（fuzz round35：IME 中文 +
// `:1,3retab` 光标落进「文」中间）。
//
// oracle（2026-10-09）：`a\t中文x` 上 `:set et` `0wl`（光标在文）
// `:1retab` → 行变 `a       中文x`（ts=8，TAB 7 格展开），getpos('.')
// = [0,1,12,0] = 文的新字节位。
#[test]
fn r35_3_retab_keeps_cursor_on_same_char() {
    let mut f = Fixture::new("a\t中文x\nsecond\n");
    f.feed([":", "s", "e", "t", " ", "t", "s", "=", "8", "<CR>"]);
    f.feed(["0", "w", "l"]); // 光标落在「文」
    assert_eq!(f.cursor(), 5, "前置：0wl 落在「文」(a=0, TAB=1, 中=2..5, 文=5..8)");
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("1retab".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(
        f.text(),
        "a       中文x\nsecond\n",
        "ts=8 下 TAB（列 1，宽 7）展开为 7 空格"
    );
    assert_eq!(f.cursor(), 11, "oracle [0,1,12,0]：光标留在「文」的新字节位 11");
}

// ---- R35-4（P1）：`:ce`/`:ri`/`:le` 后光标留在原字符上 ---------------------
//
// oracle（2026-10-09）：`  hello 中文` 上 `0ll`（光标在 h）`:ce 20` →
// 行变 `     hello 中文`，getpos('.') = [0,1,6,0] = h 的新字节位。
#[test]
fn r35_4_align_keeps_cursor_on_same_char() {
    let mut f = Fixture::new("  hello 中文\nsecond\n");
    f.feed(["0", "l", "l"]); // 光标在 h
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("ce 20".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    assert_eq!(f.text(), "     hello 中文\nsecond\n", "居中补 5 列");
    assert_eq!(f.cursor(), 5, "oracle [0,1,6,0]：光标留在 h 的新字节位 5");
}

// ---- R35-6（P1）：`r<C-E>` 邻行取不到字符的位逐个跳过，不整体放弃 ---------
//
// oracle（keys 文件通道，2026-10-09）：`abcdef` + `abX` 上 `0ll2r<C-E>`
// → 行变 `abXdef`（'c'←'X'，'d' 在邻行无字符 → 跳过），getpos('.') =
// [0,1,4,0] = 最后处理位 'd'。引擎旧实现整体 return false（文本不变 +
// 响铃）。
#[test]
fn r35_6_r_ctrl_e_skips_missing_neighbor_chars() {
    let mut f = Fixture::new("abcdef\nabX\n");
    f.feed(["0", "l", "l", "2", "r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "abXdef\nabX\n", "oracle: 'c'←'X'，'d' 跳过不替换");
    assert_eq!(f.cursor(), 3, "oracle [0,1,4,0]：光标落最后处理位 'd'");
}

// ---- R35-7（P1）：`r<C-E>` 替换比原字符短时光标落在替换字符上 --------------
//
// fuzz round35 seed=26：光标在 ｳ（3 字节）上 r<C-E> 取邻行 'e'（1 字节），
// 旧实现 `end - last_len` 用编辑前的陈旧 end，光标倒退进后面的「漢」内部
// （字节 9，不可寻址）。
#[test]
fn r35_7_r_ctrl_e_cursor_on_shorter_replacement() {
    let mut f = Fixture::new("\nｱｲｳ漢ｱｲｳ字\none\n");
    f.feed(["j", "l", "l"]); // 光标到第二行的 ｳ（字节 7..10）
    assert_eq!(f.cursor(), 7);
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "\nｱｲe漢ｱｲｳ字\none\n", "ｳ ← 邻行 col 2 的 'e'");
    assert_eq!(f.cursor(), 7, "光标落替换字符 'e' 上（字节 7），不落进「漢」内部");
    assert!(f.text().is_char_boundary(f.cursor()));
}

// ---- R35-8（P3）：`r<C-E>` 全部位都取不到字符 = 无操作（不响铃）------------
//
// vim 源码 nv_replace：NUL 位的处理是 `++curwin->w_cursor.col`（跳过），
// 无 E 报文（fuzz 抓的 `r<C-e>` 路径旧实现走 note_bell）。
#[test]
fn r35_8_r_ctrl_e_all_skipped_is_quiet_noop() {
    let mut f = Fixture::new("abｱcd\nx\n");
    f.feed(["0", "l", "l"]); // 光标在 ｱ（col 2），邻行 "x" col 2 无字符
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('e'));
    assert_eq!(f.text(), "abｱcd\nx\n", "全部跳过 = 无编辑");
    assert_eq!(f.cursor(), 2, "光标不动");
    assert_eq!(f.host.bells, 0, "无铃声（vim 无 E 报文）");
}

// ---- R35-5（P1）：范围后的行上，整段重写后光标按字节增量平移 ---------------
// 光标在编辑范围之后的行时同样不能保持裸偏移（内容整体挪了 delta 字节）。
// oracle 形状同 R35-3/4 的「marks 调整」语义：光标留在原字符。
#[test]
fn r35_5_range_rewrite_shifts_cursor_after_range() {
    let mut f = Fixture::new("中文\n\nab\tcd\n");
    f.feed(["G", "0", "f", "d"]); // 光标在第 3 行的 d 上
    let base = f.cursor();
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend("%retab".chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
    // 第 1 行无空白变化；第 3 行 `\t`（列 2，ts=4 → 宽 2）展开成 2 空格，
    // 光标行在第 1..3 行之后字节增量 +1，d 随之后移
    assert_eq!(
        f.text(),
        "中文\n\nab  cd\n",
        "ts=4 下第 3 行 TAB（列 2，宽 2）展开为 2 空格"
    );
    assert_eq!(
        f.cursor(),
        base + 1,
        "范围前的行改动把范围后光标的字节位整体平移 +1，d 仍在光标上"
    );
}
