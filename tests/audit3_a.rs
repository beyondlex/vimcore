//! audit3_a：Ex 命令 / 命令行编辑面 / `:set` 选项 独立审计（2026-10-08）。
//!
//! 每个发现一个 `#[test]`，断言写的是 **vim 9.1 oracle 的期望结果**，因此在
//! 当前工作树上应当失败（修复后即回归）。oracle 通道：`vim -Nu NONE -N -i
//! NONE -n -s keys buf`（typeahead，键序文件以 Esc/:wq 收尾；消息类经
//! v:errmsg / `execute("messages")` 采样）。选项形状按任务铁律对齐（vim
//! `-Nu NONE` 出厂 noet/ts=8/sw=8，引擎默认 et/ts=4/sw=4——凡涉及对齐处
//! 已在测试内显式 `:set` 对齐并注明）。
//!
//! ## 已证伪（oracle 已证引擎与 vim 一致，免下轮重查）
//! - 已证伪：`:/foo/` 搜索地址在同光标行有后随匹配时——vim 也跳到下一行
//!   （`a foo` 行光标 (0,0)，`:/foo/d` 删的是第二行），引擎的
//!   `line > cursor_line` 判定一致。
//! - 已证伪：`:2,2j 3`（等值双地址 + count）——vim 也 join 2-4 三行，引擎
//!   reanchor 后同样 join。
//! - 已证伪：`:sort`/`:retab` 的光标落点——vim 同样不动光标行（引擎
//!   `:sort` 落范围首行与 vim 一致；`:retab` 两侧都不动）。
//! - 已证伪：超宽行 `:ce 10` 的缩进剥离——vim 也把 `    averylongline` 变
//!   `averylongline`（trim 后不补 padding），引擎一致。
//! - 已证伪：`"/` 搜索寄存器缺失——引擎有 `Registers::store_search`，cmdline
//!   `<C-r>/` 行为与 vim 一致。
//! - 已证伪：`:reg !` 垃圾参数——vim 侧也静默（v:errmsg 空），引擎一致。
//! - 已证伪：`:s/a/Z/&` 的 `&` 旗标——vim 静默接受（旗标效果面属已知分歧
//!   #73），引擎接受行为一致。
//! - 已证伪：`:d #`——vim 也接受 `#` 为寄存器（缓冲同样删行、无报错），
//!   引擎一致（分歧收窄到 `%`/`.`/`:`，见 A6）。

mod common;

use common::Fixture;
use vimcore::key::Key;

/// 把一条 Ex 命令行逐字符送进提示符（`f.feed` 的 item 是"一个键"）。
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

// ---------------------------------------------------------------- A1（P1）
// `:retab` 的空白 run 一律从显示列 0 起量宽，无视 run 的起始列。
//
// 复现：缓冲 `ab\tcd`（TAB 在显示列 2），`:set et ts=4` 后 `:retab 4`。
// vim 9.1：TAB 只占 2 格（列 2 → 下一 ts 边界 4），et 下展开成 2 个空格
//     → "ab  cd"。
// 引擎：retab_run 把 run 当作从列 0 起步，量出 4 格 → 4 个空格
//     → "ab    cd"（静默改错宽度）。
// oracle：printf 'ab\tcd\n' > b; python3 -c "open('k','wb').write(
//   b':set et ts=4\r:retab 4\r:call writefile([getline(1)],\"out\")\r:wq\r')"
//   && vim -Nu NONE -N -i NONE -n -s k b; od -c out
//   → "a b     c d"（恰好 2 空格）。
#[test]
fn retab_measures_runs_from_their_start_column() {
    let mut f = Fixture::new("ab\tcd\n");
    // 对齐：oracle 在 -Nu NONE（noet ts=8 出厂）下先 :set et ts=4；
    // 引擎默认就是 et/ts=4，这里仍显式设置，保持两侧同形状。
    ex(&mut f, "set et ts=4");
    ex(&mut f, "retab 4");
    assert_eq!(
        f.text(),
        "ab  cd\n",
        "vim 9.1: :retab 的 run 从自身起始列量宽（TAB@列2、ts=4 → 2 空格）"
    );
}

// ---------------------------------------------------------------- A2（P1）
// `:%+1d`：`%` 带偏移时被塌缩成末地址，偏移整体丢弃。
//
// 复现：5 行缓冲，`:%+1d`。
// vim 9.1：`%` = 1,$（+1 被吸收/忽略）→ 删空整个缓冲（writefile 采出空）。
// 引擎：parse_range 把 "%+1" 的 base `%` 直接取 last（注释自认"collapsed to
//     its LAST line"），偏移丢弃 → 范围 (5,5) → 只删第 5 行。
// oracle：printf 'l1\nl2\nl3\nl4\nl5\n' > b; keys b':%%+1d\r:call
//   writefile(getline(1,"$"),"out")\r:wq\r' → out 为空（缓冲删空）。
#[test]
fn percent_range_with_offset_keeps_whole_file_semantics() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\n");
    ex(&mut f, "%+1d");
    assert_eq!(
        f.text(),
        "",
        "vim 9.1: :%+1d 删空全文件（% = 1,$，偏移不把 % 塌缩成末地址）"
    );
}

// ---------------------------------------------------------------- A3（P2）
// `:pu {N}` 的数字参数是寄存器名（:put 没有 count 语法），引擎当 count 丢弃。
//
// 复现：`dd dd dd` 后（"1=c、"2=b、"3=a，缓冲剩 "d"），`:pu 3`。
// vim 9.1：粘寄存器 3 → "d\na"。
// 引擎：ex_put 用 parse_reg_count，数字头 → count 3 且被 `let (_, _count)`
//     丢弃，寄存器落回 None → 粘匿名寄存器（"c）→ "d\nc"。
// oracle：printf 'a\nb\nc\nd\n' > b; keys b'ddddddd\x1b:pu 3\r:call
//   writefile(getline(1,"$"),"out")\r:wq\r' → out = "d\na\n"
//   （先行探针已证 3 次 dd 后 "3 恰为 "a"）。
#[test]
fn put_digit_argument_names_a_register() {
    let mut f = Fixture::new("a\nb\nc\nd\n");
    f.feed(["d", "d", "d", "d", "d", "d"]);
    ex(&mut f, "pu 3");
    assert_eq!(
        f.text(),
        "d\na\n",
        "vim 9.1: :pu 3 粘寄存器 3（:put 无 count 语法），不是把 3 当行数"
    );
}

// ---------------------------------------------------------------- A4（P2）
// `:set tw=0` 后裸 `:ce`/`:ri` 的默认宽度：vim 用 80，引擎用 `tw.max(1)`=1。
//
// 复现：`:set tw=0` 后 `:ce` 于 "ab"。
// vim 9.1：`:h :ce`——width 缺省或 0 时用 'textwidth'，tw=0 则 80 →
//     39 列填充 + "ab"（noet ts=8 出厂下填充 = 4 TAB + 7 空格）。
// 引擎：ex_align 的 `width = textwidth.max(1)` → 1，行宽 ≥ 1 全部早退 →
//     整条命令静默无操作。
// oracle：printf 'ab\n' > b; keys b':set tw=0\r:ce\r:call writefile(
//   [getline(1)],"out")\r:wq\r' → od -c out = \t \t \t \t + 7 空格 + "ab"。
#[test]
fn center_with_zero_textwidth_uses_80_columns() {
    let mut f = Fixture::new("ab\n");
    // 对齐：oracle 是 -Nu NONE 出厂 noet+ts=8，引擎侧显式对齐同形状。
    ex(&mut f, "set tw=0 noet ts=8");
    ex(&mut f, "ce");
    assert_eq!(
        f.text(),
        "\t\t\t\t       ab\n",
        "vim 9.1: tw=0 时 :ce 以 80 列居中（4 TAB + 7 空格 + ab）"
    );
}

// ---------------------------------------------------------------- A5（P2）
// 命令行 `<C-p>`/`<C-n>`（历史回溯）完全未绑定——按键被提示符静默吞掉。
//
// 复现：`:s/a/X/<CR>` 后 `j` `:` `<C-p>` `<CR>`。
// vim 9.1：c_CTRL-P 回召上一条命令行并执行 → 第 2 行也被替换 → "Xa|Xa"。
// 引擎：cmdline_key 对带 control 的 Char 落入兜底臂静默 Consumed →
//     空命令行回车无操作 → "Xa|aa"。
// oracle：printf 'aa\naa\n' > b; keys b':s/a/X/\rj:\x10\r:call writefile(
//   getline(1,"$"),"out")\r:wq\r' → out = "Xa\nXa\n"。
#[test]
fn cmdline_ctrl_p_recalls_history() {
    let mut f = Fixture::at("aa\naa\n", 0, 0);
    ex(&mut f, "s/a/X/");
    f.feed(["j", ":"]);
    f.feed_raw(Key::ctrl_char('p'));
    f.feed(["<CR>"]);
    assert_eq!(
        f.text(),
        "Xa\nXa\n",
        "vim 9.1: <C-p> 在命令行回召上一条历史并执行"
    );
}

// ---------------------------------------------------------------- A6（P2）
// Ex 侧 `:d`/`:y` 接受只读寄存器字符 `%`/`.`/`:` 为目标：vim 报
// E488 Trailing characters 且不动缓冲；引擎照常执行并污染寄存器槽。
//
// 复现 a：`:1d %`。vim：E488、缓冲不动；引擎删行 + named['%'] 被写入。
// 复现 b：`:1d .` 后 `".p`。vim：E488，`".` 仍是空（p 无物可贴）；
//     引擎：named['.'] 存进被删行 → `".p` 把刚删的行贴回来
//     （`".`「最近插入」语义被覆盖，读侧 get(LAST_INSERT) 直读该槽）。
// oracle：keys b':1d %\r:call writefile(["em=".v:errmsg],"out")\r:wq\r'
//   → "em=E488: Trailing characters: %"，缓冲保持 "a\nb\n"；
//   `:1d .` → "E488: Trailing characters: ."；`:1y %` 同报文。
#[test]
fn delete_percent_colon_dot_targets_are_rejected() {
    // a) vim 拒绝：缓冲不动 + E488 报文
    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1d %");
    assert_eq!(f.text(), "a\nb\n", "vim 9.1: :1d % 报 E488，缓冲不动");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E488: Trailing characters: %")),
        "vim 9.1 报文 E488: Trailing characters: %（现引擎无报文静默删除）"
    );

    // b) `.` 污染形状：被删行不得借 `".` 还魂
    let mut f = Fixture::at("ab\ncd\n", 0, 0);
    ex(&mut f, "1d .");
    f.feed(["\"", ".", "p"]);
    assert_eq!(
        f.text(),
        "ab\ncd\n",
        "vim 9.1: :1d . 被拒后 `\".p` 无物可贴（引擎把被删行写进 named['.']）"
    );
}

// ---------------------------------------------------------------- A7（P2）
// `:{n}j 1`：显式 count 1 的 join 应为无操作，引擎照接两行。
//
// 复现：6 行缓冲 `:5j 1`。
// vim 9.1：count = 要接的行数，1 行无可接 → 缓冲原样。
// 引擎：reanchor_range(count=1) 后 addr_count==1 不触发 `:2,2j` 等值早退，
//     lines==1 仍给 1 个接缝 → "l5 l6"。
// oracle：keys b':5j 1\r:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → out = "l1\nl2\nl3\nl4\nl5\nl6\n"（6 行原样）。
#[test]
fn join_with_explicit_count_one_is_a_noop() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\nl5\nl6\n");
    ex(&mut f, "5j 1");
    assert_eq!(
        f.text(),
        "l1\nl2\nl3\nl4\nl5\nl6\n",
        "vim 9.1: :5j 1 的 count=1 表示接 1 行 = 无操作"
    );
}

// ---------------------------------------------------------------- A8（P3）
// `:s` 的 `i`/`I` 旗标同时出现时，vim 按"后者生效"处理，引擎 `contains('i')`
// 先判——无论顺序一律大小写不敏感。
//
// 复现：`FOO\nfoo` 上 `:%s/foo/B/iI`（i 先 I 后）。
// vim 9.1：末位 I 生效 → 大小写敏感 → 只有 "foo" 行被替换 → "FOO|B"。
//     （对照 `Ii` → "B|B"，末位 i 生效。）
// 引擎：`if flags.contains('i') { insensitive } else if contains('I')` →
//     两序都是 "B|B"。
// oracle：keys b':%s/foo/B/iI\r:call writefile(getline(1,"$"),"out")\r:wq\r'
//   → out = "FOO\nB\n"；Ii 序 → "B\nB\n"。
#[test]
fn substitute_case_flags_last_one_wins() {
    let mut f = Fixture::at("FOO\nfoo\n", 0, 0);
    ex(&mut f, "%s/foo/B/iI");
    assert_eq!(
        f.text(),
        "FOO\nB\n",
        "vim 9.1: iI 同现时末位 I 生效（大小写敏感，FOO 行不替换）"
    );
}

// ---------------------------------------------------------------- A9（P3）
// `:ce`/`:ri`/`:le` 把光标挪到范围首行；vim 完全不动光标行。
//
// 复现：4 行缓冲，光标 `G`（第 4 行）后 `:1,3le`。
// vim 9.1：光标仍第 4 行（line(".") == 4）。
// 引擎：ex_align 收尾 `cursor.offset = clamp_to_line_end(line_start(first))`
//     → 跳到第 1 行。
// oracle：keys b'G:1,3le\r:call writefile(["le=".line(".")],"out")\r:wq\r'
//   → out = "le=4"。
#[test]
fn align_commands_leave_the_cursor_line_alone() {
    let mut f = Fixture::at("eee\n ddd\n  ccc\n   bbb\n", 3, 4);
    ex(&mut f, "1,3le");
    assert_eq!(
        f.line(),
        3,
        "vim 9.1: :1,3le 不移动光标行（引擎挪到范围首行）"
    );
}

// --------------------------------------------------------------- A10（P3）
// `:s` 第 4 段垃圾（`:s/a/b/c/d`）引擎只哑铃；vim 报 E488。
//
// 复现：`ab` 上 `:1s/a/b/c/d`。
// vim 9.1：v:errmsg = "E488: Trailing characters: /d"，缓冲不动。
// 引擎：split 出第 4 段后 `parts.next().is_some()` → 裸 bell，无任何报文。
// oracle：keys b':1s/a/b/c/d\r:call writefile(["em=".v:errmsg],"out")\r:wq\r'
//   → out = "em=E488: Trailing characters: /d"。
#[test]
fn substitute_extra_field_reports_e488() {
    let mut f = Fixture::at("ab\n", 0, 0);
    ex(&mut f, "1s/a/b/c/d");
    assert_eq!(f.text(), "ab\n", "两侧都不执行替换");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E488: Trailing characters: /d"),
        "vim 9.1: 第 4 段垃圾报 E488: Trailing characters: /d"
    );
}

// --------------------------------------------------------------- A11（P3）
// `:delmarks` 的两个参数面错误静默：裸 `:delm`（vim E471 Argument
// required）与倒序区间 `:delm z-a`（vim E475 Invalid argument: z-a，
// 引擎静默跳过）。
//
// oracle：keys b':delm\r:call writefile(["em=".v:errmsg],"out")\r:qa!\r'
//   → "em=E471: Argument required"；
//   keys b'mz:delm z-a\r:call writefile(["em=".v:errmsg],"out")\r:qa!\r'
//   → "em=E475: Invalid argument: z-a"。
#[test]
fn delmarks_argument_errors_are_reported() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "delm");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E471: Argument required"),
        "vim 9.1: 裸 :delm 报 E471（引擎静默无操作）"
    );

    let mut f = Fixture::at("x\n", 0, 0);
    f.feed(["m", "z"]);
    ex(&mut f, "delm z-a");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E475: Invalid argument: z-a"),
        "vim 9.1: 倒序区间报 E475（引擎静默跳过）"
    );
}

// --------------------------------------------------------------- A12（P3）
// `:set all`：vim 列出全部选项，引擎 E518: Unknown option: all。
//
// oracle：printf ':set all\n:qa!\n' | vim -es -Nu NONE -N -n /dev/null
//   → stdout 输出完整选项表（tabstop=8、shiftwidth=8、textwidth=0 …）。
#[test]
fn set_all_lists_every_option() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "set all");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("tabstop")),
        "vim 9.1: :set all 列出全部选项（含 tabstop）；引擎报 E518"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.contains("E518: Unknown option: all")),
        "all 不是未知选项名"
    );
}

// --------------------------------------------------------------- A13（P3）
// `:setglobal`（及 `:setg…` 前缀族）不存在——E492。vim 接受（引擎是单缓冲
// 全局模型，接受拼写即可）；config.rs 的 rc 侧认 `setglobal`，运行时 ex_set
// 不认，两侧分裂。
//
// oracle：printf 'x\n' > b; keys b':setglobal ts=2\r:call writefile(
//   ["em=".v:errmsg],"out")\r:qa!\r' → out = "em="（无任何报错）。
#[test]
fn setglobal_spelling_is_accepted() {
    let mut f = Fixture::at("x\n", 0, 0);
    ex(&mut f, "setglobal ts=2");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "vim 9.1 接受 :setglobal（引擎 E492: Not an editor command: setglobal ts=2）"
    );
}

// --------------------------------------------------------------- A14（P3）
// 无前次替换时 replacement 里的 `~` 应展开为空，引擎写字面 `~`。
//
// 复现：全新会话 `ab` 上 `:s/a/~`。
// vim 9.1：previous replacement 为空 → 得 "b"。
// 引擎：`last_replacement.unwrap_or("~")` → 字面 "~" → 得 "~b"。
// oracle：printf 'ab\n' > b; keys b':s/a/~\r:call writefile([getline(1)],
//   "out")\r:wq\r' → out = "b"。
#[test]
fn fresh_tilde_replacement_expands_to_empty() {
    let mut f = Fixture::at("ab\n", 0, 0);
    ex(&mut f, "s/a/~");
    assert_eq!(
        f.text(),
        "b\n",
        "vim 9.1: 无前次替换时 ~ 为空串（引擎产字面 ~）"
    );
}

// --------------------------------------------------------------- A15（P3）
// E488/E477 报文带原命令行后缀；vim 9.1 只报尾参本体 / 短文案
// （round14 的「消息带完整原行」是当年 oracle 误读——修复时需同步更新
// parity_round14 钉住的 E477 文本与既有 E488 断言）。
//
// 复现 a：`:1d 3x`。vim 显示 "E488: Trailing characters: x"
//     （vim 还把 3 当 count 吞掉，尾参只剩 x；引擎 token 是 "3x" 且追加
//     ": d 3x" 后缀）。
// 复现 b：`:1,2y!`。vim 显示 "E477: No ! allowed"；引擎追加 ": 1,2y!"。
// oracle：keys b':1d 3x\r:let m=execute("messages")\r:call
//   writefile(split(m,"\n"),"out")\r:wq\r' → 末行 "E488: Trailing
//   characters: x"（v:errmsg 同）；`:1,2y!` 同通道 → "E477: No ! allowed"。
#[test]
fn e488_e477_messages_quote_only_the_tail() {
    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1d 3x");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with("E488: Trailing characters:")),
        "应报 E488"
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s.ends_with(": 1d 3x") || s.contains(": d 3x")),
        "vim 9.1 的 E488 不携带原命令行后缀（引擎报 \"…: 3x: d 3x\"）"
    );

    let mut f = Fixture::at("a\nb\n", 0, 0);
    ex(&mut f, "1,2y!");
    assert_eq!(
        f.host.statuses.last().map(|s| s.as_str()),
        Some("E477: No ! allowed"),
        "vim 9.1 的 E477 文案无原行后缀（引擎报 \"E477: No ! allowed: 1,2y!\"）"
    );
}
