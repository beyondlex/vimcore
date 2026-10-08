//! audit5_b：Normal motion / 算子 / 文本对象 / join / 缩进位移 域独立审计
//! （第四轮独立审计，2026-10-09）。
//!
//! 每个 `#[test]` 是一个 **vim 9.1（本机 `/usr/bin/vim` = 9.1 patches
//! 1-1752）证实的 bug**，断言写 oracle 期望值，因此在当前工作树上应当
//! 失败（修复后转回归）。
//!
//! oracle 通道：
//! * typeahead：python3 写缓冲/按键 → `vim -Nu NONE -N -i NONE -n -s
//!   keys buf </dev/null`，`:call writefile(...)` 采样（每例唯一文件名）。
//! * 响铃类：expect 驱动的真 PTY（`spawn vim -Nu NONE -N -n buf` +
//!   `send`），统计输出里的 BEL 字节——typeahead 通道听不到铃声。
//! * 行为断言来源：vim `src/testdir/`（test_normal.vim /
//!   test_textobjects.vim / test_join.vim / test_shift.vim /
//!   test_charsearch*）+ 本机 typeahead/PTY 复核。
//!
//! 已对照去重：NOTES.md 已知分歧 #1-77+、BUG_AUDIT.md 50 项、
//! BUG_AUDIT2.md 59 项（A 系 TAB 虚列、B 系 aw 空白矩阵/cw≡ce/对象
//! count 跨行）、BUG_AUDIT3.md 55 项（B1-B14 刚修过本域）。
//!
//! ---------------------------------------------------------------
//! 本轮已证伪（oracle 证引擎与 vim 9.1 一致，下轮免重查）：
//! * 单行缓冲 `}` 落末字符、`d}` 行中起删到缓冲末（B13 修复在位）。
//! * 纯空白行不分段：`d}` 越过空白行删整段（motion 停、算子不停的
//!   双语义，见 B-1 的注释）。
//! * `g_`/`2g_` 落行末非空白；`_` 计数 1 = 本行首非空白（不动行）。
//! * `99go` 超界落末字符 'c'（clamp 到缓冲末字节，非末行行首）。
//! * J 光标落首行末字符（"aa\nbb" → col 3；空次行 oracle [0,1,2,0]
//!   同位）；gJ 光标落次行首字符（col 16）。
//! * J 空行/空白行/`)` 开头行的不加空格矩阵；`5J`/`10J` 计数。
//! * `gJ` 保留次行缩进、`3gJ` 计数、末行 `gJ` 响铃。
//! * `cc` 清缩进（noai）且光标 col 1；`3C`/`2S`/`2s`/`2dd` 计数；
//!   `2X` 反向删。
//! * `<<`/`>>` 显示列模型（noet ts=8 sw=2 的 TAB 行矩阵）、`>>` 于
//!   空行跳过、`>>` 光标 = 范围首行首非空白、`#define` 行正常缩进。
//! * `dge` 从词尾跨词（含两端）、从词中停在词尾、`ge` 于同词词首
//!   卡住（vim 也不响铃）；`de` 于词中只删当前词（'foo, bar' →
//!   ', bar'）。
//! * `dw` 于空白段/标点词、`cw`/`cW`/`caw` 于空白段、`cw` 于标点、
//!   `yiw` 于标点词、`y2iw` = "foo "（vim 对象计数如此）。
//! * `10~` 计数、`ß→ẞ`（U+1E9E）、`guG`/`gugu`/`gUgU` 行级双写、
//!   `gU$` 数字不受影响。
//! * `2fX` 计数、`tx` 邻近目标停住、`2tx` 停点 col 3、`;` 失败响铃
//!   不动、CJK `f最;` 列语义。
//! * `%` 于无配对字符响铃；`d%` 删配对；`%` 从非括号字符前向找括号。
//! * `d2aw`/`daw`/`daW`/`diW`/行尾 `daw` 空白与标点形状（audit2 在位）。
//! * `dis` 基本形、单句 `yis`/`yas`、句中 `yis`、末句 `yas`。
//! * `cit` 空标签、`da<`/`di<` 基本形。
//! * `yap` 段落跨度（含前导空行变体）；`dip` 于纯空白缓冲 = 一空行。
//! * `gqq` 文本面与光标落末行；`2D` 行尾 + 1 行；`d2w` 于空白起点
//!   跨行（'ab \n'）与词中起点（'ab c\n'——规则 (a) 末行形状两侧一致）。
//! * `100g_` 大 count 钳到末行末非空白（oracle [0,2,8,0]——cursor_down
//!   只在光标已在末行时 FAIL，引擎一致）；`g_`/`2g_`/`_` 基本形。
//! * `~` 光标行末钳回末字符（0~~~ → 'Ab'，引擎一致）；`2~` 于多字节
//!   行按字符计。
//! * `daw` 行中形状与光标落点（剩余文本起点）、单词行 daw 留空行、
//!   TAB 段 daw 删 TAB+次词、`d2aw`/`2caw` 计数、`yiw` 于空白取空白段。
//! * `dip` 末段剩 'aa\n\n'；`diw`/`daw` 空白段语义（audit2 在位）。
//! * `%` 从非括号字符前向找行内括号、跨行配对（(a\nb) 的 % 落 [0,2,2,0]，
//!   引擎一致——本轮一度误判，列 1→byte 换算复核算错）；`cG` 从行中改到缓冲末；`guap`
//!   行级只动本段；`>>` 于空缓冲不缩进；`d}}` 于 'a\n\n\n' 行首起点
//!   提升 linewise 只删一行；`cw`/`dw` 于 TAB 只动 TAB；`ge` 于标点词
//!   落标点尾。
//! ---------------------------------------------------------------

mod common;

use common::{edit, Fixture};

fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn unnamed(f: &Fixture) -> String {
    f.vim
        .registers
        .get('"')
        .map(|r| r.text.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- B-1（已证伪翻转 → 回归钉）
// 初稿断言 `}` 停在空白行行首（offset 6）——2026-10-09 typeahead 复验
// 推翻：`aa\n   \nbb\n` 上 `}` 落 **[0,3,2,0]**（offset 8，下一段 'bb'
// 的第二个字符——nv_paragraph 越过空白行落到下一段非空白处）。引擎的
// empty-only 判定本就与此一致，B-1 为采样误读，撤案；本测试转为钉住
// 复验过的 oracle 落点（初稿引用的 [0,2,1,0] 系另一缓冲形状的混淆）。
#[test]
fn b1_para_motion_lands_on_next_paragraph_nonblank() {
    let f = edit("aa\n   \nbb\n", 0, 0, &["}"]);
    assert_eq!(f.cursor(), 8, "oracle [0,3,2,0]: nv_paragraph 落下一段非空白（复验 2026-10-09）");
}

// ---------------------------------------------------------------- B-2（P3）
// 缓冲起点的 `b`/`B` 必须响铃。引擎的 WordBack 在原点返回「原地成功」
// 而非 stuck（motions.rs 的 b 臂没有 ge 臂那样的 stuck 分支）——静默。
// oracle（expect PTY，统计输出 BEL 字节）：printf 'one\n' 后发送 `b`/
//   `B`/`ge`/`gE`，各捕获 1 个 BEL；光标均不动。引擎侧实测：b/B 铃 0、
//   ge/gE 铃 1（ge/gE 已合规，留在断言里钉住）。
#[test]
fn b2_backward_word_motions_at_buffer_start_must_bell() {
    let cases: &[&[&str]] = &[&["b"], &["B"], &["g", "e"], &["g", "E"]];
    for keys in cases {
        let mut f = Fixture::new("one\n");
        f.feed(keys.iter().map(|k| k.to_string()).collect::<Vec<_>>());
        assert!(
            f.host.bells > 0,
            "vim: {:?} 于缓冲起点响铃（PTY 实测 1 BEL）",
            keys
        );
        assert_eq!(f.cursor(), 0, "vim: {:?} 于缓冲起点不动", keys);
    }
}

// ---------------------------------------------------------------- B-3（P1）
// `y2b` 跨空行：exclusive 规则 (a)（终点在列 1 → 终点移到上一行行尾、
// 转 inclusive）在「上一行是空行」时的形状。vim 的空行没有字符，收进
// yank 的是**前一行**的换行（"two\n"，4 字节）；引擎把空行自己的换行
// 也收进来（"two\n\n"，5 字节）。
// oracle：printf 'one two\n\nfoo bar\n'；`3Gy2b` + strtrans(getreg('"'))
//   → "two^@"（即 "two\n"）。引擎多出一个换行。
#[test]
fn b3_y2b_exclusive_rule_on_empty_previous_line() {
    let mut f = Fixture::new("one two\n\nfoo bar\n");
    f.feed(["3", "G", "y", "2", "b"]);
    assert_eq!(unnamed(&f), "two\n", "vim: y2b 跨空行 yank two+换行（oracle）");
    // 同族非空行形状（已证伪，钉住防回归）：b 停在空白行行首
    let mut f = Fixture::new("one two\n  \nfoo bar\n");
    f.feed(["3", "G", "y", "b"]);
    assert_eq!(unnamed(&f), "two\n  ", "vim: b 停在非空白空行行首");
}

// ---------------------------------------------------------------- B-4（P1）
// 块对象在光标位于块**之前**时不前向搜索。vim（:h i(）：光标不在块内
// 时向前找第一个未配对的 `(`。引擎 block_range 只向后扫，`0di)` 全落空。
// oracle：printf 'foo (bar (baz (quux)))\n'；`0di)` → "foo ()"；
//   `02di)`（'foo (bar (baz) (quux))'）→ "foo (bar () (quux))"；
//   `03di)` 于该缓冲 → 原样（无第三嵌套层，testdir）。
#[test]
fn b4_block_object_searches_forward_when_cursor_before_block() {
    let f = edit("foo (bar (baz (quux)))\n", 0, 0, &["0", "d", "i", ")"]);
    assert_eq!(f.text(), "foo ()\n", "vim: 0di) 前向找块（oracle q_di）");
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "2", "d", "i", ")"]);
    assert_eq!(f.text(), "foo (bar ())\n", "vim: 02di) 进第二层（testdir）");
    let f = edit("foo (bar (baz) (quux))\n", 0, 0, &["0", "d", "i", ")"]);
    assert_eq!(f.text(), "foo ()\n", "vim: 0di) 取第一个未配对对（oracle q_di2）");
    let f = edit("foo (bar (baz) (quux))\n", 0, 0, &["0", "3", "d", "i", ")"]);
    assert_eq!(
        f.text(),
        "foo (bar (baz) (quux))\n",
        "vim: 03di) 无第三嵌套层 → 无操作（testdir）"
    );
}

// ---------------------------------------------------------------- B-5（P2）
// 同一根因的 yank 形状：`0yi)` 前向 + count。
// oracle：printf 'foo (bar (baz (quux)))\n'；`0yi)` + getreg →
//   "bar (baz (quux))"；`03yi)` → "quux"（testdir）。引擎 yank 空。
#[test]
fn b5_yank_block_object_forward() {
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "y", "i", ")"]);
    assert_eq!(unnamed(&f), "bar (baz (quux))", "vim: 0yi) 前向（testdir）");
    let mut f = Fixture::new("foo (bar (baz (quux)))\n");
    f.feed(["0", "3", "y", "i", ")"]);
    assert_eq!(unnamed(&f), "quux", "vim: 03yi) 进第三层（testdir）");
}

// ---------------------------------------------------------------- B-6（P2）
// `cib` 于「text()」（空配对在光标之后）：vim 前向找到空 ()，在中间
// 插入。引擎对象失败，c 被取消。
// oracle：printf 'text()\n'；`0cibtext<Esc>` → "text(text)"（testdir
//   Test_inner_block_empty_paren 同款；本机 typeahead 复核一致）。
#[test]
fn b6_cib_on_empty_parens_after_cursor() {
    let mut f = Fixture::new("text()\n");
    f.feed(["0", "c", "i", "b"]);
    f.type_text("text");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "text(text)\n", "vim: cib 空 () 前向并在中间插入");
}

// ---------------------------------------------------------------- B-7（P2）
// `<`/`>` 块对象同样前向（Test_textobj_find_paren_forward）。
// oracle：printf '#include <foo.h>\n'；`0yi<` → "foo.h"；`0ya>` →
//   "<foo.h>"。引擎均空。
#[test]
fn b7_angle_block_object_forward() {
    let mut f = Fixture::new("#include <foo.h>\n");
    f.feed(["0", "y", "i", "<"]);
    assert_eq!(unnamed(&f), "foo.h", "vim: 0yi< 前向（testdir）");
    let mut f = Fixture::new("#include <foo.h>\n");
    f.feed(["0", "y", "a", ">"]);
    assert_eq!(unnamed(&f), "<foo.h>", "vim: 0ya> 前向（testdir）");
}

// ---------------------------------------------------------------- B-8（P2）
// `a'` 的空白取舍：vim 优先吃**尾随**空白，无尾随时取**前导**空白。
// 引擎两种形状都只取引号串本身。
// oracle：printf "some    'special'    string\n"；`0ya'` + getreg →
//   "'special'    "；"some    'special'string\n" → "    'special'"
//   （testdir Test_textobj_quote 同款）。
#[test]
fn b8_aquote_whitespace_asymmetry() {
    let mut f = Fixture::new("some    'special'    string\n");
    f.feed(["y", "a", "'"]);
    assert_eq!(unnamed(&f), "'special'    ", "vim: 尾随空白优先（oracle q_aq）");
    let mut f = Fixture::new("some    'special'string\n");
    f.feed(["y", "a", "'"]);
    assert_eq!(unnamed(&f), "    'special'", "vim: 无尾随取前导（testdir）");
}

// ---------------------------------------------------------------- B-9（P2）
// 算子待决态的 `v`/`V`/`<C-v>` 键：vim 用来切换 motion 的 inclusive/
// exclusive/linewise（:h o_v）。引擎 pending 相位没有这三行，`d` 后的
// `v` trie miss → 响铃取消，整个 `dvgo` 无效。
// oracle：printf 'one two three four\n'；`5|dvgo`（col 5 = 't'）→
//   "wo three four"（v 使到达端 inclusive，含光标字符删除）。
#[test]
fn b9_operator_pending_v_toggles_motion_kind() {
    let f = edit("one two three four\n", 0, 4, &["d", "v", "g", "o"]);
    assert_eq!(f.text(), "wo three four\n", "vim: dvgo 含光标字符删除（oracle t5）");
}

// ---------------------------------------------------------------- B-10（P2）
// `2yis` 的 count 形状：vim 的 is count 扩展把句间**衔接空白**并进来。
// 引擎停在第一句内层。
// oracle：printf 'A sentence.  A sentence?  A sentence!\n'；`2yis` +
//   getreg → "A sentence.  "；`3yis` → "A sentence.  A sentence?"；
//   `2yas` → "A sentence.  A sentence?  "（testdir Test_sentence 同款，
//   oracle q_2yis 复核一致）。
#[test]
fn b10_sentence_inner_count_includes_joining_whitespace() {
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["2", "y", "i", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  ", "vim: 2yis（oracle q_2yis/testdir）");
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["3", "y", "i", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  A sentence?", "vim: 3yis（testdir）");
    let mut f = Fixture::new("A sentence.  A sentence?  A sentence!\n");
    f.feed(["2", "y", "a", "s"]);
    assert_eq!(unnamed(&f), "A sentence.  A sentence?  ", "vim: 2yas（testdir）");
}

// ---------------------------------------------------------------- B-11（P2）
// `das` 光标落在句间空白（'?' 后的空格）上：vim 取**下一句**（含其后
// 尾随空白）。引擎取了前一句。
// oracle：printf 3 行缓冲；`3ggf?ldas` + getline →
//   第 3 行 = "Even with a question? And no sentence here"（oracle q_das）。
#[test]
fn b11_das_on_inter_sentence_whitespace_targets_next_sentence() {
    let mut f = Fixture::new(
        "This is a test. With some sentences!\n\nEven with a question? And one more. And no sentence here\n",
    );
    f.feed(["3", "G", "f", "?", "l", "d", "a", "s"]);
    assert_eq!(
        f.text(),
        "This is a test. With some sentences!\n\nEven with a question? And no sentence here\n",
        "vim: das 于句间空白取下一句（oracle q_das/testdir）"
    );
}

// ---------------------------------------------------------------- B-12（P2）
// `dg_`（g_ 在算子下）：inclusive 止于**末非空白字符**，行尾空白保留。
// 引擎把行尾空白一起删了。
// oracle：printf '  ab  cd  \n'；`dg_` + getline/getpos →
//   行 = "    "（前导 2 + 尾随 2 空格），光标 col 3（oracle r4）。
#[test]
fn b12_dgunderscore_keeps_trailing_whitespace() {
    let mut f = Fixture::new("  ab  cd  \n");
    f.feed(["d", "g", "_"]);
    assert_eq!(f.text(), "    \n", "vim: dg_ 留行尾空白（oracle r4）");
    assert_eq!(f.cursor(), 2, "vim: dg_ 后光标 col 3");
}

// ---------------------------------------------------------------- B-13（P3）
// 段落对象 count 超段必须响铃（testdir Test_paragraph：
// assert_beeps('normal 3yap')）。引擎静默。
// oracle（expect PTY）：printf '\n\n\nFirst line.\nSecond line.\n'；
//   发送 `2G3yap` → 捕获 1 BEL。
#[test]
fn b13_yap_count_beyond_paragraphs_must_bell() {
    let mut f = Fixture::new("\n\n\nFirst line.\nSecond line.\n");
    f.feed(["2", "G", "3", "y", "a", "p"]);
    assert!(f.host.bells > 0, "vim: 3yap 无第三段响铃（PTY 实测 1 BEL）");
}

// ---------------------------------------------------------------- B-14（P1）
// 标签对象解析被属性里引号内的 `>` 骗走：引擎 tag_range 用裸 find('>')
// 截标签，`<div attr="attr >> foo >> bar ">` 被截成 `<div attr="attr >`。
// vim 的标签解析跳过引号串。
// oracle：printf '<div attr="attr >> foo >> bar ">Hello</div>\n'；
//   `fHyit` + getreg → "Hello"（oracle t3；testdir
//   Test_string_html_objects 同款）。
#[test]
fn b14_tag_object_skips_quoted_gt_in_attributes() {
    let mut f = Fixture::new("<div attr=\"attr >> foo >> bar \">Hello</div>\n");
    f.feed(["f", "H", "y", "i", "t"]);
    assert_eq!(unnamed(&f), "Hello", "vim: yit 跳过属性里的 >>（oracle t3）");
    let mut f = Fixture::new("<div attr='attr >> foo >> bar '>Hello 123</div>\n");
    f.feed(["f", "H", "y", "i", "t"]);
    assert_eq!(unnamed(&f), "Hello 123", "vim: 单引号属性同款（testdir）");
}

// ---------------------------------------------------------------- B-15（P2）
// 跨行内层块对象：vim 的 inner 起点/终点取「下一行行首 / 上一行行尾」，
// 两侧换行都保留（testdir：di[ 于 [' ', '[', 'one [two]', 'thre', ']']
// → [' ', '[', ']']）。引擎按字节紧邻删，两侧换行被吞并线。
// oracle：printf ' \n[\none [two]\nthre\n]\n'；`3gg0di[` → 三行；
//   printf 'x = (\n    one,\n    two,\n)\n'；`di(` → ['x = (', ')']。
#[test]
fn b15_multiline_inner_block_keeps_newlines() {
    let mut f = Fixture::new(" \n[\none [two]\nthre\n]\n");
    f.feed(["3", "G", "0", "d", "i", "["]);
    assert_eq!(f.text(), " \n[\n]\n", "vim: di[ 跨行保换行（oracle u2/testdir）");
    let mut f = Fixture::new("x = (\n    one,\n    two,\n)\n");
    f.feed(["d", "i", "("]);
    assert_eq!(f.text(), "x = (\n)\n", "vim: di( 缩进内容整行删（oracle u3）");
}

// ---------------------------------------------------------------- B-16（P3）
// `aw` count 超词必须响铃（testdir Test_textobj_a_word：
// assert_beeps('normal 0y3aw')）。引擎静默。
// oracle（expect PTY）：printf 'one a\n'；发送 `y3aw` → 捕获 1 BEL。
#[test]
fn b16_y3aw_beyond_last_word_must_bell() {
    let mut f = Fixture::new("one a\n");
    f.feed(["y", "3", "a", "w"]);
    assert!(f.host.bells > 0, "vim: y3aw 词不够响铃（PTY 实测 1 BEL）");
}

// ---------------------------------------------------------------- B-17（挂账 → #[ignore]）
// testdir 的 d2at 全序列（oracle w1：`1gg0da<1pjd2at` 剩 [' ']）。引擎在
// 该序列上得 [" "," "]。已定位的事实（2026-10-09 oracle 矩阵）：
// * 爬层本体是好的：干净中间态 " <div>\n<a…>xyz</a>\n    </div>\n \n" 上
//   光标 '<a' 的 object_span_count(Tag, 2) = div 外层 (1,54)。
// * vim 在该形状上的 2at 删除域 = [div 开标签 .. 缓冲末]（j0d2at → [" "]，
//   连行 4 的 " " 一起没了；j0dat count=1 只删 a 标签，j0dit 保结构）——
//   标签块闭标签之后的尾随换行/空行被并入删除域，与 exclusive 规则 (a)
//   的何种交互尚未逆向（linewise 提升形状对不上，候选模型见
//   BUG_AUDIT4.md 挂账）。需要 PTY 专项逐形状逆向后再实现。
#[test]
#[ignore = "B-17 挂账：at 外层的尾随空白/换行归域未逆向（oracle 数据在本注释）"]
fn b17_d2at_testdir_sequence() {
    let mut f = Fixture::new("<div> \n<a href=\"foobar\" class=\"foo\">xyz</a>\n    </div>\n \n");
    f.feed([
        "1", "g", "g", "0", "d", "a", "<", "1", "p", "j", "d", "2", "a", "t",
    ]);
    assert_eq!(f.text(), " \n", "vim: testdir d2at 序列剩 [' ']（oracle w1）");
}

// ---------------------------------------------------------------- B-18（P2）
// `3cc` 于末行（count 超行）：vim 的 cc 族在行数不足时**整体失败**
// （不进入插入态，缓冲不动）。
// oracle：printf 'a\nb\n'；`G3ccX<Esc>` + getline → 'a\nb\n'（X 未生效，
// 3cc 失败后 X 是普通模式的 X——col 1 上无操作；oracle x4）。
#[test]
fn b18_counted_cc_beyond_last_line_fails() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["G", "3", "c", "c"]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "a\nb\n", "vim: 3cc 于末行整体失败（oracle x4）");
}

// ---------------------------------------------------------------- B-19（P2）
// 首行是空行时 `dap`：vim 把空行连同其后整段（含第三行）全删，
// 剩 ['']（text "\n"）。
// oracle：printf '\naa\nbb\n'；`dap` + getline → ['']（oracle y4）。
#[test]
fn b19_dap_on_leading_empty_line() {
    let mut f = Fixture::new("\naa\nbb\n");
    f.feed(["d", "a", "p"]);
    // 期望按引擎内部文本口径修正（2026-10-09）：vim 的 writefile(['']) =
    // "\n" 是【写回侧】形状（fixendofline 补终结符）；引擎内部以 "" 表示
    // 同一个「单个空行」状态（line_count 双方都 = 1，与既有 dd 清空钉一
    // 致，如 huge_counts_saturate）。oracle y4 的 [''] 与 "" 同态。
    assert_eq!(
        f.text(),
        "",
        "oracle [''](内部文本口径下同态): 首空行 dap 删空缓冲剩一空行"
    );
}

// ---------------------------------------------------------------- B-20（P2）
// `db` 自下行行首向上：b 落上一行词首，exclusive 规则 (a) 把终点移到
// **上一行行尾**（inclusive，即删到行尾字符为止）——换行存活、两行
// 不并线。
// oracle：printf 'one two\nfoo\n'；`2Gdb` + getline → ['one ', 'foo']
// （oracle v10）。
#[test]
fn b20_db_upward_keeps_line_break() {
    let f = edit("one two\nfoo\n", 1, 0, &["d", "b"]);
    assert_eq!(f.text(), "one \nfoo\n", "vim: db 向上删到上一行行尾，不并线（oracle v10）");
}
