//! Round-14 fuzz: path barrage covering this round's fix surface.
//!
//! Key-table delta relative to round13:
//! - till family repeats (`;`/`,`/count interleaved with t/T) — round14 fixed the
//!   "repeat fakes success in place" and redefined count semantics (fresh counts
//!   distinct targets; repeat counts finds after skipping an immediately adjacent target);
//! - till operator spans (`dt{c}`/`ct{c}`/`yt{c}`) — the span now reaches the target position;
//! - `"{reg}` prefixes into x/X/s/Del, `""` normalization — the register-routing surface;
//! - visual `o`/`O` (block O swaps columns on the same line; char/line O equal o);
//! - cmdline counts (`3:`, `3/pat`, cancel paths) and Ex's new error surfaces
//!   (E488 trailing args, E477 bang, E16 out-of-range, the `n` flag, escaped delimiters,
//!   `:se`/`:setlocal`/`ts&`/`ts ?`);
//! - arrow keys + BS interleaved inside a Replace session (the position stack).
//!
//! Half of the seeds pre-register `:nnoremap j gj`: keeping the new "partial command
//! continuation keys do not consult mappings" guard (FallThrough when cmd_seq is
//! non-empty) and the expansion replay under constant random barrage.
//!
//! Invariants carried over from round13: cursor/mark/`last_visual`/live selection/
//! `last_matches` all addressable; after `*` the pattern and the highlights must be in sync.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::parse_key_sequence;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 60 rounds × 120 steps ≈ 115k steps (same budget as round13)
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // round14 fix paths: till repeats and spans
    "t2",
    "t2;",
    "t2;;",
    "2t3",
    "T2",
    "T2,",
    ";",
    ",",
    "2;",
    "2,",
    "dtx",
    "dto",
    "ct2",
    "yt3",
    "dfx",
    "2fx",
    "d2t3",
    // round14 fix paths: register-prefix routing
    "\"ax",
    "\"aX",
    "\"as",
    "\"add",
    "\"ayy",
    "\"_dd",
    "\"\"dd",
    "\"\"p",
    "\"ap",
    "\"ayiw",
    // round14 fix paths: visual o / O
    "<C-v>jlo",
    "<C-v>jlO",
    "<C-v>jklO",
    "vllo",
    "vlO",
    "VjO",
    "Vj o",
    "o",
    "O",
    // round14 fix paths: cmdline counts
    "3:",
    "2:",
    "3:<CR>",
    "3:<Esc>",
    "2/x<Esc>",
    "3/foo<CR>",
    "?x<CR>",
    "V3v:",
    // round14 fix paths: Ex new error surfaces
    ":1,2d 3x",
    ":d a b",
    ":y a3",
    ":2y a 2",
    ":1,2d!",
    ":1,2y!",
    ":1000000d",
    ":2,99999d",
    ":5 +2d",
    ":0d",
    ":-1d",
    ":%s/x//n",
    ":s/a\\/b/x/",
    ":se ts?",
    ":set ts ?",
    ":set ts&",
    ":setlocal ts=8",
    ":bne",
    ":bp",
    ":ju",
    // excerpt of the round13 key table (keeping the regression pressure)
    "<C-v>jI",
    "<C-v>jA",
    "Vjd",
    "viwd",
    "gv",
    "gvd",
    "gv:",
    "mA",
    "`A",
    "q/",
    "qa",
    "q",
    "@a",
    "@:",
    "x",
    ".",
    "p",
    "J",
    "dd",
    "dw",
    "yy",
    "2dd",
    ":5+2+1d",
    ":2,2j",
    ":d _",
    ":3,4s",
    "2*",
    "yaw",
    "d-",
    "dg_",
    "ci\"",
    "ca\"",
    "d'a",
    "ma",
    ":1,2,3d",
    ":%s/x/y/",
    ":%j",
    "R",
    "<BS>",
    "<Left>",
    "<Right>",
    "/x<CR>",
    "?中<CR>",
    "2i",
    "3o",
    ":set ts=8",
    ":set noic",
    ">>",
    "guw",
    "gUU",
    "g~~",
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
];

/// Checks that an offset is addressable on `text` (a char boundary or the end).
fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} unaddressable in {text:?}"
    );
}

fn addressable(off: usize, text: &str) -> bool {
    off <= text.len() && (off == text.len() || text.is_char_boundary(off))
}

/// `check_matches=false`: when hlsearch_live_update is off, `last_matches` is allowed to
/// go stale between two refresh_highlights calls (the generation mechanism guarantees a
/// rescan before n/N consumes them — same constraint as round13).
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
fn fuzz_round14_holds_invariants() {
    let buffers = [
        "",
        "a",
        "中",
        "a2x2x2x\n",
        "a1b2c3d3e\n",
        "say \"hi\" then \"bye\" ok\n\"a\\\\\" tail\n",
        "foo   bar  baz\n   \n\nnext word\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "x3ya3za3\n2xx2yy2z2q\n",
        "a b a b a b\nmatch me and me\nfoo !\n",
        "\n\n\n",
        "   \npara two\n\n\n\ntail\n",
        "éA→ ç\nｱｲｳ\n\tindent",
        "one two three\n...\n",
        "xyz\nwxyz\n",
        "old here\na/b tail\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x14AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            // half the seeds carry `nnoremap j gj`: the cmd_seq guard + expansion replay stay under barrage
            if seed.is_multiple_of(2) {
                f.vim.keymaps.map_str_noremap(
                    vimcore::keymap::ModeClass::Normal,
                    "j",
                    "gj",
                    true,
                );
            }
            let cur = (fuzz_xorshift(&mut state) as usize) % (f.text().len() + 1);
            f.vim.set_cursor_offset(&f.buf, cur);
            // large-buffer host mode: turn off per-edit rescan (same reason as round13,
            // self-recursive macros would exhaust the pipeline budget)
            f.vim.set_hlsearch_live_update(false);
            for step in 0..STEPS {
                if f.buf.len() > 4 * 1024 {
                    f = common::Fixture::new(initial);
                    f.vim.set_hlsearch_live_update(false);
                    if seed.is_multiple_of(2) {
                        f.vim.keymaps.map_str_noremap(
                            vimcore::keymap::ModeClass::Normal,
                            "j",
                            "gj",
                            true,
                        );
                    }
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
                if fuzz_xorshift(&mut state).is_multiple_of(9) {
                    let raw = (fuzz_xorshift(&mut state) as usize) % (f.buf.len() + 2);
                    f.vim.set_cursor_offset(&f.buf, raw);
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

/// Between the random-key barrages, continuously check the core semantics fixed in round14:
/// 1. a till repeat must always advance or fail — no "moved but stayed in place" fake success;
/// 2. after a yank, `g;` must not land on the yank spot (the changelist is not polluted
///    by pure yanks).
#[test]
fn fuzz_round14_semantic_invariants() {
    let mut state: u64 = 0x00D0_0D14;
    let buffers = [
        "a2x2x2x\n",
        "a1b2c3d3e\n",
        "word\n\nnext\n",
        "aaa\nbbb\nccc\nddd\n",
    ];
    for round in 0..300u64 {
        let initial = buffers[(fuzz_xorshift(&mut state) as usize) % buffers.len()];
        let mut f = common::Fixture::new(initial);
        for step in 0..60 {
            // 1) till-repeat semantics: after parking, `;` must advance or fail (bell);
            //    never allow moved=true with an unchanged offset
            let find_char = ['2', '3', 'x', '中']
                [(fuzz_xorshift(&mut state) as usize) % 4];
            f.feed(["t"]);
            f.type_text(&find_char.to_string());
            let before = f.vim.cursor_offset();
            let bells_before = f.host.bells;
            f.feed([";"]);
            let after = f.vim.cursor_offset();
            if after == before {
                assert!(
                    f.host.bells > bells_before,
                    "round {round} step {step}: `;` faked success in place (bells did not increase)"
                );
            }
            // 2) yank stays out of the changelist: record the x edit position; after yiw,
            //    g; must land on the edit, not the yank
            let mut f2 = common::Fixture::new("line1\nline2\nline3\n");
            f2.feed(["j", "x"]); // the real edit is on line2
            f2.feed(["g", "g"]);
            f2.feed(["3", "G"]);
            f2.feed(["y", "i", "w"]); // yank line3
            f2.feed(["g", "g"]);
            f2.feed(["g", ";"]);
            assert_eq!(
                f2.vim.cursor_offset(),
                f2.buf.line_start(1),
                "round {round}: g; must land on the x edit (line2), not the yank spot"
            );
            let _ = &mut f;
        }
    }
}
