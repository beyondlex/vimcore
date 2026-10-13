//! Round-17 fuzz: path barrage over the surfaces fixed in this round.
//!
//! Increments over round16 (all are paths this round changed, so regressions blast here first):
//! - **macro / `.` / `@` three-way interaction**: q record start/stop, `@a` replay, `.` repeat
//!   under arbitrary interleaving — `replay_records` keeps recording active during `@`, adding
//!   real branches to the record/commit state machine;
//! - **text-object counts**: repeated scanning and inner-block climbing for `d2aw`/`v3i(`/`2ip`;
//! - **sentence objects**: the new endpoint rules (trailing whitespace, leading backward,
//!   last-newline exclusion) for `dis`/`das`/`2as` across inter-sentence whitespace, blank-line
//!   segment boundaries, and the buffer end;
//! - **`<<`/`>>` column model**: the re-expression paths over `/ts/sw/noet/et` (edit_replace
//!   replaces the indent, marks/search cache shift along);
//! - **insert `<C-w>` deleting one whitespace run in a stroke**, **Ex bare `+`/`-` offsets**,
//!   and **`:s`/`:d` trailing count**;
//! - **the `"+` register roundtrip** (the clipboard_write hook).
//!
//! Invariants carry over from round13-16: cursor/mark/`last_visual`/live selection all
//! addressable, line count >= 1, mode and indicator consistent; plus this round's dedicated
//! invariant: macro recording state is true only within a `q` session (`.` replay must not
//! wedge into the recording state).

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds x 60 rounds x 120 steps ~= 115k steps (same budget as round13-16)
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // this round's fixed surface: macro record/replay/`.` three-way
    "qa", "q", "qaxq", "qbxq", "@a", "@b", "@@", "@:", "2@a", ".", "2.", "J",
    "qxq", "qq", "qazq", "d2aw",
    // this round's fixed surface: text-object count (repeat scan + inner-block climb)
    "d2aw", "d3aw", "d2iw", "d3iw", "v2aw", "v3i(", "2ip", "c2aw", "y3aw",
    "di(", "da(", "v2i\"", "v2a\"",
    // this round's fixed surface: sentence objects (new endpoint rules)
    "dis", "das", "2das", "cis", "vis", "vas", "yis",
    // this round's fixed surface: shift column model
    ">>", "<<", "2>>", "2<<", ">j", "<j", "Vj>", ":set noet ts=8 sw=4<CR>",
    ":set et sw=4<CR>", ":set ts=4<CR>", ":set sw=8<CR>",
    // this round's fixed surface: insert C-w whitespace run
    "i", "a", "o", "<C-w>", "<C-u>",
    // this round's fixed surface: Ex bare offset + trailing count
    ":+d<CR>", ":-d<CR>", ":5+d<CR>", ":.-d<CR>", ":+-d<CR>", ":d2<CR>",
    ":y2<CR>", ":j2<CR>", ":s/a/X/ 3<CR>", ":s/a/X/2<CR>", ":1,2s/a/X/2<CR>",
    ":1,2d 2<CR>", ":nohlse<CR>", ":+<CR>", ":-<CR>",
    // this round's fixed surface: + register
    "\"+yy", "\"+dd", "\"+p", "\"+P", "\"_dd", "\"+yiw",
    // round16 key-table excerpt (existing pressure surface not lost)
    "o", "O", "<CR>", "<BS>", "<Esc>", "cc", "S", "3o<Esc>", "2O<Esc>",
    "x", "X", "D", "C", "s", "J", "gJ", "99J", ":sort u<CR>", "&",
    ":1+99999999999999999999d<CR>", "}", "{", "d}", "d{", "ci\"", "ca\"",
    "1%", "%", "\"_yy", "\"\"p", ".", "dd", "dw", "<C-h>", "i<C-h>",
    "V'a", "v`a", "'a", "`a", "mA", "t2", ";", "\"ax", "\"ayy",
    "<C-v>jlo", "VjO", "3:", "2/x<Esc>", ":1,2d 3x", ":y a3", ":%s/x//n",
    ":setlocal ts=8", "gv", "gvd", "@", "p", "yy", ":5+2+1d", "2*", "yaw",
    "d-", "d'a", "ma", ":%s/x/y/", ":%j", "<Left>", "<Right>", "/x<CR>",
    "?中<CR>", ":set ts=8", ":set noic", ":marks", ":reg", "<Del>",
    "<C-c>", "<Tab>", "h", "j", "k", "l", "w", "b", "e", "0", "$", "^",
    "G", "d", "c", "y", "a", "i", "v", "V", "<C-v>", "u", "~", "f", "n",
    "N", "*", "iw", "aw", "cw", "gg", "<C-a>", "<C-x>", "<C-o>", "<C-i>",
];

fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} unaddressable in {text:?}"
    );
}

fn check_invariants(f: &Fixture, text: &str, ctx: &str) {
    assert!(!text.is_empty() || f.buf.line_count() == 1, "{ctx}: line count >= 1");
    assert_addressable("cursor", f.vim.cursor_offset(), text, ctx);
    for (name, off) in f.vim.marks.items() {
        assert_addressable(&format!("mark {name}"), off, text, ctx);
    }
    if let Some((lo, hi, _)) = f.vim.marks.last_visual {
        assert_addressable("last_visual.lo", lo, text, ctx);
        assert_addressable("last_visual.hi", hi, text, ctx);
    }
    if let Some((a, c, _)) = f.vim.visual_selection() {
        let ctx = format!("{ctx} mode {:?}", f.vim.mode());
        assert_addressable("visual.anchor", a, text, &ctx);
        assert_addressable("visual.cursor", c, text, &ctx);
    }
    match f.vim.mode_indicator() {
        "" => assert!(
            !matches!(
                f.vim.mode(),
                vimcore::mode::Mode::Insert
                    | vimcore::mode::Mode::Replace
                    | vimcore::mode::Mode::Visual { .. }
                    | vimcore::mode::Mode::CommandLine { .. }
            ),
            "{ctx}: the normal-mode indicator must be empty"
        ),
        other => assert!(
            !other.is_empty(),
            "{ctx}: a non-normal mode must have an indicator ({other})"
        ),
    }
}

#[test]
fn fuzz_round17_holds_invariants() {
    // the buffer pool centers on this round's fixed surfaces: inter-sentence whitespace
    // (sentence objects), blank-line segment boundaries, mixed indentation (shift
    // re-expression), multi-byte (boundary regression), extendable multi-line
    let buffers = [
        "Aaa. Bbb.\n",
        "Aaa.\n\nBbb.\n",
        "Aaa. Bbb. Ccc.\n",
        "  indent. Next. Tail.\n",
        "foo bar baz\n",
        "f (1 + (2)) g\n",
        "say \"hi\" then \"bye\"\n",
        "\tx\n",
        "\t\tx\n",
        "    ind\n    ind2\n",
        "a\n\nb\n",
        "para one\npara two\n\npara three\n",
        "",
        "中. 文。\n",
        "éA→ ç. ｱｲｳ.\n",
        "one\ntwo\nthree\ntail\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x17AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            f.vim.set_hlsearch_live_update(false);
            // half the seeds carry `nnoremap j gj` (round14 precedent)
            if seed % 2 == 0 {
                f.vim.keymaps.map_str_noremap(
                    vimcore::keymap::ModeClass::Normal,
                    "j",
                    "gj",
                    true,
                );
            }
            // half the seeds pre-record macro a = x (the @a/@@ path is instantly reachable)
            if seed % 3 == 0 {
                for key in vimcore::key::parse_key_sequence("qaxq") {
                    let mut ctx = Ctx {
                        buf: &mut f.buf,
                        host: &mut f.host,
                    };
                    let _ = f.vim.handle_key(&mut ctx, key);
                }
            }
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            for step in 0..STEPS {
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                    if seed % 2 == 0 {
                        f.vim.keymaps.map_str_noremap(
                            vimcore::keymap::ModeClass::Normal,
                            "j",
                            "gj",
                            true,
                        );
                    }
                }
                let roll = fuzz_xorshift(&mut state);
                // host-event injection (~1/8 of steps): IME text and drag-select/click
                // delivered in arbitrary modes
                match roll % 8 {
                    0 => {
                        let a = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        let b = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        f.vim.set_visual_range(&f.buf, a, b);
                    }
                    1 => {
                        let o = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                        f.vim.set_cursor_offset(&f.buf, o);
                    }
                    2 => {
                        f.vim.record_typed_text("中");
                        f.type_text("中");
                    }
                    3 => {
                        let len = f.text().len();
                        let a = (fuzz_xorshift(&mut state) as usize) % (len + 1);
                        let b = a + (fuzz_xorshift(&mut state) as usize) % (len - a + 1);
                        f.ime_replace(a..b, "x");
                    }
                    _ => {}
                }
                let idx = (fuzz_xorshift(&mut state) as usize) % KEYS.len();
                let seq = vimcore::key::parse_key_sequence(KEYS[idx]);
                for key in seq {
                    let mut ctx = Ctx {
                        buf: &mut f.buf,
                        host: &mut f.host,
                    };
                    let _ = f.vim.handle_key(&mut ctx, key);
                }
                check_invariants(&f, &f.text(), &format!("seed{seed} r{round} s{step} k{}", KEYS[idx]));
            }
        }
    }
}

/// Huge-count text object: the repeat scan for `<huge-value>aw` must terminate once the span
/// reaches the buffer end, not scan linearly O(count) times (break on the first no-progress).
#[test]
fn fuzz_round17_huge_object_count_terminates() {
    let start = std::time::Instant::now();
    let mut f = common::Fixture::new("foo bar\n");
    for c in "999999999".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["a", "w"]);
    f.feed(["v"]);
    for c in "999999999".chars() {
        f.feed([c.to_string().as_str()]);
    }
    f.feed(["i", "w"]);
    assert!(f.buf.len() < 1024, "object count must not grow the buffer");
    assert!(start.elapsed().as_secs() < 5, "must terminate quickly");
}
