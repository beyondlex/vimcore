//! 第三十二轮独立审计（E 组）：可视模式 / 可视块 / marks 域（2026-10-07）。
//!
//! 每个 `#[test]` 对应一个新发现；断言写的是 **vim 9.1（`-Nu NONE -N -i NONE
//! -s` typeahead oracle）实证过的期望结果**，因此在当前工作树上必须失败。
//! 各测试注释给出：严重度、复现（缓冲 / (行,列) / 按键）、vim 期望 vs 引擎
//! 现状、证据（oracle 输出或标注待复核）。已知分歧账（NOTES.md 编号项、
//! BUG_AUDIT.md 50 项）已逐条排除。

mod common;

use common::{edit, Fixture};
use vimcore::mode::Mode;

// ---------------------------------------------------------------------------
// 发现 1（P1）块可视 `o` 按字节对调角点——矩形按 (行, 虚拟列) 对调才是 vim 语义
//
// 复现 A：buffer "abcdef\nabcdef\n"、(0,0)，按键 <C-v> j l o d。
//   vim 9.1（oracle：`abcdef\nabcdef\n` + \026jlod）：块不变（两行各删 "ab"）
//   → "cdef\ncdef\n"。引擎：o 后 anchor/cursor 的字节列成为块两缘，右缘列丢
//   失 → 每行只删 col 1 的 'b' → "acdef\nacdef\n"。
// 复现 B：同缓冲 (0,0)，<C-v> l o d（单行块）。
//   vim（oracle o22）：删 col 0..2 → "cdef\nabcdef\n"。
//   引擎：两角都在行 0 且列塌成 1 宽 → "acdef\nabcdef\n"。
// 复现 C：buffer "abcdef\nab\n"、(0,3)，<C-v> j o d（光标 j 到短行后虚拟列发散）。
//   vim（oracle o3）：块保持 col 3 单列 → "abef\nab\n"。
//   引擎：对调后 anchor 取钳制字节列 1 → 块变 col 1..4 → "aef\na\n"。
#[test]
fn block_o_swaps_raw_bytes_instead_of_row_col_corners() {
    // A: 两行块经 o 后仍是两行两列
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "l", "o", "d"]);
    assert_eq!(
        f.text(),
        "cdef\ncdef\n",
        "vim 9.1: o 不改变块矩形，d 删两行各 col 0..2"
    );
    // B: 单行块经 o 后仍覆盖两个选中列
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "l", "o", "d"]);
    assert_eq!(
        f.text(),
        "cdef\nabcdef\n",
        "vim 9.1: 单行块 o 只对调列端，d 删 col 0..2"
    );
    // C: 短行上 j 虚拟列发散后，o 不得把左缘拉到钳制字节列
    let f = edit("abcdef\nab\n", 0, 3, &["<C-v>", "j", "o", "d"]);
    assert_eq!(
        f.text(),
        "abef\nab\n",
        "vim 9.1: 块保持 col 3，只删第 1 行的 'd'"
    );
}

// ---------------------------------------------------------------------------
// 发现 2（P1）块可视 `O` 不刷新 block_cursor_col——陈旧虚拟列把矩形右移
//
// 复现：buffer "abcdef\nabcdef\n"、(0,0)，按键 <C-v> j l O d。
// vim 9.1（oracle o4）：O 只把光标移到同行另一角，矩形不变 → 两行各删 "ab"
// → "cdef\ncdef\n"。引擎：SwapEndsKeepCol 重排两角字节，但
// block_cursor_col 仍是 `l` 之后的 1，span 读出 col 1..2 → 两行只删 'b'
// → "acdef\nacdef\n"。
#[test]
fn block_O_leaves_stale_virtual_column() {
    let f = edit("abcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "l", "O", "d"]);
    assert_eq!(
        f.text(),
        "cdef\ncdef\n",
        "vim 9.1: O 保持矩形，d 删两行各 col 0..2"
    );
}

// ---------------------------------------------------------------------------
// 发现 3（P2）`` `> ``/`'> 落在选区末字符之后一格（字符级选区）
//
// 复现：buffer "hello world\nsecond\n"、(0,0)，按键 v l l l y ` >。
// vim 9.1（oracle o5：col("'>")=4，`` `> `` 后 col(".")=4）：'> 在最后一个被选
// 字符（"hell" 的末 'l'，1-based col 4 = offset 3）。引擎把 exclusive 端字节
// (offset 4) 当 mark → `` `> `` 停在 'o'（offset 4）。
#[test]
fn tick_gt_charwise_lands_one_past_last_selected_char() {
    let f = edit("hello world\nsecond\n", 0, 0, &["v", "l", "l", "l", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: `> 停在选区末字符 'l'（offset 3）"
    );
}

// ---------------------------------------------------------------------------
// 发现 4（P2）行级选区的 `'> 列停在行首——vim 停在末行最后一个字符
//
// 复现 A：buffer "aaa\nbbbb\nccc\n"、(0,0)，V j y ` >。
//   vim 9.1（oracle o6a：'>=(2,5)，`> 后 col(".")=4）：`> 停在末行 "bbbb" 末
//   字符（1-based col 4 = offset 7）。引擎：offset 5（line1 col1）。
// 复现 B：同缓冲 (0,0)，V y ` >。
//   vim（oracle o6b：'>=(1,4)，col(".")=3）：offset 2。引擎：offset 0。
#[test]
fn tick_gt_linewise_lands_at_line_start_instead_of_last_char() {
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "j", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        7,
        "vim 9.1: Vj 后 `> 停在末行最后一个字符 'b'（offset 7）"
    );
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "y", "`", ">"]);
    assert_eq!(
        f.cursor(),
        2,
        "vim 9.1: Vy 后 `> 停在选区行末字符 'a'（offset 2）"
    );
}

// ---------------------------------------------------------------------------
// 发现 5（P1）undo 不恢复被删行的 mark——vim 的撤销块随文本恢复 mark
//
// 复现：buffer "  foo\nbar\nbaz\n"、(0,2)（'f' 上），按键 m a d d u ` a。
// vim 9.1（oracle r8a/o7b：u 后 getpos("'a")=(1,5)，`a 后 col(".")=5）：mark
// 随撤销回到行 1（vim 9.1 把它放到末字符，1-based col 5 = offset 4）。
// 引擎：dd 的 adjust_delete 把 mark 折到删除点 0，undo 只换文本不动 mark
// → `a 落 (1,1)（offset 0）。
#[test]
fn undo_does_not_restore_marks_of_deleted_line() {
    let f = edit("  foo\nbar\nbaz\n", 0, 2, &["m", "a", "d", "d", "u", "`", "a"]);
    assert_eq!(
        f.cursor(),
        4,
        "vim 9.1: undo 后 `a 回到被删行（oracle col=5 → offset 4）"
    );
}

// ---------------------------------------------------------------------------
// 发现 6（P1）选区所在行被 dd 整行删除后 gv 复选出跨行区间——d 把两行并线
//
// 复现：buffer "aa\nbb\ncc\ndd\n"、(1,0)，按键 v l y d d g v d。
// vim 9.1（oracle o8b：dd 后 '<=(2,1) '>=(2,2)；o8：gvd 后 getline =
// ["aa","","dd"]）：marks 塌缩到幸存行 "cc" 的 col 1..2，gv 复选 "cc"，
// d 删空该行内容 → "aa\n\ndd\n"。引擎：stored (3,3) 的 `> 端经
// floor(hi-1)+clamp 停到上一行末字符 → 复选区间跨换行（sel anchor=3
// cursor=1）→ d 删 1..4 并线 → "ac\ndd\n"。
#[test]
fn gv_after_line_delete_reselects_across_line_break() {
    let f = edit("aa\nbb\ncc\ndd\n", 1, 0, &["v", "l", "y", "d", "d", "g", "v", "d"]);
    assert_eq!(
        f.text(),
        "aa\n\ndd\n",
        "vim 9.1: gv 复选幸存行内容，d 删空该行（行结构保留）"
    );
}

// ---------------------------------------------------------------------------
// 发现 7（P2）`'[`/`']`/`` `[ ``/`` `] `` 未绑定——vim 跳到最近一次
// yank/change 的首/末字符
//
// 复现：buffer "foo bar\n"、(0,0)，y i w 后按 ` ]（或 ']）。
// vim 9.1（oracle o9a/o9b：`[ → col 1，`] → col 3）：`] 停在 yank 末字符 'o'
// （offset 2）。引擎：trie Miss 响铃原地不动（`] 落 offset 0）。
#[test]
fn bracket_marks_unbound() {
    let f = edit("foo bar\n", 0, 0, &["y", "i", "w", "`", "]"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: `] 停在 yank 末字符（offset 2）");
    assert_eq!(f.host.bells, 0, "vim 9.1: `] 是真命令，不响铃");

    let f = edit("foo bar\n", 0, 0, &["y", "i", "w", "`", "["]);
    assert_eq!(
        f.host.bells, 0,
        "vim 9.1: `[ 跳到 yank 首字符（本例 offset 0 即光标处），不响铃"
    );
}

// ---------------------------------------------------------------------------
// 发现 8（P2）可视模式 `<C-o>` 应退回普通模式（光标停在可视光标端）——引擎
// 静默交给宿主、选区原样滞留
//
// 复现：buffer "aaa\nbbb\nccc\nddd\neee\nfff\n"、(0,0)，G v k k <C-o>。
// vim 9.1（oracle r3c/r3h/r3i：mode()="n"、line/col = 可视光标端 (6,1)、
// visualmode() 仍返回 "v"）：<C-o> 在可视模式退回 Normal，光标停在当前端，
// 不走跳转列表。引擎：visual trie 无此键 → ProcessOutcome::Unknown 交还宿主
// （零响铃），模式停留 Visual{Char}。
#[test]
fn ctrl_o_in_visual_should_exit_to_normal_mode() {
    let f = edit("aaa\nbbb\nccc\nddd\neee\nfff\n", 0, 0, &["G", "v", "k", "k", "<C-o>"]);
    assert_eq!(
        f.vim.mode(),
        Mode::Normal,
        "vim 9.1: 可视模式下 C-o 退出到普通模式"
    );
    assert_eq!(
        f.cursor(),
        12,
        "vim 9.1: 光标停在可视光标端（'ddd' 行首 offset 12）"
    );
}

// ---------------------------------------------------------------------------
// 发现 9（P2）`g;` 的变更表遍历次序倒挂，且首次 g; 误报 E662
//
// 复现 A：buffer "abcd\n"、(0,1)，x（变更@1）、A 后输入 " z"（变更@3）、gg、
//   g ;。vim 9.1（oracle w4：col=4 → offset 3）：第一次 g; 落最近的变更
//   （A 追加处）。引擎：落最旧的变更（offset 1）。
// 复现 B：buffer "abcd\n"、(0,1)，x、$、g ;。vim（oracle w6：col=2，无报错
//   文本）：唯一一次 g; 静默落变更处。引擎：先报 "E662: At start of
//   changelist" 并响铃，再把光标丢到 changes[0]（碰巧同为 offset 1）。
#[test]
fn changelist_walk_starts_at_oldest_and_falsely_reports_E662() {
    let mut f = Fixture::at("abcd\n", 0, 1);
    f.feed(["x"]);
    f.feed(["A"]);
    f.type_text(" z");
    f.feed(["<Esc>", "g", "g", "g", ";"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: 第一次 g; 落最近的变更（A 追加点 offset 3）"
    );

    let mut g = Fixture::at("abcd\n", 0, 1);
    g.feed(["x", "$", "g", ";"]);
    assert_eq!(g.cursor(), 1, "vim 9.1: g; 落在变更位置（offset 1）");
    assert!(
        g.host.statuses.is_empty(),
        "vim 9.1: 首次 g; 静默成功；引擎误报 {:?}",
        g.host.statuses
    );
    assert_eq!(g.host.bells, 0, "vim 9.1: 成功的 g; 不响铃");
}

// ---------------------------------------------------------------------------
// 发现 10（P3）`^/gi 停在最后一个插入字符上——vim 停在其后一格
//
// 复现：buffer "hello\n"、(0,0)，l i 后输入 "XY"、Esc、` ^（或 g i 后补打字）。
// vim 9.1（oracle z3b：`^ 后 col=4 → offset 3；w3：gi 打 Z 得 "hXYZello"）：
// '^ 记插入结束位置（末插入字符之后）。引擎记在末插入字符上（offset 2）
// → gi 打 Z 得 "hXZYello"。
#[test]
fn caret_mark_and_gi_stop_on_last_inserted_char() {
    let mut f = Fixture::at("hello\n", 0, 0);
    f.feed(["l", "i"]);
    f.type_text("XY");
    f.feed(["<Esc>", "`", "^"]);
    assert_eq!(
        f.cursor(),
        3,
        "vim 9.1: `^ 停在末插入字符之后（offset 3）"
    );
    f.feed(["g", "i"]);
    f.type_text("Z");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "hXYZello\n", "vim 9.1: gi 在 `^ 位置续写");
}

// ---------------------------------------------------------------------------
// 发现 11（P3）可视 `<` 无缩进可删时不响铃（同族：BUG_AUDIT L3 已证普通模式
// `<<` 无缩进时 vim 响铃且引擎已修，可视算子路径漏掉同一反馈）
//
// 复现：buffer "foo\n  bar\n"、(0,0)，V <。
// vim 9.1：`<` 删无可删缩进 = 失败响铃（与 L3 的普通 `<<` 同一算子语义；
// 本条未单独过 oracle，标注待复核）。引擎：静默原样（bells=0）。
#[test]
fn visual_indent_left_noop_is_silent() {
    let f = edit("foo\n  bar\n", 0, 0, &["V", "<"]);
    assert_eq!(f.host.bells, 1, "vim 9.1: 无缩进可删时可视 < 响铃");
    assert_eq!(f.text(), "foo\n  bar\n", "文本不变（此半句引擎已满足）");
}
