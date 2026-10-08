//! 第三十三轮独立审计（C 组）：可视模式 v/V/C-v 三 kind 及其交互域（2026-10-08）。
//!
//! 每个 `#[test]` 对应一个新发现；断言写的是 **vim 9.1（patches 1-1752，
//! `-Nu NONE -N -i NONE -s` typeahead oracle）实证过的期望结果**，因此在当前
//! 工作树上必须失败。各测试注释给出：严重度、复现（缓冲 / (行,列) / 按键）、
//! vim 期望 vs 引擎现状、oracle 证据（oracle 脚本输出原文）。已对照
//! NOTES.md 已知分歧（含 #58 可视 J count、#74 $-块 gv）、BUG_AUDIT.md
//! 50 项（G/K 系列）与 BUG_AUDIT2.md 59 项（E 系列：块 o/O 矩形、`'>`
//! 落点、gv 折叠、`` `[``/`` `] ``、可视 C-o、g;）逐条去重，下列条目均不在
//! 既有账上。
//!
//! 探针通道教训（本轮采样的一个假阴性来源）：以可视态结束时用 `:call
//! writefile` 采样，可视 `:` 会把 `'<,'>` 种进命令行、执行范围命令把光标
//! 拽到范围首行——gv/o 的光标采样必须在采样前先 Esc 回 Normal。
//!
//! 本轮证伪（oracle 已证引擎与 vim 一致，免下轮重查）：
//! - `$` 块状态经 V→C-v 往返存活（c1b：`<C-v>j$V<C-v>A` 两侧都在各行行尾追加）；
//! - `5h` 超越行首：vim 也钳到 col 0 并视为移动成功、不响铃（c3a/c3b，块内外一致）；
//! - 可视 `r` 后 `.`：vim 重放整个可视替换（"abcdefgh" 上 `vlllrXllll.` →
//!   "XXXXXXXX"，r4），引擎的键序重放同构；
//! - 可视 `p` 带 count：vim 按寄存器 × count 替换选区（c21 "aababab"），引擎一致；
//! - Esc 落点在**反向**选区上：两侧都落选区起点（=光标端，c7b）；
//! - 块 `o` 两次对合还原矩形（c26 "bcdef\nb"）、块 `o` 后 d（q10）、块 `o`
//!   后 `$A`（q11 各行自己行尾追加）：oracle 与引擎一致；
//! - linewise/charwise 可视 `C`/`S` 的「多行塌成一行」语义（q4/q5/q6）：
//!   引擎 `ops::apply` Change 的 effective span 与 vim 一致（分歧只剩块 C，
//!   见 C10）；
//! - `Vj>` 的光标（r10 落首行首个非空白）、`Vju` 的光标（c13 落行首）、
//!   前向选区的 gv 光标（r1 落 '> 侧末字符）、`Vj` 形 gv 的 linewise 光标
//!   （E3b 修复的 col-0 形状两侧巧合一致）；
//! - linewise `Vj<` 正常去缩进（b1）；`3g C-a` 于单行 linewise 选区（c4：
//!   两侧都只给首个数字 +3）。

mod common;

use common::edit;

// ---------------------------------------------------------------------------
// C1（P1）v/V/C-v kind 切换不重置块虚拟列——陈旧 block_cursor_col 把矩形压窄
//
// 复现 A：buffer "abc\nabc\n"、(0,0)，按键 <C-v> l v l <C-v> d。
//   vim 9.1（oracle c1a，两次复跑一致）：C-v→v→C-v 转换保留锚点/光标的
//   实际列，块 = col 0..3（单行）→ d 删掉第 1 行全部 → "\nabc\n"。
//   引擎：`VisualCmd::ToggleKind` 只改 kind，`block_cursor_col` 仍是第一次
//   `l` 留下的 1 → 块被读成 col 0..2 → 只删 "ab" → "c\nabc\n"。
// 复现 B：buffer "abc\ndef\n"、(0,0)，按键 <C-v> l j v l <C-v> d。
//   vim（oracle c1a2）：两行各删 3 列 → "\n\n"。
//   引擎：陈旧虚拟列 1 → "c\nf\n"。
//   （root cause 线索：src/state.rs `execute_visual_cmd` 的
//   `VisualCmd::ToggleKind` 臂不清理 block_cursor_col/block_anchor_vcol。）
#[test]
fn c1_toggle_kind_keeps_stale_block_virtual_column() {
    let f = edit("abc\nabc\n", 0, 0, &["<C-v>", "l", "v", "l", "<C-v>", "d"]);
    assert_eq!(f.text(), "\nabc\n", "vim 9.1: v→C-v 转换后块为 col 0..3，d 删整行内容");
    let f = edit("abc\ndef\n", 0, 0, &["<C-v>", "l", "j", "v", "l", "<C-v>", "d"]);
    assert_eq!(f.text(), "\n\n", "vim 9.1: 两行各删 col 0..3");
}

// ---------------------------------------------------------------------------
// C2（P1）j/k 的块列钳制是永久性的——空行/短行上走一遭后矩形宽度不再恢复
//
// 复现 A：buffer "abcdef\n\nabcdef\n"、(0,0)，按键 <C-v> 3 l j k d。
//   vim 9.1（oracle c2）：curswant 是列记忆，j 到空行矩形在该行呈零宽，
//   k 回来后恢复 col 0..4 → 两行各删 "abcd" → "ef\n\nef\n"。
//   引擎：goto_motion 的 Up/Down 臂把 min(col, 着陆行宽) 写回
//   block_cursor_col 本体（src/state.rs ~1959），col 永久变 0 → 只删各
//   行 'a' → "bcdef\n\nabcdef\n"。
// 复现 B（短行）：buffer "abcdef\nab\nabcdef\n"、(0,0)，同键。
//   vim（oracle c2c）：→ "ef\nab\nabcdef\n"（中间行内容保留）。
//   引擎：col 钳到 2 后回不来 → "def\n\ndef\n"。
//   （E1c 修的「在短行上时矩形收窄」本身与 vim 一致；缺口是 vim 只在
//   该行收窄显示，curswant 从不因 j/k 改写。）
#[test]
fn c2_jk_clamp_destroys_block_width_permanently() {
    // oracle 复验（2026-10-08）：`<C-v>3ljk` 之后 k 把选区折叠回首行，
    // `d` 只删 row 0 的 cols 0..4（初稿 case A 的 "ef\n\nef" 是通道伪影
    // ——与本文件头部警告的采样陷阱同源）；case B 与复验一致。语义点：
    // curswant 不因 j/k 永久收窄（旧引擎把 min(col,row宽) 写回 bcc，
    // 之后连 row 0 的宽度都回不来）。
    let f = edit("abcdef\n\nabcdef\n", 0, 0, &["<C-v>", "3", "l", "j", "k", "d"]);
    assert_eq!(f.text(), "ef\n\nabcdef\n", "vim 9.1: k 折叠选区，块宽在 row 0 恢复满宽");
    let f = edit("abcdef\nab\nabcdef\n", 0, 0, &["<C-v>", "3", "l", "j", "k", "d"]);
    assert_eq!(
        f.text(),
        "ef\nab\nabcdef\n",
        "vim 9.1: 短行内容不参与删除，row 0 删 col 0..4"
    );
}

// ---------------------------------------------------------------------------
// C3（P1）块可视 g C-a 的 count 公式：vim 逐行 +count*(i+1)，引擎 +count+i
//
// 复现：buffer "1\n1\n1\n"、(0,0)，按键 <C-v> j j 2 g <C-a>。
//   vim 9.1（oracle r11/g5）：3\n5\n7（第 i 行 +2*(i+1)）。
//   引擎：SequentialIncrement 用 `base + i`（src/state.rs
//   VisualCmd::SequentialIncrement）→ 3\n4\n5。count=1 时两公式重合
//   （K1 的探针恰好只压了 count=1）。
#[test]
fn c3_block_gca_count_multiplies_per_line() {
    let f = edit("1\n1\n1\n", 0, 0, &["<C-v>", "j", "j", "2", "g", "<C-a>"]);
    assert_eq!(f.text(), "3\n5\n7\n", "vim 9.1: 第 i 行 +count*(i+1) → 3/5/7");
}

// ---------------------------------------------------------------------------
// C4（P1）块可视 g C-a 无视块列边界——把块外整行的数字也递增
//
// 复现：buffer "x1\nx2\nx3\n"、(0,0)（块只罩住各行的 'x' 列），按键
// <C-v> g <C-a>。
//   vim 9.1（oracle c5）：块列内没有数字 → 缓冲原样不动。
//   引擎：SequentialIncrement 对 span 覆盖的「行」逐行把光标摆到行首再
//   增量（span 又是 anchor==cursor 的对角 charwise 跨度，实际只处理第 1
//   行）→ 第 1 行变 "x2" → "x2\nx2\nx3\n"。两处都错：作用行与作用列都
//   越出块边界。
#[test]
fn c4_block_gca_ignores_block_columns() {
    let f = edit("x1\nx2\nx3\n", 0, 0, &["<C-v>", "g", "<C-a>"]);
    assert_eq!(f.text(), "x1\nx2\nx3\n", "vim 9.1: 块列内无数字时 g C-a 不动缓冲");
}

// ---------------------------------------------------------------------------
// C5（P1）块可视 `>` 响铃拒绝——vim 对三种 kind 都缩进高亮行
//
// 复现：buffer "ab\ncd\n"、(0,0)，按键 <C-v> j >。（引擎默认 et/ts=4/sw=4
// 与 oracle 的 `:set ts=4 sw=4 et` 对齐。）
//   vim 9.1（oracle f1）：两行各缩进一格 → "    ab\n    cd\n"，光标落
//   首行行首（offset 0），无铃声。
//   引擎：apply_block_operator 的 `_ =>` 兜底臂对 Indent* 只响铃
//   （src/state.rs ~2903）→ 文本不动 + bell。
//   对照：linewise `Vj>` 引擎与 vim 一致（r10，光标落首个非空白）。
#[test]
fn c5_block_gt_indents_instead_of_bell() {
    let f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", ">"]);
    assert_eq!(f.text(), "    ab\n    cd\n", "vim 9.1: 块 > 缩进高亮行（et 4 空格）");
    assert_eq!(f.cursor(), 0, "vim 9.1: 光标落首行行首");
    assert_eq!(f.host.bells, 0, "vim 9.1: 成功的缩进不响铃");
}

// ---------------------------------------------------------------------------
// C6（P1）块可视 I/A 丢弃 count——vim 把键入文本在每行重复 count 次
//
// 复现 A：buffer "aa\nbb\n"、(0,0)，按键 <C-v> j 3 I，键入 "-"，Esc。
//   vim 9.1（oracle c9）："---aa\n---bb\n"。
//   引擎：visual_key 的 I/A 直通 begin_block_insert，insert_repeat 只在
//   execute_command 的 EnterInsert 臂装配、且显式排除块会话 → "-aa\n-bb\n"。
// 复现 B（A 同族）：buffer "ab\ncd\n"、(0,0)，<C-v> j 3 A + "-" + Esc。
//   vim（oracle d2）："a---b\nc---d\n"；引擎："a-b\nc-d\n"。
#[test]
fn c6_block_insert_ignores_count_repeat() {
    let mut f = edit("aa\nbb\n", 0, 0, &["<C-v>", "j", "3", "I"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "---aa\n---bb\n", "vim 9.1: 3I 在每行键入三份文本");

    let mut f = edit("ab\ncd\n", 0, 0, &["<C-v>", "j", "3", "A"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a---b\nc---d\n", "vim 9.1: 3A 在每行行尾重复三份");
}

// ---------------------------------------------------------------------------
// C7（P2）进入可视模式的 count 被丢弃——vim 的 {count}v/V/C-v 预选 count 个单元
//
// 复现 A：buffer "abcdef\n"、(0,0)，按键 3 v x。
//   vim 9.1（oracle c10）：3v 选中 3 个字符 → "def\n"。
//   引擎：EnterVisual 臂忽略 count（src/state.rs ~3905）→ 选 1 字符 → "bcdef\n"。
// 复现 B：buffer "aaa\nbbb\nccc\n"、(0,0)，3 V x。vim（oracle c19）删 3 行 →
//   空缓冲；引擎删 1 行 → "bbb\nccc\n"。
// 复现 C：buffer "abcdef\nabcdef\n"、(0,0)，3 <C-v> x。vim（oracle c20）删
//   第 1 行 3 列 → "def\nabcdef\n"；引擎删 1 列 → "bcdef\nabcdef\n"。
//   （与 audit2 证伪项「3vj 的 count 被 vim 丢弃」不同账：那是选区 motion
//   前的 count，这里 count 属于 v/V/C-v 键本身。）
#[test]
fn c7_count_on_visual_entry_selects_count_units() {
    let f = edit("abcdef\n", 0, 0, &["3", "v", "x"]);
    assert_eq!(f.text(), "def\n", "vim 9.1: 3v 预选 3 个字符");
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["3", "V", "x"]);
    assert_eq!(f.text(), "", "vim 9.1: 3V 预选 3 行");
    let f = edit("abcdef\nabcdef\n", 0, 0, &["3", "<C-v>", "x"]);
    assert_eq!(f.text(), "def\nabcdef\n", "vim 9.1: 3C-v 预选 3 列块");
}

// ---------------------------------------------------------------------------
// C8（P1）块可视 X 删掉整行——vim 在块模式下按块列删（同 x）
//
// 复现：buffer "ab\ncd\nef\n"、(0,0)，按键 <C-v> j X。
//   vim 9.1（oracle c11a）：块 1 列（col 0）→ 删掉前两行各 1 字符 →
//   "b\nd\nef\n"。
//   引擎：X 注册成 LinewiseOp(Delete)（src/tables.rs ~664，oracle 探针当时
//   只压过 charwise 形）→ 前两行整行消失 → "ef\n"。
#[test]
fn c8_block_x_deletes_block_not_lines() {
    let f = edit("ab\ncd\nef\n", 0, 0, &["<C-v>", "j", "X"]);
    assert_eq!(f.text(), "b\nd\nef\n", "vim 9.1: 块模式下 X 删块列（此处 1 列）");
}

// ---------------------------------------------------------------------------
// C9（P1）块可视 Y 按行 yank——vim 在块模式下 yank 矩形块
//
// 复现：buffer "ab\ncd\nef\n"、(0,0)，按键 <C-v> j Y j P。
//   vim 9.1（oracle c11b）：Y 拿到 1 列块寄存器 ["a","c"]，光标回块首
//   (0,0)；j 到第 2 行后 P 按块粘贴在 col 0 前 → "ab\nacd\ncef\n"。
//   引擎：Y → LinewiseOp(Yank) → 按行 yank ["ab","cd"]，光标滞留在可视
//   光标端 (1,0)；j P 按行粘出两整行 → "ab\ncd\nab\ncd\nef\n"。
#[test]
fn c9_block_y_yanks_block_not_lines() {
    let f = edit("ab\ncd\nef\n", 0, 0, &["<C-v>", "j", "Y", "j", "P"]);
    assert_eq!(f.text(), "ab\nacd\ncef\n", "vim 9.1: 块 Y yank 矩形，P 块粘回");
}

// ---------------------------------------------------------------------------
// C10（P1）块可视 C 按行级 change 塌行——vim 每个高亮行各保留一行进入插入
//
// 复现：buffer "abcd\nefgh\nijkl\n"、(0,0)，按键 <C-v> j C，键入 "-"，Esc。
//   vim 9.1（oracle q4；c12a 同形）：两行各变 "-" → "-\n-\nijkl\n"。
//   引擎：C/S 共用 LinewiseOp(Change)，effective span 删掉两行内容**含
//   中间换行** → 塌成一行 "-\nijkl\n"（S 的语义；charwise/linewise 的
//   C 与 S 塌缩两侧一致已证伪，缺口只在块 C）。
#[test]
fn c10_block_c_keeps_one_line_per_highlighted_line() {
    let mut f = edit("abcd\nefgh\nijkl\n", 0, 0, &["<C-v>", "j", "C"]);
    f.type_text("-");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "-\n-\nijkl\n", "vim 9.1: 块 C 逐行 change，行数不塌");
}

// ---------------------------------------------------------------------------
// C11（P2）Esc 退出可视模式后光标被搬回选区起点——vim 停在可视光标端
//
// 复现 A：buffer "abcdef\n"、(0,0)，按键 l l l v l l l <Esc>。
//   vim 9.1（oracle e1）：光标停在可视光标端 offset 5。
//   引擎：exit_visual 无条件 `cursor.offset = lo`（src/state.rs ~2725）→
//   锚点列 offset 3。
// 复现 B：buffer "aaa\nbbb\nccc\n"、(0,0)，V j <Esc>。vim（oracle e3）光标
//   (2,1)=offset 4；引擎 offset 0。
// 复现 C：buffer "abcdef\nabcdef\nabcdef\n"、(0,0)，<C-v> j j <Esc>。
//   vim（oracle e6）光标 (3,1)=offset 12；引擎 offset 0。
//   （反向选区两侧一致落选区起点=光标端，已证伪；缺口是全部前向选区。）
#[test]
fn c11_esc_keeps_cursor_at_visual_end() {
    let f = edit("abcdef\n", 0, 0, &["l", "l", "l", "v", "l", "l", "l", "<Esc>"]);
    assert_eq!(f.cursor(), 5, "vim 9.1: Esc 后光标停在可视光标端");
    let f = edit("aaa\nbbb\nccc\n", 0, 0, &["V", "j", "<Esc>"]);
    assert_eq!(f.cursor(), 4, "vim 9.1: linewise 前向选区 Esc 光标在末行");
    let f = edit("abcdef\nabcdef\nabcdef\n", 0, 0, &["<C-v>", "j", "j", "<Esc>"]);
    // (3,1) 1-based = 第 3 行行首 = offset 14（探针初稿把 \n 算漏了）
    assert_eq!(f.cursor(), 14, "vim 9.1: 块选区 Esc 光标在末行行首");
}

// ---------------------------------------------------------------------------
// C12（P2）gv 恢复反向选区时光标落在 '> 侧——vim 回到当初的光标端
//
// 复现：buffer "abcdef\n"、(0,0)，按键 l l l v h <Esc> g v（选区 col 2..3，
// 锚在 3、光标在 2）。
//   vim 9.1（oracle r2，Esc 后 Normal 态采样）：gv 把光标放回原可视光标端
//   offset 2。
//   引擎：RestoreVisual 一律 cursor = floor(hi-1)（src/state.rs ~4840）→
//   offset 3。前向选区两公式重合（r1 已证伪）；last_visual 不记「哪端是
//   当时光标」是根因。
#[test]
fn c12_gv_backward_cursor_returns_to_original_end() {
    let f = edit("abcdef\n", 0, 0, &["l", "l", "l", "v", "h", "<Esc>", "g", "v"]);
    assert_eq!(f.cursor(), 2, "vim 9.1: gv 反向恢复落在原光标端 offset 2");
}

// ---------------------------------------------------------------------------
// C13（P2）gv 恢复 linewise 选区丢光标列——vim 保留当初的列，引擎钳回首非空白
//
// 复现：buffer "aaa\nbbbb\nccc\n"、(0,0)，按键 V $ j <Esc> g v。
//   vim 9.1（oracle r3）：$ 把 curswant 定在行尾虚列，j 带列下移，gv 后
//   光标回到 (2,4)（"bbbb" 末字符，offset 7）。
//   引擎：RestoreVisual 的 Line 臂落 `first_non_blank(末行)`（注释引用的
//   「audit E3b follow-up」探针用的是 col 0 的 Vj 形，两种落法巧合重合）
//   → offset 4。
#[test]
fn c13_gv_linewise_restores_cursor_column() {
    let f = edit("aaa\nbbbb\nccc\n", 0, 0, &["V", "$", "j", "<Esc>", "g", "v"]);
    assert_eq!(f.cursor(), 7, "vim 9.1: gv 恢复 linewise 选区保留光标列");
}

// ---------------------------------------------------------------------------
// C14（P3）块可视 `<` 的光标不动——vim 文本不变（quirk）但光标落首行首非空白
//
// 复现：buffer "    ab\n    cd\n"、(0,0)，按键 <C-v> j <。（et/ts=4/sw=4
// 与 oracle 对齐。）
//   vim 9.1（oracle f2/r9）：文本不变（块 `<` 不去缩进是 vim 自身行为），
//   光标落 (1,5) = offset 4（首行首个非空白），无失败感。
//   引擎：apply_block_operator 兜底臂响铃且光标滞留可视端 offset 7。
//   （文本面两侧一致；差异在光标落点与铃声。）
#[test]
fn c14_block_lt_moves_cursor_to_first_non_blank() {
    let f = edit("    ab\n    cd\n", 0, 0, &["<C-v>", "j", "<"]);
    assert_eq!(f.text(), "    ab\n    cd\n", "vim 9.1: 块 < 不改文本（两侧一致）");
    assert_eq!(f.cursor(), 4, "vim 9.1: 光标落首行首个非空白");
    assert_eq!(f.host.bells, 0, "vim 9.1: 不作失败反馈");
}

// ---------------------------------------------------------------------------
// C15（P1）块列模型整链按 1 格计 TAB——块选区跨 TAB 行的列边界全错
//
// 复现：buffer "\tab\nabcdef\n"、(0,0)，按键 <C-v> l j d。（引擎默认 ts=4，
// oracle 以 `:set ts=4` 对齐。）
//   vim 9.1（oracle r5，两次复跑一致）：块的右缘按 TAB 展开后的虚列计
//   → 第 1 行只删 'a'（TAB 幸存）、第 2 行删 "ef" → "\t\nabcd\n"。
//   引擎：span_from_visual_block / block_row_range 全走 ts=1 的
//   display_column / char_display_width（src/ops.rs ~478、~396；audit2 A1
//   只修了 `|`/j/k 的落点函数）→ 块读成 col 0..2 → 第 1 行删 "\ta"、
//   第 2 行删 "ab" → "b\ncdef\n"。
//   （vim 对压在 TAB 上的块缘的确切取整规则未在本次审计内完全逆向，但
//   oracle 的期望文本稳定可复现；引擎侧的 ts=1 模型与它必然分歧。）
#[test]
fn c15_block_columns_count_tab_as_one_cell() {
    let f = edit("\tab\nabcdef\n", 0, 0, &["<C-v>", "l", "j", "d"]);
    assert_eq!(f.text(), "\t\nabcd\n", "vim 9.1: 块缘按 tabstop 虚列计（TAB 幸存）");
}
