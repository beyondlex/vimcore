//! 第二轮独立审计 D 组（Ex 命令 / 范围地址 / 命令行编辑键 / `:set` 选项）。
//!
//! 15 个新发现，每例一个 `#[test]`，断言按 vim 9.1（`-Nu NONE -N`）的
//! oracle 实证结果书写——当前引擎全部不满足，跑挂即复现。
//! 证据标注见各测试注释与提交报告。

mod common;

use common::{edit, Fixture};

// ------------------------------------------------------- :pu 的范围地址

/// D1（P1）`:{range}pu` 的范围被整体丢弃：`:0pu` 应粘到**文件顶部**（第 1
/// 行之上），`:5pu` 应粘到第 5 行之下；引擎一律粘在**光标行**旁。
/// oracle（vim 9.1）：五行缓冲 `:1y a` + `:0pu a` → `a|a|b|c|d|e`、光标第 1 行。
#[test]
fn put_range_address_moves_insertion_point() {
    // 光标放在缓冲中部：vim 粘到文件顶部，引擎（丢范围）会粘在光标行旁
    let mut f = edit("a\nb\nc\nd\ne\n", 2, 0, &[":", "1", "y", " ", "a", "<CR>"]);
    f.feed([":", "0", "p", "u", " ", "a", "<CR>"]);
    assert_eq!(f.text(), "a\na\nb\nc\nd\ne\n", ":0pu a 必须粘在第 1 行之上");
    assert_eq!(f.line(), 0, "光标应落新粘的第 1 行");
}

/// D2（P2）`:put` 的光标应落**最后一条**粘入行的首个非空白（`:h :put`）；
/// 引擎复用普通 `p` 路径，落在**第一条**粘入行。
/// oracle：三行寄存器 `:$pu` 后 `line(".")=8`（粘入 6-8 行，光标在第 8 行）。
#[test]
fn put_cursor_on_last_pasted_line() {
    let mut f = edit("one\ntwo\nthree\n", 2, 0, &[":", "1", ",", "3", "y", " ", "b", "<CR>"]);
    f.feed([":", "p", "u", " ", "b", "<CR>"]);
    // 粘在第 3 行之下：新行 4/5/6，vim 光标在第 6 行（0 基 5）
    assert_eq!(f.line(), 5, ":pu 多行寄存器后光标应停最后一条粘入行");
}

// ------------------------------------------------------- 命令行编辑键

/// D3（P1）cmdline `<Up>` 应按**已输入前缀**过滤历史（vim 9.1 cmdline.txt
/// c_<Up>："recall older command-line from history, whose beginning matches
/// the current command-line"）；引擎无视前缀直接回放最新一条——会在 CR 时
/// 执行一条 vim 根本不会选的命令。
/// 探针设计：历史 `:1d`/`:2d`/`:3d`（最新 3d），缓存区剩 `m2/m4` 后输入
/// `:2` + `<Up>` + CR：vim 回放 `:2d` 删掉 m4；引擎回放 `:3d` 在两行缓冲上
/// E16、什么都不删。
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
    assert_eq!(f.text(), "m2\nm4\n", "前置三条 :d 两侧一致");
    f.feed([":", "2", "<Up>", "<CR>"]);
    assert_eq!(f.text(), "m2\n", "前缀 2 应回放 :2d（删 m4），而非最新的 :3d");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "不应回放 :3d 并在两行缓冲上报 E16：{:?}",
        f.host.statuses
    );
}

/// D4（P2）`:delm A-Z` 只删**大写**文件标记；引擎把区间两端 `to_ascii_lowercase`
/// 后删掉 a-z——小写标记 `a` 被误删（数据丢失）。
/// oracle：`ma` + `:delm A-Z` 后 `:marks` 仍列出标记 a。
#[test]
fn delmarks_uppercase_range_spares_lowercase() {
    let mut f = edit("a\nb\n", 0, 0, &["m", "a"]);
    f.feed([":", "d", "e", "l", "m", " ", "A", "-", "Z", "<CR>"]);
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E20")),
        "小写标记 a 不在 :delm A-Z 的删除范围，不应 E20：{:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- :set 选项

/// D5（P3）`:set inv{name}` 是 vim 的布尔反转拼写（oracle：`:set invic` 无错、
/// ic 从默认 off 翻成 on）；引擎 E518 "Unknown option: invic"。
#[test]
fn set_inv_spelling_toggles_boolean() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "i", "n", "v", "i", "c", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        ":set invic 不该报错：{:?}",
        f.host.statuses
    );
}

/// D6（P3）`:set {bool}&` 把布尔选项重置为默认（oracle：`:set ic&` 无错、
/// ic 回到默认 off）；引擎只对数值选项实现了 `&`，布尔形式 E518。
#[test]
fn set_ampersand_resets_boolean_option() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "i", "c", "&", "<CR>"]);
    assert!(
        f.host.statuses.is_empty(),
        ":set ic& 不该报错：{:?}",
        f.host.statuses
    );
}

/// D7（P3）`:set foo?`（未知名的查询形式）vim 报
/// "E518: Unknown option: foo?"（oracle）；引擎只响铃，零状态消息。
#[test]
fn set_unknown_name_query_reports_e518() {
    let mut f = edit("a\n", 0, 0, &["m"]);
    f.feed([":", "s", "e", "t", " ", "f", "o", "o", "?", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E518: Unknown option: foo?"),
        "未知选项的查询形式要报 E518：{:?}",
        f.host.statuses
    );
}

/// D8（P3）`:set ts?` 的 vim 输出带**两个前导空格**（"  tabstop=8"，oracle
/// redir 实证；布尔 on 同样 "  ignorecase"）；引擎裸发 "tabstop=4"。
#[test]
fn set_query_output_two_space_padding() {
    let f = edit("a\n", 0, 0, &[":", "s", "e", "t", " ", "t", "s", "?", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s == "  tabstop=4"),
        "查询输出应是 vim 的 '  tabstop=4'（两个前导空格）：{:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- :le / :ce 对齐

/// D9（P1）`:le {indent}` 的缩进参数被完全忽略（cmdline.rs ex_align 里
/// `let _ = indent;`）：vim `:le 4` 于 "ab" → "    ab"（oracle LE1），
/// `:1,2left 2` → 两行各补 2 空格（oracle LE3）；引擎只做去缩进左对齐。
#[test]
fn left_align_honors_indent_argument() {
    let f = edit("ab\n", 0, 0, &[":", "l", "e", " ", "4", "<CR>"]);
    assert_eq!(f.text(), "    ab\n", ":le 4 应前缀 4 个空格");

    let f = edit("ab\ncd\n", 0, 0, &[":", "1", ",", "2", "l", "e", " ", "2", "<CR>"]);
    assert_eq!(f.text(), "  ab\n  cd\n", "带范围的 :le 2 每行都补 2 空格");
}

/// D10（P1）`noet` 下 `:ce`/`:ri` 的填充要按 'tabstop' 先制表后空格
/// （oracle od 字节：`ab` + `:ce 40`，noet ts=8 → `\t\t   ab`＝2 制表 + 3
/// 空格；et 下才是全空格）；引擎一律 `" ".repeat` 纯空格。
#[test]
fn center_padding_composes_tabs_under_noet() {
    let mut f = edit("ab\n", 0, 0, &[":", "s", "e", "t", " ", "n", "o", "e", "t", "<CR>"]);
    f.feed([":", "c", "e", " ", "4", "0", "<CR>"]);
    assert_eq!(
        f.text(), "\t\t   ab\n",
        "noet 下 :ce 40 的 19 列填充 = 2 个制表 + 3 空格"
    );
}

// ------------------------------------------------------- :retab

/// D11（P3）`:retab 0` / `:retab! 0` 在 vim 里合法（0 = 按当前 'tabstop'
/// 重排，oracle：noet 下 8 空格行 `:retab! 0` → `\tab`、无报错）；引擎对
/// 0 报 E488 Trailing characters 且不动缓冲。
#[test]
fn retab_zero_uses_current_tabstop() {
    let mut f = edit("        ab\n", 0, 0, &[":", "s", "e", "t", " ", "n", "o", "e", "t", "<CR>"]);
    f.feed([":", "r", "e", "t", "a", "b", "!", " ", "0", "<CR>"]);
    assert_eq!(f.text(), "\tab\n", ":retab! 0 应按默认 ts=8 把 8 空格收成 1 个制表");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E488")),
        "retab 0 合法，不该报 E488：{:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- 退出/列表命令族

/// D12（P2）`:wqa`（连同 `:qa`/`:exit`/`:wqall`/`:xa` 等拼写）在 vim 里
/// 保存并退出（oracle：盘上文件被改写）；引擎 E492，宿主收不到关闭请求。
#[test]
fn quit_all_spellings_wqa_exit_exist() {
    let f = edit("a\n", 0, 0, &[":", "w", "q", "a", "<CR>"]);
    assert!(
        f.host.close_requested,
        ":wqa 应请求关闭缓冲（vim 保存并退出）"
    );
    assert!(
        f.host.statuses.is_empty(),
        ":wqa 不是未知命令：{:?}",
        f.host.statuses
    );

    let f = edit("a\n", 0, 0, &[":", "e", "x", "i", "t", "<CR>"]);
    assert!(f.host.close_requested, ":exit 是 :x 的合法拼写");
    assert!(
        f.host.statuses.is_empty(),
        ":exit 不是未知命令：{:?}",
        f.host.statuses
    );
}

/// D13（P3）`:marks {name}` 按名列出标记（oracle：`:marks a` 只输出 a 一行）；
/// 引擎只认裸 `:marks`，带参数直接 E492。
#[test]
fn marks_accepts_name_arguments() {
    let mut f = edit("aa\n", 0, 0, &["m", "a"]);
    f.feed([":", "m", "a", "r", "k", "s", " ", "a", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("a  line")),
        ":marks a 应列出标记 a：{:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        ":marks {{name}} 不该 E492：{:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- 命令行特殊键

/// D14（P3）c_CTRL-V 把下一个键**字面**插入命令行（oracle：`:` + C-v + Esc
/// + CR → 命令行含字面 ^[，执行报 "E492: Not an editor command: ^["）；
/// 引擎吞掉 C-v，Esc 直接取消整个提示符，随后 CR 落回普通模式。
#[test]
fn cmdline_ctrl_v_quotes_next_key() {
    let mut f = edit("a\n", 0, 0, &[]);
    f.feed([":", "<C-v>", "<Esc>", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E492")),
        "C-v 引注的 Esc 应作为字面 ^[ 进入命令行并被执行（E492）：{:?}",
        f.host.statuses
    );
}

// ------------------------------------------------------- 范围地址

/// D15（P2）地址偏移里 `+` 与数字之间的空格：vim 的 `:5 + 2` 解析为 8
/// （oracle：裸地址光标落第 8 行；`:5 + 1d` 删第 7 行；`:5 + 2d 2` 删第
/// 8-9 行——三探针一致）；引擎把 `+` 当裸偏移（+1）提前截断，`:5 + 2d`
/// 会删**第 6 行**——差着两条行。
#[test]
fn address_offset_accepts_space_before_count() {
    let text = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
    let f = edit(text, 0, 0, &[":", "5", " ", "+", " ", "2", "d", "<CR>"]);
    assert_eq!(
        f.text(),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl9\nl10\n",
        "vim 的 :5 + 2d 删第 8 行（l8）"
    );
}

// ------------------------------------------------------- 保留：Fixture 被引用性

/// 占位断言（防止 unused import 警告漂移）：Fixture 类型在 D14 里以
/// `edit(..)` 的返回值间接使用，这里显式钉一次。
#[test]
fn fixture_type_is_referenced() {
    let f = Fixture::new("x\n");
    assert_eq!(f.text(), "x\n");
}
