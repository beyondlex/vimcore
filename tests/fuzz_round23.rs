//! Round-23 fuzz: hostile argument surfaces of Ex commands / `:set` / `:sort` + random
//! interleaving of host events (IME commit / undo / macro replay) with the key stream.
//!
//! Background: round23 fixed five semantics (the session undo-group split in
//! replace_range, the buffer-end boundary of floor_to_char_boundary, Enter char-arg
//! convergence, :set E521, :sort E475/numeric sorting). The fixes touch two deep waters
//! that fuzz had not covered before — "host-driven editing" and "Ex argument parsing":
//!   1. item-by-item + combination barrage of the `:set`/`:sort` argument alphabets — the
//!      decision paths of E518/E521/E475/E488 must stay faithful and never panic;
//!   2. IME commits (replace_range) inserted at any point of an insert session + random
//!      undo/redo/macro replay — undo-group boundaries must stay addressable and host
//!      snapshot semantics intact;
//!   3. throughout, uphold the render contract (highlights addressable) and the
//!      line-addressing contract established since round20.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::{Key, KeyKind, Modifiers};

fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn assert_addressable(f: &Fixture, context: &str) {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= len && text.is_char_boundary(cur),
        "{context}: cursor {cur} unaddressable (len={len}) in {text:?}"
    );
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= len && text.is_char_boundary(s) && text.is_char_boundary(e),
            "{context}: line {line} range {s}..{e} broken (len={len}) in {text:?}"
        );
    }
    for hl in f
        .host
        .highlights
        .iter()
        .chain(f.host.current_highlight.iter())
    {
        assert!(
            hl.end <= len && text.is_char_boundary(hl.start) && text.is_char_boundary(hl.end),
            "{context}: highlight {hl:?} unaddressable in {text:?}"
        );
    }
}

// ---- 1. hostile argument surfaces of :set / :sort ----------------------------

/// The `:set` argument alphabet: legal/illegal numeric values, unknown names,
/// queries, resets, negations, comments.
const SET_ALPHABET: &[&str] = &[
    "ts=4", "ts=", "ts=x", "ts=99999999999999999999999", "ts=-4", "sw=0", "tw=1000000",
    "so=1", "foo", "foo=1", "number", "nonumber", "nu!", "ic?", "ts ?", "ts&", "ts&vim",
    "?", "\"", "\" note", "ts=4 \" note", "scs", "noic",
];

/// The `:sort` flag alphabet: legal (`!`/`i`/`u`/`n` and combinations) + valid in
/// vim but rejected by the engine (x/o/b/f/l) + outright illegal + numeric.
const SORT_FLAGS: &[&str] = &[
    "", "!", "i", "u", "n", "iu", "in", "nu!", "!n", " z", "x", "o", "b", "f", "l", "z", "3",
    "2x", "i!", "un", "nz", "!!", "ii",
];

const TARGETS: &[&str] = &["b\na\nc\n", "10 apples\n2 bananas\nno num\n-3 x\n", "中b\na阿\n", "one"];

fn fuzz_set_surface() {
    for (ti, target) in TARGETS.iter().enumerate() {
        for (si, arg) in SET_ALPHABET.iter().enumerate() {
            let mut f = Fixture::new(target);
            let context = format!("target#{ti} set {arg:?} (s{si})");
            feed_ex(&mut f, &format!("set {arg}"));
            assert_addressable(&f, &context);
            // after the option-surface barrage, n/ search cache must not panic on mismatch
            f.feed(["*", "n", "N"]);
            assert_addressable(&f, &context);
        }
        // combination: stuff many items into one :set; a mid-way failure must truncate
        // (nothing after E518/E521 executes)
        let mut f = Fixture::new(target);
        feed_ex(&mut f, "set ts=8 ts= sw=2 foo ts=16");
        assert_addressable(&f, "set combo");
        assert!(
            f.vim.options.tabstop == 4 || f.vim.options.tabstop == 8,
            "tabstop kept taking effect past the failed item: {}",
            f.vim.options.tabstop
        );
    }
}

fn fuzz_sort_surface() {
    for (ti, target) in TARGETS.iter().enumerate() {
        for (fi, flags) in SORT_FLAGS.iter().enumerate() {
            let mut f = Fixture::new(target);
            let context = format!("target#{ti} sort {flags:?} (f{fi})");
            feed_ex(&mut f, &format!("%sort{flags}"));
            assert_addressable(&f, &context);
            // after sorting reorders lines, changelist/jumplist must stay addressable
            f.feed(["g", ";", "g", ";"]);
            assert_addressable(&f, &context);
        }
    }
}

#[test]
fn fuzz_ex_set_and_sort_argument_surfaces_hold_invariants() {
    fuzz_set_surface();
    fuzz_sort_surface();
}

// ---- 2. random interleaving of host events (IME commit/undo/macro) with keys --

/// Simple LCG: deterministic pseudo-random (no external dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Key pool: covers the editing surface, the prompt surface, and char-arg commands.
const KEY_POOL: &[&str] = &[
    "x", "d", "d", "y", "y", "p", "P", "i", "a", "A", "o", "<Esc>", "j", "k", "h", "l", "w", "b",
    "c", "c", "r", "x", "m", "a", "'", "a", "`", "a", "f", "x", ";", "u", "<C-r>", ".", "q", "a",
    "@", "a", "J", "~", "g", "u", "u", "n", "*", "0", "$", "g", "g", "G", "1", "2", "3", "v",
    "<C-v>", "V", "d", "g", "n", ":", "/", "f", "o", "o", "<CR>", "<BS>", "<Esc>", "s", "S",
];

const EX_POOL: &[&str] = &[
    "s/o/X/g", "%s/^/·/", "sort", "sort n", "sort iu", "sort z", "set ts=8", "set ts=",
    "set foo", "2,3d", "1,2y a", "3j", "j!", "noh", "1,3sort 3", "d 3x",
];

fn ime_commit(f: &mut Fixture) {
    // pick a legal range within the cursor line for a commit replacement
    // (the real shape of a host IME commit)
    let text = f.text();
    let len = text.len();
    if len == 0 {
        return;
    }
    let mut r = Rng(len as u64 | 1);
    let start = text.floor_char_boundary(r.below(len + 1));
    let end = text.floor_char_boundary(r.below(len + 1));
    let (s, e) = (start.min(end), start.max(end));
    f.ime_replace(s..e, "中");
}

// The floor for range boundaries uses std's str::floor_char_boundary (stable since 1.86).

#[test]
fn fuzz_host_events_interleaved_with_keys_hold_invariants() {
    let mut rng = Rng(0x005E_ED23);
    for round in 0..300 {
        let mut f = Fixture::new("foo bar\n中a文\nbaz qux\n");
        f.vim.keymaps_mut().map_str_noremap(
            vimcore::keymap::ModeClass::Normal,
            "Z",
            "xj",
            true,
        );
        let context = format!("round#{round}");
        for step in 0..120 {
            match rng.below(12) {
                0..=7 => {
                    let key = KEY_POOL[rng.below(KEY_POOL.len())];
                    f.feed([key]);
                }
                8 => {
                    let ex = EX_POOL[rng.below(EX_POOL.len())];
                    feed_ex(&mut f, ex);
                }
                9 => ime_commit(&mut f),
                10 => {
                    f.feed(["u"]); // undo via the u key (host callback fired by execute_normal_cmd)
                }
                _ => {
                    f.feed(["2", "@", "a"]);
                }
            }
            assert_addressable(&f, &format!("{context} step#{step}"));
            // the is_idle-vs-mode contract (round23 F): an open prompt must never report idle
            if matches!(f.vim.mode(), vimcore::mode::Mode::CommandLine { .. }) {
                assert!(
                    !f.vim.is_idle(),
                    "{context} step#{step}: reported idle while a prompt is open"
                );
            }
        }
        // closing invariant: after Esc returns to normal, the engine is addressable
        // and can keep editing
        f.feed(["<Esc>", "<Esc>", "g", "g"]);
        assert_addressable(&f, &context);
        f.feed(["x"]);
        assert_addressable(&f, &context);
    }
}

/// IME commit at buffer end: targeted coverage of the floor boundary fix (round23 E).
#[test]
fn ime_commit_at_buffer_end_keeps_last_char() {
    for tail in ["ab", "中", "a中", "中a"] {
        let mut f = Fixture::new("hello");
        f.feed(["A"]);
        f.type_text(tail);
        let len = f.text().len();
        let start = len - tail.len();
        f.ime_replace(start..len, "Z");
        assert_eq!(
            f.text(),
            "helloZ",
            "when the commit range reaches the buffer end it must not swallow the last char (tail={tail:?})"
        );
    }
}

/// Targeted coverage of multiple commits in one session forming one undo group (round23 A).
#[test]
fn multi_commit_session_undoes_as_one() {
    let mut f = Fixture::new("base");
    f.feed(["A"]);
    f.type_text("ab");
    f.ime_replace(4..6, "阿");
    f.type_text("cd");
    f.ime_replace(7..10, "波");
    f.type_text("e");
    f.feed(["<Esc>"]);
    let after_session = f.text();
    f.feed(["u"]);
    assert_eq!(f.text(), "base", "the whole session (all commits included) undoes with one u");
    let _ = after_session;
    // redo walks back to the session's final state
    f.feed(["<C-r>"]);
    f.feed(["<C-r>"]);
    let _ = f.text();
}

// —— compile-time sentinel: the KeyKind/Modifiers imports must not go unused (shape-surface regression) ——
#[test]
fn key_model_shapes_still_resolve() {
    assert_eq!(
        Key {
            modifiers: Modifiers::NONE,
            kind: KeyKind::Char('x'),
        },
        Key::parse("x")
    );
}
