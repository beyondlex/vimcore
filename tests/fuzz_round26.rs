//! Fuzz round 26 (added during the round-27 review window): two invariants of the host history.
//!
//! The engine delegates undo/redo to the host (`begin_undo_group`/`undo`/`redo`) and only
//! announces groups itself at each editing funnel. The previous 25 fuzz rounds checked only "the
//! legality of the state after a single edit step" and never the **history axis itself**. This
//! round adds the two invariants that can only be caught by running a random key sequence all
//! the way down with `u`:
//!
//!   1. **Reversibility**: undoing to the bottom and then redoing to the top must return the
//!      text byte-for-byte to the state before the undo — catches "the host redo stack is
//!      asymmetric with the undo stack" class defects;
//!   2. **Completeness**: after undoing to the bottom the text must equal the **initial
//!      buffer** — catches "some editing path forgot to go through the `edit_*` funnel and never
//!      entered any undo group" class defects (an edit whose group was never opened cannot be
//!      undone and is silently lost between two `u` presses).
//!
//! The key table mixes in host events (IME composition replacement, clicks); these paths
//! announce groups too and are the seams where "group ownership" is most easily wrong (the
//! owns_group rule of replace_range). The corpus contains multi-byte text; no mid-run buffer
//! switch is added — swapping the host buffer necessarily resets the host's own history stack,
//! which is the host's consistency responsibility (the reset semantics of round20) and not part
//! of this invariant.

mod common;

use common::Fixture;
use vimcore::mode::Mode;

/// Simple LCG: deterministic randomness (no external dependencies).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Corpus: ASCII / multi-byte / empty lines / last line without a newline (the history
/// invariants are content-independent; the corpus only diversifies the editing paths).
const BUFFERS: &[&str] = &[
    "hello world\nsecond line\nthird\n",
    "aaa bbb aaa\n\nccc\n",
    "中文行\nmultiline\ntail",
    "a\n",
    "one",
    "x y z\n1 2 3\nfoo bar\n",
];

/// Single-key pool (the granularity of feed is one Key): motions/editing/char args/visual/Ex/
/// macros/history. `u`/`<C-r>` are in the pool too — the random walk along the history axis
/// leaves the undo stack at varying depths.
const KEY_POOL: &[&str] = &[
    "x", "d", "d", "y", "y", "p", "P", "i", "a", "o", "O", "<Esc>", "u", "<C-r>", "c", "w", "S",
    "J", "~", ">", ">", "<", "<", "D", "C", "s", "r", "x", "<C-a>", "<C-x>", "3", "x", "v", "d",
    "y", "<C-v>", "I", "A", "d", ":", "e", "s", "c", "\n", "&", ".", "q", "a", "q", "@", "a",
    "g", "J", "g", "q", "q", "f", "x", "t", "y", "m", "a", "'", "a", "`", "a", "G", "g", "g",
    "n", "N", "*", "#", "1", "2", "0", "j", "k", "h", "l", "w", "b", "e", "0", "$", "^", "z",
    "z", "<BS>", "<C-w>", "<Del>", "<C-o>", "x", "g", "u", "u", "g", "U", "U",
];

/// Return to the normal idle state: Esc dismisses the prompt/visual/insert and a pending char
/// arg. `is_idle` covers count/register/operator/char-arg/queue; the mode must also be back in
/// normal to count as arrived.
fn force_normal(f: &mut Fixture) {
    for _ in 0..6 {
        if f.vim.is_idle() && f.vim.mode() == Mode::Normal {
            return;
        }
        f.feed(["<Esc>"]);
    }
    panic!(
        "force_normal: cannot get back to the normal idle state (mode={:?})",
        f.vim.mode()
    );
}

/// Undo to the bottom (using the engine's history-edge bell as the stop signal), returning the
/// number of steps.
fn undo_to_bottom(f: &mut Fixture, cap: usize) -> usize {
    let mut depth = 0;
    loop {
        let before = f.host.bells;
        f.feed(["u"]);
        if f.host.bells > before {
            return depth;
        }
        depth += 1;
        assert!(depth <= cap, "the u loop did not reach the history bottom in {cap} steps (mode={:?})", f.vim.mode());
    }
}

/// Body of the history invariants: first take a random walk, then undo to the bottom and assert
/// the initial text, then redo to the top and assert the text the walk ended with.
fn sweep(f: &mut Fixture, initial: &str, context: &str) {
    force_normal(f);
    let text_now = f.text();

    let depth = undo_to_bottom(f, 800);
    assert_eq!(
        f.text(),
        initial,
        "{context}: text after undo-to-bottom ≠ the initial buffer — some edit never entered any undo group (depth={depth})"
    );

    for _ in 0..depth {
        f.feed(["<C-r>"]);
    }
    force_normal(f);
    assert_eq!(
        f.text(),
        text_now,
        "{context}: undo-all/redo-all did not return to the text before the undo (depth={depth})"
    );
}

/// Random walk over key sequences + host events (IME replacement / clicks).
fn random_walk(f: &mut Fixture, rng: &mut Rng, steps: usize) {
    for _ in 0..steps {
        match rng.below(12) {
            // IME composition replacement: swap a stretch of text near the cursor (an editing
            // channel parallel to the key stream)
            0 => {
                let len = f.text().len();
                let a = rng.below(len + 1);
                let b = (a + rng.below(6)).min(len);
                let text = f.text();
                if text.is_char_boundary(a) && text.is_char_boundary(b) {
                    f.ime_replace(a..b, "中文x");
                }
            }
            // Host click (does not change the text, only moves the cursor)
            1 => {
                let len = f.text().len();
                f.vim.set_cursor_offset(&f.buf, rng.below(len + 1));
            }
            _ => {
                let k = KEY_POOL[rng.below(KEY_POOL.len())];
                f.feed([k]);
            }
        }
    }
}

// 24 seeds × 40 rounds × (60..200 steps) ≈ 130k keys (seconds under opt3)
const SEEDS: usize = 24;
const ROUNDS: usize = 40;

#[test]
fn fuzz_history_undo_all_reaches_initial_and_redo_all_restores() {
    for seed in 0..SEEDS {
        let mut rng = Rng(0x27_00_00_01 + seed as u64);
        for round in 0..ROUNDS {
            let initial = BUFFERS[rng.below(BUFFERS.len())];
            let mut f = Fixture::new(initial);
            let steps = 60 + rng.below(140);
            random_walk(&mut f, &mut rng, steps);
            sweep(
                &mut f,
                initial,
                &format!("seed {seed} round {round}"),
            );
        }
    }
}

/// Directed-corpus variant: IME interleaving on a multi-byte buffer (the floor/group-ownership
/// seams).
#[test]
fn fuzz_history_multibyte_with_ime_interleave() {
    for seed in 0..8 {
        let mut rng = Rng(0x27_00_00_02 + seed as u64);
        for round in 0..20 {
            let initial = "中文行\nmultiline emoji😀\n尾行无换行";
            let mut f = Fixture::new(initial);
            for _ in 0..(40 + rng.below(80)) {
                if rng.below(4) == 0 {
                    let len = f.text().len();
                    let a = rng.below(len + 1);
                    let b = (a + rng.below(6)).min(len);
                    let text = f.text();
                    if text.is_char_boundary(a) && text.is_char_boundary(b) {
                        f.ime_replace(a..b, "组");
                    }
                } else {
                    let k = KEY_POOL[rng.below(KEY_POOL.len())];
                    f.feed([k]);
                }
            }
            sweep(
                &mut f,
                initial,
                &format!("multibyte seed {seed} round {round}"),
            );
        }
    }
}
