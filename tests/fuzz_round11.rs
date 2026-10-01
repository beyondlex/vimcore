//! 第十一轮扩展 fuzz：覆盖本轮修复的路径 + mark 算子面。
//!
//! 相对 round10 的增量键表：
//! - mark 跳转作为算子目标（`d'a`/`d\`a`/`y'a`/`c'a`——本轮修复的整条
//!   缺失路径，JumpMark 臂过去不消化算子）；
//! - 新范围语义（`:2,5` 空命令、`:1,2,3d` 多地址、`:,3d`/`:2,d` 空地址、
//!   `&` 旗标重放、`:s/x/y/g` + `&` 组合）；
//! - 替换重放/查询（`:&`、`:s`、`gi`、`gv` 在 visual 变更后）；
//! - Replace 模式 BS 恢复栈与 `R` 大会话。
//!
//! 不变量沿用 round10：光标/mark/`last_visual`/live 选区/`last_matches`
//! 全部可寻址，外加轮末 `g;`/`<C-o>` 后光标仍可寻址（changelist/jumplist
//! 的暴露面经 floor 守卫）。

mod common;

use common::Fixture;
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
    // 本轮新增：mark 算子面（修复路径）
    "d'a",
    "d`a",
    "y'a",
    "y`a",
    "c'a",
    "c`a",
    "2d'a",
    "ma",
    "mb",
    "'a",
    "`a",
    "'b",
    // 本轮新增：Ex 范围语义（修复路径）
    ":2,5",
    ":1,2,3d",
    ":,3d",
    ":2,d",
    ":1,3y",
    ":.,$d",
    ":s/x/y/g",
    ":&",
    ":s",
    ":%s/x/y/",
    ":%j",
    // 本轮新增：Replace 会话与恢复栈
    "R",
    "<BS>",
    // 既有覆盖（round10 键表节选，保持回归压力）
    "<Up>",
    "<Down>",
    "/x<CR>",
    "?a<CR>",
    "2i",
    "3o",
    "5a",
    "9I",
    "7A",
    "4gi",
    "2gI",
    ":set ts=8",
    ":set sw=2",
    ":set noet",
    ":set et",
    ">>",
    "3>>",
    "guw",
    "gUw",
    "g~w",
    "Vgq",
    "o",
    "O",
    "<C-r>",
    "yiw",
    ":y a",
    ":2y",
    ":d",
    ":sort u",
    ":action Foo",
    ":w",
    ":q",
    ":'<,'>d",
    "(",
    ")",
    "{",
    "}",
    "ge",
    "gE",
    "|",
    "H",
    "M",
    "L",
    "+",
    "-",
    "<C-e>",
    "<C-y>",
    "Y",
    "i\"",
    "a\"",
    "i'",
    "a'",
    "iW",
    "aW",
    "is",
    "as",
    "i[",
    "a[",
    "i{",
    "a{",
    "iB",
    "aB",
    "i<",
    "a<",
    "daw",
    "das",
    "dap",
    "<C-w>",
    "3D",
    "3C",
    "99D",
    "r<CR>",
    "<Tab>",
    ":substitute/x/y/",
    ":s/x",
    ":2j 3",
    ":bfirst",
    ":blast",
    "gq}",
    "v$gq",
    "<C-v>jU",
    "<C-v>jI",
    "<C-v>jA",
    "<C-v>jc",
    "Vp",
    "vp",
    ":s/(/x/",
    ":2",
    ":noh",
    "qa",
    "q",
    "@a",
    ":",
    "g;",
    "g,",
    "h",
    "j",
    "k",
    "l",
    "w",
    "b",
    "e",
    "0",
    "$",
    "^",
    "g",
    "G",
    "d",
    "c",
    "y",
    "p",
    "P",
    "x",
    "X",
    "s",
    "S",
    "D",
    "C",
    "r",
    "a",
    "i",
    "v",
    "V",
    "<C-v>",
    "u",
    "<Esc>",
    "J",
    "gJ",
    ">",
    "<",
    "gu",
    "gU",
    "g~",
    "~",
    "f",
    "t",
    ";",
    "%",
    "n",
    "N",
    "*",
    "#",
    "iw",
    "aw",
    "i(",
    "a(",
    "it",
    "ip",
    "ap",
    "dd",
    "dw",
    "yy",
    "cc",
    "cw",
    "gg",
    "zz",
    "m",
    "`",
    "'",
    ".",
    "<C-a>",
    "<C-x>",
    "<C-o>",
    "<C-i>",
    "1",
    "3",
    "/",
    "?",
    "<CR>",
    ":d a",
    ":1,2j",
    ":j!",
    ":%d",
    ":$d",
    ":sort iu",
    ":sort!",
    ":s/x/y/",
    ":s//y/",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
    "gv",
    "gn",
    "gN",
    "R",
    "\"a",
    "\"1p",
    "\"_dd",
];

/// 校验一个偏移在 `text` 上可寻址（字符边界或末尾）。
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

#[test]
fn fuzz_round11_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        "let s = \"hi\"; // <div id=\"x\">t</div>\nf(a, [b], {c})\n",
        "foo bar 中文 baz 👨‍👩‍👧 tail\nsecond 中文 line\n\nlast",
        "aaaa\nbbbb\ncccc\ndddd",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "aBc\ndEf\nGhI\n",
        "0x1f 077 0b101 -99\n42 0099",
        "one two three\nfour five\n",
        "Do not quote 'this' or `that`, ok?\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0x0BAD_C0DE ^ seed.wrapping_mul(0x2722_0A95);
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
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    if fuzz_xorshift(&mut state).is_multiple_of(3) {
                        f.type_text("a\nb中");
                    } else {
                        f.type_text("t中x");
                    }
                }
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
                if let Some((a, c, _)) = f.vim.visual_selection() {
                    assert_addressable("visual.anchor", a, &text, &ctx);
                    assert_addressable("visual.cursor", c, &text, &ctx);
                }
                for (i, m) in f.vim.search.last_matches.iter().enumerate() {
                    assert_addressable(&format!("match{i}.start"), m.start, &text, &ctx);
                    assert_addressable(&format!("match{i}.end"), m.end, &text, &ctx);
                }
            }
        }
    }
}

/// changelist / jumplist 的暴露面：`g;`/`g,`/`<C-o>`/`<C-i>` 任意轰炸后
/// 光标必须可寻址（两个列表的 floor 守卫常驻压力）。
#[test]
fn change_and_jump_list_walks_stay_addressable() {
    let buffers = [
        "",
        "a",
        "中\n文\n",
        "foo bar\nbaz 中文 qux\n\ntail",
    ];
    let keys = [
        "x", "dd", "yy", "p", "cc", "cw", "1", "o", "O", "g;", "g,", "<C-o>",
        "<C-i>", "u", "<C-r>", ":s/x/y/", ":d", ":j", "j", "k", "ma", "'a",
        "d'a", "y`a", "gv", "<Esc>", "i", "<BS>", "a", "3x", ".",
    ];
    for seed in 0..8u64 {
        let mut state = 0xFEED_FACE ^ seed.wrapping_mul(0x9E37_79B9);
        for round in 0..60 {
            let mut f = Fixture::new(buffers[round % buffers.len()]);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            for step in 0..200usize {
                let k = keys[(fuzz_xorshift(&mut state) as usize) % keys.len()];
                for key in parse_key_sequence(k) {
                    f.feed_raw(key);
                }
                if step.is_multiple_of(7) {
                    // 每逢 7 步走一遍历史，逼出列表越界
                    for _ in 0..5 {
                        f.feed_raw(vimcore::key::Key::parse("g;"));
                    }
                    for _ in 0..3 {
                        f.feed_raw(vimcore::key::Key::parse("<C-i>"));
                    }
                }
                let text = f.text();
                let ctx = format!("seed {seed} round {round} step {step} key {k}");
                assert_addressable("cursor", f.vim.cursor_offset(), &text, &ctx);
            }
        }
    }
}
