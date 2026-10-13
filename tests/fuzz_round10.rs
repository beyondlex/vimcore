//! Round-10 extended fuzz: paths added after the review gap-fill + standing
//! pressure on the allocation-cap guards.
//!
//! Key-table delta over round9:
//! - cmdline history browsing (`<Up>`/`<Down>` at the `/`/`?`/`:` prompts) and
//!   `/x<CR>` searches;
//! - the full text-object set (`i"/a"/i'/a'/iW/aW/is/as/i[/a[/i{/a{/iB/aB/i</a<`);
//! - count-repeat insert (`2i`/`3o`/`5a`/`9I`/`7A`/`4gi`/`2gI`) — the
//!   allocation-cap path fixed this round;
//! - `:set` numeric/boolean options (ts/sw/et/ai/tw, changing indent and
//!   formatting behavior);
//! - indent and case-operation counts (`>>`/`<<`/`3>>`/`guw`/`gUw`/`g~w`/`gugu`/
//!   `gUU`/`g~~`/`gww`), visual endpoint swap (`o`/`O`);
//! - insert-mode `<C-r>` register paste; Ex `:y`/`:d` variants, `:sort` flags,
//!   `:s` flags (`g`/`i`) and `:&`, `:action`, `:w`/`:q`;
//! - sentence/paragraph/screen-line motions (`(`/`)`/`{`/`}`/`H`/`M`/`L`/`+`/
//!   `-`/`|`) and `<C-e>`/`<C-y>` scrolling;
//! - IME-injected text gains newline-carrying variants (the autoindent path).
//!
//! Invariants carry over from round9, plus search-cache match addressability:
//! the cursor / all marks / `last_visual` / the live selection / the `gv`
//! rebuilt selection / offsets in `last_matches` must all be addressable.

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
    // new this round: history browsing and search
    "<Up>",
    "<Down>",
    "/x<CR>",
    "?a<CR>",
    "/<CR>",
    // new this round: count-repeat insert (allocation-cap path)
    "2i",
    "3o",
    "5a",
    "9I",
    "7A",
    "4gi",
    "2gI",
    // new this round: :set options (change indent/line-breaking behavior)
    ":set ts=8",
    ":set sw=2",
    ":set noet",
    ":set et",
    ":set tw=20",
    ":set noai",
    ":set ts?",
    // new this round: indent/case/format counts and variants
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
    // new this round: visual endpoint swap
    "o",
    "O",
    // new this round: insert register paste (<C-r> is redo in normal mode; stress both)
    "<C-r>",
    "yiw",
    // new this round: Ex yank/delete/sort/substitute variants and misc
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
    // new this round: sentence/paragraph/screen-line/scroll
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
    // new this round: the full text-object set
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
    // new this round: mark interactions
    "ma",
    "mb",
    "'a",
    "`a",
    // existing coverage (round9 key-table excerpt, keeping regression pressure)
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

/// Check that an offset is addressable in `text` (a char boundary or the end).
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} not addressable in {text:?}"
    );
}

#[test]
fn fuzz_round10_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        // quote/tag/bracket-rich: the bombing ground for text objects and :s flags
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
                // IME typing path: random injection in insert/replace mode
                // (including multiline text, going through autoindent expansion)
                if matches!(f.vim.mode(), vimcore::Mode::Insert | vimcore::Mode::Replace)
                    && fuzz_xorshift(&mut state).is_multiple_of(4)
                {
                    if fuzz_xorshift(&mut state).is_multiple_of(3) {
                        f.type_text("a\nb中");
                    } else {
                        f.type_text("t中x");
                    }
                }
                // host clicks: random landing points (deliberately including
                // non-char boundaries — the entry must floor)
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

// ---- Ex argument-surface bombing: arbitrary argument combos beyond fixed command strings --

/// Panic hunt over the Ex command surface + arbitrary argument combinations:
/// regex injection, odd delimiters, huge line numbers, negative offsets, mark
/// name injection, non-ASCII arguments.
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
            // run a random 1-4 Ex commands in sequence
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
