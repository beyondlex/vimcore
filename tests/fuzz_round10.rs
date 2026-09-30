//! 第十轮扩展 fuzz：覆盖审查补漏后的路径 + 分配上限守卫的常驻压力。
//!
//! 相对 round9 的增量键表：
//! - cmdline 历史浏览（`<Up>`/`<Down>` 在 `/`/`?`/`:` 提示符）与 `/x<CR>` 搜索；
//! - 文本对象全集（`i"/a"/i'/a'/iW/aW/is/as/i[/a[/i{/a{/iB/aB/i</a<`）；
//! - count-repeat insert（`2i`/`3o`/`5a`/`9I`/`7A`/`4gi`/`2gI`）——本轮
//!   修复的分配上限路径；
//! - `:set` 数值/布尔选项（改 ts/sw/et/ai/tw，改变缩进与格式化行为）；
//! - 缩进与大小写操作计数（`>>`/`<<`/`3>>`/`guw`/`gUw`/`g~w`/`gugu`/`gUU`/`g~~`/
//!   `gww`）、visual 端点交换（`o`/`O`）；
//! - insert 模式 `<C-r>` 寄存器粘贴；Ex 的 `:y`/`:d` 变体、`:sort` 旗标、
//!   `:s` 旗标（`g`/`i`）与 `:&`、`:action`、`:w`/`:q`；
//! - 句子/段落/屏幕行 motion（`(`/`)`/`{`/`}`/`H`/`M`/`L`/`+`/`-`/`|`）与
//!   `<C-e>`/`<C-y>` 滚动；
//! - IME 注入文本新增带换行变体（autoindent 路径）。
//!
//! 不变量沿用 round9，另加 search 缓存匹配可寻址：光标/全部 mark/
//! `last_visual`/live 选区/`gv` 重建选区/`last_matches` 的偏移全部可寻址。

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
    // 本轮新增：历史浏览与搜索
    "<Up>",
    "<Down>",
    "/x<CR>",
    "?a<CR>",
    "/<CR>",
    // 本轮新增：count-repeat insert（分配上限路径）
    "2i",
    "3o",
    "5a",
    "9I",
    "7A",
    "4gi",
    "2gI",
    // 本轮新增：:set 选项（改变缩进/换行行为）
    ":set ts=8",
    ":set sw=2",
    ":set noet",
    ":set et",
    ":set tw=20",
    ":set noai",
    ":set ts?",
    // 本轮新增：缩进/大小写/格式化的计数与变体
    ">>",
    "<<",
    "3>>",
    ">j",
    "<j",
    "guw",
    "gUw",
    "g~w",
    "gugu",
    "gUU",
    "g~~",
    "gww",
    "Vgq",
    // 本轮新增：visual 端点交换
    "o",
    "O",
    // 本轮新增：insert 寄存器粘贴（正常模式下 <C-r> 是 redo，两边都压）
    "<C-r>",
    "yiw",
    // 本轮新增：Ex yank/delete/sort/substitute 变体与杂项
    ":y a",
    ":2y",
    ":%y",
    ":d",
    ":1,2d",
    ":sort u",
    ":sort",
    ":s/x/y/g",
    ":s/o/0/i",
    ":&",
    ":action Foo",
    ":w",
    ":q",
    ":5",
    ":'<,'>d",
    // 本轮新增：句子/段落/屏幕行/滚动
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
    // 本轮新增：文本对象全集
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
    // 本轮新增：mark 交互
    "ma",
    "mb",
    "'a",
    "`a",
    // 既有覆盖（round9 键表节选，保持回归压力）
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
    "Vp",
    "vp",
    "gq}",
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
fn fuzz_round10_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        // 引号/标签/括号丰富：文本对象与 :s 旗标的轰炸场
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
        let mut state: u64 = 0x9E37_79C9 ^ seed.wrapping_mul(0x85EB_CA6B);
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
                // IME 打字路径：insert/replace 模式下随机注入（含多行文本，
                // 走 autoindent 展开）
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    if fuzz_xorshift(&mut state).is_multiple_of(3) {
                        f.type_text("a\nb中");
                    } else {
                        f.type_text("t中x");
                    }
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

// ---- Ex 参数面轰炸：固定命令串之外的任意参数组合 ------------------------------

/// Ex 命令面 + 任意参数组合的 panic 猎捕：正则注入、怪分隔符、巨行号、
/// 负偏移、mark 名注入、非 ASCII 参数。
#[test]
fn ex_argument_fuzz_holds() {
    let cmds = [
        ":s/^/$/g",
        ":s/\\x",
        ":s/(/x/",
        ":s/[/",
        "x/",
        ":s/x/y",
        ":s//x/",
        ":s/\\\\/y/",
        ":s/中/文/g",
        ":s/./中/",
        ":s/中*/-/",
        ":5,2d",
        ":99d",
        ":99y",
        ":0d",
        ":$y",
        ":+2,-1j",
        ":.+1d",
        ":'a,'bd",
        ":5j 99",
        ":sort!iu",
        ":sort ix",
        ":sort 中",
        ":y 中",
        ":d !",
        ":d ! 2",
        ":y a b",
        ":set ts=0",
        ":set sw=abc",
        ":set ts?",
        ":set 中",
        ":set!",
        ":marks",
        ":reg",
        ":action",
        ":action 中 空格",
        ":action  a  b ",
        ":",
        ":5",
        ":-5",
        ":%",
        ":@a",
        ":noh!",
        ":w!",
        ":q a",
        ":99999999999999999999d",
        ":'<",
        ":'<,$d",
        ":.,.+999999999d",
        ":s/\\/",
        ":s/a\\/b/c/",
        ":substitute",
        ":substitutex/y/",
        ":sor",
        ":jjoin",
    ];
    let buffers = ["", "a\nb\n", "中文\nx/y\n", "a(b[c]d)e\n", "foo bar\n"];
    for seed in 0..64u64 {
        let mut state = 0xDEADBEEF ^ seed.wrapping_mul(0x9E37_79B9);
        for round in 0..40 {
            let mut f = Fixture::new(buffers[round % buffers.len()]);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            // 随机 1-4 条 Ex 命令串行执行
            for _ in 0..1 + fuzz_xorshift(&mut state) % 4 {
                let cmd = cmds[(fuzz_xorshift(&mut state) as usize) % cmds.len()];
                for c in cmd.chars() {
                    let key = if c == '\n' {
                        vimcore::key::Key::named("enter")
                    } else {
                        vimcore::key::Key::char(c)
                    };
                    f.feed_raw(key);
                }
                f.feed_raw(vimcore::key::Key::named("enter"));
                let text = f.text();
                let off = f.vim.cursor_offset();
                assert!(
                    off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
                    "seed {seed} round {round} cmd {cmd}: cursor {off} bad in {text:?}"
                );
                for (name, mo) in f.vim.marks.items() {
                    assert!(
                        mo <= text.len() && (mo == text.len() || text.is_char_boundary(mo)),
                        "seed {seed} round {round} cmd {cmd}: mark {name} {mo} bad in {text:?}"
                    );
                }
            }
        }
    }
}
