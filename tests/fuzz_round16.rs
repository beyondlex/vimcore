//! Round-16 fuzz: key-stream barrage over the paths touched by this round's fixes.
//!
//! Increments over round15:
//! - the autoindent empty-line lifecycle (did_ai): opening a line with o/O, splitting a line
//!   with `<CR>`, `<BS>` at the line start, cc/S restoring the indent, Esc exiting — the
//!   interaction of stripping order and count repetition;
//! - empty-text counted repeats (`3o`/`2O`/`3i`/`3A`/`3R` + Esc) and abstaining motions;
//! - the bell path of the failed-delete family (x/X/D/J/gJ/s) (a silent regression here would
//!   first break behavior outside the invariants, so the key table guarantees the path is hit);
//! - **host event injection**: random `set_visual_range` / `set_cursor_offset` /
//!   `type_text` / `replace_range` delivered in any mode (including insert/cmdline) —
//!   in the old implementation a drag-select during insert/cmdline tore the mode machine apart
//!   (round16 #4).
//!
//! The invariants carry over from round13-15: cursor/mark/`last_visual`/live selection are all
//! addressable, line count >= 1, and mode and indicator agree.

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

// 16 seeds × 60 rounds × 120 steps ≈ 115k steps (same budget as round13-15)
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // This round's fix surface: the autoindent empty-line lifecycle
    "o", "O", "<CR>", "<BS>", "<Esc>", "cc", "S", "cc<Esc>", "o<CR>", "o<BS>",
    // This round's fix surface: empty-text counted repeats + counted repeats with text
    "3o", "2O", "5o", "3i", "3A", "3R", "2Rab", "3o<Esc>", "2O<Esc>",
    // This round's fix surface: the failed delete/join family
    "x", "X", "D", "C", "s", "J", "2J", "3J", "gJ", "99J",
    // This round's fix surface: the :sort decorated-sort path (the semantic invariants are
    // pinned by parity)
    ":sort u<CR>", ":%sort i<CR>", ":2,4sort! <CR>",
    // Excerpt of the round15 key table (keeping the existing pressure surface)
    ":s/a/b\\/c/<CR>", "&", ":1+99999999999999999999d<CR>", "}", "{", "d}",
    "d{", "2}", "ci\"", "ca\"", "1%", "2%", "50%", "%", "d2%", "\"_yy",
    "\"\"p", "3Rab", "\"1P", ".", "2.", "dd", "dw", "<C-h>", "i<C-h>",
    "R<C-h>", "V'a", "v`a", "'a", "`a", "mA", "t2", "2t3", ";", "dtx",
    "\"ax", "\"ayy", "<C-v>jlo", "<C-v>jlO", "VjO", "3:", "3:<CR>",
    "2/x<Esc>", "3/foo<CR>", ":1,2d 3x", ":y a3", ":%s/x//n", ":s/a\\/b/x/",
    ":setlocal ts=8", ":bne", "gv", "gvd", "@a", "@:", "p", "yy", ":5+2+1d",
    ":3,4s", "2*", "yaw", "d-", "d'a", "ma", ":%s/x/y/", ":%j", "<Left>",
    "<Right>", "/x<CR>", "?中<CR>", ":set ts=8", ":set noic", ":set scs",
    ":setlocal ts?", ":brew", ">>", "guw", "<C-r>", "yiw", ":'<,'>d", "h",
    "j", "k", "l", "w", "b", "e", "0", "$", "^", "G", "d", "c", "y", "a",
    "i", "v", "V", "<C-v>", "u", "gJ", "~", "f", "n", "N", "*", "iw", "aw",
    "cw", "gg", "<C-a>", "<C-x>", "<C-o>", "<C-i>", ":marks", ":reg",
    "<Del>", "<C-c>", "<Tab>",
];

fn assert_addressable(what: &str, off: usize, text: &str, ctx: &str) {
    assert!(
        off <= text.len() && (off == text.len() || text.is_char_boundary(off)),
        "{ctx}: {what} offset {off} is not addressable in {text:?}"
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
    // The mode indicator is self-consistent (the visible surface of the host status bar)
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
    // last_matches is not checked: the harness calls set_hlsearch_live_update(false) throughout
}

#[test]
fn fuzz_round16_holds_invariants() {
    // The buffer pool revolves around this round's fix surface: indented lines (the ai
    // stripping surface), empty lines (the failed-delete family), whitespace-only lines,
    // multi-byte text, growable multi-line buffers
    let buffers = [
        "",
        "a",
        "中",
        "    ind\n",
        "    ind\nzz\n",
        "a\n\nb\n",
        "a\n   \nb\n",
        "\n\n\n",
        "one\ntwo\nthree\ntail\n",
        "abc\n",
        "aaaa\nbbbb\ncccc\ndddd",
        "éA→ ç\nｱｲｳ\n\tindent",
        "x\n",
        "    foo\n    bar\n",
        "L0\nL1\nL2\nL3\nL4\nL5\n",
        "say \"hi\" then \"bye\"\n",
    ];
    for seed in 0..SEEDS {
        let mut state: u64 = 0x16AE_5EED ^ seed.wrapping_mul(0x2722_0A95);
        for round in 0..ROUNDS {
            let initial = buffers[round % buffers.len()];
            let mut f = common::Fixture::new(initial);
            f.vim.set_hlsearch_live_update(false);
            // Half of the seeds carry `nnoremap j gj` (the round14 precedent)
            if seed % 2 == 0 {
                f.vim.keymaps.map_str_noremap(
                    vimcore::keymap::ModeClass::Normal,
                    "j",
                    "gj",
                    true,
                );
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
                // Host event injection (~1/8 of the steps): drag-select/click delivered in any
                // mode — the old implementation tore the mode machine apart when this happened
                // during insert/cmdline; IME text and the IME replacement path are delivered at
                // arbitrary points
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

/// Huge-count empty repeat: the repeat count of `<huge value>o<Esc>` must be clamped by the
/// byte budget and terminate (fix round16-1: `"\n".repeat` goes through clamped_repeat_count).
#[test]
fn fuzz_round16_huge_empty_repeat_terminates() {
    let start = std::time::Instant::now();
    for count in [999_999_999usize, 1_000_000_000, 12345] {
        let mut f = common::Fixture::new("x\n");
        for c in count.to_string().chars() {
            f.feed([c.to_string().as_str()]);
        }
        f.feed(["o", "<Esc>"]);
        // 16MB budget → at most 16 million newlines, the buffer cannot blow up
        assert!(f.buf.len() < 20 * 1024 * 1024);
    }
    assert!(start.elapsed().as_secs() < 30, "must terminate quickly");
}
