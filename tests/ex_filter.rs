//! `:{range}!{filter}` ex 过滤与裸 `:!{cmd}` shell 逃逸（2026-10-09，为 tk 的
//! Bench 补齐 `:%!<cmd>` 拼写）。断言口径与仓库一致：**vim 9.1 实证**
//! （oracle 通道同 audit3_b 的注释）。`!` 提示符（`!!`/`!{motion}`）的过滤
//! 已由 audit3_b B3 覆盖，这里只测 ex 形态。
//!
//! 已对照分歧：vim 的 `:!` 里 `|` 归 shell（`:!echo a|b` 输出 "a|b"），引擎
//! 的 `!` 分支排在 `|` bar 切分之前，一致。

mod common;

use common::{edit, Fixture};

/// `:%!tr a-z A-Z`：整个缓冲过过滤器。这是 tk Bench 的主用例。
#[test]
fn percent_filter_whole_buffer() {
    let f = edit(
        "hello\nworld\n",
        0,
        0,
        &[":", "%", "!", "t", "r", " ", "a", "-", "z", " ", "A", "-", "Z", "<CR>"],
    );
    assert_eq!(f.text(), "HELLO\nWORLD\n", "vim: %!tr 全缓冲过滤");
}

/// `:2!rev`：单行范围过滤，别行不动。
#[test]
fn single_line_range_filter() {
    let f = edit(
        "ab\ncd\n",
        1,
        0,
        &[":", "2", "!", "r", "e", "v", "<CR>"],
    );
    assert_eq!(f.text(), "ab\ndc\n", "vim: 2!rev 只过滤第二行");
}

/// 范围内建钳制（E16 之后仍显式钳制一次以容忍 `:2,99!...`）——不，vim 对
/// 越界地址 + 命令报 E16 且什么都不跑；这里验证的正是 E16 路径未被
/// `!` 分支抢先吞掉。
#[test]
fn out_of_buffer_range_still_e16() {
    let f = edit(
        "ab\ncd\n",
        0,
        0,
        &[":", "2", ",", "9", "!", "r", "e", "v", "<CR>"],
    );
    assert_eq!(f.text(), "ab\ncd\n", "vim: 越界范围 + 命令 = E16，缓冲不动");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E16")),
        "应报 E16：{:?}",
        f.host.statuses
    );
}

/// 裸 `:!{cmd}`：shell 逃逸——缓冲不动，输出进状态行（vim 把输出显示在
/// 消息区；引擎的 status 是单行，取首行 + 行数提示）。
#[test]
fn bare_bang_is_shell_escape() {
    let f = edit(
        "hello\n",
        0,
        0,
        &[":", "!", "e", "c", "h", "o", " ", "h", "i", "<CR>"],
    );
    assert_eq!(f.text(), "hello\n", "vim: 裸 :! 不改缓冲");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("hi")),
        "输出应进状态行：{:?}",
        f.host.statuses
    );
}

/// 空 `:!`：E471（vim: Argument required），缓冲不动。
#[test]
fn bare_bang_empty_argument_e471() {
    let f = edit("hello\n", 0, 0, &[":", "!", "<CR>"]);
    assert_eq!(f.text(), "hello\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E471")),
        "应报 E471：{:?}",
        f.host.statuses
    );
}

/// 过滤参与 undo：`:%!tr` 之后 `u` 回到原文（与 `!!` 提示符路径同一
/// run_shell_filter 的 undo 组语义）。
#[test]
fn percent_filter_is_undoable() {
    let mut f = edit(
        "hello\nworld\n",
        0,
        0,
        &[
            ":", "%", "!", "t", "r", " ", "a", "-", "z", " ", "A", "-", "Z", "<CR>",
        ],
    );
    assert_eq!(f.text(), "HELLO\nWORLD\n");
    f.feed(["u"]);
    assert_eq!(f.text(), "hello\nworld\n", "vim: 过滤可一步 undo");
}

/// `:` 里的 `|` 归 shell：`:!echo 'a|b'` 的输出含 `|`（vim 对 `:!` 不做
/// bar 切分——这正是 `!` 分支要排在 bar 切分之前的原因；不引号时管道由
/// shell 自己解释，vim 与引擎一致）。
#[test]
fn bang_bar_belongs_to_shell() {
    let f = edit(
        "x\n",
        0,
        0,
        &[
            ":", "!", "e", "c", "h", "o", " ", "'", "a", "|", "b", "'", "<CR>",
        ],
    );
    assert_eq!(f.text(), "x\n", "缓冲不动");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("a|b")),
        "引号内的管道符应原样到达 shell：{:?}",
        f.host.statuses
    );
}

/// 编译期占位：保持 Fixture 导入被用到（首个测试前阻止 unused 告警）。
#[allow(dead_code)]
fn _uses(f: &Fixture) {
    let _ = f.text();
}
