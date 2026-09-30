//! 第八轮扩展 fuzz：覆盖本轮修复路径与新算子的多种子确定性轰炸。
//!
//! 相对 `fuzz_round7` 的增量：
//! - 键表补齐本轮改动路径：`gq}`/`gw`/块选 `U`/`u`/`~`、`Vp`/`vp`、`&`、
//!   非法 `:s`、宏录放夹 Ex 命令（`qa:xq@a`）、裸行号 `:2`、`g;`/`g,`；
//! - 不变量加两条：visual 选区两端可寻址；`gv` 重建的选区可寻址；
//! - 注入宿主点击（`set_cursor_offset` 随机落点，含非边界偏移）。

mod common;

use vimcore::key::parse_key_sequence;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

const KEYS: &[&str] = &[
    // 本轮修复路径
    "gq}", "gqq", "gww", "gqgq", "v$gq", "<C-v>jU", "<C-v>ju", "<C-v>j~", "Vp", "vp", "VP", "&",
    ":s/(/x/", ":2", ":noh", "qa", "q", "@a", ":", "g;", "g,",
    // 既有覆盖（节选 round7 键表，保持回归压力）
    "h", "j", "k", "l", "w", "b", "e", "0", "$", "^", "g", "G", "d", "c", "y", "p", "P", "x", "X",
    "s", "S", "D", "C", "r", "a", "i", "o", "O", "v", "V", "<C-v>", "u", "<Esc>", "J", "gJ", ">",
    "<", "gu", "gU", "g~", "~", "f", "t", "F", "T", ";", "%", "n", "N", "*", "#", "iw", "aw",
    "i\"", "a\"", "i(", "a(", "it", "ip", "ap", "dd", "dw", "yy", "cc", "cw", "gg", "zz", "m", "`",
    "'", ".", "<C-a>", "<C-x>", "<C-r>", "<C-o>", "<C-i>", "1", "3", "/", "?", "<CR>", ":", ":d a",
    ":1,2j", ":j!", ":%d", ":$d", ":sort iu", ":sort!", ":s/x/y/", ":s//y/", ":marks", ":reg",
    "<Del>", "<C-c>", "gv", "gn", "gN", "R", "gi", "gI", "\"a", "\"1p", "\"_dd",
];

/// 校验一个偏移在 `text` 上可寻址（字符边界或末尾）。
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

#[test]
fn fuzz_round8_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
        "x\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "aaaa bb cc ddd\nee ff\n\ngg hh ii jj kk ll\n",
        "abc\n ghi\ntail\n",
        "aBc\ndEf\nGhI\n",
        "你好, world 123 -45\n#tag\"",
        "a;b|c,d<e>f?g",
        "\n\n\n",
        "0x1f 077 0b101 -99\n42 0099",
        "one two three\nfour five\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0xD00D_8BAD ^ seed.wrapping_mul(0x9E37_79B9);
        for round in 0..100 {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            for step in 0..160 {
                let k = KEYS[(fuzz_xorshift(&mut state) as usize) % KEYS.len()];
                for key in parse_key_sequence(k) {
                    f.feed_raw(key);
                }
                // IME 打字路径：insert/replace 模式下随机注入（含多行文本）
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    f.type_text("t中x");
                }
                // 宿主点击：随机落点（刻意含非字符边界——入口须 floor）
                if fuzz_xorshift(&mut state).is_multiple_of(11) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
                }
                let ctx = format!("seed {seed} round {round} step {step} key {k}");
                let text = f.text();
                assert_addressable("cursor", f.vim.cursor_offset(), &text, &ctx);
                for (name, off) in f.vim.marks.items() {
                    assert_addressable(&format!("mark {name}"), off, &text, &ctx);
                }
                if let Some((lo, hi, _)) = f.vim.marks.last_visual {
                    assert_addressable("last_visual.lo", lo, &text, &ctx);
                    assert_addressable("last_visual.hi", hi, &text, &ctx);
                }
                // live 选区与 gv 重建选区的两端都可寻址
                if let Some((a, c, _)) = f.vim.visual_selection() {
                    assert_addressable("visual.anchor", a, &text, &ctx);
                    assert_addressable("visual.cursor", c, &text, &ctx);
                }
            }
        }
    }
}
