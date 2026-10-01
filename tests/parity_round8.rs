//! 第八轮检视回归测试。
//!
//! 流程沿用前几轮：读码列可疑点 → vim 9.1 探针实证（`-es` 脚本）→
//! 修复 → 这里入册。每条测试标注实证方式。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

// ---- 1. gq 从行首格式化（charwise 起点不得复制光标前文本） ------------------

/// vim 9.1 探针：光标在 'bb' 上 `gq}`，首行从 column 0 整行重排
/// （['aaaa bb cc ddd','ee ff'] → ['aaaa bb cc ddd ee ff']）。
/// 旧行为替换区间从 span.start（行中）开始而重排文本含整行 →
/// 光标前的 "aaaa " 复制一份（"aaaa aaaa bb cc ddd ee ff"）。
#[test]
fn gq_formats_from_line_start_with_midline_cursor() {
    let mut f = Fixture::at("aaaa bb cc ddd\nee ff\n\ngg hh ii jj kk ll\n", 0, 5);
    f.feed(["g", "q", "}"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa bb cc ddd ee ff\n\ngg hh ii jj kk ll\n",
        "gq 从行首格式化，不得复制光标前的文本"
    );
}

/// 同款修正覆盖 charwise visual 起点：`v$gq` 在窄 textwidth 下从行首
/// 重排（vim 9.1 探针：装得下时单行不变；关键是光标前的文本不得重复）。
#[test]
fn gq_formats_from_line_start_with_visual_span() {
    let mut f = Fixture::at("aaaa bb cc ddd\nee ff\n", 0, 5);
    f.feed([":", "s", "e", "t", " ", "t", "w", "=", "1", "0", "\n"]);
    f.feed(["v", "$", "g", "q"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "aaaa bb cc\nddd\nee ff\n",
        "v$ 的 gq 从行首整行重排（旧行为会复制光标前的 aaaa ）"
    );
}

// ---- 2. 非变更 Ex 命令不进 `.` 重放 ------------------------------------------

/// vim 9.1 探针：`:2`（纯移动）+ `x` 后 `gg` `.`，vim 只重放 `x`
/// （"one"→"ne"）。旧行为把 `:2<CR>` 留在录制里，`.` 变成
/// 「跳到 L2 再删」——重放位置完全错行。
#[test]
fn dot_ignores_nonmutating_ex_commands() {
    let mut f = Fixture::new("one\ntwo\nthree\n");
    f.feed([":", "2", "\n"]); // 无编辑的移动
    f.feed(["x"]); // last_change = x（"two" → "wo"）
    f.feed(["g", "g"]);
    f.feed(["."]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ne\nwo\nthree\n",
        ". 只重放 x，不得重放之前的 :2 移动"
    );
}

/// E492 未知命令同样不污染（旧行为下 `:foo` 后的下一个变更会带着它）。
#[test]
fn dot_ignores_failed_ex_commands() {
    let mut f = Fixture::new("ab\n");
    f.feed([":", "f", "o", "o", "\n"]); // E492
    f.feed(["x"]);
    f.feed(["."]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "\n", "x 连续两次删完");
}

/// `:noh`（查询/清除类）后 `.` 仍重放的是变更本身。
#[test]
fn dot_after_noh_repeats_the_change_only() {
    let mut f = Fixture::new("abc\n");
    f.feed(["x"]);
    f.feed([":", "n", "o", "h", "\n"]);
    f.feed(["."]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "c\n");
}

// ---- 3. visual 块选大小写算子（u/U/~） ----------------------------------------

/// vim 9.1 探针：`<C-v>jllU` 把块列换成大写，光标落块起点。
/// 旧行为响铃不动（apply_block_operator 只认 Delete/Yank/Change）。
#[test]
fn block_visual_uppercase_maps_rows() {
    let mut f = Fixture::at("abc\ndef\nghi\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "l", "U"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ABC\nDEF\nghi\n",
        "块 U 覆盖两行三列（j 后 ll 使块宽 3）"
    );
    assert_eq!(f.line(), 0, "光标落块起点行");
}

#[test]
fn block_visual_toggle_and_lowercase() {
    // <C-v>j~ 的块宽为 1（光标字符本身）：只翻转每行首字符
    let mut f = Fixture::at("aBc\ndEf\n", 0, 0);
    f.feed(["<C-v>", "j", "~"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "ABc\nDEf\n", "块 ~ 只覆盖块列");

    let mut f = Fixture::at("ABC\nDEF\n", 0, 1);
    f.feed(["<C-v>", "j", "u"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "AbC\nDeF\n", "块 u 只小写块列");
}

/// 多字节行上的块 U：块列按显示列解析（CJK 宽 2），替换走 edit_replace，
/// 行内字节偏移不漂移。
#[test]
fn block_visual_case_on_multibyte_rows() {
    let mut f = Fixture::at("中文ab\n日本cd\n", 0, 6); // byte 6 = 'a'（显示列 4）
    f.feed(["<C-v>", "j", "U"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "中文Ab\n日本Cd\n",
        "块 U 覆盖两行的 'a'/'c' 列"
    );
}

// ---- 4. Vp 行级粘贴不破坏存活的缩进行 ----------------------------------------

/// vim 9.1 探针：寄存器 "XY"（linewise），V 选 ['abc'] 后 p，存活的缩进行
/// ' ghi' 完好（['XY',' ghi','tail']）。旧行为在 delete_span 停放的光标
/// （first_non_blank，行中）插入 → " XY" 且 'ghi' 缩进丢失。
#[test]
fn linewise_put_replace_pastes_at_line_start() {
    let mut f = Fixture::new("abc\n ghi\ntail\nXY\n");
    f.feed(["G", "\"", "a", "y", "y"]); // "a{XY}
    f.feed(["d", "d"]); // 移除制造行
    assert_eq!(f.buf.slice(0..f.buf.len()), "abc\n ghi\ntail\n");
    f.feed(["g", "g"]);
    f.feed(["V", "p"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "XY\n ghi\ntail\n",
        "Vp 的行级粘贴落选区起始行的行首，存活缩进行完好"
    );
}

/// 多行选区同款：粘贴行整体替换选中的多行。
#[test]
fn linewise_put_replace_multi_line_selection() {
    let mut f = Fixture::new("aaa\nbbb\nccc\nXY\n");
    f.feed(["G", "\"", "a", "y", "y", "d", "d"]);
    f.feed(["g", "g"]);
    f.feed(["V", "j", "p"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "XY\nccc\n");
}

// ---- 5. 非法模式的 :s 不成为「上一条替换」 ------------------------------------

/// `:s/(/x/`（编译失败）不能顶掉上一条可用的 `:s`——`&` 应重放最近一条
/// 编译成功的命令（旧行为在 build() 之前就记入 last_substitute）。
#[test]
fn amp_skips_substitute_with_invalid_pattern() {
    let mut f = Fixture::new("foo\n");
    f.feed([":", "s", "/", "o", "/", "0", "/", "\n"]); // 合法，先建立 last
    f.feed([":", "s", "/", "(", "/", "x", "/", "\n"]); // 非法模式，响铃
    f.feed(["&"]); // 应重放 o->0
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "f00\n",
        "& 重放最近一条编译成功的 :s，非法模式不得顶替"
    );
}

// ---- 6. 块插入会话的跨行封锁（fuzz round8 抓取） -------------------------------

/// 块会话中行首退格会把上一行合并进来，打飞复制偏移（fuzz：复制在缓冲
/// 末尾之外 insert）。修复后响铃拒绝，退出时复制依然落到正确的行上。
#[test]
fn block_session_rejects_backspace_line_join() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["j"]); // 光标行 = line 1（打字行；行首 BS 才会触发并线）
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X"); // line1 = "Xcd"，block.text = "X"
    // <Left> + <Del> 删掉缓冲里的打字 X：vim 9.1 实证（block-insert 探针）
    // 该会话的副本随打字文本一起消失，退出后不复製——round 8 当时的预期
    // 「X 仍复制」与 vim 相反，本轮（13）按探针改正
    f.feed(["<Left>", "<Del>"]);
    assert_eq!(f.buf.slice(0..f.buf.len()), "ab\ncd\nef\n");
    // 行首 BS：旧行为并线 ab/cd 打飞复制偏移；现在拒绝
    f.feed(["<BS>"]);
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ab\ncd\nef\n",
        "打字文本已被删，会话发散——vim 探针：不复製任何行"
    );
}

/// round13：块会话的 BS 只在打字末尾删最后一个打字字符（vim 9.1 实证
/// `ab<BS>` → "a"+副本"a"；`ab<Left><BS>` 中 BS 是空操作，绝不啃行内容）。
#[test]
fn block_session_bs_only_deletes_typed_tail() {
    // 末尾 BS：删 b，副本缩短为 "a"
    let mut f = Fixture::new("cd\nef\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("ab");
    f.feed(["<BS>", "escape"]);
    assert_eq!(f.text(), "acd\naef\n", "末尾 BS 删打字字符并同步缩短副本");

    // Left 后 BS：空操作（不删行内容），副本保持 "ab"
    let mut f2 = Fixture::new("cd\nef\n");
    f2.feed(["<C-v>", "j", "I"]);
    f2.type_text("ab");
    f2.feed(["<Left>", "<BS>", "escape"]);
    assert_eq!(f2.text(), "abcd\nabef\n", "中途 BS 空操作，副本保持完整打字文本");
}

/// 块会话中 <Del> 删除换行同样会并线——拒绝。
#[test]
fn block_session_rejects_delete_newline() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed(["<C-v>", "j", "I"]); // 打字行 = 光标行 line 1，复制目标 = line 0
    f.type_text("X"); // line1 = "Xcd"，block.text = "X"
                      // 打字行末 <Del>：旧行为删掉 \n 并线
    f.feed(["<End>", "<Del>"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "ab\nXcd\n",
        "跨行 <Del> 被拒绝"
    );
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "Xab\nXcd\n",
        "复制不受 <Del> 影响，仍落到其余块行"
    );
}

/// 块会话中含换行的 IME 文本会拆开打字行——整体拒绝（与 <CR> 锁同款），
/// 且被拒绝的文本不得进入退出时的复制（record 与 insert 的契约）。
#[test]
fn block_session_rejects_newline_in_typed_text() {
    let mut f = Fixture::new("ab\ncd\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("x\ny"); // 含换行：整体拒绝
    assert_eq!(f.buf.slice(0..f.buf.len()), "ab\ncd\n", "含 \\n 文本被拒绝");
    f.type_text("z"); // 拒绝后正常打字仍然生效
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "zab\nzcd\n",
        "只有真正落进缓冲的文本参与复制"
    );
}

/// 块会话中的宿主点击不得移动打字点（fuzz：点击把光标甩到另一行，
/// 复制偏移写出缓冲末尾）。
#[test]
fn block_session_ignores_host_click() {
    let mut f = Fixture::new("ab\ncd\nef\n");
    f.feed(["<C-v>", "j", "I"]);
    f.type_text("X");
    // 宿主点击缓冲末尾（line 2）：会话期间必须被忽略
    let len = f.buf.len();
    f.vim.set_cursor_offset(&f.buf, len);
    f.type_text("Y");
    f.feed(["escape"]);
    assert_eq!(
        f.buf.slice(0..f.buf.len()),
        "XYab\nXYcd\nef\n",
        "点击不移动打字点，复制完整落在块行上"
    );
}

// ---- 7. 体验补充（块选 O / :marks 特殊标记 / :bN 别名） ------------------------

/// 【第十四轮修正】本测试固化的 probe10 是一次误读。vim 9.1 三次独立探针
/// 一致：块 `O` 光标保持所在**行**、换到块的另一**列**角（`:h v_O` "the
/// cursor moves to the other corner in the same line"），anchor 镜像换列、
/// 矩形不变；单行块是真实的水平换角而非 no-op；字符/行 visual 的 `O`
/// 等同 `o`（不响铃）。三次探针：anchor(0,0)+cursor(1,2) → cursor(1,0)；
/// anchor(1,1)+cursor(2,2)（本几何）→ cursor(2,1)；单行 (0,2) → (0,0)。
#[test]
#[allow(non_snake_case)] // O 是按键名
fn block_visual_O_moves_to_other_row_same_col() {
    let mut f = Fixture::new("aaaa\nbbbb\ncccc\ndddd\n");
    f.feed(["2", "g", "g", "l"]); // line1 col1 (0-based)
    f.feed(["<C-v>", "j", "l"]); // block lines1-2 cols1-2，cursor (2,2)
    f.feed(["O"]);
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(f.buf.offset_to_line(cursor), 2, "O 保持光标所在行");
    assert_eq!(
        vimcore::buffer::display_column(&f.buf, cursor),
        1,
        "光标换到块的另一列角"
    );
    assert_eq!(f.buf.offset_to_line(anchor), 1, "anchor 行不变");
    assert_eq!(
        vimcore::buffer::display_column(&f.buf, anchor),
        2,
        "anchor 镜像换列（矩形不变）"
    );

    // 单行块 O：真实的水平换角（vim 探针 (0,2) → (0,0)）
    let mut f = Fixture::new("aaaa\nbbbb\n");
    f.feed(["<C-v>", "l"]);
    f.feed(["O"]);
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(f.buf.offset_to_line(cursor), 0);
    assert_eq!(vimcore::buffer::display_column(&f.buf, cursor), 0, "单行块 O 换角");
    assert_eq!(vimcore::buffer::display_column(&f.buf, anchor), 1);
}

/// 【第十四轮修正】字符/行 visual 的 `O` 等同 `o`（vim 9.1 探针：
/// `vllO` → cursor (0,0)、`VjO` → (0,0)，无铃声）。
#[test]
#[allow(non_snake_case)] // O 是按键名
fn char_visual_O_mirrors_ends_like_o() {
    let mut f = Fixture::new("abcdef\n");
    f.feed(["v", "l", "l", "l", "O"]);
    assert_eq!(f.host.bells, 0, "char visual 的 O 不响铃");
    let (anchor, cursor, _) = f.vim.visual_selection().unwrap();
    assert_eq!(cursor, 0, "光标镜像到对角");
    assert_eq!(anchor, 3);
}

/// `:marks` 现在也列出 `.`（最后变更）与 `^`（最后插入退出）。
#[test]
fn marks_listing_shows_change_and_insert_specials() {
    let mut f = Fixture::new("alpha\nbeta\n");
    f.feed(["x"]); // 变更 → `.
    f.feed(["i"]); // 插入会话 → `^
    f.type_text("Z");
    f.feed(["escape"]);
    f.feed([":", "m", "a", "r", "k", "s", "\n"]);
    let msgs = f.host.statuses.join("\n");
    assert!(msgs.contains(".  line 1"), ":marks 应列出 `.`（{msgs}）");
    assert!(msgs.contains("^  line 1"), ":marks 应列出 `^`（{msgs}）");
}

/// `:bN` 是 `:bprev` 的 vim 别名（旧行为 E492）。
#[test]
#[allow(non_snake_case)] // bN 是命令名
fn bN_aliases_bprev() {
    let mut f = Fixture::new("text\n");
    f.feed([":", "b", "N", "\n"]);
    assert_eq!(f.host.bells, 0, ":bN 应命中 bprev 而非 E492 响铃");
}
