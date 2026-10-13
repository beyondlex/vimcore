//! Round-20 fuzz: path bombardment of the command-line face (`:` `/` `?` prompts).
//!
//! Delta relative to round19: in the round16-19 key tables Ex lines were
//! **whole named-key strings** (fixed spellings like `:3j\n`); this round
//! switches to burst-style input ("enter prompt → feed malicious alphabet
//! char by char → finish with Enter/Esc"), specifically targeting the
//! `parse_range`/`ex_substitute`/`parse_reg_count`/history/C-w/C-u parsers
//! that are only reached inside the cmdline. Plus a round-specific invariant:
//! **highlight ranges published to the host must be fully addressable**
//! (start/end on char boundaries and ≤ buffer length) — a search-cache
//! mismatch or offset-adjustment bug makes the host slice panic; this is the
//! engine's rendering contract toward the host.

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 50 rounds × 100 steps = 80k steps (same order as round13-19)
const SEEDS: u64 = 16;
const ROUNDS: usize = 50;
const STEPS: usize = 100;

/// Malicious prompt alphabet: range syntax/separators/register names/regex metachars/multibyte.
// Safety valve (same as round35): the alphabet contains **no `!`** — ex's `:{range}!{filter}`
// branch hands subsequent chars to a real `sh -c` execution (`>` and other metachars would drop
// files in cwd). Deterministic coverage of the `!` form lives in tests/ex_filter.rs.
const EX_ALPHABET: &[char] = &[
    '0', '1', '2', '3', '9', '$', '%', '.', '\'', '+', '-', ',', ';', ' ', '/', '?', '\\',
    '&', '|', '~', '{', '}', '(', ')', '[', ']', '<', '>', '#', '*', '^', '=', '"', 'a', 'b', 'd',
    'e', 'g', 'i', 'j', 'k', 'l', 'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    'J', 'S', 'D', '中', '文', '\u{1f600}',
];

/// Normal-mode scatter keys (keep the pipeline from parking in the same state; no full Ex spellings needed).
const MODE_KEYS: &[&str] = &[
    "w", "b", "e", "0", "$", "j", "k", "h", "l", "gg", "G", "x", "dd", "yy", "p", "P", "u",
    "<C-r>", "v", "V", "o", "O", "i", "a", "A", "escape", "n", "N", "*", "%", ".", ">>", "gqj",
    "daw", "\"ay", "\"ap", "@a", "qa", "q", "f", "a", ";", ",", "gv", "~", "J", "<C-w>",
];

const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\n",
    "中文测试\n多字节行\n",
    "foo bar baz\n\n\nqux\n   \ntail\n",
    "one",
    "",
    "x\n",
    "    indented\ntab\there\n",
    "aaa bbb aaa bbb\nccc\naaa\n",
    "😀 emoji\nline\n",
    "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
];

fn buffer_reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    let text = BUFFERS[(seed as usize) % BUFFERS.len()];
    *f.buf.0.borrow_mut() = text.to_owned();
    f.vim.cursor.offset = 0;
    f.vim.cursor.desired_col = None;
    // When the host swaps buffers it must drop old views — first let the engine republish for the
    // new text, so any out-of-bounds highlight captured afterwards is the engine's own fault
    // (not stale accounting from before the reset)
    let mut ctx = Ctx {
        buf: &mut f.buf,
        host: &mut f.host,
    };
    f.vim.refresh_highlights(&mut ctx);
    true
}

fn invariants_hold(f: &Fixture) -> Result<(), String> {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    if buf.line_count() == 0 {
        return Err("line_count == 0".into());
    }
    let cur = f.vim.cursor.offset;
    if cur > len || !text.is_char_boundary(cur) {
        return Err(format!("cursor {cur} unaddressable (len={len})"));
    }
    for (name, off) in f.vim.marks.items() {
        let o = off.min(len);
        let o = (0..=o).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        if buf.offset_to_line(o) >= buf.line_count() {
            return Err(format!("mark {name} at {o} past last line"));
        }
    }
    // Rendering contract: every highlight range the host receives must be directly sliceable
    for hl in f.host.highlights.iter().chain(f.host.current_highlight.iter()) {
        if hl.end > len || !text.is_char_boundary(hl.start) || !text.is_char_boundary(hl.end) {
            return Err(format!(
                "highlight {hl:?} unaddressable (len={len}) in {text:?}"
            ));
        }
    }
    Ok(())
}

/// One command-line burst: prompt + several malicious chars + a finishing key.
fn cmdline_burst(f: &mut Fixture, rng: &mut u64, prompt: char, trace: &mut Vec<String>) {
    let mut keys: Vec<String> = vec![prompt.to_string()];
    let n = (fuzz_xorshift(rng) % 24) as usize + 1;
    for _ in 0..n {
        // 1/6 chance per step to inject a <...> editing key, otherwise take a char from the alphabet
        if fuzz_xorshift(rng).is_multiple_of(6) {
            keys.push(
                ["<BS>", "<C-w>", "<C-u>", "<up>", "<down>", "<left>", "<right>", "<Esc>", "<Tab>"]
                    [(fuzz_xorshift(rng) % 9) as usize]
                    .to_owned(),
            );
        } else {
            keys.push(EX_ALPHABET[(fuzz_xorshift(rng) as usize) % EX_ALPHABET.len()].to_string());
        }
    }
    // 2/3 chance to execute with Enter, otherwise cancel with Esc (both tail paths' accounting get bombed)
    keys.push(if !fuzz_xorshift(rng).is_multiple_of(3) {
        "<CR>".to_owned()
    } else {
        "<Esc>".to_owned()
    });
    // Feed token by token: single alphabet chars, <...> as whole named keys (matching feed's Key::parse domain)
    trace.extend(keys.iter().cloned());
    f.feed(keys);
}

/// Feed one Ex line character by character (`:` + chars + Enter).
fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

#[test]
fn fuzz_round20_cmdline_face_holds_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        for round in 0..ROUNDS {
            let mut f = Fixture::new(BUFFERS[((seed as usize) + round) % BUFFERS.len()]);
            let mut trace: Vec<String> = Vec::new();
            for _ in 0..STEPS {
                if fuzz_xorshift(&mut rng).is_multiple_of(20) {
                    let r = fuzz_xorshift(&mut rng);
                    if buffer_reset_when_idle(&mut f, r) {
                        trace.push(format!("RESET {}", (r as usize) % BUFFERS.len()));
                    }
                }
                let roll = fuzz_xorshift(&mut rng) % 100;
                if roll < 34 {
                    trace.push("burst :".into());
                    cmdline_burst(&mut f, &mut rng, ':', &mut trace);
                } else if roll < 44 {
                    trace.push("burst /".into());
                    cmdline_burst(&mut f, &mut rng, '/', &mut trace);
                } else if roll < 50 {
                    trace.push("burst ?".into());
                    cmdline_burst(&mut f, &mut rng, '?', &mut trace);
                } else if roll < 60 {
                    let c = ['\n', '\x7f', '\x08', '\t'][(roll % 4) as usize];
                    trace.push(format!("raw {:?}", c));
                    // Direct char feeding (including raw ctrl forms): IME normalization keys
                    f.feed_raw(Key::char(c));
                } else {
                    let k = MODE_KEYS[(roll as usize * 7) % MODE_KEYS.len()];
                    trace.push(format!("feed {k}"));
                    f.feed([k]);
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!("seed={seed} round={round}: {e}\ntrace: {trace:?}");
                }
            }
        }
    }
}

/// Directed: every truncation form of the `:s` trailing count must not slide outside [range end line, EOF].
#[test]
fn fuzz_substitute_count_never_slides_start_upward() {
    for n in [1usize, 2, 3, 7, 8, 9, 10, 11, 99, 1_000_000] {
        for start in 0..10usize {
            let mut f = Fixture::new("a1\na2\na3\na4\na5\na6\na7\na8\na9\na10\n");
            feed_ex(&mut f, &format!("{}s/a/X/{}", start + 1, n));
            let changed = f.text().lines().filter(|l| l.starts_with('X')).count();
            // vim model: from the start line, min(n, remaining) lines get changed, all ≥ start
            let expect = (start..start + n).take_while(|&i| i < 10).count();
            assert_eq!(
                changed, expect,
                "n={n} start={start}: got {:?}",
                f.text()
            );
            if let Some((first_changed, _)) = f.text().lines().enumerate().find(|(_, l)| l.starts_with('X')) {
                assert_eq!(first_changed, start, "n={n} start={start} slid");
            }
        }
    }
}
