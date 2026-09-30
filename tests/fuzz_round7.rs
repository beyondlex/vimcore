//! 第七轮扩展 fuzz：多种子确定性按键轰炸（第七轮审查引入）。
//!
//! 既有 `fuzz_random_key_sequences_hold_invariants` 是单种子 500 轮；这里
//! 用多种子扩大覆盖（第七轮两种子就抓到两个新 bug：`gn` 多字节匹配光标
//! mid-char、块插入会话后光标偏移陈旧）。不变量与既有 fuzz 相同：
//! 不 panic、光标与全部存储偏移恒可寻址。

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
    "h", "j", "k", "l", "w", "b", "e", "0", "$", "^", "g", "G", "d", "c", "y", "p", "P", "x", "X",
    "s", "S", "D", "C", "r", "a", "i", "o", "O", "v", "V", "<C-v>", "u", "<Esc>", "J", "gJ", ">",
    "<", "gu", "gU", "g~", "gqq", "~", "f", "t", "F", "T", ";", "%", "n", "N", "*", "#", "iw",
    "aw", "i\"", "a\"", "i(", "a(", "it", "ip", "ap", "dd", "dw", "yy", "cc", "cw", "gg", "zz",
    "zt", "zb", "m", "`", "'", "q", "@", ".", "<C-a>", "<C-x>", "<C-r>", "<C-o>", "<C-i>", "1",
    "2", "3", "9", "/", "?", "<CR>", ":", "noh", "w", "q", "<C-e>", "<C-y>", "<C-f>", "<C-b>",
    "<C-d>", "<C-u>", "ge", "g_", "gn", "gN", "dgn", "cgn", "gv", "o", "gI", "gi", "R", "\"a",
    "\"Ay", "\"1p", "\"_dd", "\"+p", ":d a", ":d 3", ":y a", ":sort iu", ":sort!", ":1,2j",
    ":j!", ":%d", ":$d", ":5", ":s/x/y/", ":s//y/", "&", ":marks", ":reg", "<Home>", "<End>",
    "<Del>", "<C-c>",
];

#[test]
fn fuzz_multi_seed_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
        "x\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "你好, world 123 -45\n#tag\"",
        "a;b|c,d<e>f?g",
        "\n\n\n",
        "0x1f 077 0b101 -99\n42 0099",
    ];
    for seed in 0..12u64 {
        let mut state: u64 = 0x5EED_2026 ^ seed.wrapping_mul(0x9E37_79B9);
        for round in 0..120 {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            for step in 0..160 {
                let k = KEYS[(fuzz_xorshift(&mut state) as usize) % KEYS.len()];
                for key in parse_key_sequence(k) {
                    f.feed_raw(key);
                }
                // IME 打字路径：insert/replace 模式下随机注入组合文本
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    f.type_text("tx中");
                }
                let text = f.text();
                let co = f.vim.cursor_offset();
                assert!(
                    co <= text.len() && (co == text.len() || text.is_char_boundary(co)),
                    "seed {seed} round {round} step {step} key {k}: cursor {co} bad in {text:?}"
                );
                for (name, off) in f.vim.marks.items() {
                    assert!(
                        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
                        "seed {seed} round {round} step {step} key {k}: mark {name} {off} bad in {text:?}"
                    );
                }
                if let Some((lo, hi)) = f.vim.marks.last_visual {
                    assert!(
                        lo <= text.len()
                            && (lo == text.len() || text.is_char_boundary(lo))
                            && hi <= text.len()
                            && (hi == text.len() || text.is_char_boundary(hi)),
                        "seed {seed} round {round} step {step} key {k}: last_visual {lo}..{hi} bad in {text:?}"
                    );
                }
            }
        }
    }
}
