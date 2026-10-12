//! audit9_b：第二轮「新鲜探索」红探针（智能体 B 域：块可视 C-v / 块编辑 / 文本对象）。
//!
//! 每个 `#[test]` 对应本机 `/usr/bin/vim`（9.1 patches 1-1752）用
//! `vim -Nu NONE -N -i NONE -n -s <keysfile> <buf>` 复核过的行为。全部在修复前
//! 为红（引擎输出与 oracle 不符），修复后转绿；注释里保留 oracle 证据。
//!
//! 引擎默认与 vim 的差异（`et=on sw=4 ts=4`）在需要时用 `:set` 对齐——本文件
//! 的用例不依赖这些默认值（除 TAB 用例显式声明 ts=4，与引擎默认一致）。
//!
//! 按键记号：空白分词；`<t:XY>` 走 IME 文本通道（host 对可打印文本的投递契约），
//! 其余走 Key 通道。`+` 在 `<t:...>` 内表示空格。

mod common;

use common::Fixture;

/// 跑一组按键：`<t:...>` 走文本通道，其余走按键通道（与 audit6_b 的
/// `run_case` 同一契约）。
fn run(buf: &str, line: usize, col: usize, keys: &str) -> Fixture {
    let mut f = Fixture::at(buf, line, col);
    for tok in keys.split_whitespace() {
        if tok.starts_with("<t:") && tok.ends_with('>') {
            f.type_text(&tok[3..tok.len() - 1].replace('+', " "));
        } else {
            f.feed([tok]);
        }
    }
    f
}

// ---------------------------------------------------------------------------
// E9-1（P1）块可视 `$` 的左缘是**锚点列**，不是第 0 列
//
// 复现：buffer "abcdef\ngh\nijklmn\n"、(0,2)（光标在 'c'），按键 <C-v> j $ d。
// vim 9.1（ts 无关）：块左缘停在锚点列 3（'c'），第 1 行删列 3..6 "cdef" 留
// "ab"，第 2 行 "gh" 太短、列 3 越界 → 不删，结果 "ab\ngh\nijklmn\n"；寄存器
// `"` = blockwise 宽 4、内容 "cdef"。
// 引擎旧模型把 `$` 块当成"每行从第 0 列到行尾"（锚点列丢失），删成 "\n\nijklmn"。
#[test]
fn e9_1_block_dollar_left_edge_is_anchor_column() {
    let f = run("abcdef\ngh\nijklmn\n", 0, 2, "<C-v> j $ d");
    assert_eq!(
        f.text(),
        "ab\ngh\nijklmn\n",
        "vim 9.1: `$` 块左缘=锚点列，短行不删（第 1 行留 ab，第 2 行 gh 不动）"
    );
}

// ---------------------------------------------------------------------------
// E9-2（P1）块可视 `S` 是**行级 change**，不是块级 change-to-EOL
//
// 复现：buffer "long line here\nabc\ndef\n"、(0,0)，按键 <C-v> j l l S Z <Esc>。
// vim 9.1：`S` 把选中的整行按 linewise 删除后进入插入 → 缓冲区 "Z\ndef\n"，
// 寄存器 `"` = linewise "long line here\nabc\n"（nv_subst：visual `S` 先把
// VIsual_mode 置 'V' 再当 'c'）。引擎旧代码把 `S` 与 `C` 一起走
// BlockChangeToEnd → "Z\nZ\ndef\n"（块级、寄存器 blockwise）。
#[test]
fn e9_2_block_S_is_linewise_change() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l S <t:Z> <Esc>");
    assert_eq!(f.text(), "Z\ndef\n", "vim 9.1: 块 S = 整行 linewise change");
}

// ---------------------------------------------------------------------------
// E9-3（P1）块可视 `A` 退出后光标停在**首行块左缘**
//
// 复现：buffer "abcdef\nghijkl\nmnopqr\n"、(0,1)（光标 'b'），
// 按键 <C-v> j l A Z <Esc>。块列 2..3（'bc'/'hi'）。
// vim 9.1：文本 "abcZdef\nghiZjkl\nmnopqr\n"，光标 (1,2) = 首行左缘 'b' 处
// （op_insert 令 w_cursor.col = oap->start.col，`A` 也回到左缘）。
// 引擎旧公式停在**副本行**（append 点）+ 文本长 → (1,4)。
#[test]
fn e9_3_block_A_parks_on_first_row_left_edge() {
    let f = run("abcdef\nghijkl\nmnopqr\n", 0, 1, "<C-v> j l A <t:Z> <Esc>");
    assert_eq!(f.text(), "abcZdef\nghiZjkl\nmnopqr\n");
    assert_eq!(f.cursor(), 1, "vim 9.1: `A` 退出后停在首行块左缘（列 2）");
}

// ---------------------------------------------------------------------------
// E9-4（P1）块可视 `c` 退出后光标停在**首行**
//
// 复现：buffer "long line here\nabc\ndef\n"、(0,0)，按键 <C-v> j l l c Z <Esc>。
// vim 9.1：文本 "Zg line here\nZ\ndef\n"，光标 (1,1) = 首行第 1 列（op_change
// 在**首行**插入，退出后停在最后输入的字符）。引擎旧代码在**末行**打字，退出后
// 停在末行 (2,1)。
#[test]
fn e9_4_block_c_parks_on_first_row() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l c <t:Z> <Esc>");
    assert_eq!(f.text(), "Zg line here\nZ\ndef\n");
    assert_eq!(f.line(), 0, "vim 9.1: 块 `c` 在首行打字并停在首行");
}

// ---------------------------------------------------------------------------
// E9-5（P1）块可视 `C` 退出后光标停在**首行**（同 `c` 的行问题）
//
// 复现：buffer "long line here\nabc\ndef\n"、(0,0)，按键 <C-v> j l l C Z <Esc>。
// vim 9.1：文本 "Z\nZ\ndef\n"，光标 (1,1) = 首行。引擎旧代码停在末行 (2,1)。
#[test]
fn e9_5_block_C_parks_on_first_row() {
    let f = run("long line here\nabc\ndef\n", 0, 0, "<C-v> j l l C <t:Z> <Esc>");
    assert_eq!(f.text(), "Z\nZ\ndef\n");
    assert_eq!(f.line(), 0, "vim 9.1: 块 `C` 在首行打字并停在首行");
}

// ---------------------------------------------------------------------------
// E9-6（P1）块可视 `l`/`h` 的虚列按**字符显示宽度**推进（宽字符 = 2 格）
//
// 复现：buffer "中文字\n中文字\n"、(0,0)，按键 <C-v> j l x。
// 锚点在 '中'（显示列 0-1），`l` 移到 '文'（列 2），块 = 列 [0,2] → 覆盖
// 中+文 → 删 "中文" 留 "字"，结果 "字\n字\n"。引擎旧代码 `l` 只把虚列 +1
// （到列 1，仍指 '中'）→ 块 [0,1] → 只删 '中' 留 "文字"。
#[test]
fn e9_6_block_l_advances_by_display_width_wide_char() {
    let f = run("中文字\n中文字\n", 0, 0, "<C-v> j l x");
    assert_eq!(f.text(), "字\n字\n", "vim 9.1: 宽字符 `l` 按 2 格推进，删 中文");
}

// ---------------------------------------------------------------------------
// E9-7（P1）TAB 行上的块列边界按 tabstop 展开（锚点在 TAB 上不吸附到 TAB 末端）
//
// 复现：buffer "\tabc\n\tdef\n"、(0,0)（光标在 TAB），按键 <C-v> j l d。
// vim 9.1（ts=4，与引擎默认一致）：块左缘 = 锚点显示列 0（TAB 起点），右缘 =
// 光标字符 'd'/'a' 的显示列 4 → 块 [0,4] 覆盖 TAB(0-3)+'a'(4) → 删 "\ta"/"\te"
// 留 "bc"/"ef"。寄存器 `"` = blockwise 宽 5、内容 "\ta\n\te"。
// 引擎旧代码有"锚点在 TAB 上→左缘吸附到 TAB 末端 4"的特判分支，删成 "\tc"/"\tf"。
#[test]
fn e9_7_block_on_tab_row_uses_tabstop_columns() {
    let f = run("\tabc\n\tdef\n", 0, 0, "<C-v> j l d");
    assert_eq!(f.text(), "bc\nef\n", "vim 9.1: 块左缘=TAB 起点列 0，覆盖 TAB+'a'");
}

// ---------------------------------------------------------------------------
// E9-8（P2）块 `$` + `A`：退出后光标停在首行锚点列
//
// 复现：buffer "abcdef\ngh\nijklmn\n"、(0,2)，按键 <C-v> j $ A Z <Esc>。
// vim 9.1：文本 "abcdefZ\nghZ\nijklmn\n"，光标 (1,3) = 首行锚点列 3。
#[test]
fn e9_8_block_dollar_A_parks_on_anchor_column() {
    let f = run("abcdef\ngh\nijklmn\n", 0, 2, "<C-v> j $ A <t:Z> <Esc>");
    assert_eq!(f.text(), "abcdefZ\nghZ\nijklmn\n");
    assert_eq!(f.cursor(), 2, "vim 9.1: `$` 块 A 退出停在首行锚点列 3");
}
