//! audit3_b：Normal motion、算子跨度、文本对象域 独立审计（2026-10-08）。
//!
//! 每个发现一个 `#[test]`，断言写的是 **vim 9.1（`-Nu NONE -N -i NONE -n -s`
//! typeahead 实证）的期望结果**，因此在当前工作树上应当失败。oracle 通道：
//! `python3 -c "open('b.txt','wb').write(<初始字节>)"` +
//! `python3 -c "open('k.txt','wb').write(<按键>)"` +
//! `vim -Nu NONE -N -i NONE -n -s k.txt b.txt </dev/null`，文件必须以
//! `:wq\n` 收尾；缓冲采样用 `:call writefile([...],"out")`。注意 vim 9.1
//! 默认 'fixendofline' 开：noeol 缓冲经修改后写回时补回末尾换行——下列
//! noeol 形状的期望值均按写回字节（即 vim 内部缓冲文本）计。
//!
//! 范围：src/motions.rs + src/word.rs + src/objects.rs + src/ops.rs 的
//! span_from_motion + src/tables.rs 命令表 + cmdline.rs 的 search motion。
//! 已对照 NOTES.md 77 项已知分歧、BUG_AUDIT.md 50 项、BUG_AUDIT2.md
//! 59 项去重（audit2 的 A 系列刚修了 TAB 虚列/节跳转，B 系列刚修了
//! aw 空白矩阵/cw≡ce/对象 count 跨行——本轮找的是它们没覆盖的形状）。
//!
//! ---------------------------------------------------------------
//! 本轮证伪清单（oracle 已证引擎与 vim 9.1 一致，下轮免重查）：
//! * `diw` 于词间空白：vim 同样删空白段（'a b' 2|diw → 'ab'）。
//! * void 标签 `<br>`：'<p>a<br>b</p>' 在 a / b / `<br>` 的 `<` 上 dit
//!   均得 '<p></p>'——vim 的未配对开标签同样不参与配对，引擎一致。
//! * noeol 末行 `dd`：vim 写回 "aaa\n"（fixendofline），引擎文本一致。
//! * `dw` 于「词 + 尾随空白」行尾缓冲（'x  ' / '.  ' 单行）：vim 留一个
//!   空行（"\n"），引擎的 single-w 钳制一致；'x  \nbar' 的 dw 保换行，一致。
//! * `dgg`@第1行 / `dG`@末行：vim 删整行，引擎 linewise span 一致。
//! * `d10|` 于短行：vim 只删到钳制列（'abcde' → 'e'），引擎一致。
//! * 反向 `d?pat` 终点落在行首：exclusive 规则 (a) 字节等价于裸 exclusive
//!   span（'foo pat\nnext' 2G0d?pat → 'foo \nnext'），引擎一致。
//! * `daw` 于行尾词：vim 只删词本身、换行/空行都留（'ab\ncd' 1|daw →
//!   '\ncd'），引擎一致（merges_following_line_break 只管空白起点）。
//! * `}` 于缓冲末落在最后一字符（getcurpos col=3 on 'bbb'），d} 列 0 形状
//!   与引擎的 phantom 提升一致（noeol 'aaa\nbbb' 1|d} → 空）。
//! ---------------------------------------------------------------

mod common;

use common::{edit, Fixture};

// ------------------------------------------------ B1（P1）noeol 末词 w 族跨度差一字

/// **B1（P1）**：noeol 缓冲上 `w` 停在末字符（字节模型），vim 内部缓冲恒有
/// 尾换行——`w` 落到缓冲末的幻影空行，于是 `dw`/`cw` 覆盖整个末词。引擎
/// 只删到末字符前一位，**静默留下末字符**（noeol 文件上的数据破坏形状）。
/// oracle：printf 'abc def'（无尾换行）+ `5|dw` + :wq → 文件字节
/// "abc \n"；printf 'abc' + `dw` → "\n"；`5|cwXY<Esc>` → "abc XY\n"。
/// 引擎：dw 得 "abc f" / "c"，cw 得 "abc XYf"。
#[test]
fn b1_noeol_last_word_dw_cw_one_char_short() {
    // 断言口径：oracle 采样的是文件写回字节，vim 的 fixendofline 在写回时
    // 补上末换行；引擎缓冲无文件模型，按内部文本断言（与 audit2 C 系列
    // noeol 形状的约定一致）。语义点：`w` 落缓冲末的幻影行，dw/cw 覆盖
    // 整个末词。
    let f = edit("abc def", 0, 4, &["d", "w"]);
    assert_eq!(f.text(), "abc ", "vim: noeol 末词 dw 删掉整个 def（w 落幻影行）");

    let f = edit("abc", 0, 0, &["d", "w"]);
    assert_eq!(f.text(), "", "vim: 单词 noeol 缓冲 dw 删空");

    let mut f = edit("abc def", 0, 4, &["c", "w"]);
    f.type_text("XY");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abc XY", "vim: noeol 末词 cw ≡ ce 覆盖整个 def，f 不得残留");
}

// ------------------------------------------------ B2（P2）行尾 `dl`/`cl`/`d<Space>` 应像 x

/// **B2（P2）**：`l`/`<Space>` 在行尾无法移动时，算子下的 vim 语义是
/// **包含光标字符**（`dl` ≡ `x`，`:h x` 明示 dl 同义）。引擎的 Motion::Right
/// 返回 stuck → 算子响铃取消，随后键漏成普通命令。
/// oracle：'ab' + `$dl` → "a"；`$d `（d+Space）→ "a"；`$clXY<Esc>` → "aXY"。
/// 引擎：三个形状全不动（cl 还把 X 变成 X 退格删除）。
#[test]
fn b2_exclusive_l_under_operator_includes_char_at_line_end() {
    let f = edit("ab\n", 0, 1, &["d", "l"]);
    assert_eq!(f.text(), "a\n", "vim: dl 于行尾 ≡ x，删最后字符");

    let f = edit("ab\n", 0, 1, &["d", " "]);
    assert_eq!(f.text(), "a\n", "vim: d<Space> 于行尾同 dl（Space 是 l 的别名）");

    let mut f = edit("ab\n", 0, 1, &["c", "l"]);
    f.type_text("XY");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "aXY\n", "vim: cl 于行尾改写最后字符");
}

// ------------------------------------------------ B3（P2）`!` 过滤算子缺失

/// **B3（P2）**：`!{motion}{cmd}` 过滤算子完全未绑定（Operator 枚举无
/// Filter，tables.rs 无 `!` 行）。vim 9.1：'hello' 上 `!!echo XY<CR>` 把
/// 当前行替换为 shell 命令输出 "XY"。
/// oracle：printf 'hello\n'；按键 `!!echo XY\r:wq\n` → 文件 "XY\n"。
/// 引擎：`!` 响铃丢弃，后续键变成普通模式垃圾键（e/c/h/o 移动、X 删字符、
/// Y 进插入），缓冲被误改。
#[test]
fn b3_filter_operator_unbound() {
    let f = edit("hello\n", 0, 0, &["!", "!", "e", "c", "h", "o", " ", "X", "Y", "<CR>"]);
    assert_eq!(f.text(), "XY\n", "vim: !!echo XY<CR> 用命令输出替换当前行");
}

// ------------------------------------------------ B4（P2）`=` 缩进算子缺失

/// **B4（P2）**：`={motion}` 重缩进算子完全未绑定（无 Indent 算子行，
/// `==`/`={motion}`/可视 `=` 全部响铃）。vim 9.1 用内建缩进规则重排：
/// 无 filetype 时可用 'lisp'（内建选项）做确定性 oracle——
/// oracle：printf '(defun foo\nbar)\n'；`:set lisp\r2gg==:wq\n` →
/// 第二行得 " bar)"（lisp 缩进对齐）。引擎：'lisp' 选项与 `=` 算子双缺，
/// 缓冲原样（后续还有 E518 噪声）。
#[test]
fn b4_indent_operator_unbound() {
    let mut f = Fixture::at("(defun foo\nbar)\n", 1, 0);
    f.feed([":"]);
    f.type_text("set lisp");
    f.feed(["<CR>", "2", "g", "g", "=", "="]);
    assert_eq!(f.text(), "(defun foo\n bar)\n", "vim: == 按 'lisp' 规则重缩进当前行");
}

// ------------------------------------------------ B5（P3）`gm` 半屏行跳转缺失

/// **B5（P3）**：`gm`（光标移到当前屏幕行的中点）未绑定。引擎里 `g` 未命中
/// 丢弃后 `m` 变成 MarkSet 等待参数，光标原地不动。
/// oracle：200 字符单行（80 列终端折行）`1|gm` → getcurpos col 41
/// （0-based 40，首屏行中点）。引擎：光标留在 0。
#[test]
fn b5_gm_middle_of_screenline_unbound() {
    let line = "x".repeat(200);
    let f = edit(&format!("{line}\n"), 0, 0, &["g", "m"]);
    assert_eq!(f.cursor(), 40, "vim: gm 落屏幕行中点（第 41 列）");
}

// ------------------------------------------------ B6（P2）`gj`/`gk` 不按屏幕行移动

/// **B6（P2）**：引擎把 `gj`/`gk` 直接绑成 `j`/`k`（tables.rs 注释
/// "no soft wrap in v1"）。vim 在 'wrap'（默认开）下按**屏幕行**移动：
/// 200 字符单行上 `gj` 留在本缓冲行、落到下一显示行首（col 81 1-based）。
/// oracle：200 个 'x' + `\nnext\n`；`1|gj` → getcurpos [0,1,81,…]。
/// 引擎：j 跳到缓冲第 2 行。
#[test]
fn b6_gj_gk_move_screen_lines_not_buffer_lines() {
    let line = "x".repeat(200);
    let f = edit(&format!("{line}\nnext\n"), 0, 0, &["g", "j"]);
    assert_eq!(f.line(), 0, "vim: gj 停在同一缓冲行（软折行内下移一屏行）");
    assert_eq!(f.cursor(), 80, "vim: gj 落在第二显示行首（0-based 列 80）");
}

// ------------------------------------------------ B7（P2）搜索算子跨行列 1 不提升 linewise

/// **B7（P2）**：`d/pat` 的 span 在 cmdline.rs::search_motion_operator 里
/// 手工拼 `OpSpan { linewise: false }`，**绕过**了 ops::span_from_motion 的
/// 列 1 exclusive→linewise 提升规则（`:h exclusive`）。vim 中该形状是
/// LINEWISE 删除：寄存器按行记账，`p` 整行贴出。
/// oracle：'aaa\nxyz\n' `1|d/xyz<CR>p` → 文件 "xyz\naaa\n"（aaa 整行贴到
/// xyz 之下）。引擎：charwise 删除 + charwise 粘贴 → "xaaayz\n"。
#[test]
fn b7_search_motion_column1_missing_linewise_promotion() {
    let f = edit("aaa\nxyz\n", 0, 0, &["d", "/", "x", "y", "z", "<CR>", "p"]);
    assert_eq!(f.text(), "xyz\naaa\n", "vim: d/pat 列 1 落点提升为 linewise，p 整行贴出");
}

// ------------------------------------------------ B8（P2）裸 CR 被当空白类

/// **B8（P2）**：word.rs::char_class 用 `char::is_whitespace`——裸 `\r`
/// （文件里的 ^M，非换行）归 Blank，`w`/`b` 自由越过。vim 的 utf_class 只把
/// ' ' 和 '\t' 记空白，\r 是 class 1（标点）——`w` 必须停在 ^M 上。
/// oracle：printf 'ab\rcd\n'；`1|w` → getcurpos col 3（0-based 2，正停在
/// ^M）。引擎：w 落到 'c'（0-based 3）。
#[test]
fn b8_bare_cr_is_punct_for_word_motions() {
    let f = edit("ab\rcd\n", 0, 0, &["w"]);
    assert_eq!(f.cursor(), 2, "vim: w 停在裸 ^M（标点类），不越过");
}

// ------------------------------------------------ B9（P2）`go` 字节跳转缺失且危险降级

/// **B9（P2）**：`go`（`:h go`，光标移到缓冲第 [count] 字节）未绑定。
/// 引擎 trie miss 丢弃 `g` 后 `o` 带着前缀 count 执行——**5go 会开 5 个新行
/// 并进入插入模式**（count 在 miss 重试中被丢弃，实测开 1 行）——
/// audit2 F1 的 gd 同款形状，但这条毁数据。
/// oracle：'abcdefgh\n' 上 `5go` → getcurpos col 5（0-based 4，字节 5），
/// 缓冲不变。引擎：开 5 行 + Insert。
#[test]
fn b9_go_byte_motion_unbound_degrades_to_count_open_line() {
    let f = edit("abcdefgh\n", 0, 0, &["5", "g", "o"]);
    assert_eq!(f.text(), "abcdefgh\n", "vim: go 只跳字节，不开行");
    assert_eq!(f.cursor(), 4, "vim: 5go 落在第 5 字节（'e'）");
}

// ------------------------------------------------ B10（P2）z<CR>/z./z- 缺失且 z. 降级为 `.`

/// **B10（P2）**：滚动族 `z<CR>`（当前行滚到窗顶）、`z.`（滚到窗中）、
/// `z-`（滚到窗底）未绑定（引擎只有 zz/zt/zb）。`z.` 的降级最危险：
/// `z` 丢弃后 `.` 执行 **RepeatChange——重复上一次修改**。
/// oracle：'hello' 上 `xz.` → 文件 "ello"（z. 只滚动，不重复 x）；
/// 'a\nb\nc\n' 第 2 行 `z<CR>` → 缓冲/光标行均不变（getcurpos 行 2）。
/// 引擎：z. 把 x 又删一个字符（"llo"）；z<CR> 触发 CR motion 下移一行。
#[test]
fn b10_z_enter_zdot_zminus_unbound_zdot_repeats_change() {
    let f = edit("hello\n", 0, 0, &["x", "z", "."]);
    assert_eq!(f.text(), "ello\n", "vim: z. 是滚动命令，绝不重复上一次修改");

    let f = edit("a\nb\nc\n", 1, 0, &["z", "<CR>"]);
    assert_eq!(f.line(), 1, "vim: z<CR> 只滚动视图，光标不动");
}

// ------------------------------------------------ B11（P2）`d]]` 是 exclusive motion

/// **B11（P2）**：vim 的 `]]` 是 **exclusive** motion（`:h ]]`，且注明
/// exclusive-linewise 适用）——算子跨度到节首行行首，终点在列 1 时按规则
/// (a) 折回上一行行尾并转 inclusive：起点行**行头保留**。引擎把 `]]` 建成
/// Linewise（motions.rs Section 非 method 臂），整行整行地删。
/// oracle：'aaa\nbbb\n{\nccc\n' 光标 (0,1) `d]]` → 文件 "a\n{\nccc\n"
/// （"a" 头与 `{` 行都幸存）。引擎：linewise 删 1-3 行 → "ccc\n"。
#[test]
fn b11_section_forward_operator_is_exclusive() {
    let f = edit("aaa\nbbb\n{\nccc\n", 0, 1, &["d", "]", "]"]);
    assert_eq!(f.text(), "a\n{\nccc\n", "vim: d]] exclusive 规则 (a) 保行头与节行");
}

// ------------------------------------------------ B12（P2）`]m`/`]M` 不搜光标行

/// **B12（P2）**：motions.rs 的 Section 臂 `line += step` 先行——**光标自己
/// 那一行从不参与搜索**。方法括号按定义不在列 1，所以 `]m`/`]M` 最常见的
/// 目标（本行光标之后的方法括号）永远找不到：响铃原地不动。vim 的 `]m`
/// 从光标处向前搜，本行命中。
/// oracle：'foo x {\nbar\n' `1|]m` → getcurpos [0,1,7,…]（0-based 6，本行
/// 的 `{`）。引擎：两个形状都响铃、光标原地。
///
/// 口径注（oracle 复验两轮，2026-10-08）：方法族匹配**任意列**的括号、
/// 扫描光标行。`]M` 的落点规则（源码 nv_bracket_block PHASE1/2 + oracle
/// 矩阵逆向）：第一个括号 b1——b1 是闭括号 → 落 b1；b1 是开括号 → 其
/// 配对存在且配对之后还有括号 → 落**配对**，否则落 b1（未终结的方法落
/// 开括号）。`if (a) {\nx\n} y\n`：b1 = `{`，配对 `}` 存在但其后无括号
/// → 落 `{`（offset 7，oracle 1|8 两次复验）。`]m` = 第一个括号。
#[test]
fn b12_method_motions_never_search_the_cursor_line() {
    let f = edit("foo x {\nbar\n", 0, 0, &["]", "m"]);
    assert_eq!(f.cursor(), 6, "vim: ]m 命中光标行上的方法开括号");

    let f = edit("if (a) {\nx\n} y\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 7, "oracle 1|8: 未终结方法的开括号");

    // 配对存在且其后还有括号 → 落配对（oracle 3|1）
    let f = edit("if (a) {\nx\n} y\nz }\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 11, "oracle 3|1: 落方法的闭配对");

    // 纯闭括号在前 → 落它（oracle 1|9）
    let f = edit("abc def }\nmore\n", 0, 0, &["]", "M"]);
    assert_eq!(f.cursor(), 8, "oracle 1|9: b1 即闭括号");
}

// ------------------------------------------------ B14（P2）`d]m` 吞掉 `{`

/// **B14（P2）**：vim 的 `]m` 是 **exclusive**（`:h ]m`）——算子删到 `{`
/// 之前，`{` 本身保留（普通跳转光标照常停在 `{` 上）。引擎把 method 臂
/// 建成 Inclusive（motions.rs：`Some(o) if method => …Inclusive`），
/// `d]m` 把 `{` 一起删掉。
/// oracle：'foo\nbar {\nbaz\n' `1|d]m` → 文件 "{\nbaz\n"（删 "foo\nbar "）。
/// 引擎：inclusive span → 删成 "\nbaz\n"（`{` 被吞）。
#[test]
fn b14_method_forward_operator_is_exclusive() {
    let f = edit("foo\nbar {\nbaz\n", 0, 0, &["d", "]", "m"]);
    assert_eq!(f.text(), "{\nbaz\n", "vim: d]m 删到开花括号之前，开花括号幸存");
}

// ------------------------------------------------ B13（P1）`d}` 幻影落点从行中起删并线

/// **B13（P1）**：`} ` 冲出缓冲末时落在末行之后的幻影行（列 1）。起点在
/// 行中（非首非空白）时 vim 走 exclusive 规则 (a)：终点折回**最后一行行尾**
/// 并转 inclusive——行结构保留（"a\n"）。引擎的 phantom 分支用
/// `clamp_to_line_end` 把落点拉回缓冲**末行**后再算 `line_end(target_line-1)`，
/// 得到的是**光标自己那行**的行尾——跨行换行被删，两行**并线**（数据损坏形状）。
/// oracle：'aaa\nbbb\n' 光标 (0,1) `d}` → 文件 "a\n"；noeol 'aaa\nbbb' 同键
/// → "a\n"（fixendofline 补回）。引擎：两形状均得 "abbb\n"/"abbb"。
#[test]
fn b13_brace_phantom_midline_start_joins_lines() {
    let f = edit("aaa\nbbb\n", 0, 1, &["d", "}"]);
    assert_eq!(f.text(), "a\n", "vim: 段落算子从行中冲出缓冲末，行结构保留");

    let f = edit("aaa\nbbb", 0, 1, &["d", "}"]);
    // 内部文本口径（fixendofline 是写回行为，见 B1 注）
    assert_eq!(f.text(), "a", "vim: noeol 同形状（行结构保留，无并线）");
}

