//! 第九轮检视回归测试。
//!
//! 流程沿用前几轮：读码列可疑点 → vim 9.1 探针实证（`-es` 脚本）→
//! 修复 → 这里入册。每条测试标注实证方式。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

// ---- 1. gq 不再翻倍空行/空白行 -----------------------------------------------

/// vim 9.1 探针（`ggVGgq` over ["","",""]）：空行原样保留——旧行为对每个
/// 空行都做一次空段落 flush（多出终止 \n），三行空行翻倍成七行。
#[test]
fn gq_keeps_blank_lines_one_to_one() {
    let mut f = Fixture::new("\n\n\n");
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n\n\n", "空行 1:1 保留");
}

/// vim 9.1 探针（tw=10，["hello world foo bar baz qux","","","tail"]）：
/// 段落重排后两个空行仍是两个；换行后段落/wrap 行为不变。
#[test]
fn gq_mixed_paragraphs_and_blank_lines() {
    let mut f = Fixture::new("hello world foo bar baz qux\n\n\ntail\n");
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "hello\nworld foo\nbar baz\nqux\n\n\ntail\n"
    );
}

/// vim 9.1 探针（["para","   ","tail"] 同款）：纯空白行 gq 后**逐字保留**
/// （三个空格仍是三个空格），旧行为会塌成空行。
#[test]
fn gq_keeps_whitespace_only_lines_verbatim() {
    let mut f = Fixture::new("hello world foo bar baz qux\n   \ntail\n");
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["g", "g", "V", "G", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "hello\nworld foo\nbar baz\nqux\n   \ntail\n"
    );
}

// ---- 2. D/C 行计数 -------------------------------------------------------------

/// vim 9.1 探针（['aaaa','bbbb','cccc','dddd'] 光标 (1,2) `3D`）：
/// ['a','dddd']——删除从光标到下方第 3 行行尾。旧行为忽略 count。
#[test]
fn d_with_count_deletes_through_count_lines() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd", 0, 1);
    f.feed(["3", "D"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "a\ndddd");
    assert_eq!(f.line(), 0);
}

/// vim 9.1 探针（同缓冲 (1,2) `2Cnew<Esc>`）：['anew','cccc','dddd']——
/// 光标列保留、下方 2 行内容清掉后进入插入。旧行为只改当前行。
#[test]
fn c_with_count_changes_through_count_lines() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc\ndddd", 0, 1);
    f.feed(["2", "C"]);
    assert!(f.vim.mode() == vimcore::Mode::Insert);
    f.type_text("new");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "anew\ncccc\ndddd");
}

/// 计数越过缓冲尾时饱和（vim 9.1 探针：['aaaa','bbbb'] (1,2) `99D` →
/// ['a']——最后的换行一起删掉）。
#[test]
fn d_count_saturates_at_buffer_end() {
    let mut f = Fixture::at("aaaa\nbbbb", 0, 1);
    f.feed(["9", "9", "D"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "a");
}

// ---- 3. insert Tab 按显示列对齐 -------------------------------------------------

/// vim 9.1 探针（expandtab ts=4，'中文' 行尾 Tab）：插入 **4** 个空格——
/// 对齐按显示列（中文宽 2，显示列 4 已是 ts 的倍数，vim 补满到 8）。
/// 旧行为按字节列（6 % 4 = 2）只插 2 个。
#[test]
fn insert_tab_aligns_by_display_column_after_wide_chars() {
    let mut f = Fixture::at("中文", 0, 6);
    f.feed(["A"]);
    f.vim.options.expandtab = true;
    f.vim.options.tabstop = 4;
    f.feed_raw(vimcore::key::Key::named("tab"));
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文    ");
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文    x");
}

// ---- 4. insert <C-w> 行首并线（vim 语义）+ 块会话封锁 ---------------------------

/// vim 9.1 探针（['aaaa','bbbb'] 光标 (2,1) `i<C-w>x<Esc>`）：
/// ['aaaaxbbbb']——行首的 <C-w> 与上一行并线（只删换行），旧行为
/// 钳在行首不动（'aaaa\nxbbbb'）。
#[test]
fn insert_ctrl_w_at_line_start_joins_previous_line() {
    let mut f = Fixture::at("aaaa\nbbbb", 1, 0);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaaxbbbb");
}

/// 行中 <C-w> 不跨行：'aa bb cc' 光标在 'c' 上 <C-w> 删掉 "bb "，
/// 行为与 vim 一致（词 + 前导空白一起删）。
#[test]
fn insert_ctrl_w_mid_line_stays_within_line() {
    let mut f = Fixture::at("aa bb cc", 0, 6);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aa xcc");
}

/// 块插入会话中 <C-w> 不得并线：复制偏移假设每行独立（fuzz round8 的
/// 行首 BS 同族）。响铃拒绝；行内的缩进/词删除不受影响。
#[test]
fn insert_ctrl_w_never_joins_lines_mid_block_session() {
    let mut f = Fixture::at("aaaa\nbbbb\ncccc", 0, 0);
    f.feed(["<C-v>", "j", "I"]);
    let bells_before = f.host.bells;
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('w'),
    });
    assert_eq!(f.host.bells, bells_before + 1, "跨行 <C-w> 响铃拒绝");
    f.type_text("X");
    f.feed(["<Esc>"]);
    // 行结构保持三行（没有并线），键入仍复制到两行
    assert_eq!(f.buf.line_count(), 3);
    assert_eq!(f.buf.slice(0..f.buf.len()), "Xaaaa\nXbbbb\ncccc");
}

// ---- 5. Ex 命令的 changelist / `.` mark 记在受影响行 ---------------------------

/// vim：`:4s` 后 `.` mark 与 changelist 指向被替换行。两次替换后 g; 应
/// 回到**上一次**（第一次）替换的行——旧行为 changelist 记的是命令前的
/// 旧光标（一直在 line 0），g; 无处可去。
#[test]
fn ex_substitute_records_changelist_on_substituted_line() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\nfive\n");
    f.feed(["g", "g"]);
    f.feed([":", "4", "s", "/", "o", "/", "0", "/", "<CR>"]);
    assert_eq!(f.line(), 3, "光标落在最后一个替换行");
    assert_eq!(
        f.vim.marks.last_change.map(|o| f.buf.offset_to_line(o)),
        Some(3),
        "`.` mark 记在替换行"
    );
    f.feed(["g", "g"]);
    f.feed([":", "2", "s", "/", "w", "/", "W", "/", "<CR>"]);
    assert_eq!(f.line(), 1);
    f.feed(["g", ";"]);
    assert_eq!(f.line(), 3, "g; 回到上一次替换行（旧行为留在 line 1）");
}

/// :d 同族：changelist / `.` mark 记在删除发生的行（存活行），不是命令前
/// 的旧光标。
#[test]
fn ex_delete_records_changelist_on_surviving_line() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed(["g", "g", "j"]); // 光标先挪到 line 1，与删除后的落点区分开
    f.feed([":", "1", ",", "2", "d", "<CR>"]);
    assert_eq!(
        f.vim.marks.last_change.map(|o| f.buf.offset_to_line(o)),
        Some(0),
        "`.` mark 记在存活的 three 行（旧行为记在旧光标 line 1）"
    );
}

// ---- 6. changelist 两端的错误消息分方向 -----------------------------------------

/// vim：changelist 起点之外是 E662（At start），终点之外是 E663（At end）。
/// 旧行为两个方向都报 E662。
#[test]
fn newer_change_at_end_reports_e663() {
    let mut f = Fixture::new("one\n");
    f.feed(["i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    f.feed(["g", ","]);
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E663")),
        "前进方向到顶报 E663，实际 {:?}",
        f.host.statuses
    );
}

#[test]
fn older_change_at_start_reports_e662() {
    let mut f = Fixture::new("one\n");
    f.feed(["i"]);
    f.type_text("x");
    f.feed(["<Esc>"]);
    f.feed(["g", ";"]); // 只有一条变更，g; 无更早的条目
    assert!(
        f.host.statuses.iter().any(|s| s.starts_with("E662")),
        "后退方向到底报 E662，实际 {:?}",
        f.host.statuses
    );
}

// ---- 7. r<CR> 光标落在下一行首 --------------------------------------------------

/// vim 9.1 探针（['abc','def'] `r<CR>`）：光标 (2,1)——替换成换行后落在
/// 新下一行的第一个字符上。旧行为 clamp 回上一行行首。
#[test]
fn replace_with_newline_lands_cursor_on_next_line() {
    let mut f = Fixture::at("abc\ndef", 0, 0);
    f.feed(["r"]);
    f.feed_raw(vimcore::key::Key::enter());
    assert_eq!(f.buf.slice(0..f.buf.len()), "\nbc\ndef");
    assert_eq!(f.line(), 1, "光标落在下一行");
    assert_eq!(f.cursor(), f.buf.line_start(1));
}

// ---- 8. :j 带计数 / :substitute 全拼 ---------------------------------------------

/// vim 9.1 探针（['aaaa'..'dddd'] `:2j 3`）：从 range 末行起并 3 行
/// → ['aaaa','bbbb cccc dddd']。旧行为忽略计数只并 2、3 行。
#[test]
fn ex_join_with_count() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\ndddd\n");
    f.feed([":", "2", "j", " ", "3", "<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaa\nbbbb cccc dddd\n");
}

/// vim 的 `:[range]s[ubstitute]` 全拼：`:%substitute/a/b/` 与 `:%s/a/b/`
/// 同义（无 g 旗标每行只换首个匹配）。旧行为 E492。
#[test]
fn substitute_full_spelling() {
    let mut f = Fixture::new("aaa\naba\n");
    f.feed([":"]);
    // Ex 命令行按字符喂（feed 的领域是「单个键」，多字符裸串是 Named 键）
    for c in "%substitute/a/b/".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "baa\nbba\n");
    assert!(
        f.host.statuses.iter().any(|s| s.contains("substitution")),
        "替换计数反馈仍在，实际 {:?}",
        f.host.statuses
    );
}

// ---- 10. `:s/foo` 缺失 replacement = 空；块 p 寄存器行耗尽留空 -----------------

/// vim 9.1 探针（`:s/foo` 无尾部分隔符）：等价 `:s/foo//`——删除匹配，
/// 不是响铃也不复用上一条命令的 replacement。
#[test]
fn substitute_without_replacement_deletes_the_match() {
    let mut f = Fixture::new("foo\n");
    f.feed([":"]);
    for c in "s/foo".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n");
    assert_eq!(f.host.bells, 0, "不响铃（旧行为缺 replacement 直接响铃）");
}

/// vim 9.1 探针（2 行块寄存器贴到 3 行块选区）：第三行的覆盖区间**直接
/// 删除**（空串），不循环也不重复末行。旧行为把寄存器末行贴上去。
#[test]
fn block_put_with_short_register_leaves_exhausted_rows_empty() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\n");
    f.feed(["g", "g", "<C-v>", "j", "l", "y"]); // 2 行块 "aa"/"bb"
    f.feed(["g", "g", "l", "l", "<C-v>", "j", "j", "l", "l", "p"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaa\nbbbb\ncc\n");
}

/// `<C-u>` 在行首与上一行并线（vim 9.1 探针：['aaaa','bbbb'] (2,1)
/// `i<C-u>x<Esc>` → ['aaaaxbbbb']），旧行为钳在行首不动。
#[test]
fn insert_ctrl_u_at_line_start_joins_previous_line() {
    let mut f = Fixture::at("aaaa\nbbbb", 1, 0);
    f.feed(["i"]);
    f.feed_raw(vimcore::key::Key {
        modifiers: vimcore::key::Modifiers::ctrl(),
        kind: vimcore::key::KeyKind::Char('u'),
    });
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "aaaaxbbbb");
}

// ---- 12. insert_change_pos 跨行移动后 floor（fuzz round9 抓取） -----------------

/// fuzz round9（seed 7）抓取：insert 会话首敲位置（insert_change_pos）是
/// 裸 offset，编辑漏斗不调整——会话中 BS 并线把文本挪动后，exit 记进
/// changelist 的偏移落在多字节字符中间，`:marks` 列 `.` 行时宿主
/// offset_to_line panic。修复：exit 时 floor 到当前文本的字符边界。
#[test]
fn insert_change_pos_floored_after_mid_session_line_join() {
    // "中文\nab\n"：行 1 首敲位置 = offset 5；BS 并线删掉 \n(4) 后文本变
    // "中文zab\n"，offset 5 落在 '文'（3..6）中间——旧行为把它原样记进
    // changelist，`:marks` 列 `.` 行时宿主 offset_to_line panic。
    let mut f = Fixture::at("中文\nab\n", 1, 0);
    f.feed(["i"]);
    f.type_text("z"); // insert_change_pos = 5
    f.feed(["<left>", "<BS>"]); // 并线：文本变 "中文zab\n"
    f.feed(["<Esc>"]); // exit_insert 记 changelist
    let text = f.text();
    assert_eq!(text, "中文zab\n");
    match f.vim.marks.last_change {
        Some(o) => assert!(
            o <= text.len() && (o == text.len() || text.is_char_boundary(o)),
            "changelist 记位必须在字符边界上（fuzz round9 抓到 mid-char），got {o}"
        ),
        None => panic!("changelist 应有记录"),
    }
    // `:marks` 列出 `.` 不得 panic
    f.feed([":"]);
    for c in "marks".chars() {
        f.feed_raw(vimcore::key::Key::char(c));
    }
    f.feed(["<CR>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "中文zab\n");
}

// ---- 11. :bfirst / :blast ---------------------------------------------------------

/// NOTES.md 第八轮「悬而未决」：宿主 trait 补 `first_buffer`/`last_buffer`
/// 后放行 `:bfirst`/`:blast`（vim 别名 `:bf`/`:bl`/`:brewind`）。
#[test]
fn bfirst_and_blast_reach_the_host() {
    let mut f = Fixture::new("one\n");
    f.feed([":", "b", "f", "i", "r", "s", "t", "<CR>"]);
    assert!(f.host.first_last_calls == vec![true]);
    f.feed([":", "b", "l", "a", "s", "t", "<CR>"]);
    assert!(f.host.first_last_calls == vec![true, false]);
    // 空缓冲列表（宿主返回 false）→ bell
    f.host.first_last_result = false;
    let bells = f.host.bells;
    f.feed([":", "b", "f", "<CR>"]);
    assert!(f.host.bells > bells, "宿主拒绝时响铃");
}
