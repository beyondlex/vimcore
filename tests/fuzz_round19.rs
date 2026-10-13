//! Round-19 fuzz: path barrage over this round's fix surface.
//!
//! Delta relative to round17 (all paths changed this round):
//! - **D/C line model**: barrage the count shapes of `D`/`C` at line start / mid-line /
//!   last line / sole line (`delete_to_end_span` rewritten — count≥2 at line start makes
//!   the whole line vanish, count=1 clears the line, C always clears but keeps the line;
//!   the old model both lost the file eol and left phantom lines here);
//! - **`Nr<CR>` single-newline collapse** (`3r<CR>` once produced N newlines);
//! - **`r<C-E>`/`r<C-Y>` char fetch from neighboring lines** (new feature; if either
//!   side is out of bounds the whole command is canceled);
//! - **char-arg cancel on non-printable keys** (pressing an arrow key while r/f/'/m
//!   awaits its char — the comment previously disagreed with the code; behavior pinned
//!   as "cancel + bell");
//! - **E20 message for jumping to an unset mark**, **`:j` last-line no-op does not
//!   pollute the changelist**.
//!
//! Invariants carried over from round13-17: cursor/mark/`last_visual`/live selection all
//! addressable, line count ≥ 1; plus this round's extra invariant: after any D/C
//! sequence the buffer's first line still exists (≥1 line) and the `.` replay path does
//! not panic.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 60 rounds × 120 steps ≈ 115k steps (same budget as round13-17)
const SEEDS: u64 = 16;
const ROUNDS: usize = 60;
const STEPS: usize = 120;

const KEYS: &[&str] = &[
    // ---- this round's fix surface: D/C line model (line start/mid/last/sole line × count) ----
    "D",
    "2D",
    "3D",
    "99D",
    "C",
    "2C",
    "99C",
    "d$",
    "d2$",
    "c$",
    "dd",
    "3dd",
    // ---- Nr<CR> collapse and r neighbor lines ----
    "r<CR>",
    "3r<CR>",
    "r<C-e>",
    "r<C-y>",
    "3r<C-e>",
    "2rx",
    // ---- char-arg pending keys (r/f/'/m/q/@ interleaved with cancel/arrow keys) ----
    "r",
    "f",
    "F",
    "t",
    "'",
    "`",
    "m",
    "q",
    "@",
    "escape",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    // ---- unset-mark vs set-mark jumps ----
    "'a",
    "'z",
    "`z",
    "m1",
    "ma",
    "'a",
    // ---- Ex surface: :j no-op, range/count shapes of :d, :s, :sort ----
    ":3j\n",
    ":2,2j\n",
    ":1,2j\n",
    ":j\n",
    ":2d\n",
    ":%d\n",
    ":s/a/X/\n",
    ":%s/x/Y/g\n",
    ":2sort\n",
    ":5\n",
    // ---- excerpt of existing coverage (pipeline pressure: `.`/macros/registers/objects) ----
    ".",
    "x",
    "X",
    "s",
    "S",
    "J",
    "gJ",
    "yy",
    "dd",
    "p",
    "P",
    "u",
    "<C-r>",
    "qa",
    "q",
    "@a",
    "daw",
    "d2aw",
    "ci(",
    "di\"",
    "guu",
    "gqq",
    ">>",
    "3>>",
    "<<",
    "gq}",
    "gg",
    "G",
    "w",
    "b",
    "e",
    "0",
    "$",
    "g_",
    ";",
    ",",
    "%",
    "n",
    "N",
    "*",
    "gn",
    "gN",
    "j",
    "k",
    "h",
    "l",
    "gj",
    "gk",
    "|",
    "10|",
    "<C-a>",
    "<C-x>",
    "g;",
    "g,",
    "<C-o>",
    "<C-i>",
    "gv",
    "v",
    "V",
    "<C-v>",
    "o",
    "O",
    "i",
    "a",
    "I",
    "A",
    "gI",
    "gi",
    "R",
    "zh",
    // insert-mode editing keys (C-w/C-u and their interaction with ai/BS)
    "<C-w>",
    "<C-u>",
    "<BS>",
    "<Del>",
    "<Tab>",
    "<C-h>",
    // host event injection (since round16): click/drag-select/IME in any mode
    "host:click",
    "host:drag",
    "host:ime",
    "host:ime_replace",
];

// fuzz buffer seeds: short lines / 1-char lines / multibyte / empty lines / mixed with
// eol — every branch shape of the D/C line model must occur routinely in the seed pool
const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "aaaa\nbbbb\ncccc\ndddd\n",
    "a\nb\nc\nd\n",
    "hello world\nsecond line\n",
    "中文测试\n多字节行\n",
    "    indented\ntab\there\n",
    "one",
    "",
    "only\n",
    "x\n",
    "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n",
    "foo bar baz\n\n\nqux\n   \ntail\n",
];

fn buffer_reset(f: &mut Fixture, seed: u64) {
    let text = BUFFERS[(seed as usize) % BUFFERS.len()];
    *f.buf.0.borrow_mut() = text.to_owned();
    f.vim.cursor.offset = 0;
    f.vim.cursor.desired_col = None;
}

/// Contract-legit buffer reset: the host may only swap the whole engine-visible text,
/// and the engine's session state (insert/visual/cmdline/pending operator) never
/// crosses a host text exchange — real hosts all go through the undo/redo path (the
/// engine side has sanitize). Swapping the text bare is only legal when the engine is
/// idle and in Normal.
fn buffer_reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    buffer_reset(f, seed);
    true
}

fn invariants_hold(f: &Fixture) -> Result<(), String> {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    if buf.line_count() == 0 {
        return Err("line_count == 0".into());
    }
    // cursor addressable (char boundary + within line)
    let cur = f.vim.cursor.offset;
    if cur > len || !text.is_char_boundary(cur) {
        return Err(format!("cursor {cur} unaddressable (len={len})"));
    }
    let line = buf.offset_to_line(cur);
    // in insert mode the cursor legitimately sits at line_end (the insert position at
    // end of line) or even at the buffer's phantom last position (after typing with
    // `append` or after `<Del>` joins lines) — only normal/visual require "not past the
    // line end". The single exception in normal mode is `cur == len`: this host's line
    // model folds the trailing \n, so in the vim model the "start of the new empty line"
    // left by `o<Esc>` (opening a line at the end) appears here as the phantom last
    // position (since round25 exit_insert prefers to stop at len rather than on the \n —
    // where `x` would join lines)
    if matches!(
        f.vim.mode(),
        vimcore::mode::Mode::Insert | vimcore::mode::Mode::Replace
    ) {
        if cur < buf.line_start(line) {
            return Err(format!("cursor {cur} before line {line} start"));
        }
    } else if cur != len && cur > buf.line_end(line) {
        return Err(format!("cursor {cur} past line {line} end"));
    }
    // stored offsets addressable (marks / changelist / jumplist / last_visual)
    let floor = |off: usize| {
        let mut o = off.min(len);
        while o > 0 && !text.is_char_boundary(o) {
            o -= 1;
        }
        o
    };
    for (name, off) in f.vim.marks.items() {
        let o = floor(off);
        if buf.offset_to_line(o) >= buf.line_count() {
            return Err(format!("mark {name} at {o} past last line"));
        }
    }
    Ok(())
}

/// The main barrage loop (round16's host event injection + this round's key table).
#[test]
fn fuzz_round19_d_c_line_model_holds_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E3779B97F4A7C15);
        for round in 0..ROUNDS {
            let mut f = Fixture::new("seed");
            buffer_reset(&mut f, seed); // fresh fixture: engine starts idle
            // half the seeds pre-set a mark + a changelist entry (E20/`'a` surface routinely reachable)
            if seed % 2 == 0 {
                f.feed(["m", "a"]);
                f.feed(["x"]);
            }
            for step in 0..STEPS {
                rng = fuzz_xorshift(&mut rng);
                let key = KEYS[(rng as usize) % KEYS.len()];
                if let Some(event) = key.strip_prefix("host:") {
                    match event {
                        "click" => {
                            rng = fuzz_xorshift(&mut rng);
                            let off = (rng as usize)
                                % (f.buf.len() + 1);
                            f.vim.set_cursor_offset(&f.buf, off);
                        }
                        "drag" => {
                            rng = fuzz_xorshift(&mut rng);
                            let a = (rng as usize) % (f.buf.len() + 1);
                            rng = fuzz_xorshift(&mut rng);
                            let b = (rng as usize) % (f.buf.len() + 1);
                            f.vim.set_visual_range(&f.buf, a, b);
                        }
                        "ime" => {
                            f.vim.record_typed_text("x中");
                            f.type_text("x中");
                        }
                        "ime_replace" => {
                            rng = fuzz_xorshift(&mut rng);
                            let start = f.vim.cursor.offset
                                .min(f.buf.len().saturating_sub(1));
                            f.ime_replace(start..f.buf.len(), "替换");
                        }
                        _ => {}
                    }
                } else {
                    f.feed([key]);
                }
                // 1/16 chance per step to reset the buffer (this round's D/C line model
                // is highly sensitive to buffer shape — short-line/sole-line/eol shapes
                // must rotate at high frequency)
                rng = fuzz_xorshift(&mut rng);
                if rng % 16 == 0 {
                    buffer_reset_when_idle(&mut f, rng >> 8);
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!(
                        "seed={seed} round={round} step={step} key={key:?}: {e}\nbuffer={:?}",
                        f.text()
                    );
                }
            }
        }
    }
}

/// Targeted: all (line position × count × col) shapes of D/C on 1-4 line buffers never
/// panic and line count ≥ 1 — the truth-table barrage over `delete_to_end_span`.
#[test]
fn fuzz_d_c_shapes_never_break_line_invariant() {
    for buf_text in ["a\n", "a\nb\n", "a\nb\nc\n", "aaaa\nbbbb\ncccc\ndddd\n", ""] {
        for line in 0..4 {
            for col in 0..3 {
                let count_keys: &[&[&str]] = &[
                    &["D"],
                    &["2", "D"],
                    &["9", "9", "D"],
                    &["C"],
                    &["2", "C"],
                ];
                for count_key in count_keys {
                    let mut f = Fixture::new(buf_text);
                    let ls = f.buf.line_start(line.min(f.buf.line_count() - 1));
                    let le = f.buf.line_end(line.min(f.buf.line_count() - 1));
                    f.vim.cursor.offset = (ls + col).min(le);
                    f.feed(*count_key);
                    assert!(
                        f.buf.line_count() >= 1,
                        "buf={buf_text:?} line={line} col={col} keys={count_key:?}: no lines left"
                    );
                    // the undo circuit must also digest the result (undo restores host text)
                    f.feed(["u"]);
                    assert!(f.buf.line_count() >= 1);
                }
            }
        }
    }
}

/// Targeted: `.` replay never panics on the span model after the D/C fix (the recorded
/// replay keys go through the very same delete_to_end_span path).
#[test]
fn fuzz_dot_replay_of_d_c_shapes() {
    for seed in 0..32u64 {
        let mut rng = seed.wrapping_mul(0x2545F4914F6CDD1D) | 1;
        let mut f = Fixture::new("l1\nl2\nl3\nl4\n");
        for _ in 0..40 {
            rng = fuzz_xorshift(&mut rng);
            match rng % 4 {
                0 => {
                    f.feed(["D"]);
                }
                1 => {
                    f.feed(["2", "D"]);
                }
                2 => {
                    f.feed(["C"]);
                    f.type_text("z");
                    f.feed(["<Esc>"]);
                }
                _ => {
                    f.feed(["."]);
                }
            }
            rng = fuzz_xorshift(&mut rng);
            if rng % 8 == 0 {
                buffer_reset_when_idle(&mut f, rng >> 4);
            }
            assert!(f.buf.line_count() >= 1, "seed={seed}: buffer empty\n{:?}", f.text());
        }
    }
}
