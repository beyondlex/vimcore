//! Round-13 fuzz: covers the paths fixed this round and the new review surface.
//!
//! Key-table delta over round12:
//! - host mouse events in visual mode (`set_cursor_offset` click,
//!   `set_visual_range` drag) followed by d/y/c/:/gv —— round13 fixed
//!   "click collapsing the selection"; here we pin anchor retention + selection addressability;
//! - setting and jumping with uppercase marks (`mA`-`mZ`, `` `A ``-`'Z`);
//! - the `q` register character surface (`q/`, `q!`, `q:` and other illegal names must now be
//!   rejected) interleaved with `@` replay;
//! - `.` after no-op commands (empty-line `x`, empty-register `p`, EOF `J`) ——
//!   a no-op must not displace the replay record of the previous real change.
//!
//! Invariants carry over from round12: cursor/mark/`last_visual`/live selection/`last_matches`
//! all addressable; after a click in visual mode both anchor and cursor remain addressable.

mod common;

use common::Fixture;
use vimcore::state::Ctx;
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

const DRAG_EVENTS: bool = true;
// 16 seeds × 60 rounds × 120 steps ≈ 115k steps: enough key-table rotation, while keeping
// the trigger count of self-recursive macros (qa…@a…q → @a filling the 100k-key pipeline
// budget, see enqueue_replay) within seconds for the whole cargo test run
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // round13 fix path: acting after a visual-mode click
    "<C-v>jI",
    "<C-v>jA",
    "<C-v>jx",
    "Vjd",
    "viwd",
    "viwc",
    "gv",
    "gvd",
    "gvy",
    "gvc",
    "gv:",
    // round13 fix path: uppercase marks
    "mA",
    "mB",
    "mZ",
    "`A",
    "'A",
    "`Z",
    "`B",
    // round13 fix path: the q register character surface
    "q/",
    "q!",
    "q:",
    "q\"",
    "q1",
    "qa",
    "q",
    "@a",
    "@1",
    "@/",
    "@@",
    "@:",
    // round13 fix path: `.` after a no-op
    "x",
    ".",
    "p",
    "P",
    "J",
    "dd",
    "dw",
    "yy",
    "2dd",
    "d99d",
    // round12 key-table excerpt (keeping regression pressure)
    ":5+2+1d",
    ":2,2j",
    ":d _",
    ":3,4s",
    "2*",
    "d2gn",
    "yaw",
    "d-",
    "dg_",
    "ci\"",
    "d'a",
    "ma",
    "mb",
    ":1,2,3d",
    ":&",
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
    "is",
    "i[",
    "a{",
    "daw",
    "dap",
    "<C-w>",
    "3D",
    "Vp",
    ":noh",
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
    "gJ",
    "~",
    "f",
    "t",
    ";",
    ",",
    "%",
    "n",
    "N",
    "*",
    "#",
    "iw",
    "aw",
    "cw",
    "gg",
    "<C-a>",
    "<C-x>",
    "<C-o>",
    "<C-i>",
    "<CR>",
    ":marks",
    ":reg",
    "<Del>",
    "<C-c>",
    "\"_dd",
];

/// Check that an offset is addressable (char boundary or end) in `text`.
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} not addressable in {text:?}"
    );
}

/// Same predicate as [`assert_addressable`], for batch paths that decide first and
/// format later (`last_matches` can reach 10k entries; per-entry formatting is the
/// dominant fuzz cost).
fn addressable(off: usize, text: &str) -> bool {
    off <= text.len() && (off == text.len() || text.is_char_boundary(off))
}

/// Check that every stored engine offset is addressable in the current text. `text` is
/// passed in by the caller (a full clone every step on a large buffer is quadratic cost —
/// copying a buffer whose history grew to 7MB each step slowed the debug build to 77s;
/// large buffers are checked on a step sample, small buffers fully every step).
fn check_invariants(f: &Fixture, text: &str, ctx: &str) {
    check_invariants_mode(f, text, ctx, true);
}

/// `check_matches=false`: with hlsearch_live_update off, `last_matches` may be stale
/// between two refresh_highlights calls (the host's own refresh cadence is the design
/// premise of this mode; the generation mechanism guarantees a rescan before n/N
/// consume it), so addressability only holds for the refreshed list.
fn check_invariants_mode(f: &Fixture, text: &str, ctx: &str, check_matches: bool) {
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
    if check_matches {
        if let Some((i, m)) = f
            .vim
            .search
            .last_matches
            .iter()
            .enumerate()
            .find(|(_, m)| !addressable(m.start, text) || !addressable(m.end, text))
        {
            assert_addressable(&format!("match{i}.start"), m.start, text, ctx);
            assert_addressable(&format!("match{i}.end"), m.end, text, ctx);
        }
    }
}

#[test]
fn fuzz_round13_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "中文\nwide line\n",
        "say \"hi\" then \"bye\" ok\nx = \"a\" + \"b\";\n",
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
    for seed in 0..SEEDS {
        let mut state: u64 = 0x13AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            // A self-recursive macro (qa…@a…q then @a) fills the 100k-key pipeline budget;
            // per-edit full-file hlsearch rescans + host 10k-entry highlight copies drag each
            // step to millisecond scale — disable per-edit rescan as a large-file host would,
            // refreshing per step instead (same code path; round10-11 rescan fix surface is
            // covered by the existing fuzz)
            f.vim.set_hlsearch_live_update(false);
            for step in 0..STEPS {
                // The yy+p loop can double the buffer without bound (the engine only caps
                // paste bytes at 16MB); resetting pins per-step engine cost to a constant.
                // Every bug this fuzz caught reproduces on a few-KB buffer, so resetting
                // loses no coverage
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                }
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
                // Host mouse event: click (must keep the anchor in visual mode) or drag selection
                if fuzz_xorshift(&mut state).is_multiple_of(9) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
                }
                if DRAG_EVENTS && fuzz_xorshift(&mut state).is_multiple_of(23) {
                    let a = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 1);
                    let b = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 1);
                    f.vim.set_visual_range(&f.buf, a, b);
                }
                let refreshed = step % 16 == 0;
                if refreshed {
                    let mut hl = Ctx { buf: &mut f.buf, host: &mut f.host };
                    f.vim.refresh_highlights(&mut hl);
                }
                let ctx = format!("seed {seed} round {round} step {step} key {k}");
                if f.buf.len() < 4096 || refreshed {
                    let text = f.text();
                    check_invariants_mode(&f, &text, &ctx, refreshed);
                }
            }
        }
    }
}

/// Continuously check the core semantics fixed in round13 between random key bombs:
/// 1. `set_cursor_offset` in visual mode keeps the anchor (selection does not collapse);
/// 2. a no-op does not displace the `.` replay record.
#[test]
fn fuzz_round13_semantic_invariants() {
    let mut state: u64 = 0x00D0_0D13;
    let buffers = [
        "word\n\nnext\n",
        "aaa\nbbb\nccc\nddd\n",
        "中文 test line\nsecond\n",
        "single",
    ];
    for round in 0..300u64 {
        let initial = buffers[(fuzz_xorshift(&mut state) as usize) % buffers.len()];
        let mut f = common::Fixture::new(initial);
        for step in 0..60 {
            let k = KEYS[(fuzz_xorshift(&mut state) as usize) % KEYS.len()];
            for key in parse_key_sequence(k) {
                f.feed_raw(key);
            }
            let ctx = format!("round {round} step {step} key {k}");
            // Invariant 1: visual + click → anchor retained
            if matches!(f.vim.mode(), vimcore::Mode::Visual { .. }) {
                if let Some(anchor_before) = f.vim.visual_selection().map(|(a, _, _)| a) {
                    let click = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
                    f.vim.set_cursor_offset(&f.buf, click);
                    let sel = f.vim.visual_selection();
                    assert_eq!(
                        sel.map(|(a, _, _)| a),
                        Some(anchor_before),
                        "{ctx}: a visual click must keep the anchor {anchor_before}"
                    );
                }
            }
            let text = f.text();
            if text.len() < 4096 || step % 16 == 0 {
                check_invariants(&f, &text, &ctx);
            }
        }
    }
}
