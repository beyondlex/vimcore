//! audit8_a：Ex / `:s` / `:set` / insert 缩进的对拍回归（独立审计·域 A）
//!
//! 用例来自黑盒对拍：同一（初始缓冲, 按键）喂给引擎与本机 `/usr/bin/vim`
//! （9.1 patches 1-1752，`-Nu NONE -N -i NONE -n`，前置 `:set ic scs ts=4
//! tw=78 et` 把引擎有意分歧的默认值钉成 oracle 前提），`:wq` 写回后逐字节
//! 比较缓冲文本。下表 `want` 就是 vim 侧的实测值。
//!
//! 按键按**原始字节**存（`<CR>` 已展开成 `\r`、`<C-h>` 成 `\u{8}`），
//! [`feed`] 把字节还原成引擎键名——不再造一层 DSL 往返。表内没有
//! 插入模式下打字的用例：那种字符是宿主的文本通道（引擎只把它们
//! 收进 `pending_unknown_chars` 等宿主落地），要写就得像真宿主那样
//! 调 `type_text`（本轮 `0<C-d>` 用例的教训）。
//!
//! 161 例里 10 例是已知分歧，不入表：`:g`/`:v`/`:normal`（分歧账目 #12）、
//! `:copy`/`:move`（`:t`/`:co`/`:m` 整族未实现，红探针在 `tests/audit6_a.rs`
//! 的 `copy_command_missing`/`move_command_missing`）、`:<C-h>d`（空命令行
//! 退格，`-s` 与 expect PTY 两条通道给出互相矛盾的 vim 结果，见 NOTES 挂账）。
//!
//! 4 例（`cmd_u`/`range_pct`/`range_dot_dollar`/`hist_prefix`）两端文本都是
//! 空串——这是 `-s` 通道 timeout 假阴性的典型形状，已用 expect PTY 复核确认
//! 命令真的执行了（oracle 纪律：响铃/方向键类走 PTY）。

mod common;
use common::Fixture;

/// 原始按键字节 → 引擎键名回灌（与对拍脚本喂 vim 的字节流同源）。
fn feed(f: &mut Fixture, keys: &str) {
    let bytes = keys.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // 方向键是 ESC + [ + 字母，必须先于裸 ESC 判定
        let arrow = match bytes.get(i..i + 3) {
            Some(b"\x1b[A") => Some(("<Up>", 3)),
            Some(b"\x1b[B") => Some(("<Down>", 3)),
            Some(b"\x1b[C") => Some(("<Right>", 3)),
            Some(b"\x1b[D") => Some(("<Left>", 3)),
            _ => None,
        };
        if let Some((name, len)) = arrow {
            f.feed([name]);
            i += len;
            continue;
        }
        let name: String = match b {
            0x0d | 0x0a => "<CR>".to_owned(),
            0x09 => "<Tab>".to_owned(),
            0x7f => "<BS>".to_owned(),
            0x1b => "<Esc>".to_owned(),
            0x08 => "<C-h>".to_owned(),
            0x01..=0x07 | 0x0e..=0x18 | 0x1a..=0x1f => {
                format!("<C-{}>", (b + b'A' - 1) as char).to_lowercase()
            }
            _ => {
                // 可打印字符走 KEY 事件（与对拍脚本同源）：`:` 在提示符下
                // 必须是键，type_text 的 IME 通道会把命令行喂歪
                let ch = keys[i..].chars().next().unwrap();
                f.feed([ch.to_string()]);
                i += ch.len_utf8();
                continue;
            }
        };
        f.feed([name.as_str()]);
        i += 1;
    }
}

fn run(init: &str, keys: &str) -> String {
    let mut f = Fixture::new(init);
    feed(&mut f, keys);
    f.text()
}

fn statuses(init: &str, keys: &str) -> Vec<String> {
    let mut f = Fixture::new(init);
    feed(&mut f, keys);
    f.host.statuses.clone()
}

fn last_status(init: &str, keys: &str) -> String {
    statuses(init, keys)
        .pop()
        .unwrap_or_else(|| "<无消息>".to_owned())
}

/// (标签, 初始缓冲, 按键字节, vim 9.1 实测缓冲)
const CASES: &[(&str, &str, &str, &str)] = &[
        ("set_nonu_q", "x\n", ":set nonumber?\r", "x\n"),
        ("set_nu_q", "x\n", ":set number?\r", "x\n"),
        ("set_all_reset", "x\n", ":set ts=9\r:set all&\r", "x\n"),
        ("s_n_glob", "aa\nba\naa\n", ":%s/a/X/n\r", "aa\nba\naa\n"),
        ("s_n_globg", "aa\nba\naa\n", ":%s/a/X/gn\r", "aa\nba\naa\n"),
        ("s_lead_ws_flag", "aa\nbb\n", ":s/a/b/ e\r", "aa\nbb\n"),
        ("s_digit_g", "aa\nbb\ncc\n", ":s/a/b/2g\r", "aa\nbb\ncc\n"),
        ("star_word", "foo bar foo\n", "*", "foo bar foo\n"),
        ("hash_word", "foo bar foo\n", "2l#", "foo bar foo\n"),
        ("n_after_slash", "foo bar foo\n", "/bar\rn", "foo bar foo\n"),
        ("N_after_slash", "foo bar foo\n", "/bar\rN", "foo bar foo\n"),
        ("qmark_search", "foo bar foo\n", "?foo\rn", "foo bar foo\n"),
        ("count_search", "a a a a\n", "3/a\r", "a a a a\n"),
        ("search_e_offset", "foo bar\nbaz\n", "/bar/e\rx", "foo ba\nbaz\n"),
        ("search_b_offset", "foo bar\nbaz\n", "/bar/b\rx", "foo ar\nbaz\n"),
        ("search_line_offset", "foo bar\nbaz\n", "/bar/+1\rx", "foo bar\naz\n"),
        ("s_amp", "xx\n", ":s/x/&X/\r", "xXx\n"),
        ("s_tilde_noprev", "xx\n", ":s/x/~/\r", "x\n"),
        ("s_upper", "xx\n", ":s/x/\\u&/\r", "Xx\n"),
        ("s_cr_split", "foo bar\n", ":s/foo/F\\rB/\r", "F\nB bar\n"),
        ("s_gg", "xaxax\n", ":s/a/B/gg\r", "xBxax\n"),
        ("s_empty_repl", "xax\n", ":s/a//g\r", "xx\n"),
        ("d_range", "aa\nbb\ncc\ndd\n", ":1,3d\r", "dd\n"),
        ("y_count_put", "aa\nbb\ncc\ndd\n", ":1,2y 3\r:put\r", "aa\nbb\ncc\ndd\nbb\ncc\ndd\n"),
        ("join_count", "aa\nbb\ncc\ndd\n", ":2j 3\r", "aa\nbb cc dd\n"),
        ("join_bang", "aa\nbb\ncc\n", ":1,2j!\r", "aabb\ncc\n"),
        ("sort_bang", "a\nc\nb\n", ":sort!\r", "c\nb\na\n"),
        ("sort_n", "x\n5\na\n-3\n", ":sort n\r", "x\na\n-3\n5\n"),
        ("put_reg", "aa\nbb\n", ":1,1y\r:put a\r", "aa\nbb\n"),
        ("marks_a", "aa\nbb\n", ":2\rma\r", "aa\nbb\n"),
        ("delm", "aa\nbb\n", "ma\r:delm a\r", "aa\nbb\n"),
        ("reg_arg", "aa\nbb\n", ":1y\r:reg 0\r", "aa\nbb\n"),
        ("ce_width", "aa\n", ":set tw=10\r:ce\r", "    aa\n"),
        ("ri_width", "aa\n", ":set tw=10\r:ri\r", "        aa\n"),
        ("le_indent", "  aa\n", ":le\r", "aa\n"),
        ("retab", "\ta\n", ":set et\r:retab 4\r", "    a\n"),
        ("undo_redo", "aa\n", "x\r:undo\r:redo\r", "a\n"),
        ("amp_amp", "xax\nyay\n", ":1s/a/B/\r:2&&\r", "xBx\nyBy\n"),
        ("amp", "xax\nyay\n", ":1s/a/B/\rj&\r", "xBx\nyBy\n"),
        ("tilde_cmd", "xax\nyay\n", ":1s/a/B/\rj:~\r", "xBx\nyBy\n"),
        ("s_bad_flag", "aa\n", ":s/a/b/z\r", "aa\n"),
        ("s_count0", "aa\n", ":s/a/b/0\r", "aa\n"),
        ("set_bad", "x\n", ":set foo\r", "x\n"),
        ("set_num_bad", "x\n", ":set ts=x\r", "x\n"),
        ("set_ts0", "x\n", ":set ts=0\r", "x\n"),
        ("cmd_w", "x\n", ":set ts=4\u{17}\r", "x\n"),
        ("cmd_u", "x\n", ":set ts=4\u{15}d\r", ""),
        ("hist_up", "a\nb\nc\n", ":1d\r:2d\r:\u{1b}[A\r", "b\n"),
        ("visual_colon", "aa\nbb\n", "Vj:s/a/X/\r", "Xa\nbb\n"),
        ("range_plus", "aa\nbb\ncc\n", ":+d\r", "aa\ncc\n"),
        ("range_pct", "aa\nbb\ncc\n", ":%d\r", ""),
        ("range_dollar", "aa\nbb\ncc\n", ":$d\r", "aa\nbb\n"),
        ("range_dot_dollar", "aa\nbb\ncc\n", ":.,$d\r", ""),
        ("range_reverse", "aa\nbb\ncc\n", ":3,1d\r", "aa\nbb\ncc\n"),
        ("range_oob", "aa\nbb\ncc\n", ":9d\r", "aa\nbb\ncc\n"),
        ("range_bare_oob", "aa\nbb\ncc\n", ":9\r", "aa\nbb\ncc\n"),
        ("set_inv", "x\n", ":set invic\r", "x\n"),
        ("set_reset_bool", "x\n", ":set ic\r:set ic&\r", "x\n"),
        ("s_multiple_flags", "aa\nbb\n", ":s/a/b/gi\r", "bb\nbb\n"),
        ("s_case_I", "Aa\naa\n", ":s/a/X/gI\r", "AX\naa\n"),
        ("search_star_cjk", "中文 中文\n", "*", "中文 中文\n"),
        ("search_n_reapply_e", "foo bar\nbaz bar\n", "/bar/e\rnx", "foo bar\nbaz ba\n"),
        ("search_N_back", "foo bar\nbaz bar\n", "/bar\rNx", "foo bar\nbaz ar\n"),
        ("search_qmark_n", "foo bar\nbaz bar\n", "?bar\rnx", "foo ar\nbaz bar\n"),
        ("star_gstar", "foobar foo\n", "g*", "foobar foo\n"),
        ("star_mid", "abc foo abc\n", "4l*", "abc foo abc\n"),
        ("s_UL_case", "abc\n", ":s/abc/\\U&\\E/\r", "ABC\n"),
        ("s_Lcase", "ABC\n", ":s/ABC/\\L&/\r", "abc\n"),
        ("s_lfirst", "abc\n", ":s/abc/\\l&/\r", "abc\n"),
        ("s_prev_repl", "ax\nbx\n", ":1s/a/NN/\r:2s/b/~/\r", "NNx\nNNx\n"),
        ("s_amp_escape", "ax\n", ":s/a/\\&X/\r", "&Xx\n"),
        ("s_tilde_escape", "ax\n", ":s/a/\\~X/\r", "~Xx\n"),
        ("s_backslash", "ax\n", ":s/a/x\\\\y/\r", "x\\yx\n"),
        ("s_sep_hash", "a#b\n", ":s#a#X#\r", "X#b\n"),
        ("s_sep_comma", "a,b\n", ":s,a,X,\r", "X,b\n"),
        ("s_esc_sep", "a/b\n", ":s/a\\/b/X/\r", "X\n"),
        ("s_tab", "ab\n", ":s/a/\\t/\r", "\tb\n"),
        ("range_mark", "aa\nbb\ncc\n", "2Gma\r:'a\rd\r", "aa\n"),
        ("range_marks_pair", "aa\nbb\ncc\n", "1Gma2Gmb\r:'a,'bd\r", "cc\n"),
        ("range_search_fwd", "aa\nbb\ncc\ndd\n", ":/bb/\rd\r", "aa\ndd\n"),
        ("range_search_back", "aa\nbb\ncc\ndd\n", "G:?bb?\rd\r", "aa\ndd\n"),
        ("range_search_pair", "aa\nbb\ncc\ndd\n", ":/bb/,/cc/d\r", "aa\ndd\n"),
        ("range_comma_open", "aa\nbb\ncc\n", "2G:,d\r", "aa\ncc\n"),
        ("range_semicolon", "aa\nbb\ncc\n", ":1;2d\r", "cc\n"),
        ("range_plus2", "aa\nbb\ncc\n", ":1+2d\r", "aa\nbb\n"),
        ("range_2search", "aa\nbb\ncc\ndd\n", ":1/cc/d\r", "aa\nbb\ndd\n"),
        ("range_bare_mark", "aa\nbb\n", "2G:'\r", "aa\nbb\n"),
        ("colon_print", "aa\nbb\n", ":1p\r", "aa\nbb\n"),
        ("colon_num", "aa\nbb\n", ":2\rx", "aa\nb\n"),
        ("colon_numrange", "aa\nbb\ncc\n", ":1,2\rx", "aa\nb\ncc\n"),
        ("k_cmd", "aa\nbb\n", ":ka\r:'ad\r", "aa\nbb\n"),
        ("put_zero", "aa\nbb\n", ":1y\r:0pu\r", "aa\naa\nbb\n"),
        ("put_above", "aa\nbb\n", ":1y\r:2pu!\r", "aa\naa\nbb\n"),
        ("put_named", "aa\nbb\n", "\"ayy\r:2pu a\r", "aa\nbb\naa\n"),
        ("y_into_a", "aa\nbb\n", ":1y a\r", "aa\nbb\n"),
        ("d_into_a", "aa\nbb\n", ":1d a\r", "bb\n"),
        ("d_bang", "aa\nbb\n", ":1d!\r", "aa\nbb\n"),
        ("y_bang", "aa\nbb\n", ":1y!\r", "aa\nbb\n"),
        ("d_count", "aa\nbb\ncc\ndd\n", ":1d 2\r", "cc\ndd\n"),
        ("y_count", "aa\nbb\ncc\ndd\n", ":1y 2\r", "aa\nbb\ncc\ndd\n"),
        ("d_reg_count", "aa\nbb\ncc\ndd\n", ":1d a 2\r", "cc\ndd\n"),
        ("join_plain", "aa\nbb\ncc\n", ":1j\r", "aa bb\ncc\n"),
        ("join_two_addr", "aa\nbb\ncc\n", ":1,1j\r", "aa\nbb\ncc\n"),
        ("join_count1", "aa\nbb\ncc\n", ":1j 1\r", "aa\nbb\ncc\n"),
        ("retab_noet", "\ta\n", ":retab\r", "    a\n"),
        ("retab_bang", "        a\n", ":set noet ts=8\r:retab! 4\r", "\t\ta\n"),
        ("sort_u", "b\na\na\nb\n", ":sort u\r", "a\nb\n"),
        ("sort_i", "b\nA\na\n", ":sort i\r", "A\na\nb\n"),
        ("sort_iu", "b\nA\na\n", ":sort iu\r", "A\nb\n"),
        ("le_num", "    aa\n", ":le 2\r", "  aa\n"),
        ("ce_num", "aa\n", ":ce 10\r", "    aa\n"),
        ("ri_num", "aa\n", ":ri 10\r", "        aa\n"),
        ("set_noic", "x\n", ":set noic\r", "x\n"),
        ("set_ic_toggle", "x\n", ":set ic!\r", "x\n"),
        ("set_inv_ts", "x\n", ":set invts\r", "x\n"),
        ("set_query_sp", "x\n", ":set ts ?\r", "x\n"),
        ("set_bare_num", "x\n", ":set ts\r", "x\n"),
        ("set_all_list", "x\n", ":set all\r", "x\n"),
        ("set_comment", "x\n", ":set ts=4 \" note\r", "x\n"),
        ("set_unknown_eq", "x\n", ":set foo=1\r", "x\n"),
        ("set_reset_ts", "x\n", ":set ts=9\r:set ts&\r", "x\n"),
        ("cmd_r_reg", "aa\nbb\n", "\"ayy\r:\"x\r", "aa\nbb\n"),
        ("cmd_cr_reg", "aa\nbb\n", "\"ayy\r:\u{12}a\r", "aa\nbb\n"),
        ("cmd_cr_eq", "x\n", ":\"\u{12}=1+2\r\r", "x\n"),
        ("cmd_literal_cv", "x\n", ":e\u{16}\u{1b}x\r", "x\n"),
        ("hist_prefix", "a\nb\nc\n", ":1d\r:2d\r:1\u{1b}[A\r", ""),
        ("hist_ctrl_p", "a\nb\nc\n", ":1d\r:2d\r:\u{10}\r", "b\n"),
        ("delm_range", "aa\nbb\n", "ma\rmb\r:delm a-b\r", "aa\nbb\n"),
        ("delm_bang", "aa\nbb\n", "ma\rmb\r:delm!\r", "aa\nbb\n"),
        ("delm_bare", "aa\nbb\n", ":delm\r", "aa\nbb\n"),
        ("reg_bare", "aa\nbb\n", "\"ayy\r:reg\r", "aa\nbb\n"),
        ("noh", "aa\nbb\n", "/a\r:noh\r", "aa\nbb\n"),
        ("s_g_flag_upper", "aa\n", ":s/a/X/gI\r", "XX\n"),
        ("s_only_pattern", "aaa\n", ":s/a\r", "aa\n"),
        ("s_empty_pat", "aaa\n", "/a\r:s//X/\r", "Xaa\n"),
        ("s_repeat_bare", "ax\nyx\n", ":1s/a/B/\r:2s\r", "Bx\nyx\n"),
        ("sub_count_span", "a1\na2\na3\na4\n", ":1s/a/X/ 3\r", "X1\nX2\nX3\na4\n"),
        ("undo_ex", "aa\n", "x\r:undo\r", "aa\n"),
        ("redo_ex", "aa\n", "x\ru\r:redo\r", "a\n"),
        ("wq_forced", "aa\n", ":wq\r", "aa\n"),
        ("buf_first_last", "aa\n", ":bf\r", "aa\n"),
        ("action_cmd", "aa\n", ":action SomeId\r", "aa\n"),
        ("s_noic", "Aa\n", ":set noic\r:s/a/X/\r", "AX\n"),
        ("s_ic_scs_upper", "aA\n", ":set ic scs\r:s/A/X/\r", "aX\n"),
        ("s_noic_n", "Aa\n", ":set noic\r:s/a/X/n\r", "Aa\n"),
        ("s_noic_I_flag", "Aa\n", ":set ic\r:s/a/X/I\r", "AX\n"),
        ("s_leading_ws_count", "aa\nbb\ncc\n", ":s/a/b/ 2\r", "ba\nbb\ncc\n"),
        ("s_flag_g_then_ws", "aa\n", ":s/a/b/g 2\r", "bb\n"),
        ("s_huge_count", "aa\n", ":s/a/b/99999999999999999999\r", "aa\n"),
        ("s_i32max", "aa\n", ":s/a/b/2147483647\r", "aa\n"),
        ("s_i32max1", "aa\n", ":s/a/b/2147483646\r", "ba\n"),
];

/// 对拍表：引擎文本必须逐字节等于 vim 9.1 的实测写回结果。
#[test]
fn parity_with_vim_oracle() {
    let mut bad: Vec<String> = Vec::new();
    for (label, init, keys, want) in CASES {
        let got = run(init, keys);
        if got != *want {
            bad.push(format!("{label}: want {want:?} got {got:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} cases diverged:\n{}",
        bad.len(),
        CASES.len(),
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// 消息面（缓冲文本看不到的一半）——逐项对应本轮修复
// ---------------------------------------------------------------------------

/// A-1：`no` 前缀从不反转显示值——`:set noic?` 在 'ignorecase' 为 ON 时报
/// `ignorecase`（9.1 redir 实证：`:set ic` + `:set noic?` → "  ignorecase"；
/// `:set noic` + `:set ic?` → "noignorecase"）。旧实现带 `negated` 标志，
/// 打印的是相反状态。
#[test]
fn set_query_never_inverts_the_no_prefix() {
    assert_eq!(last_status("x\n", ":set ic\r:set noic?\r"), "  ignorecase");
    assert_eq!(
        last_status("x\n", ":set noic\r:set ic?\r"),
        "  noignorecase"
    );
    assert_eq!(last_status("x\n", ":set ic\r:set ic?\r"), "  ignorecase");
}

/// A-6（收口时的复核新发现）：`no{值选项}?` 在 vim 里解析到该值选项本身
/// （9.1 实证：`:set ts=6` + `:set notabstop?` → "  tabstop=6"），旧实现
/// 认定 `notabstop` 不存在。
#[test]
fn set_query_resolves_no_prefix_on_value_options() {
    assert_eq!(
        last_status("x\n", ":set ts=6\r:set notabstop?\r"),
        "  tabstop=6"
    );
    assert_eq!(last_status("x\n", ":set ts=6\r:set nots?\r"), "  tabstop=6");
}

/// A-2：`:set all&` 是**重置**不是列表（9.1 实证：`:set ts=9` + `:set all&` +
/// `:set ts?` → "  tabstop=8"）。引擎重置的目标是引擎自己的默认值（'ts'=4 是
/// 分歧账目 #2），这里钉的是「值回到默认」这一语义。
#[test]
fn set_all_amp_resets_every_option() {
    assert_eq!(
        last_status("x\n", ":set ts=9\r:set all&\r:set ts?\r"),
        "  tabstop=4"
    );
    assert_eq!(
        last_status("x\n", ":set noic\r:set all&\r:set ic?\r"),
        "  ignorecase"
    );
    // `:set all`（不带 &）仍然逐项列出
    let listed = statuses("x\n", ":set all\r");
    assert!(
        listed.len() > 1,
        "`:set all` 应逐项列出，实得 {listed:?}"
    );
    assert!(listed.iter().any(|s| s == "  tabstop=4"));
}

/// A-3：`:s` 的旗标串必须紧邻第三分隔符，其前的空白只有在「整段是纯 count」
/// 时才合法（9.1 实证：`:s/a/b/ e` → "E488: Trailing characters: e" 且文本
/// 不动；`:s/a/b/2g` → E488 引用 "g"）。旧实现接受 `/ e` 并照做替换。
#[test]
fn sub_flags_must_hug_the_delimiter() {
    assert_eq!(
        last_status("aa\nbb\n", ":s/a/b/ e\r"),
        "E488: Trailing characters: e"
    );
    assert_eq!(run("aa\nbb\n", ":s/a/b/ e\r"), "aa\nbb\n");
    assert_eq!(
        last_status("aa\nbb\ncc\n", ":s/a/b/2g\r"),
        "E488: Trailing characters: g"
    );
    assert_eq!(run("aa\nbb\ncc\n", ":s/a/b/2g\r"), "aa\nbb\ncc\n");
    // 纯 count 前允许空白，并把范围拉长到 count 行
    assert_eq!(run("a1\na2\na3\na4\n", ":1s/a/X/ 3\r"), "X1\nX2\nX3\na4\n");
    assert_eq!(run("aa\n", ":s/a/b/g 2\r"), "bb\n");
}

/// A-3 附带：count 0 是 E939、超限是 E1510 且**按 clamp 后的值**报告（9.1
/// 实证：`:s/a/b/999…` → "E1510: Value too large: 9223372036854775807"、
/// `:s/a/b/2147483647` → "E1510: Value too large: 2147483647"，而
/// `:s/a/b/2147483646` 合法且真的替换出 "ba"）。
#[test]
fn sub_count_error_messages_match_vim() {
    assert_eq!(
        last_status("aa\n", ":s/a/b/0\r"),
        "E939: Positive count required"
    );
    assert_eq!(
        last_status("aa\n", ":s/a/b/99999999999999999999\r"),
        "E1510: Value too large: 9223372036854775807"
    );
    assert_eq!(
        last_status("aa\n", ":s/a/b/2147483647\r"),
        "E1510: Value too large: 2147483647"
    );
    assert_eq!(run("aa\n", ":s/a/b/2147483646\r"), "ba\n");
}

/// A-4：`n` 旗标按「替换会怎么做」来数——无 `g` 时每行只算一次（9.1 实证：
/// `:s/a/X/n` 于 "aa" → "1 match on 1 line"、`:%s/a/X/n` 于三行各一命中 →
/// "3 matches on 3 lines"、`:%s/a/X/gn` → "5 matches on 3 lines"）。旧实现
/// 永远数尽所有出现。
#[test]
fn sub_n_counts_like_the_substitution_would() {
    assert_eq!(last_status("aa\n", ":s/a/X/n\r"), "1 match on 1 line");
    assert_eq!(
        last_status("aa\nba\naa\n", ":%s/a/X/n\r"),
        "3 matches on 3 lines"
    );
    assert_eq!(
        last_status("aa\nba\naa\n", ":%s/a/X/gn\r"),
        "5 matches on 3 lines"
    );
    assert_eq!(last_status("ab\nab\n", ":%s/a/X/n\r"), "2 matches on 2 lines");
}

/// A-5：`i`/`I` 之外**不碰**大小写设定——无旗标时跟随 'ignorecase'/'smartcase'
/// （9.1 实证：`:set noic` + `:s/a/X/` 于 "Aa" → "AX"，只命中小写 a；
/// `:set ic` + `:s/a/X/gI` → "AX"）。旧实现无条件
/// `case_insensitive(true)`，`:set noic` 形同无效。
#[test]
fn sub_without_flags_follows_ignorecase() {
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/\r"), "AX\n");
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/n\r"), "Aa\n");
    assert_eq!(
        last_status("Aa\n", ":set noic\r:s/a/X/n\r"),
        "1 match on 1 line"
    );
    assert_eq!(run("Aa\n", ":set ic\r:s/a/X/gI\r"), "AX\n");
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/gi\r"), "XX\n");
}

// ---------------------------------------------------------------------------
// insert `<C-t>`/`<C-d>`：vim 是 ROUND 到 shiftwidth 倍数，不是 ±shiftwidth
// ---------------------------------------------------------------------------
//
// 全部形状为 vim 9.1 实证（`:set et ts=4 sw=4 noai`，键序一律 `0` 锚定——
// `-s` 通道的起始光标落在行末，不锚定就是假结果）。

/// 用 `<C-t>`/`<C-d>` 后打一个 `X` 来同时钉住「新缩进」和「光标落点」。
fn shift_then_type(init: &str, col: usize, key: &str) -> (String, usize) {
    let mut f = Fixture::new(init);
    f.feed(["0"]);
    for _ in 0..col {
        f.feed(["l"]);
    }
    f.feed(["i"]);
    f.feed([key]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    (f.text(), f.cursor())
}

/// `get_new_sw_indent(round=TRUE)` 的上升臂：进到位数上取的下一个倍数
/// （9.1：缩进 0/2/3/4/6/8 → 4/4/4/8/8/12）。旧实现是 `old + sw`，非倍数
/// 缩进一步偏到位移之外（2 空格 → 6）。
#[test]
fn insert_ctrl_t_rounds_indent_up_to_a_shiftwidth_multiple() {
    // (缩进, 光标列, 新缩进, X 的字节位)
    for (indent, col, want_indent, want_byte) in [
        (0usize, 0usize, 4usize, 4usize),
        (2, 0, 4, 2),
        (2, 1, 4, 3),
        (2, 2, 4, 4),
        (3, 0, 4, 1),
        (4, 0, 8, 4),
        (6, 0, 8, 2),
        (8, 0, 12, 4),
    ] {
        let init = format!("{}foo\n", " ".repeat(indent));
        let (text, byte) = shift_then_type(&init, col, "<C-t>");
        let want = format!(
            "{}X{}foo\n",
            " ".repeat(want_byte),
            " ".repeat(want_indent - want_byte)
        );
        assert_eq!(text, want, "<C-t> 于 {indent} 空格缩进、列 {col}");
        assert_eq!(byte, want_byte, "<C-t> 后光标列（{indent} 空格、列 {col}）");
    }
}

/// 下降臂：退到位数下取整的前一个倍数（9.1：缩进 2/4/6/8 → 0/0/4/4；
/// 恰为倍数时再降一格，所以 4 → 0 而不是 4 → 4 的「原地假成功」）。
/// 光标落在缩进内部时钳到行首（列位移为负没有列可退）。
#[test]
fn insert_ctrl_d_rounds_indent_down_to_a_shiftwidth_multiple() {
    for (indent, col, want_indent) in [
        (2usize, 0usize, 0usize),
        (4, 0, 0),
        (4, 3, 0),
        (4, 4, 0),
        (6, 0, 4),
        (8, 0, 4),
    ] {
        let init = format!("{}foo\n", " ".repeat(indent));
        let (text, byte) = shift_then_type(&init, col, "<C-d>");
        // X 一律落在字节 0（光标钳到行首），后跟新缩进
        let want = format!("X{}foo\n", " ".repeat(want_indent));
        assert_eq!(text, want, "<C-d> 于 {indent} 空格缩进、列 {col}");
        assert_eq!(byte, 0, "<C-d> 后光标钳到行首（{indent} 空格、列 {col}）");
    }
}

/// `0<C-d>`：先打的 `0` 是命令字符不是文本，退格把缩进**清零**（vim
/// ins_shift 走 change_indent 的 INDENT_SET 臂），而不是「减一个
/// shiftwidth」。9.1 实证：8 空格缩进上 `i0<C-d>` → "foo"。
#[test]
fn insert_zero_ctrl_d_clears_all_indent() {
    let mut f = Fixture::new("        foo\n");
    f.feed(["i"]);
    // insert-mode printables are the HOST's text channel (the engine declines
    // the raw key so an IME can compose), same as the `X` marker above
    f.type_text("0");
    f.feed(["<C-d>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "foo\n");
    assert_eq!(f.cursor(), 0);
}

/// TAB 缩进在 `expandtab` 下被整体重写为空格（9.1 实证：`\t\tfoo` 上
/// `0i<C-t>X` → 12 列宽缩进、X 落在字节 4）。
#[test]
fn insert_shift_rewrites_tab_indent_under_expandtab() {
    let (text, byte) = shift_then_type("\t\tfoo\n", 0, "<C-t>");
    assert_eq!(text, "    X        foo\n");
    assert_eq!(byte, 4);
    let (text, byte) = shift_then_type("\t\tfoo\n", 0, "<C-d>");
    assert_eq!(text, "X    foo\n");
    assert_eq!(byte, 0);
}
