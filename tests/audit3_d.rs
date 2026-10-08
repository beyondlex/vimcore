//! audit3_d：寄存器 / 宏 / `.` 重放 / undo-redo / 插入-Replace 模式 独立审计
//! （2026-10-08，第三轮）。
//!
//! 每个发现一个 `#[test]`，断言写的是 **vim 9.1 oracle 的期望结果**，因此在
//! 当前工作树上应当失败。oracle 证据写在各测试注释里：
//! - typeahead 通道：`cd /tmp/ora && printf '<初始缓冲>' > N.txt && python3 -c
//!   "open('k','wb').write(b'<按键字节>:wq\n')" && vim -Nu NONE -N -i NONE -n
//!   -s k N.txt </dev/null`，读回 N.txt / writefile 采样；
//! - undo/交互通道（typeahead 不等键、undo 块合并是通道伪影）：expect 驱动的
//!   真 PTY 逐键 0.35s 投递（/tmp/ora/pty3.exp）。
//!
//! 范围：src/registers.rs、src/state.rs 的宏/`.`/undo 段、src/insert_mode.rs、
//! src/ops.rs 寄存器漏斗、src/cmdline.rs `@:`/`:undo` 面。
//!
//! 已证伪的怀疑（oracle 对照引擎一致，免下轮重查）：
//! - 小删除重复进 "1（经典 1P-点 习惯用法）：oracle B 证伪——`x.` 后
//!   @1=''、@-='b'，引擎一致（复核 15）；
//! - `i<C-v>` 十进制/八进制口径：oracle E1-E3——vim 9.1 `C-v 65`/`C-v 065`/
//!   `C-v x41` 全部得 'A'（十进制 + 显式 o/x 前缀），引擎一致；
//! - 行首 <BS> 并线（bs=2 边界）：oracle F/F2——vim -Nu NONE 默认即允许
//!   BS 跨行首（并线发生），引擎一致；行首无输入时 C-w/C-u 均无操作也已对齐；
//! - undo 光标（普通命令）：oracle M1-M3——`Axy<Esc>u`/`ofoo<Esc>u`/行中
//!   `x u` 的行列与引擎「首个编辑点」模型一致；
//! - undo 粒度（`.` 是否独立 undo 步）：typeahead 通道会合并 undo 块（通道
//!   伪影），PTY 逐键实证 `x.` `u` 只撤点重放那一步；引擎每命令一组，一致；
//! - 算子待决中按 `u`：oracle W1/W2——vim 直接吞掉 u（撤销不执行）并取消
//!   算子；引擎 Pending-trie Miss 同款吞键+取消，一致；
//! - `.` 重放寄存器前缀 / yank 不清 redo / 小删除只进 `"-` / 块可视算子的
//!   `.` 重放 / 计数插入里的 C-u（净文本为空，重复不粘出陈旧文本）/
//!   Replace 模式 C-r 按打字覆盖（oracle Z21："Xac"）：全部一致；
//! - `J`/`r` 不写寄存器（oracle AA1/AA3）：引擎漏斗不经过 store_delete，一致；
//! - Ex `:d` 推移编号环（oracle L2：@1='l2'）：引擎同一漏斗，一致；
//! - `3ifoo<C-w>x<Esc>`（计数插入里 C-w 取消重复，oracle W5="xzz"）与
//!   `3Rab<C-w>x<Esc>`（oracle W6="xzzz"）：引擎同样放弃重复，一致；
//! - 宏跨模式录制（type_text 的 Text 步进 macro_capture）、`qA` 追加、`@@`
//!   链、`:reg` 含 "-/"/ 特殊槽：读码+既有探针一致。
//! 另记轻微分歧（不立项）：`:reg` 行版式（vim 为「两空格+类型+两空格+
//! "名+三空格+内容」，引擎为 "名在前的自有版式，PTY 采样式见 oracle RG）。

mod common;

use common::Fixture;
use std::matches;
use vimcore::key::Key;
use vimcore::mode::Mode;

// ------------------------------------------------ `.` 重放 × count（oracle A / PTY V10）

/// **发现 1（P1）**：`.` 的 [count] 应当**替换**被重放变更里记录的 count
/// （`:h .` "Repeat last change, with count replaced with [count]"），
/// 引擎却把 `last_change` 里的旧 count 键原样重放再乘以新 count。
/// oracle（PTY 逐键 + typeahead 双通道）：`abcdef` 上 `3x` 得 `def`，随后
/// `2.` vim 得 `f`（重放 x、count=2）；引擎把 [3,x] 重放两轮 → `def` 上先删
/// 2 个再删 1 个 → 空行。
#[test]
fn dot_count_replaces_recorded_count_not_multiplies() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["3", "x"]);
    assert_eq!(f.text(), "def\n");
    f.feed(["2", "."]);
    assert_eq!(f.text(), "f\n", "vim 9.1: 2. 重放 x 且 count 取 2，不是 3x×2");
}

/// **发现 1b（P2）**：同一根因的计数插入变体。`3iab<Esc>` 后 `2.`：vim 重放
/// 插入会话、count 替换为 2（净效果 "ab"×5，12 字节）；引擎把 [3,i,Text,Esc]
/// 重放两轮，每轮 exit_insert 又按 count=3 复制 → "ab"×9。
/// oracle（typeahead）：`zz` 上 `3iab<Esc>2.` → getline(1) 长度 12
/// （"ababaababbzz"，5 份 "ab"）；引擎得 20 字节（9 份 "ab"）。
#[test]
fn dot_after_counted_insert_replays_session_with_new_count() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababzz\n");
    f.feed(["2", "."]);
    let text = f.text();
    let line = text.trim_end_matches('\n');
    assert_eq!(
        line.chars().count(),
        12,
        "vim 9.1: 2. 后共 5 份 \"ab\"（12 字节）；引擎把 3i 再跑两轮得 9 份"
    );
}

// ------------------------------------------------ 宏（oracle D / X1 / Z）

/// **发现 2（P1）**：宏执行中任一命令失败（响铃）必须**中止整条重放**——
/// 本轮剩余键与 `@` 的剩余次数都不再执行（`:h @` 错误中止语义）。
/// 引擎把 count 轮全部预入 pending_keys，失败后照常继续。
/// oracle：`xa\nab\n` 上 `qafxDq`（录制轮在 "xa" 上成功：fx 命中 col0，D 删成
/// 空行）；光标 j 到 "ab" 行后 `2@a`：vim 里 round1 的 fx 找不到 x → 整条中止，
/// 缓冲保持 ["","ab"]；引擎继续执行 D 把 "ab" 删空、round2 再跑一遍 →
/// ["",""]。
#[test]
fn macro_error_aborts_remaining_keys_and_rounds() {
    let mut f = Fixture::at("xa\nab\n", 0, 0);
    f.feed(["q", "a", "f", "x", "D", "q"]);
    assert_eq!(f.text(), "\nab\n", "录制轮本身在第一行成功执行");
    f.feed(["j", "2", "@", "a"]);
    assert_eq!(
        f.text(),
        "\nab\n",
        "vim 9.1: fx 失败中止整条 @a，第二行 'ab' 幸存；引擎继续执行 D 把它删空"
    );
}

/// **发现 3（P2）**：`@:`（重放最近 Ex 命令行）接受 [count]，`2@:` 把命令行
/// 执行两遍；引擎的 `@:` 分支完全无视 count、只执行一次。
/// oracle：`a\na\na\n` 上 `:1d<CR>` 后 `2@:` → 三行全删（缓冲空）；引擎只剩
/// 一行（`1d` + 一次重放）。
#[test]
fn at_colon_honors_count() {
    let mut f = Fixture::at("a\na\na\n", 0, 0);
    f.feed([":", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "a\na\n");
    f.feed(["2", "@", ":"]);
    assert_eq!(
        f.text(),
        "",
        "vim 9.1: 2@: 把 :1d 等价命令再执行两遍（逐行删光）；引擎只执行一遍"
    );
}

/// **发现 4（P2）**：vim 的宏**就是寄存器**——`"ayy` 会覆写寄存器 a 里录好的
/// 宏（Z2 oracle：`qaxqj\"ayy` 后 @a='YY\n'），`@a` 从此重放的是 yank 宏；
/// 引擎的宏槽（macros map）与寄存器文件是两个存储，`"ayy` 后 `@a` 仍重放
/// 旧键序。
/// oracle：`ZZ\nYY\n` 上 `qaxq`（录下 x），j 到第二行 `"ayy`，再 `@a`：
/// vim 缓冲不变（@a 重放 yy，再次 yank 第二行）；引擎重放录制体里的 x，
/// 把 'Y' 删掉 → "Z\nY"。
#[test]
fn yank_into_register_overwrites_recorded_macro() {
    let mut f = Fixture::at("ZZ\nYY\n", 0, 0);
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.text(), "Z\nYY\n", "录制即执行：x 删掉第一个 Z");
    f.feed(["j", "\"", "a", "y", "y"]);
    f.feed(["@", "a"]);
    assert_eq!(
        f.text(),
        "Z\nYY\n",
        "vim 9.1: \"ayy 覆写宏，@a 重放 yy（无文本变化）；引擎重放旧宏删掉 Y"
    );
}

/// **发现 4b（P2）**：同一身份的另一面——录好的宏内容必须**可粘贴**：
/// `qaxq` 后寄存器 a 的内容是 "x"（Z1 oracle：string(@a)='x'，`"ap` 粘出
/// "ZZx"）；引擎的录制只进 macros map，寄存器 a 仍是空的，`"ap` 响铃无物。
#[test]
fn recorded_macro_text_is_pasteable_from_the_register() {
    let mut f = Fixture::at("ZZ\n", 0, 0);
    f.feed(["q", "a", "x", "q"]);
    f.feed(["$", "\"", "a", "p"]);
    // oracle 复验（2026-10-08）：录制即执行，x 删掉首个 Z 后缓冲为 "Z"，
    // `"ap` 粘出 'x' → "Zx"（初稿的 "ZZx" 算术滑笔）
    assert_eq!(
        f.text(),
        "Zx\n",
        "vim 9.1: \"ap 粘出宏文本 'x'；引擎宏/寄存器分离，无物可粘"
    );
}

// ------------------------------------------------ Replace 模式（oracle H）

/// **发现 5（P2）**：计数 Replace 的重复条件与 net 文本——`3Rab<BS><Esc>`：
/// 退格不取消重复、每轮的净文本 "a" 重复 3 次（oracle：`zzzz` 上得
/// "aaaz"）；引擎在退格后光标/锚失配，重复整段放弃 → "az"（或等价的
/// 单轮净文本）。
#[test]
fn r_mode_count_repeat_survives_backspace() {
    let mut f = Fixture::at("zzzz\n", 0, 0);
    f.feed(["3", "R"]);
    f.type_text("ab");
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(
        f.text(),
        "aaaz\n",
        "vim 9.1: 3Rab<BS><Esc> 每轮净文本 'a' 共三份；引擎放弃重复"
    );
}

// ------------------------------------------------ 插入 <C-v> 数字（oracle E4）

/// **发现 6（P3）**：`i<C-v>0` 后按 Esc：vim 把不完整的数字输入整个取消——
/// 不插入任何字符、Esc 照常退出插入模式（oracle E4：`z` 上
/// `i<C-v>0<Esc>:wq` 后缓冲仍 "z"、:wq 已执行=已退出 insert）；引擎把 Esc
/// 当数字串终结符、按值 0 解析出 NUL 字符插进缓冲，且 Esc 被消费后**仍留在
/// 插入模式**。
#[test]
fn cv_zero_then_esc_cancels_and_exits_insert() {
    let mut f = Fixture::at("z\n", 0, 0);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    // 数字必须走键管线（i_CTRL-V 的收位器在 insert_key 里拦在 printable 之前）
    f.feed(["0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "z\n",
        "vim 9.1: <C-v>0<Esc> 取消数字输入，不插 NUL；引擎插入 \\0"
    );
    assert!(
        matches!(f.vim.mode(), Mode::Normal),
        "vim 9.1: Esc 退出插入模式；引擎把 Esc 吃进数字收集器仍留在 insert"
    );
}

// ------------------------------------------------ Ex 面辅助（oracle AA5）

/// **发现 7（P3）**：`:u`/`:un`（`:undo` 的缩写；oracle AA5/UN：改动后
/// `:u<CR>` 与 `:un<CR>` 都恢复 "abc"）在引擎报 E492——Ex 表只收
/// `undo`/`und`/`undo!`/`und!`，不收 `u`/`un`。
#[test]
fn ex_single_letter_u_abbreviation_undoes() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["x"]);
    f.feed([":", "u", "<CR>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: :u 等价 :u[ndo]，恢复被删字符；引擎 E492 静默不动"
    );

    let mut g = Fixture::at("abc\n", 0, 0);
    g.feed(["x"]);
    g.feed([":", "u", "n", "<CR>"]);
    assert_eq!(
        g.text(),
        "abc\n",
        "vim 9.1: :un 同为 :undo 缩写；引擎 E492"
    );
}

// ------------------------------------------------ 计数插入 × <C-t>（oracle K1）

/// **发现 12（P3）**：计数插入会话里的 `<C-t>`：vim 的重复按**键序**逐轮重放，
/// 每轮的缩进位移都生效（oracle K1：noet ts=8 sw=8 下 `3i<C-t>x<Esc>` 于
/// "ab" 得 "\t\t\txxxab"——三轮各抬一个 shiftwidth，且行首的打字点随缩进
/// 右移，x 落在缩进之后）；引擎把 C-t 的位移当作会话外编辑，`InsertRepeat`
/// 只账打字文本，且行首（at==ls）的光标不随缩进右移——得 "xxx\tab"
///（丢两轮缩进、x 还落在了缩进前面）。
#[test]
fn counted_insert_ctrl_t_shifts_repeat_every_round() {
    let mut f = Fixture::at("ab\n", 0, 0);
    // oracle 环境（vim -Nu NONE）：noet + ts=8 + sw=8
    f.feed([":", "s", "e", "t", " ", "n", "o", "e", "t", " ", "t", "s", "=", "8", " ", "s", "w", "=", "8", "<CR>"]);
    f.feed(["3", "i"]);
    f.feed_raw(Key::ctrl_char('t'));
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "\t\t\txxxab\n",
        "vim 9.1: 3i<C-t>x 每轮重放 C-t，三个 shiftwidth；引擎只账 x 丢轮次位移"
    );
}

// ------------------------------------------------ 定向复核（期望与 oracle 一致的形状，失败即新发现）

/// **复核 8（预期一致）**：`.` 重放含寄存器前缀的删除——oracle C：`aa\nbb\ncc`
/// 上 `"add`、j、`.`，vim 把第二行删进寄存器 a（重放 `"a` 前缀，@a='cc\n'）。
#[test]
fn dot_replay_carries_the_register_prefix() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["\"", "a", "d", "d"]);
    assert_eq!(f.text(), "bb\ncc\n");
    f.feed(["j", "."]);
    assert_eq!(
        f.text(),
        "bb\n",
        "vim 9.1: . 重放 \"add（含前缀），cc 行被删进寄存器 a"
    );
}

/// **复核 9（预期一致）**：yank 不清账——`x`、`"ayy`、`.`：vim 重放的是
/// 更早的 x（AA21 oracle："abcdef" 上得 "cdef"）。
#[test]
fn yank_does_not_overwrite_the_dot_record() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["x"]);
    f.feed(["\"", "a", "y", "y"]);
    f.feed(["."]);
    assert_eq!(f.text(), "cdef\n", "vim 9.1: yank 不进 redo，. 重放 x");
}

/// **复核 10（预期一致）**：小删除连续两次都只进 `"-`、不动编号环
/// （oracle B：`x.` 后 @1=''、@-='b'）。行为面用粘贴探针：`"-p` 在光标后
/// 粘出第二次删除的 'b'；`"1P` 无物可粘（"1 从未被写入）。
#[test]
fn repeated_small_deletes_stay_out_of_the_numbered_ring() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["x", "."]);
    assert_eq!(f.text(), "cdef\n");
    f.feed(["\"", "-", "p"]);
    assert_eq!(f.text(), "cbdef\n", "vim 9.1: \"- 粘出第二次小删除 'b'");
    let mut g = Fixture::at("abcdef\n", 0, 0);
    g.feed(["x", ".", "\"", "1", "P"]);
    assert_eq!(g.text(), "cdef\n", "vim 9.1: 小删除不进 \"1，\"1P 无物可粘");
}

// ------------------------------------------------ undo 光标（oracle Y1 / Z22）

/// **发现 11（P2）**：undo 的光标应落在**被撤销变更的第一行**上；对 Ex 范围
/// 命令引擎把「begin_undo_group 提示游标」（= 命令前的光标位置）当作变更
/// 位置，undo 后光标停在命令前的行而不是被删/被换的行。
/// oracle Y1（typeahead）：5 行缓冲、光标行 1，`:5d<CR>` 后 `u` →
/// line(".") = 5（被删行）；oracle Z22：`G` 后 `:1,2s/x/Y/<CR>` 再 `u` →
/// line(".") = 1（首个变更行）。
#[test]
fn undo_cursor_lands_on_the_first_line_of_the_undone_ex_change() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\nl5\n", 0, 0);
    f.feed([":", "5", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl3\nl4\n");
    f.feed(["u"]);
    assert_eq!(
        f.line(),
        4,
        "vim 9.1: :5d 后 u 光标落第 5 行（被撤销删除的范围行）；引擎落命令前行 1"
    );

    let mut g = Fixture::at("x1\nx2\nx3\nx4\nx5\n", 4, 0);
    g.feed([":", "1", ",", "2", "s", "/", "x", "/", "Y", "/", "<CR>"]);
    g.feed(["u"]);
    assert_eq!(
        g.line(),
        0,
        "vim 9.1: :1,2s 后 u 光标落第 1 行（首个变更行）；引擎落 G 所在第 5 行"
    );
}

// ------------------------------------------------ 计数插入 × C-u（oracle Y3）

/// **发现 12（P2）**：计数插入会话里 `<C-u>` 清空已输入文本后，剩余轮次按
/// **净文本（空）** 继续——oracle Y3：`zz` 上 `3iab<C-u><Esc>` → 缓冲仍 "zz"
/// （三轮的净效果都是空，重复照发但不粘出任何东西）。引擎的 C-u 路径不裁
/// `InsertRepeat` 的账、光标回到锚点又满足重复条件 → 按陈旧的 "ab" 整段复制。
#[test]
fn counted_insert_ctrl_u_repeats_the_empty_net_text() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("ab");
    f.feed_raw(Key::ctrl_char('u'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "zz\n",
        "vim 9.1: 3iab<C-u><Esc> 净效果为空；引擎把陈旧的 ab 复制出来"
    );
}

// ------------------------------------------------ 块可视算子的 `.`（对照 D11 的插入侧）

/// **复核 13（预期一致）**：块可视**算子**变更（`<C-v>jlx`）之后 `.` 应当
/// 重放同形块删除（vim 的 `.` 会重做可视块变更；D11 只修了块**插入**侧）。
#[test]
fn dot_after_block_visual_operator_redoes_the_block_delete() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "x"]);
    assert_eq!(f.text(), "\n\ncc\n", "块删除 rows0-1 cols0-1");
    f.feed(["G", ".", "k"]);
    assert_eq!(
        f.text(),
        "\n\n\n",
        "vim 9.1: . 重放块可视删除，把 cc 行的 cc 也删掉"
    );
}
