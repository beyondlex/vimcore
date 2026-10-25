//! Two macro issues caught by round-36 fuzzing, now regressions (2026-10-11).
//!
//! The fuzz round36 macro session could record a "self-playing" register
//! (`@a` ended up in `"a`), and the engine then multiplied the queue out to
//! ~50 GB before hitting the key-count valve. The root cause is two missing
//! pieces of vim semantics:
//!
//! 1. **`q` is completely inert while a register executes**: vim's `nv_record`
//!    gate is `reg_executing == 0` (`src/normal.c`), so a `q` during playback
//!    neither starts a recording, nor stops one, **nor swallows its char
//!    argument** (the following key runs as a command as usual).
//! 2. **The replay enqueue budget must respect the queue's headroom**:
//!    `enqueue_replay` pre-expanded the full `MAX_PIPELINE_STEPS` on every
//!    call, so under self-recursion each `@` added another 100k keys and
//!    production far outpaced consumption.
//!
//! Oracle channel: `/usr/bin/vim` (9.1 patches 1-1752) + expect PTY,
//! `:redir` sampling (2026-10-11):
//! - `:let @b='qax'` + `@b` → `a='' rec='' l2='defx'`
//! - `:let @c='q'` + `qa@cxq` → `a='@cx' rec=''`
//! - control group `qayxq` → `a='yx' rec=''`
//!
//! The same round also closed an independent Ex-side infinite mutual
//! recursion (`:@a` self-reference → stack overflow, see R36-4; vim's
//! `E169: Command too recursive` gate).

mod common;

use common::Fixture;

// ---- R36-1: `q` during playback neither starts a recording nor eats its arg --
//
// Setup: `"byw` loads the first line `qax` into b (the register content is the
// macro text); the cursor moves to the start of the second line.
// Before the fix: `q` armed the char-arg and ate `a` → started a recording
// into a, and `x` fell back to the normal side deleting the char under the
// cursor (`def` → `ef`) while `macro_recording()` became Some('a').
// After the fix: `q` is inert, `a` runs as a plain append into insert mode,
// and `x` there is "text delivered by the host" — this harness only feeds
// keys and delivers no text, so the buffer keeps `def`.
#[test]
fn r36_1_q_does_not_open_a_recording_while_a_register_executes() {
    let mut f = Fixture::new("qax\ndef\n");
    f.feed(["\"", "b", "y", "w"]);
    f.feed(["j"]);
    f.feed(["@", "b"]);
    assert_eq!(
        f.vim.macro_recording(),
        None,
        "oracle P1: reg_recording() is empty — q during playback must not start a recording"
    );
    assert_eq!(
        f.text(),
        "qax\ndef\n",
        "oracle P1: q does not eat `a`, and `x` is not a normal-side delete"
    );
}

// ---- R36-2: `q` during playback does not stop a live recording ---------------
//
// `:let @c='q'` + `qa@cxq` → `"a` is `@cx` (three steps): the `q` played from
// `@c` did not end the session, so the following `x` keeps being recorded; the
// real stop is the trailing hand-typed `q`.
// Before the fix: the played `q` stopped early → `"a` held only `@c` (two
// steps), and the trailing `q` instead armed a new recording.
#[test]
fn r36_2_q_does_not_stop_a_live_recording_while_a_register_executes() {
    let mut f = Fixture::new("qfoo\n");
    f.feed(["\"", "c", "y", "l"]);
    assert_eq!(f.vim.macro_len('c'), 0, "not yet synced into the macro table (lands on @c)");
    f.feed(["q", "a"]);
    assert_eq!(f.vim.macro_recording(), Some('a'), "qa starts the recording");
    f.feed(["@", "c"]);
    assert_eq!(
        f.vim.macro_recording(),
        Some('a'),
        "oracle P2: the played q does not stop the session"
    );
    f.feed(["x"]);
    f.feed(["q"]);
    assert_eq!(f.vim.macro_recording(), None, "only the trailing hand-typed q stops it");
    assert_eq!(
        f.vim.macro_len('a'),
        3,
        "oracle P2: \"a is the three steps @cx (@, c, x all recorded as literal keys)"
    );
}

// ---- R36-3: a self-playing register no longer blows the queue up astronomically --
//
// `"a` contains exactly `@a` → every played `@` re-enqueues. The engine's
// backstop is the `MAX_PIPELINE_STEPS` safety valve, but the budget must be
// computed from the **queue's remaining headroom**: otherwise each consumed
// `@` pushes another 100k keys and 100k iterations mean ~10^10 `Key`s
// (fuzz round36's 50 GB). The expected failure shape of this assertion is a
// hang/OOM, so it only verifies that it converges.
#[test]
fn r36_3_self_replaying_macro_terminates_with_a_bounded_queue() {
    let mut f = Fixture::new("@afoo\n");
    f.feed(["\"", "a", "y", "2", "l"]);
    f.feed(["@", "a"]);
    f.feed(["<Esc>"]);
    assert!(
        f.vim.is_idle(),
        "a self-recursive macro must be closed by the safety valve, leaving no pending queue"
    );
    assert_eq!(f.text(), "@afoo\n", "playback must not change the buffer text");
}

// ---- R36-4: a self-referencing `:@` must hit vim's E169, not a stack overflow --
//
// The register content is exactly `@b` → unbounded `execute_ex ↔ ex_at` mutual
// recursion (fuzz round17's `has overflowed its stack`). Vim's backstop is in
// `do_cmdline()`: `call_depth >= 200` → `emsg("E169: Command too recursive")`
// and `return FAIL` (`src/ex_docmd.c:707-737`).
//
// oracle (expect PTY, 2026-10-11): `:let @b='@b'` + `:@b` →
// screen shows `E169: Command too recursive`, `v:errmsg` the same text, buffer
// unmodified; the engine side keeps working afterwards. (`'@b'` without the
// colon is the actual fuzz round17 shape: the engine's `:@` does not strip a
// leading colon from the content, so `:let @a=':@a'` gives E492 here — a
// divergence from vim's `do_one_cmd()` "Skip ':' and any white space",
// booked in NOTES; it does not affect this convergence.)
#[test]
fn r36_4_recursive_at_command_stops_at_e169() {
    let mut f = Fixture::new("foo\nbar\n");
    f.vim
        .registers.lock().unwrap()
        .store_named_plain('b', "@b".to_owned());
    f.feed([":", "@", "b", "<CR>"]);
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E169: Command too recursive"),
        "oracle: recursion to 200 levels reports E169, got {:?}",
        f.host.statuses
    );
    assert_eq!(f.text(), "foo\nbar\n", "oracle: recursion must not change the buffer");
    assert!(f.vim.is_idle(), "the recursion must converge completely");
    // the stack must unwind all the way (ex_depth reset); the engine cannot stay stuck in the error state
    f.feed(["x"]);
    assert_eq!(f.text(), "oo\nbar\n", "normal mode works as usual after E169");
}

// ---- R36-5: legitimate shallow nesting is unaffected by the gate -------------
//
// 200 is a **cap**, not a ban: `:source` and function calls can nest, and vim
// behaves the same. `:@a` → `:@b` → `:2`/`:3` (depth 3) must run through to
// the last command.
// The contents carry no leading colon — the engine's `:@` does not strip one
// (see the NOTES divergence in the R36-4 remark).
#[test]
fn r36_5_shallow_at_nesting_still_runs() {
    let mut f = Fixture::new("l1\nl2\nl3\n");
    f.vim.registers.lock().unwrap().store_named_plain('b', "2".to_owned());
    f.vim
        .registers.lock().unwrap()
        .store_named_plain('a', "@b\n3".to_owned());
    f.feed([":", "@", "a", "<CR>"]);
    assert!(
        !f
            .host
            .statuses
            .iter()
            .any(|s| s.contains("E169: Command too recursive")),
        "shallow nesting must not trip the recursion gate: {:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.starts_with("E492")),
        "both nested sub-commands must be recognized: {:?}",
        f.host.statuses
    );
    // the two sub-commands run in order; the cursor lands on the line of the last command `:3`
    assert_eq!(f.line(), 2, "the last :3 took effect (0-based line 2)");
}
