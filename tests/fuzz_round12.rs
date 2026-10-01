//! 第十二轮扩展 fuzz：覆盖本轮修复的路径。
//!
//! 相对 round11 的增量键表：
//! - Ex 范围新语义：偏移链（`:5+2+1d`）、多字节 mark 名（`:'中d`、
//!   `:,'中d`）、等值两地址（`:2,2j`）、显式 count（`:1,2d 1`）、
//!   特殊寄存器（`:d _`、`:d +`）、带范围替换重放（`:3,4s`）；
//! - 搜索跳转 count（`2*`、`2#`、`3gN`、`d2gn`、`c2gN`）与空白对象
//!   （`yaw`、`caw`、`d2aw`）；
//! - 边缘 motion（`d-`、`d+`、`2-`、`3+`）与空行 `$`/`g_` 算子。
//!
//! 不变量沿用 round11：光标/mark/`last_visual`/live 选区/`last_matches`
//! 全部可寻址。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
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
    // 本轮新增：Ex 范围偏移链与多字节 mark（修复路径）
    ":5+2+1d",
    ":3-1d",
    ":2+1",
    ":'中d",
    ":,'中d",
    ":2,'中y",
    ":'a,+2d",
    // 本轮新增：等值范围 / 显式 count / 特殊寄存器（修复路径）
    ":2,2j",
    ":3,3d",
    ":2,2y",
    ":1,2d 1",
    ":1,2y 1",
    ":2j 3",
    ":d _",
    ":d +",
    ":y a 2",
    ":3,4s",
    // 本轮新增：搜索 count 与空白对象（修复路径）
    "2*",
    "2#",
    "3gN",
    "2gn",
    "d2gn",
    "c2gN",
    "yaw",
    "caw",
    "d2aw",
    "y2aw",
    // 本轮新增：边缘 motion 与空行算子（修复路径）
    "d-",
    "d+",
    "2-",
    "3+",
    "d$",
    "dg_",
    "d2$",
    "y$",
    "c$",
    // 本轮新增：引号对象在字符串之间（修复路径）
    "ci\"",
    "ca\"",
    "di\"",
    // round11 键表节选（保持回归压力）
    "d'a",
    "ma",
    "mb",
    "'a",
    ":2,5",
    ":1,2,3d",
    ":,3d",
    ":&",
    ":s",
    ":%s/x/y/",
    ":%j",
    "R",
    "<BS>",
    "/x<CR>",
    "?a<CR>",
    "/中<CR>",
    "2i",
    "3o",
    ":set ts=8",
    ":set noic",
    ":set ic",
    ":set hls!",
    ">>",
    "guw",
    "Vgq",
    "<C-r>",
    "yiw",
    ":d",
    ":sort u",
    ":'<,'>d",
    "(",
    "{",
    "ge",
    "H",
    "+",
    "-",
    "<C-e>",
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
    "daw",
    "das",
    "dap",
    "<C-w>",
    "3D",
    "<C-v>jU",
    "<C-v>jI",
    "Vp",
    ":noh",
    "qa",
    "q",
    "@a",
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
    "G",
    "d",
    "c",
    "y",
    "p",
    "P",
    "x",
    "s",
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
    "dd",
    "dw",
    "yy",
    "cc",
    "cw",
    "gg",
    ".",
    "<C-a>",
    "<C-x>",
    "<C-o>",
    "<C-i>",
    "<CR>",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
    "gv",
    "\"_dd",
];

/// 校验一个偏移在 `text` 上可寻址(字符边界或末尾)。
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} 偏移 {off} 不可寻址 in {text:?}"
    );
}

#[test]
fn fuzz_round12_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        // 多字符串行:引号对象在字符串之间的高压面
        "say \"hi\" then \"bye\" ok\nx = \"a\" + \"b\";\n",
        // 词间空白 + 纯空白行 + 真空行:aw/w/d$ 的高压面
        "foo   bar  baz\n   \n\nnext word\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "0x1f 077 0b101 -99\n42 0099",
        "a b a b a b\nmatch me and me\n",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "one two three\nfour five\n",
        "Do not quote 'this' or `that`, ok?\n",
    ];
    for seed in 0..16u64 {
        let mut state: u64 = 0xDEAD_BEEF ^ seed.wrapping_mul(0x2722_0A95);
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

/// Ex 范围参数的随机组合轰炸:地址/偏移/分隔符/寄存器/count 任意拼接,
/// 命令只允许失败(E16/E20/E78/E486/bell),不允许 panic。
#[test]
fn ex_range_garbage_never_panics() {
    let addresses = [
        "", ".", "$", "%", "0", "1", "5", "99", "'a", "'中", "'<", "'>", "''",
    ];
    let offsets = ["", "+1", "-1", "+2-1", "+99", "-99", "+99999999999999999999"];
    let seps = [",", ";"];
    let commands = ["", "d", "d _", "d +", "y", "y a", "j", "j!", "s", "sort", ">", "w", "q", "1"];
    let buffers = ["a\nb\nc\nd\ne\n", "中文\nabc\n", ""];
    let mut state: u64 = 0xCAFE_0001;
    for round in 0..600 {
        let mut line = String::from(":");
        let addr_a = addresses[(fuzz_xorshift(&mut state) as usize) % addresses.len()];
        line.push_str(addr_a);
        if fuzz_xorshift(&mut state).is_multiple_of(2) {
            line.push_str(seps[(fuzz_xorshift(&mut state) as usize) % seps.len()]);
            let addr_b = addresses[(fuzz_xorshift(&mut state) as usize) % addresses.len()];
            line.push_str(addr_b);
        }
        let off = offsets[(fuzz_xorshift(&mut state) as usize) % offsets.len()];
        line.push_str(off);
        if fuzz_xorshift(&mut state).is_multiple_of(3) {
            let off2 = offsets[(fuzz_xorshift(&mut state) as usize) % offsets.len()];
            line.push_str(off2);
        }
        let cmd = commands[(fuzz_xorshift(&mut state) as usize) % commands.len()];
        line.push_str(cmd);
        let mut f = Fixture::new(buffers[round % buffers.len()]);
        f.vim.set_cursor_offset(&f.buf, 2.min(f.text().len()));
        // feed as Ex: colon already in `line`, then each char, then CR
        for ch in line.chars().skip(1) {
            f.feed_raw(vimcore::key::Key::char(ch));
        }
        f.feed_raw(vimcore::key::Key::parse("<CR>"));
        let text = f.text();
        let ctx = format!("round {round} line {line:?}");
        assert_addressable("cursor", f.vim.cursor_offset(), &text, &ctx);
        assert!(f.buf.line_count() >= 1, "{ctx}: line count underflow");
    }
}
