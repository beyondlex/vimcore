//! 第二十一轮（第 22 轮检视期内新增）fuzz：`:s` 替换串转义面。
//!
//! 背景：round22 修复了替换串 `\r`（换行）与 `\n`（NUL）的字面穿透——
//! 此前它们原样写进缓冲，修复后替换可以**改变行数**、可以引入 NUL 字节。
//! 这是 ex_substitute 全新触达的状态空间（行号漂移、`last_sub_line`
//! 拆行落点、marks/高亮在多行 replace 下的调整），本轮用恶意替换字母表
//! 轰炸并断言：
//!   1. 引擎不变量（光标/高亮/marks 可寻址——round20 的契约延续）；
//!   2. **行寻址契约**：替换后每一行的 `line_start/line_end/offset_to_line`
//!      必须自洽（拆行实现把替换文本 join 进原范围，写错一行就崩）；
//!   3. NUL 字节（`\n` 的 vim 语义）不破坏任何寻址数学。
//!
//! 另附 CJK 面的 search-motion 定向（round21 新功能的多字节形态）。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;

fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn assert_addressable(f: &Fixture, context: &str) {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= len && text.is_char_boundary(cur),
        "{context}: cursor {cur} unaddressable (len={len}) in {text:?}"
    );
    // 行寻址契约：每行的起止都是字符边界且区间有序
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= len && text.is_char_boundary(s) && text.is_char_boundary(e),
            "{context}: line {line} range {s}..{e} broken (len={len}) in {text:?}"
        );
        assert_eq!(
            buf.offset_to_line(s),
            line,
            "{context}: line {line} start {s} resolves to line {}",
            buf.offset_to_line(s)
        );
    }
    // 渲染契约：高亮区间可寻址（round20 契约延续到拆行替换面）
    for hl in f
        .host
        .highlights
        .iter()
        .chain(f.host.current_highlight.iter())
    {
        assert!(
            hl.end <= len && text.is_char_boundary(hl.start) && text.is_char_boundary(hl.end),
            "{context}: highlight {hl:?} unaddressable in {text:?}"
        );
    }
}

/// 替换串恶意字母表：转义对（\r \n \\ \/）与裸元字符交错。
const REPL_ALPHABET: &[&str] = &[
    "X", "\\r", "\\n", "\\\\", "\\/", "$0", "$1", "&", "\\x", "\\\\", "\\r\\n", "\\\\',", "中",
    "\\r中", "\\n\\",
];

const PATTERN_ALPHABET: &[&str] = &["a", "中", "$", "^", "a*", "[ab]", "(a|b)", "\\\\r", "."];

const TARGETS: &[&str] = &[
    "aaa\nbbb\n",
    "中文中\n文\n",
    "a\n",
    "a",
    "aba aba\n\n中a文\n",
    "aaa",
];

#[test]
fn fuzz_substitute_replacement_escapes_hold_line_addressing() {
    for (i, target) in TARGETS.iter().enumerate() {
        for (pi, pat) in PATTERN_ALPHABET.iter().enumerate() {
            for (ri, rep) in REPL_ALPHABET.iter().enumerate() {
                let mut f = Fixture::new(target);
                let context = format!("target#{i} s/{pat}/{rep}/ (p{pi} r{ri})");
                feed_ex(&mut f, &format!("%s/{pat}/{rep}/g"));
                assert_addressable(&f, &context);
                // 拆行后再跑一遍 n/N/高亮重发布——行号缓存的失配面
                f.feed(["n", "N"]);
                assert_addressable(&f, &context);
                // marks 可寻址（拆行编辑后的 adjust_replace 面）
                f.feed(["g", "g"]);
                f.feed(["m", "a"]);
                feed_ex(&mut f, "%s/a/b/g");
                assert_addressable(&f, &context);
            }
        }
    }
}

/// 定向：`%s/a/\r/` 的光标必须落在**拆行后**的最后替换行（vim 9.1：
/// "aaa\nbbb" → 落 "aa" 行；g 形态 → 落第 4 行）。
#[test]
fn substitute_line_split_cursor_lands_post_split() {
    let mut f = Fixture::new("aaa\nbbb\n");
    feed_ex(&mut f, "%s/a/\\r/");
    assert_eq!(f.text(), "\naa\nbbb\n");
    assert_eq!(f.line(), 1, "cursor on the post-split 'aa' line");

    let mut g = Fixture::new("aaa\nbbb\n");
    feed_ex(&mut g, "%s/a/\\r/g");
    assert_eq!(g.text(), "\n\n\n\nbbb\n");
    assert_eq!(g.line(), 3, "cursor on the 4th line (last split slot)");

    // 中间行替换：上方行数不变，拆行只发生在替换行内；光标落在拆行后
    // 替换区的最后一行（vim 9.1 探针：line(".") = 4）
    let mut h = Fixture::new("keep1\naba\nkeep2\n");
    feed_ex(&mut h, "2s/a/\\r/g");
    assert_eq!(h.text(), "keep1\n\nb\n\nkeep2\n");
    assert_eq!(h.line(), 3);
}

/// NUL 字节（`\n` 替换）不破坏寻址数学，且与 vim 的字节形态一致。
#[test]
fn substitute_nul_replacement_keeps_addressing() {
    let mut f = Fixture::new("foo bar");
    feed_ex(&mut f, "s/o/X\\nY/");
    assert_eq!(f.text(), "fX\u{0}Yo bar");
    // 光标/marks/高亮在含 NUL 的文本上仍然可寻址
    f.feed(["g", "g", "m", "a"]);
    f.feed(["/", "Y"]);
    f.feed_raw(Key::enter());
    assert_addressable(&f, "nul-replacement");
    // vim 9.1 字节探针：fX 00 59 6f 20 62 61 72
    assert!(f.text().contains('\u{0}'));
}

/// 定向：CJK 匹配的 search-motion——span 与选区扩展都必须落在字符边界。
/// （round21 语义已 vim 实证，此处钉多字节形态。）
#[test]
fn search_motion_multibyte_spans_stay_on_char_boundaries() {
    // 操作符面：d/文<CR> 删到「文」的字节起点
    let mut f = Fixture::new("中中 文字\n尾\n");
    f.feed(["d", "/"]);
    for c in "文".chars() {
        f.feed_raw(Key::char(c));
    }
    f.feed_raw(Key::enter());
    let text = f.text();
    assert!(
        text.is_char_boundary(f.vim.cursor.offset),
        "cursor mid-char after d/文: {} in {text:?}",
        f.vim.cursor.offset
    );
    // vim 语义：d/pat 删 [cursor, match_start)——「中中 」被删
    assert_eq!(text, "文字\n尾\n");

    // visual 面：v?中<CR> 向上扩展，选区端点可寻址
    let mut g = Fixture::new("上\n中文下\n");
    g.feed(["j"]);
    g.feed(["l"]);
    g.feed(["v", "?"]);
    for c in "中".chars() {
        g.feed_raw(Key::char(c));
    }
    g.feed_raw(Key::enter());
    let text2 = g.text();
    let cur = g.vim.cursor.offset;
    assert!(
        text2.is_char_boundary(cur),
        "v?中 cursor {cur} mid-char in {text2:?}"
    );
    assert!(matches!(g.vim.mode(), vimcore::mode::Mode::Visual { .. }));
}

/// `&` 重放拆行替换：存储的是转义前的原始行，重放必须再次拆行（不双拆）。
#[test]
fn ampersand_repeats_splitting_substitute_once() {
    // & 忽略存储的范围（vim 9.1 探针：:1s + 移动 + & 作用于当前行），
    // 且只重放一次（已拆过的行不再变）
    let mut f = Fixture::new("aba\naba\n");
    feed_ex(&mut f, "1s/a/\\r/");
    assert_eq!(f.text(), "\nba\naba\n");
    f.feed(["j", "j"]); // 到第 3 行（aba）
    f.feed(["&"]);
    assert_eq!(f.text(), "\nba\n\nba\n");
    // 裸 `:s` 重放：键入的范围生效（存储的 "1" 被剥掉）——旧引擎存储范围
    // 随行重放，`&` 重打绝对行号、裸 :s 在带范围存储后直接 E492。
    // :2s = 1-based 第 2 行（0-based 1，"ba"）→ 首 a 拆行
    let mut g = Fixture::new("aba\naba\n");
    feed_ex(&mut g, "1s/a/\\r/");
    feed_ex(&mut g, "2s");
    assert_eq!(g.text(), "\nb\n\naba\n");
}
