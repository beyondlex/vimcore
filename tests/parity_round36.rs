//! 第三十六轮 fuzz 抓取的两个宏问题回归（2026-10-11）。
//!
//! fuzz round36 的宏会话能录出"自播放"的寄存器（`@a` 落进 `"a`），引擎侧
//! 于是把队列乘性展开到 ~50 GB 才撞上按键计数阀。根因是两条 vim 语义缺失：
//!
//! 1. **寄存器执行期间 `q` 完全惰性**：vim `nv_record` 的门槛是
//!    `reg_executing == 0`（`src/normal.c`），所以播放中的 `q` 既不开录制、
//!    也不关录制、**还不吞字符参数**（紧随的键照常作为命令执行）。
//! 2. **重放入队预算要看队列余量**：`enqueue_replay` 每次调用都按
//!    `MAX_PIPELINE_STEPS` 全额预展开，自递归时每个 `@` 又加 10 万键，
//!    生产远快于消费。
//!
//! oracle 通道：`/usr/bin/vim`（9.1 patches 1-1752）+ expect PTY，
//! `:redir` 采样（2026-10-11）：
//! - `:let @b='qax'` + `@b` → `a='' rec='' l2='defx'`
//! - `:let @c='q'` + `qa@cxq` → `a='@cx' rec=''`
//! - 控制组 `qayxq` → `a='yx' rec=''`
//!
//! 同轮还收口了一条独立的 Ex 侧无限互递归（`:@a` 自引用 → 栈溢出，
//! 见 R36-4；vim 的 `E169: Command too recursive` 门槛）。

mod common;

use common::Fixture;

// ---- R36-1：播放中的 `q` 不开录制、不吃参数 --------------------------------
//
// 前置：`"byw` 把首行 `qax` 装进 b（寄存器内容即宏文本），游点移到第二行首。
// 修复前：`q` 武装 char-arg 吃掉 `a` → 开录制进 a、`x` 落回 normal 侧删掉
// 游点字符（`def` → `ef`）且 `macro_recording()` 变 Some('a')。
// 修复后：`q` 惰性，`a` 作为普通 append 进插入模式，`x` 在那里是"宿主投递的
// 文本"——本 harness 只喂键、不投递文本，所以缓冲区保持 `def`。
#[test]
fn r36_1_q_does_not_open_a_recording_while_a_register_executes() {
    let mut f = Fixture::new("qax\ndef\n");
    f.feed(["\"", "b", "y", "w"]);
    f.feed(["j"]);
    f.feed(["@", "b"]);
    assert_eq!(
        f.vim.macro_recording(),
        None,
        "oracle P1：reg_recording() 为空——播放中的 q 不得开启录制"
    );
    assert_eq!(
        f.text(),
        "qax\ndef\n",
        "oracle P1：q 不吃 `a`，`x` 不是 normal 侧删除"
    );
}

// ---- R36-2：播放中的 `q` 不停止正在进行的录制 ------------------------------
//
// `:let @c='q'` + `qa@cxq` → `"a` 是 `@cx`（三步）：`@c` 播出的 `q` 没有结束
// 会话，其后的 `x` 继续被录；真正的停止是末尾手按的 `q`。
// 修复前：播出的 `q` 提前停止 → `"a` 只剩 `@c`（两步），末尾的 `q` 反而去
// 武装一次新的录制。
#[test]
fn r36_2_q_does_not_stop_a_live_recording_while_a_register_executes() {
    let mut f = Fixture::new("qfoo\n");
    f.feed(["\"", "c", "y", "l"]);
    assert_eq!(f.vim.macro_len('c'), 0, "尚未同步到宏表（@c 时才落地）");
    f.feed(["q", "a"]);
    assert_eq!(f.vim.macro_recording(), Some('a'), "qa 开录制");
    f.feed(["@", "c"]);
    assert_eq!(
        f.vim.macro_recording(),
        Some('a'),
        "oracle P2：播出的 q 不停止会话"
    );
    f.feed(["x"]);
    f.feed(["q"]);
    assert_eq!(f.vim.macro_recording(), None, "末尾手按的 q 才是停止");
    assert_eq!(
        f.vim.macro_len('a'),
        3,
        "oracle P2：\"a 是 @cx 三步（@、c、x 都按原键录入）"
    );
}

// ---- R36-3：自播放寄存器不再把队列吃成天文数字 ----------------------------
//
// `"a` 内容就是 `@a` → 每个播出的 `@` 都重新入队。引擎的收口是
// `MAX_PIPELINE_STEPS` 安全阀，但预算必须按**队列余量**算：否则每个被消费的
// `@` 又推 10 万键，10 万次循环即 ~10^10 个 `Key`（fuzz round36 的 50 GB）。
// 这条断言的"预期失败形态"是挂死/爆内存，因此只验它能收口。
#[test]
fn r36_3_self_replaying_macro_terminates_with_a_bounded_queue() {
    let mut f = Fixture::new("@afoo\n");
    f.feed(["\"", "a", "y", "2", "l"]);
    f.feed(["@", "a"]);
    f.feed(["<Esc>"]);
    assert!(
        f.vim.is_idle(),
        "自递归宏必须被安全阀收口，不得留下待决队列"
    );
    assert_eq!(f.text(), "@afoo\n", "播放不该改动缓冲文本");
}

// ---- R36-4：`:@` 自引用必须撞 vim 的 E169，而不是栈溢出 --------------------
//
// 寄存器内容就是 `@b` → `execute_ex ↔ ex_at` 无限互递归
// （fuzz round17 的 `has overflowed its stack`）。vim 的收口在
// `do_cmdline()`：`call_depth >= 200` → `emsg("E169: Command too recursive")`
// 并 `return FAIL`（`src/ex_docmd.c:707-737`）。
//
// oracle（expect PTY，2026-10-11）：`:let @b='@b'` + `:@b` →
// 屏幕 `E169: Command too recursive`，`v:errmsg` 同文，缓冲未被改动；
// 引擎侧随后照常工作。（`'@b'` 不带冒号是 fuzz round17 的实际形态：
// 引擎的 `:@` 内容不剥前导冒号，`:let @a=':@a'` 在这里是 E492——
// 与 vim `do_one_cmd()` 的 "Skip ':' and any white space" 分歧，
// 记账 NOTES，不影响本条收口。）
#[test]
fn r36_4_recursive_at_command_stops_at_e169() {
    let mut f = Fixture::new("foo\nbar\n");
    f.vim
        .registers
        .store_named_plain('b', "@b".to_owned());
    f.feed([":", "@", "b", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E169: Command too recursive"),
        "oracle：递归到 200 层报 E169，实得 {:?}",
        f.host.statuses
    );
    assert_eq!(f.text(), "foo\nbar\n", "oracle：递归不得改动缓冲");
    assert!(f.vim.is_idle(), "递归必须完全收口");
    // 栈必须一路回卷（ex_depth 复位），引擎不能卡在错误态
    f.feed(["x"]);
    assert_eq!(f.text(), "oo\nbar\n", "E169 之后 normal 侧照常");
}

// ---- R36-5：合法的浅嵌套不受闸门影响 --------------------------------------
//
// 200 是**上限**而非禁令：`:source`、函数调用都能嵌套，vim 同样如此。
// `:@a` → `:@b` → `:2`/`:3`（深度 3）必须正常执行到末条。
// 内容不带前导冒号——引擎的 `:@` 不剥它（见 R36-4 附注的 NOTES 分歧）。
#[test]
fn r36_5_shallow_at_nesting_still_runs() {
    let mut f = Fixture::new("l1\nl2\nl3\n");
    f.vim.registers.store_named_plain('b', "2".to_owned());
    f.vim
        .registers
        .store_named_plain('a', "@b\n3".to_owned());
    f.feed([":", "@", "a", "<CR>"]);
    assert!(
        !f
            .host
            .statuses
            .iter()
            .any(|s| s.contains("E169: Command too recursive")),
        "浅嵌套不得触发递归闸门：{:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.starts_with("E492")),
        "嵌套的两条子命令都得被识别：{:?}",
        f.host.statuses
    );
    // 两条子命令依次执行，游点落在最后一条 `:3` 的行上
    assert_eq!(f.line(), 2, "末条 :3 生效（0 基行 2）");
}
