//! 第十三轮回归：读码探针 + fuzz 轰炸 + 三路审查发现的全部修复面锁定。
//! 所有期望行为均经 vim 9.1（`/usr/bin/vim`，patches 1-1752）探针实证。

mod common;

use common::{edit, Fixture};
use vimcore::buffer::VimBuffer;
use vimcore::keymap::ModeClass;
use vimcore::registers::{RegisterKind, Registers};
use vimcore::state::KeyResult;

/// PROBE: vim 9.1 — clicking (mouse) inside visual mode moves the cursor and
/// EXTENDS the selection from the original anchor (`:h visual-use`: a mouse
/// click while visual moves the cursor; the anchor stays). If the engine's
/// `set_cursor_offset` overwrites `visual_anchor` with the click point, the
/// selection collapses to zero width.
#[test]
fn probe_click_in_visual_keeps_anchor() {
    let mut f = Fixture::new("abcdef");
    f.feed(["v"]);
    f.vim.set_cursor_offset(&f.buf, 4); // click on 'e'
    let sel = f.vim.visual_selection();
    assert_eq!(
        sel,
        Some((0, 4, vimcore::mode::VisualKind::Char)),
        "click in visual must keep the anchor at 0 (vim: selection follows the cursor)"
    );
}

/// PROBE: `mA` — uppercase marks are global-file marks in vim; a
/// single-buffer engine may treat them as local, but set+jump must work.
#[test]
fn probe_uppercase_mark_set_and_jump() {
    let mut f = edit("one\ntwo\nthree\n", 0, 0, &["m", "A"]);
    assert_eq!(f.vim.marks.get('A'), Some(0), "mA must store a mark");
    f.feed(["G", "`", "A"]);
    assert_eq!(f.vim.cursor_offset(), 0);
}

/// PROBE: `q1` — vim only accepts a-zA-Z0-9 as macro registers; a random
/// char (e.g. `q/`) must be rejected, not silently recorded.
#[test]
fn probe_macro_register_charset() {
    let mut f = Fixture::new("abc\n");
    f.feed(["q", "/"]);
    assert!(f.vim.macro_recording().is_none(), "q/ must not start a recording");
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 1, "qa…q records into a");
}

/// PROBE: `""p` — an empty register spelling pastes the unnamed register.
#[test]
fn probe_double_quote_register() {
    let f = edit("ab\ncd\n", 0, 0, &["yy", "j", "\"\"", "p"]);
    assert!(
        f.text().contains("ab\n"),
        "empty register spec = unnamed; got {:?}",
        f.text()
    );
}

/// PROBE: `g;`/`g,` with an empty changelist and huge counts must not panic.
#[test]
fn probe_changelist_edge() {
    let mut f = Fixture::new("x\n");
    f.feed(["g;", "g,", "g;", "10g;", "10g,"]);
    assert_eq!(f.vim.cursor_offset(), 0);
}

/// PROBE: C-o/C-i walking with an empty jumplist, then real jumps.
#[test]
fn probe_jumplist_walk() {
    let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
    f.feed(["<C-o>", "<C-i>", "G", "<C-o>", "<C-o>", "<C-i>"]);
    assert!(f.vim.cursor_offset() < f.buf.len());
}

/// PROBE: octal increment growing past its width (`0777` + C-a → `01000`).
#[test]
fn probe_octal_increment_overflow() {
    let f = edit("0777\n", 0, 0, &["<C-a>"]);
    assert_eq!(
        f.text(),
        "01000\n",
        "octal must grow past its width, got {:?}",
        f.text()
    );
}

/// PROBE: `:set`-describe with garbage names must return None, not panic.
/// (The `?` suffix is stripped by the cmdline caller, not by `describe`.)
#[test]
fn probe_describe_garbage() {
    let o = vimcore::options::Options::default();
    assert_eq!(o.describe("notabstop"), None);
    assert_eq!(o.describe("nosuchopt"), None);
    assert_eq!(o.describe("nu").as_deref(), Some("nonumber"));
}

/// PROBE: register semantics — uppercase append merges linewise; blackhole
/// delete leaves the unnamed register untouched.
#[test]
fn probe_register_semantics() {
    let mut r = Registers::default();
    r.store_yank(Some('a'), "one\n".into(), RegisterKind::Linewise);
    r.store_delete(Some('A'), "two\n".into(), RegisterKind::Linewise);
    let reg = r.get('a').unwrap();
    assert_eq!(reg.text, "one\ntwo\n", "uppercase append merges, got {:?}", reg);
    assert_eq!(reg.kind, RegisterKind::Linewise);

    let mut r2 = Registers::default();
    r2.store_yank(None, "keep\n".into(), RegisterKind::Linewise);
    r2.store_delete(Some('_'), "gone\n".into(), RegisterKind::Linewise);
    assert_eq!(r2.get('"').unwrap().text, "keep\n");
}

/// PROBE (round-19 correction): the old pin here claimed `3D` at col 0
/// "empties line 1" — that was REASONING, never a real probe. Byte-level
/// vim 9.1 PTY probes (round 19): `count >= 2` from the line start deletes
/// the covered lines WHOLE (`3D` → "dddd\n", `2D` on a/b/c → "c\n",
/// `99D` on two lines → empty buffer), while count=1 empties the cursor
/// line (see review_regressions). With the cursor ON a char (col 1) the
/// round-9 probe shape holds (`99D` → "a").
#[test]
fn probe_count_d_shapes() {
    let f = edit("aaaa\nbbbb\ncccc\ndddd\n", 0, 0, &["3", "D"]);
    assert_eq!(f.text(), "dddd\n", "3D from col 0 deletes covered lines whole, got {:?}", f.text());

    let f2 = edit("aaaa\nbbbb\n", 0, 0, &["9", "9", "D"]);
    assert_eq!(f2.text(), "", "99D from (0,0) deletes the whole buffer, got {:?}", f2.text());

    let f3 = edit("aaaa\nbbbb\n", 0, 1, &["9", "9", "D"]);
    // round-19 correction: the old pin "a" claimed the final newline goes
    // too — that was reasoning, not a probe. Byte-level vim 9.1 PTY probe
    // (`ggl2D` on "aaaa\nbbbb\n" then :wq → file "a\n"): the file's final
    // newline SURVIVES a from-mid-line count-D.
    assert_eq!(f3.text(), "a\n", "99D from col 1 keeps the text before the cursor AND the file eol");
}

/// PROBE: a self-expanding mapping pair must trip the depth guard, not hang
/// or corrupt the buffer.
#[test]
fn probe_mapping_ping_pong_terminates() {
    let mut f = Fixture::new("hello\n");
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "x", "y");
    f.vim.keymaps_mut().map_str(ModeClass::Normal, "y", "x");
    let r = f.feed_raw(vimcore::key::Key::char('x'));
    assert_eq!(r, KeyResult::Consumed);
    assert_eq!(f.text(), "hello\n", "ping-pong mapping aborts without editing");
}

/// PROBE: `0` typed while a count is pending is a count digit (`10j`).
/// `0` alone must stay the line-start motion (documented, but guard the
/// boundary: `100` then escape then `0`).
#[test]
fn probe_zero_after_escape_is_motion() {
    let f = edit("abc\ndef\n", 0, 2, &["10", "escape", "0"]);
    assert_eq!(f.vim.cursor_offset(), 0, "0 after cancel is the line-start motion");
}

/// PROBE: `.` after a no-op command (`x` on an empty line) must not replay
/// a stale change (vim: `.` with no last change beeps).
#[test]
fn probe_dot_with_no_change_bells() {
    let mut f = Fixture::new("\nfoo\n");
    f.feed(["x"]);
    let bells_before = f.host.bells;
    f.feed(["."]);
    assert!(
        f.host.bells > bells_before,
        "`.` with no recorded change must bell (vim E30-ish)"
    );
    assert_eq!(f.text(), "\nfoo\n");
}

/// PROBE: huge count on `dd` stays bounded and does not panic.
#[test]
fn probe_huge_count_dd() {
    let digits = ["9"; 20];
    let mut keys: Vec<&str> = digits.to_vec();
    keys.extend(["d", "d"]);
    let f = edit("a\nb\n", 0, 0, &keys);
    assert_eq!(f.text(), "", "count saturates, both lines die");
}

// ---- 数据丢失面 -----------------------------------------------------------

/// `~` 消费整个字素簇但只映射基字符：组合符与 ZWJ 家族尾巴必须原样保留
/// （vim 9.1：`~` 后 `E`+U+0301 仍是 3 字符；emoji 家族完全不变）。
#[test]
fn tilde_preserves_grapheme_tail() {
    // 分解形 é（e + U+0301）+ x：~ 只翻 e 的大小写
    let f = edit("e\u{0301}x\n", 0, 0, &["~"]);
    assert_eq!(f.text(), "E\u{0301}x\n", "组合符尾巴必须保留");

    // ZWJ emoji 家族：~ 在基字符上，家族保持 9 字节不动，光标右移
    // ZWJ emoji 家族：~ 消费整个家族但全部原样写回（基字符无大小写），
    // 一个成员都不能少（vim 9.1 探针：家族完全不变）
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F466}";
    let f2 = edit(&format!("{family} b\n"), 0, 0, &["~"]);
    assert_eq!(f2.text(), format!("{family} b\n"), "家族成员一个都不能少");
}

/// linewise 寄存器的尾部空行在 p/P 下存活（vim 9.1：["a",""] 的寄存器
/// 粘出两行）。
#[test]
fn linewise_put_keeps_trailing_empty_lines() {
    use vimcore::registers::RegisterKind;
    // P above: 5 行
    let mut f = Fixture::new("a\n\nz\n");
    f.feed(["g", "g", "V", "j", "y"]); // register = "a\n\n"
    f.feed(["G", "P"]);
    assert_eq!(f.text(), "a\n\na\n\nz\n", "P 保留寄存器的空行");

    // p past buffer end（末行无换行）：["a",""]
    let mut f2 = Fixture::new("z");
    f2.vim
        .registers
        .store_yank(None, "a\n\n".into(), RegisterKind::Linewise);
    f2.feed(["p"]);
    // ["z","a",""] 的字节文本是 "z\na\n"（末尾空行以换行结尾）
    assert_eq!(f2.text(), "z\na\n", "p 保留寄存器的空行");
}

// ---- 块会话面 -------------------------------------------------------------

/// 块会话退出时若打字行增量 ≠ 副本长度（发散会话），跳过复制而非写出
/// 错位副本。确定性复现 fuzz 抓到的 mid-char insert：3o 残留的
/// count-repeat 曾在块会话退出时先插入文本，把复制偏移顶进多字节字符。
#[test]
fn block_session_replication_divergence_safe() {
    // 块 I 会话中途用 <Del> 删掉打字文本（vim 探针：不复製任何行）
    let mut f = Fixture::new("ab中\ncd中\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    f.feed(["<Left>", "<Del>", "escape"]);
    assert_eq!(f.text(), "ab中\ncd中\n", "发散会话不复製（vim 9.1 实证）");
}

/// 关闭 hlsearch 逐编辑重扫后，宿主的 refresh_highlights 必须真正刷新
/// 匹配缓存与高亮（曾被 hlsearch_live_update 守卫短路成永久空操作）。
#[test]
fn refresh_highlights_works_with_live_update_off() {
    let mut f = Fixture::new("fox dog fox\n");
    f.vim.set_hlsearch_live_update(false);
    f.feed(["/", "f", "o", "x", "<CR>"]);
    f.feed(["x"]); // 编辑删掉一个 o：live update 关闭，缓存允许过期
    let mut ctx = vimcore::state::Ctx {
        buf: &mut f.buf,
        host: &mut f.host,
    };
    f.vim.refresh_highlights(&mut ctx);
    // 缓存与高亮都刷新到当前文本（1 个 fox：第一个被 x 咬掉一个 o）
    assert_eq!(f.vim.search.last_matches.len(), 1, "显式刷新更新匹配缓存");
    assert_eq!(f.host.highlights.len(), 1, "显式刷新发布新高亮");
}

// ---- visual 行语义（vim 探针：v_D/v_X 删整行、v_Y 行复制、v_C=v_S 行改） --

#[test]
fn visual_linewise_spellings() {
    // v_D 删覆盖行
    let f = edit("abcd\nefgh\nijkl\n", 0, 0, &["v", "l", "D"]);
    assert_eq!(f.text(), "efgh\nijkl\n", "v_D 行删除");

    // v_Y 行复制（p 开新行）
    let f2 = edit("abcd\nefgh\n", 0, 0, &["v", "l", "Y", "p"]);
    assert_eq!(f2.text(), "abcd\nabcd\nefgh\n", "v_Y 行复制");

    // v_C 行修改（保留行，清内容，进插入）
    let mut f3 = edit("abcd\nefgh\nijkl\n", 0, 0, &["v", "j", "l", "C"]);
    f3.type_text("x");
    f3.feed(["escape"]);
    assert_eq!(f3.text(), "x\nijkl\n", "v_C 行修改");

    // v_x 保持字符语义
    let f4 = edit("abcd\nefgh\n", 0, 0, &["v", "l", "x"]);
    assert_eq!(f4.text(), "cd\nefgh\n", "v_x 字符删除");
}

// ---- ap 段落跨度（方向化）--------------------------------------------------

#[test]
fn ap_paragraph_span_directional() {
    // 文本行：吞掉后面全部连续空行（vim: [aaa,bbb,"","",ccc] 2Gdap → [ccc]）
    let f = edit("aaa\nbbb\n\n\nccc\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f.text(), "ccc\n", "全部尾随空行一起删除");

    // 空行：空行块 + 整个下一段落（不含它的尾随空行）
    let f2 = edit("aaa\n\n\nbbb\nccc\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f2.text(), "aaa\n", "空行会话吞整个下一段落");

    let f3 = edit("aaa\n\nbbb\n\nx\n", 1, 0, &["d", "a", "p"]);
    assert_eq!(f3.text(), "aaa\n\nx\n", "下一段落自己的尾随空行保留");
}

// ---- 块 A 中带补齐（vim 探针："123456"/"12" → "123 X456"/"12  X"）---------

#[test]
fn block_append_pads_to_append_column() {
    // 注意：引擎的块 visual `l` 不越过行尾（vim 的块可进入虚拟列——记录在
    // NOTES 分歧 #35），所以短行放在块中间构造
    let mut f = Fixture::new("123456\n1\n123456\n");
    f.feed(["<C-v>", "j", "j", "l", "A"]);
    f.type_text("_X");
    f.feed(["escape"]);
    assert_eq!(f.text(), "12_X3456\n1 _X\n12_X3456\n", "短行补齐到追加列");
}

// ---- Ex 命令面 ------------------------------------------------------------

#[test]
fn ex_bang_forms_and_abbreviations() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed([":", "w", "!", "<CR>"]);
    assert_eq!(f.host.saved, 1, ":w! 保存（曾 E492）");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "2", "d", "e", "<CR>"]);
    assert_eq!(f2.text(), "a\nc\n", ":2de 缩写删除");

    let f3 = edit("ab\n", 0, 0, &[":", "s", "u", "/", "a", "/", "b", "/", "<CR>"]);
    assert_eq!(f3.text(), "bb\n", ":su 是 :substitute 的缩写");
}

#[test]
fn ex_garbage_count_reports_e488_and_deletes_nothing() {
    // 【第十四轮修正】vim 9.1 探针 P13：`:1,2d 3x` 报 E488 且**不删除**。
    // 第十三轮把垃圾 count 当 0（=无 count）照常删整个 typed range——
    // 相邻修复方向对了，终点错了；第十四轮的 parse_reg_count 现在报错。
    let f = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "2", "d", " ", "3", "x", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nc\n", "垃圾尾参 = E488，缓冲不动");
    let last = f.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E488"), "got {last:?}");
}

#[test]
fn ex_address_whitespace_and_midrange_percent() {
    // 【第十四轮修正】地址内空白跳过用**范围内**的例子证明（vim 9.1）：
    // `:2 +1d` 删第 3 行。越界地址 `:5 +2` 现按探针 P15 报 E16（第十二轮
    // 注释里的「clamp 到 $」是当时实现的期望，非 vim 行为）。
    let f = edit("a\nb\nc\nd\n", 0, 0, &[":", "2", " ", "+", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "a\nb\nd\n", "地址内空白被跳过");

    let f3 = edit("a\nb\nc\n", 0, 0, &[":", "5", " ", "+", "2", "d", "<CR>"]);
    let last = f3.host.statuses.last().map(String::as_str).unwrap_or("");
    assert!(last.starts_with("E16"), "越界地址 = E16，got {last:?}");

    let f2 = edit("a\nb\nc\n", 0, 0, &[":", "1", ",", "%", "d", "<CR>"]);
    assert_eq!(f2.text(), "", "mid-range % = 1,$（曾是 cursor line）");
}

#[test]
fn ex_comment_lines() {
    // 行首 " 整行注释静默跳过（曾 E492）
    let mut f = Fixture::new("a\n");
    f.feed([":", " ", "\"", " ", "s", "c", "r", "a", "t", "c", "h", "<CR>"]);
    assert_eq!(f.text(), "a\n");
    assert!(f.host.bells == 0, "注释行不响铃");

    // :set 尾注释
    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "s", "e", "t", " ", "t", "s", "=", "8", " ", "\"", " ", "x", "<CR>"]);
    assert!(f2.host.bells == 0, ":set ts=8 \" x 静默生效");
}

#[test]
fn cmdline_editing_chords() {
    // C-u 清行、C-w 删词
    let mut f = Fixture::new("a\n");
    f.feed([":", "f", "o", "o"]);
    f.feed_raw(vimcore::key::Key::ctrl_char('u'));
    f.feed(["w"]);
    f.feed(["<CR>"]);
    assert_eq!(f.text(), "a\n", "C-u 清掉 foo，只剩 :w");
    assert_eq!(f.host.saved, 1);

    let mut f2 = Fixture::new("a\n");
    f2.feed([":", "f", "o", "o", " ", "b", "a"]);
    f2.feed_raw(vimcore::key::Key::ctrl_char('w'));
    assert_eq!(f2.vim.cmdline.buffer, "foo ", "C-w 删最后一个词");
}

#[test]
fn cmdline_history_dedups_nonconsecutive() {
    use vimcore::key::Key;
    let mut f = Fixture::new("a\n");
    for line in ["/foo", "/bar", "/foo"] {
        let mut chars = line.chars();
        f.feed_raw(Key::char(chars.next().unwrap())); // 提示符
        for c in chars {
            f.feed_raw(Key::char(c));
        }
        f.feed_raw(Key::enter());
    }
    let history = f.vim.cmdline.history.get(&'/').unwrap();
    assert_eq!(history, &vec!["bar".to_owned(), "foo".to_owned()], "旧重复搬走");
}

// ---- 宏与映射面 -----------------------------------------------------------

#[test]
fn macro_register_rules() {
    use vimcore::keymap::ModeClass;
    // qA 追加到 a 的录制
    let mut f = Fixture::new("abc\n");
    f.feed(["q", "a", "x", "q"]);
    f.feed(["j"]);
    f.feed(["q", "A", "x", "q"]);
    assert_eq!(f.vim.macro_len('a'), 2, "qA 追加到 a");
    assert_eq!(f.vim.macro_len('A'), 0, "没有独立的 A 槽");

    // q 前的 count 不泄漏
    let mut f2 = Fixture::new("abc\n");
    f2.feed(["2", "q", "a", "q"]);
    f2.feed(["x"]);
    assert_eq!(f2.text(), "bc\n", "count 2 在 q 后不放大 x");
    let _ = ModeClass::Normal;
}

/// mapping 前缀压住的多个 declined 打印键必须按序交给宿主（单槽 IOU
/// 曾把 j 丢掉——`:imap jk <Esc>` 后打 `jx` 得到 `xXabc`）。
#[test]
fn declined_printables_queue_in_order() {
    use vimcore::keymap::ModeClass;
    let mut f = Fixture::new("abc\n");
    f.vim.keymaps_mut().map_str(ModeClass::Insert, "jk", "<Esc>");
    f.feed(["i"]);
    f.feed(["j"]); // mapping 前缀：引擎答 Consumed，欠宿主一个 j
    f.feed(["x"]); // x 打破前缀：j、x 都应排队待宿主放置
    let pending = f.vim.take_pending_unknown_chars();
    assert_eq!(pending, vec!['j', 'x'], "两个 declined 字符都欠着，按序");
}

// ---- 配置面 ---------------------------------------------------------------

#[test]
fn config_g_mapleader_and_tab_separator() {
    use vimcore::config;
    let cfg = config::parse("let g:mapleader = \",\"\nnnoremap <Leader>w :w<CR>\n");
    assert!(
        !cfg.mappings.is_empty(),
        "g:mapleader 生效，<Leader>w 解析为 ,w"
    );
    assert_eq!(cfg.mappings[0].lhs.len(), 2, "leader + w 两个键");

    let cfg2 = config::parse("nnoremap\t<Leader>x ix<CR>");
    assert!(!cfg2.ignored.iter().any(|l| l.contains("nnoremap\t")), "TAB 分隔不进 ignored");
    assert_eq!(cfg2.mappings.len(), 1, "TAB 分隔的映射正常解析");
}