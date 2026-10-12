//! 第六轮审计探针（Ex / 命令行 / :s / :set / marks / registers 域）。
//!
//! 每个探针先用 `/usr/bin/vim`（VIM 9.1）实证，再钉住引擎行为；修复前
//! 应当失败，修复后转为回归。全部为绿即代表本轮新增 bug 已清零。

mod common;

use common::Fixture;

/// 把一条 Ex 命令行逐字符送进提示符（`f.feed` 的 item 是「一个键」）。
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn last_status(f: &Fixture) -> String {
    f.host.statuses.last().cloned().unwrap_or_default()
}

// ======================================================== :set 数值语法

// oracle（`vim -Nu NONE -N -i NONE -n -s`，`:set ts=-1`）：
//   E487: Argument must be positive: ts=-1
// 引擎旧行为：`value.parse::<usize>("-1")` 失败 → E521（把负号当语法错误）。
// 数值选项的解析语法 = 可选 `-` + 数字、整串消费；负值一律 E487。
#[test]
fn set_negative_value_is_e487_not_e521() {
    for (opt, val) in [("ts", "-1"), ("sw", "-1"), ("tw", "-1"), ("so", "-1")] {
        let mut f = Fixture::new("a\n");
        ex(&mut f, &format!("set {opt}={val}"));
        assert_eq!(
            last_status(&f),
            format!("E487: Argument must be positive: {opt}={val}"),
            "`:{opt}={val}` 应报 E487（oracle）；实际 {:?}",
            f.host.statuses
        );
    }
}

// oracle：`:set ts=+4` → E521: Number required after =: ts=+4
// 引擎旧行为：Rust 的 `"+4".parse::<usize>()` = Ok(4)，静默接受。
#[test]
fn set_leading_plus_is_e521() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=+4");
    assert_eq!(
        last_status(&f),
        "E521: Number required after =: ts=+4",
        "`+4` 不是合法数字（oracle E521）：{:?}",
        f.host.statuses
    );
}

// oracle：`-0` 解析为 0，故 `:set ts=-0` → E487（tabstop 必须为正），
// 而 `:set sw=-0` → 接受（0 合法）。负号标记法会把 `-0` 误判为负。
#[test]
fn set_negative_zero_follows_zero_rules() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=-0");
    assert_eq!(
        last_status(&f),
        "E487: Argument must be positive: ts=-0",
        "`:set ts=-0` = ts=0 → E487：{:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set sw=-0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E4")),
        "`:set sw=-0` = sw=0 → 合法：{:?}",
        f.host.statuses
    );
}

// oracle：E487/E474 消息引用**用户键入的名字**（别名或全名）：
//   `:set tabstop=0` → "E487: Argument must be positive: tabstop=0"
// 引擎旧行为把名字硬编码成 "ts"。
#[test]
fn set_error_message_quotes_typed_name() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set tabstop=0");
    assert_eq!(
        last_status(&f),
        "E487: Argument must be positive: tabstop=0",
        "消息应引用键入名 tabstop：{:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set tabstop=10000");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument: tabstop=10000",
        "消息应引用键入名 tabstop：{:?}",
        f.host.statuses
    );
}

// oracle：'tabstop' 的真正上界是 9999（`:set ts=9999` 接受，`:set ts=10000`
// → "E474: Invalid argument: ts=10000"）；引擎旧上界 1e6 让 10000 悄悄通过。
#[test]
fn set_tabstop_ceiling_is_9999() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=9999");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E474")),
        "`:set ts=9999` 合法：{:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=10000");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument: ts=10000",
        "`:set ts=10000` 超上界（oracle E474）：{:?}",
        f.host.statuses
    );
}

// ======================================================== :marks 过滤

// oracle（redir）：`:marks z`（z 是合法但未设的名字）→
//   E283: No marks matching "z"（无表头）
// 引擎旧行为：无条件先打表头，然后一行不列，静默。
#[test]
fn marks_unknown_filter_reports_e283() {
    let mut f = Fixture::new("aa\n");
    ex(&mut f, "marks z");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E283: No marks matching \"z\""),
        "`:marks z` 应报 E283：{:?}",
        f.host.statuses
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s == "mark line  col file/text"),
        "E283 时不应输出表头：{:?}",
        f.host.statuses
    );
}

// oracle：`:marks az`（a 存在、z 不存在）→ 表头 + a 行，无 E283。
#[test]
fn marks_partial_filter_lists_existing_without_error() {
    let mut f = Fixture::new("aa\nbb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "marks az");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "mark line  col file/text"),
        "`:marks az` 应输出表头：{:?}",
        f.host.statuses
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with(" a") && s.contains("aa")),
        "`:marks az` 应列出 a：{:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E283")),
        "`:marks az` 有命中，不报 E283：{:?}",
        f.host.statuses
    );
}

// ======================================================== :delmarks

// oracle：`:2m/c`（**搜索地址**作目标）→ 行 2 移到 /c 命中行之后：
//   [a,c,b,d,e]，无报错。引擎旧行为把 `/c` 当非法地址报 E16。
#[test]
fn move_to_search_address() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/c");
    assert_eq!(f.text(), "a\nc\nb\nd\ne\n", "`:2m/c` 应移到 /c 之后");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "搜索地址合法：{:?}",
        f.host.statuses
    );
    // `:2t/c` 复制到 /c 之后
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2t/c");
    assert_eq!(f.text(), "a\nb\nc\nb\nd\ne\n", "`:2t/c` 应复制到 /c 之后");
}

// oracle：`:m` 目标地址的错误消息各不相同（引擎旧行为一律 E16）：
//   `:2m'a`（未设）→ E20: Mark not set；`:2m'@` → E78: Unknown mark；
//   `:2m/`（无上次搜索）→ E35: No previous regular expression；
//   `:2m/zzz`（未命中）→ E486: Pattern not found: zzz。
#[test]
fn move_destination_error_messages() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m'a");
    assert_eq!(last_status(&f), "E20: Mark not set");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m'@");
    assert_eq!(last_status(&f), "E78: Unknown mark");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/");
    assert_eq!(last_status(&f), "E35: No previous regular expression");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/zzz");
    assert_eq!(last_status(&f), "E486: Pattern not found: zzz");
}

// 引擎旧行为：把 `!` 当"清空 a-z"、再把 x 删掉，无报错。
#[test]
fn delmarks_bang_with_argument_is_e474() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks! x");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument",
        "`:delmarks! x` 应报 E474：{:?}",
        f.host.statuses
    );
    // a 未被删除（命令整体失败）
    f.feed([":", "'", "a", "<CR>"]);
    assert_eq!(f.line(), 0, "失败后标记 a 仍在原位");
}

// oracle：`:delmarks !`（空格后的 ! 不是 forceit，是非法字符）→
//   E475: Invalid argument: !
// 引擎旧行为：token "!" 触发清空 a-z。
#[test]
fn delmarks_spaced_bang_is_e475() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks !");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: !",
        "`:delmarks !` 应报 E475：{:?}",
        f.host.statuses
    );
}

// oracle：`:delmarks @` → E475: Invalid argument: @（引擎旧行为静默忽略）。
// `:delmarks a @` → 先删 a，再在 @ 处 E475（消息 = 从该字节起的剩余参数）。
#[test]
fn delmarks_invalid_char_is_e475_with_tail() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks @");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: @",
        "`:delmarks @` 应报 E475：{:?}",
        f.host.statuses
    );

    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks a @");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: @",
        "`:delmarks a @` 应报 E475（尾部 = @）：{:?}",
        f.host.statuses
    );
}

// oracle：`:delmarks ab` 逐字符处理 → 同时删 a 和 b（引擎旧行为按空白切分
// 后把 "ab" 当尾随字符报错）。
#[test]
fn delmarks_concatenated_names_delete_each() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed(["m", "a", "j", "m", "b", "j", "m", "c"]);
    ex(&mut f, "delmarks ab");
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "a 应被删除（跳转报 E20）：{:?}",
        f.host.statuses
    );
    f.feed([":", "'", "c", "<CR>"]);
    assert_eq!(f.line(), 2, "c 未被删除，仍可跳转");
}

// oracle：`"` 起注释，终止参数 → `:delmarks a " note` 只删 a（不误删
// c/o/m/e/n/t），`:delmarks "` → E471。
#[test]
fn delmarks_double_quote_starts_comment() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed(["m", "a", "j", "j", "m", "c"]);
    ex(&mut f, "delmarks a \" note");
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "a 应被删除：{:?}",
        f.host.statuses
    );
    f.feed([":", "'", "c", "<CR>"]);
    assert_eq!(f.line(), 2, "c 不应被注释文本误删");

    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "delmarks \"");
    assert_eq!(
        last_status(&f),
        "E471: Argument required",
        "`:delmarks \"` 注释后参数为空 → E471：{:?}",
        f.host.statuses
    );
}
