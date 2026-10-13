//! audit3_d: independent audit of registers / macros / `.` replay / undo-redo / insert-Replace mode
//! (2026-10-08, round 3).
//!
//! One `#[test]` per finding; the assertions hold the **expected results of the vim 9.1 oracle**, so
//! they should fail on the current working tree. Oracle evidence is recorded in each test's comment:
//! - typeahead channel: `cd /tmp/ora && printf '<initial buffer>' > N.txt && python3 -c
//!   "open('k','wb').write(b'<key bytes>:wq\n')" && vim -Nu NONE -N -i NONE -n -s k N.txt
//!   </dev/null`, read N.txt back / sample with writefile;
//! - undo/interaction channel (typeahead does not wait for keys; undo-block merging is a channel
//!   artifact): a real PTY driven by expect delivering keys one by one at 0.35s (/tmp/ora/pty3.exp).
//!
//! Scope: src/registers.rs, the macro/`.`/undo sections of src/state.rs, src/insert_mode.rs, the
//! register funnel of src/ops.rs, the `@:`/`:undo` surface of src/cmdline.rs.
//!
//! Suspicions already falsified (oracle vs engine agree; skip next round):
//! - repeated small deletes going into "1 (the classic 1P-dot idiom): falsified by oracle B — after
//!   `x.` @1='' and @-='b'; engine agrees (re-check 15);
//! - the decimal/octal convention of `i<C-v>`: oracle E1-E3 — vim 9.1 `C-v 65`/`C-v 065`/`C-v x41`
//!   all yield 'A' (decimal + explicit o/x prefixes); engine agrees;
//! - <BS> at line start joining lines (the bs=2 boundary): oracle F/F2 — `vim -Nu NONE` allows BS
//!   across the line start by default (the join happens); engine agrees; C-w/C-u at line start with
//!   nothing typed being no-ops is also aligned;
//! - undo cursor (ordinary commands): oracle M1-M3 — the row/column of `Axy<Esc>u`/`ofoo<Esc>u`/mid-line
//!   `x u` agrees with the engine's "first edit position" model;
//! - undo granularity (whether `.` is an independent undo step): the typeahead channel merges undo
//!   blocks (channel artifact); PTY per-key evidence shows `x.` then `u` undoes only the dot-replay
//!   step; the engine groups one command per group — agrees;
//! - pressing `u` in operator-pending: oracle W1/W2 — vim swallows the u directly (no undo executes)
//!   and cancels the operator; the engine's Pending-trie Miss does the same swallow+cancel — agrees;
//! - `.` replay carrying the register prefix / yank not clearing redo / small deletes only into `"-` /
//!   `.` replay of block-visual operators / C-u inside counted inserts (net text empty, the repeat
//!   pastes nothing stale) / Replace-mode C-r overwriting by typing (oracle Z21: "Xac"): all agree;
//! - `J`/`r` not writing registers (oracle AA1/AA3): the engine's funnel never reaches store_delete — agrees;
//! - Ex `:d` shifting the numbered ring (oracle L2: @1='l2'): the engine uses the same funnel — agrees;
//! - `3ifoo<C-w>x<Esc>` (C-w cancels the repeat inside a counted insert, oracle W5="xzz") and
//!   `3Rab<C-w>x<Esc>` (oracle W6="xzzz"): the engine also abandons the repeat — agrees;
//! - macros recorded across modes (type_text's Text step feeding macro_capture), `qA` append, `@@`
//!   chaining, `:reg` including the "-/"/ special slots: code reading + existing probes agree.
//! Also noted, minor divergences (not filed): the line format of `:reg` (vim uses "two spaces + type
//! + two spaces + name + three spaces + content", the engine has its own format with the name first;
//! see oracle RG for the PTY sampling expression).

mod common;

use common::Fixture;
use std::matches;
use vimcore::key::Key;
use vimcore::mode::Mode;

// ------------------------------------------------ `.` replay × count (oracle A / PTY V10)

/// **Finding 1 (P1)**: `.`'s [count] should **replace** the count recorded in the replayed change
/// (`:h .` "Repeat last change, with count replaced with [count]"), but the engine replays the old
/// count keys from `last_change` verbatim and then multiplies by the new count.
/// oracle (PTY per-key + typeahead, both channels): on `abcdef`, `3x` yields `def`, then `2.` gives
/// `f` in vim (replay x with count=2); the engine runs the [3,x] replay for two rounds → from `def`
/// it deletes 2 then 1 more → empty line.
#[test]
fn dot_count_replaces_recorded_count_not_multiplies() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["3", "x"]);
    assert_eq!(f.text(), "def\n");
    f.feed(["2", "."]);
    assert_eq!(f.text(), "f\n", "vim 9.1: 2. replays x with count taken as 2, not 3x×2");
}

/// **Finding 1b (P2)**: the counted-insert variant of the same root cause. After `3iab<Esc>`, `2.`:
/// vim replays the insert session with the count replaced by 2 (net effect "ab"×5, 12 bytes); the
/// engine replays [3,i,Text,Esc] for two rounds and each round's exit_insert copies again with
/// count=3 → "ab"×9.
/// oracle (typeahead): on `zz`, `3iab<Esc>2.` → getline(1) length 12 ("ababaababbzz", 5 copies of
/// "ab"); the engine yields 20 bytes (9 copies of "ab").
#[test]
fn dot_after_counted_insert_replays_session_with_new_count() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("ab");
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "abababzz\n");
    f.feed(["2", "."]);
    let text = f.text();
    let line = text.trim_end_matches('\n');
    assert_eq!(
        line.chars().count(),
        12,
        "vim 9.1: after 2. there are 5 copies of \"ab\" (12 bytes); the engine runs the 3i for two more rounds getting 9 copies"
    );
}

// ------------------------------------------------ macros (oracle D / X1 / Z)

/// **Finding 2 (P1)**: if any command fails during macro execution (bell), the **entire replay must
/// abort** — neither the remaining keys of this round nor the remaining `@` count executes (`:h @`
/// error-abort semantics). The engine pre-enqueues all count rounds into pending_keys and keeps
/// going after a failure.
/// oracle: on `xa\nab\n`, `qafxDq` (the recording round succeeds on "xa": fx hits col0, D deletes to
/// an empty line); after moving the cursor with j to the "ab" line, `2@a`: in vim, round1's fx finds
/// no x → the whole thing aborts, buffer stays ["","ab"]; the engine continues executing D, deleting
/// "ab" empty, and round2 runs again → ["",""].
#[test]
fn macro_error_aborts_remaining_keys_and_rounds() {
    let mut f = Fixture::at("xa\nab\n", 0, 0);
    f.feed(["q", "a", "f", "x", "D", "q"]);
    assert_eq!(f.text(), "\nab\n", "the recording round itself executes successfully on the first line");
    f.feed(["j", "2", "@", "a"]);
    assert_eq!(
        f.text(),
        "\nab\n",
        "vim 9.1: fx failing aborts the whole @a, the second line 'ab' survives; the engine kept executing D and deleted it"
    );
}

/// **Finding 3 (P2)**: `@:` (replay the last Ex command line) accepts [count]; `2@:` executes the
/// command line twice; the engine's `@:` branch completely ignores the count and executes once.
/// oracle: on `a\na\na\n` after `:1d<CR>`, `2@:` → all three lines deleted (empty buffer); the
/// engine leaves one line (`1d` + one replay).
#[test]
fn at_colon_honors_count() {
    let mut f = Fixture::at("a\na\na\n", 0, 0);
    f.feed([":", "1", "d", "<CR>"]);
    assert_eq!(f.text(), "a\na\n");
    f.feed(["2", "@", ":"]);
    assert_eq!(
        f.text(),
        "",
        "vim 9.1: 2@: executes the :1d-equivalent command two more times (deletes every line); the engine executes it once"
    );
}

/// **Finding 4 (P2)**: vim's macros **are registers** — `"ayy` overwrites the macro recorded in
/// register a (oracle Z2: after `qaxqj\"ayy`, @a='YY\n'), and `@a` thereafter replays the yank macro;
/// the engine's macro slot (the macros map) and the register file are two separate storages, so
/// after `"ayy` the `@a` still replays the old key sequence.
/// oracle: on `ZZ\nYY\n`, `qaxq` (records x), j to the second line, `"ayy`, then `@a`: vim's buffer
/// is unchanged (@a replays yy, yanking the second line again); the engine replays the x from the
/// recorded body and deletes the 'Y' → "Z\nY".
#[test]
fn yank_into_register_overwrites_recorded_macro() {
    let mut f = Fixture::at("ZZ\nYY\n", 0, 0);
    f.feed(["q", "a", "x", "q"]);
    assert_eq!(f.text(), "Z\nYY\n", "recording also executes: x deletes the first Z");
    f.feed(["j", "\"", "a", "y", "y"]);
    f.feed(["@", "a"]);
    assert_eq!(
        f.text(),
        "Z\nYY\n",
        "vim 9.1: \"ayy overwrites the macro, @a replays yy (no text change); the engine replayed the old macro and deleted the Y"
    );
}

/// **Finding 4b (P2)**: the other face of the same identity — a recorded macro's content must be
/// **pasteable**: after `qaxq` the content of register a is "x" (oracle Z1: string(@a)='x', `"ap`
/// pastes "ZZx"); the engine's recording only goes into the macros map, register a stays empty, and
/// `"ap` rings with nothing to paste.
#[test]
fn recorded_macro_text_is_pasteable_from_the_register() {
    let mut f = Fixture::at("ZZ\n", 0, 0);
    f.feed(["q", "a", "x", "q"]);
    f.feed(["$", "\"", "a", "p"]);
    // oracle re-verified (2026-10-08): recording also executes; after x deletes the first Z the
    // buffer is "Z", `"ap` pastes 'x' → "Zx" (the draft's "ZZx" was an arithmetic slip)
    assert_eq!(
        f.text(),
        "Zx\n",
        "vim 9.1: \"ap pastes the macro text 'x'; the engine separates macro/register storage and has nothing to paste"
    );
}

// ------------------------------------------------ Replace mode (oracle H)

/// **Finding 5 (P2)**: the repeat condition and net text of counted Replace — `3Rab<BS><Esc>`: the
/// backspace does not cancel the repeat, and each round's net text "a" repeats 3 times (oracle: on
/// `zzzz` the result is "aaaz"); after the backspace the engine's cursor/anchor mismatch abandons
/// the whole repeat → "az" (or an equivalent single-round net text).
#[test]
fn r_mode_count_repeat_survives_backspace() {
    let mut f = Fixture::at("zzzz\n", 0, 0);
    f.feed(["3", "R"]);
    f.type_text("ab");
    f.feed(["<BS>", "<Esc>"]);
    assert_eq!(
        f.text(),
        "aaaz\n",
        "vim 9.1: 3Rab<BS><Esc> yields 'a' as each round's net text, three copies; the engine abandoned the repeat"
    );
}

// ------------------------------------------------ insert <C-v> digits (oracle E4)

/// **Finding 6 (P3, corrected by the audit6 re-verification)**: `i<C-v>0` then Esc.
///
/// The original expectation ("cancel the digit entry, do not insert NUL") was **wrong** — it
/// misread the oracle. Re-check against the authoritative 9.1 oracle (`edit.c:get_literal`: with
/// `i==1, cc==0` it goes through `if (cc == 0) cc = '\n'` → returns NUL; the terminating key ESC is
/// pushed back via `vungetc` and then exits insert mode normally):
///   `printf 'z\n'` → keys `i\x160\x1b:call writefile(getline(1,"$"),out)\r:wq\r`
///   → `od -c out` = `\0 z \n` (the NUL inserted before 'z'), `:wq` executing = insert already exited.
/// After the fix the engine matches this: insert `\0` and Esc exits insert mode.
#[test]
fn cv_zero_then_esc_inserts_nul_and_exits_insert() {
    let mut f = Fixture::at("z\n", 0, 0);
    f.feed(["i"]);
    f.feed_raw(Key::ctrl_char('v'));
    // digits must go through the key pipeline (the i_CTRL-V digit collector is intercepted in
    // insert_key before the printable path)
    f.feed(["0"]);
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "\0z\n",
        "vim 9.1: <C-v>0<Esc> inserts NUL (stored internally as NL) and then exits insert mode"
    );
    assert!(
        matches!(f.vim.mode(), Mode::Normal),
        "vim 9.1: the terminating Esc exits insert mode as usual"
    );
}

// ------------------------------------------------ Ex surface helpers (oracle AA5)

/// **Finding 7 (P3)**: `:u`/`:un` (the abbreviations of `:undo`; oracles AA5/UN: after a change both
/// `:u<CR>` and `:un<CR>` restore "abc") report E492 in the engine — the Ex table only accepts
/// `undo`/`und`/`undo!`/`und!`, not `u`/`un`.
#[test]
fn ex_single_letter_u_abbreviation_undoes() {
    let mut f = Fixture::at("abc\n", 0, 0);
    f.feed(["x"]);
    f.feed([":", "u", "<CR>"]);
    assert_eq!(
        f.text(),
        "abc\n",
        "vim 9.1: :u is equivalent to :u[ndo] and restores the deleted character; the engine silently does nothing with E492"
    );

    let mut g = Fixture::at("abc\n", 0, 0);
    g.feed(["x"]);
    g.feed([":", "u", "n", "<CR>"]);
    assert_eq!(
        g.text(),
        "abc\n",
        "vim 9.1: :un is likewise an abbreviation of :undo; the engine gives E492"
    );
}

// ------------------------------------------------ counted insert × <C-t> (oracle K1)

/// **Finding 12 (P3)**: `<C-t>` inside a counted insert session: vim's repeat replays the **key
/// sequence** round by round, so every round's indent shift takes effect (oracle K1: under noet
/// ts=8 sw=8, `3i<C-t>x<Esc>` on "ab" yields "\t\t\txxxab" — three rounds each raising one
/// shiftwidth, and the typing point at the line start moves right with the indent, x landing after
/// the indent); the engine treats the C-t shift as an edit outside the session, `InsertRepeat` only
/// accounts for the typed text, and the cursor at line start (at==ls) does not shift right with the
/// indent — yielding "xxx\tab" (two rounds of indent lost, and x landed before the indent).
#[test]
fn counted_insert_ctrl_t_shifts_repeat_every_round() {
    let mut f = Fixture::at("ab\n", 0, 0);
    // oracle environment (vim -Nu NONE): noet + ts=8 + sw=8
    f.feed([":", "s", "e", "t", " ", "n", "o", "e", "t", " ", "t", "s", "=", "8", " ", "s", "w", "=", "8", "<CR>"]);
    f.feed(["3", "i"]);
    f.feed_raw(Key::ctrl_char('t'));
    f.type_text("x");
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "\t\t\txxxab\n",
        "vim 9.1: 3i<C-t>x replays C-t every round, three shiftwidths; the engine only accounts the x and loses the per-round shift"
    );
}

// ------------------------------------------------ Directed re-checks (shapes expected to agree with the oracle; a failure means a new finding)

/// **Re-check 8 (expected to agree)**: `.` replaying a delete with a register prefix — oracle C: on
/// `aa\nbb\ncc` do `"add`, j, `.`; vim deletes the second line into register a (the `"a` prefix is
/// replayed, @a='cc\n').
#[test]
fn dot_replay_carries_the_register_prefix() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["\"", "a", "d", "d"]);
    assert_eq!(f.text(), "bb\ncc\n");
    f.feed(["j", "."]);
    assert_eq!(
        f.text(),
        "bb\n",
        "vim 9.1: . replays \"add (prefix included), the cc line is deleted into register a"
    );
}

/// **Re-check 9 (expected to agree)**: yank does not clear the ledger — `x`, `"ayy`, `.`: vim replays
/// the earlier x (oracle AA21: on "abcdef" the result is "cdef").
#[test]
fn yank_does_not_overwrite_the_dot_record() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["x"]);
    f.feed(["\"", "a", "y", "y"]);
    f.feed(["."]);
    assert_eq!(f.text(), "cdef\n", "vim 9.1: yank does not enter redo, . replays x");
}

/// **Re-check 10 (expected to agree)**: two consecutive small deletes both go only into `"-` and do
/// not touch the numbered ring (oracle B: after `x.` @1='' and @-='b'). The behavior side uses paste
/// probes: `"-p` pastes the second delete's 'b' after the cursor; `"1P` has nothing to paste ("1 was
/// never written).
#[test]
fn repeated_small_deletes_stay_out_of_the_numbered_ring() {
    let mut f = Fixture::at("abcdef\n", 0, 0);
    f.feed(["x", "."]);
    assert_eq!(f.text(), "cdef\n");
    f.feed(["\"", "-", "p"]);
    assert_eq!(f.text(), "cbdef\n", "vim 9.1: \"- pastes the second small delete 'b'");
    let mut g = Fixture::at("abcdef\n", 0, 0);
    g.feed(["x", ".", "\"", "1", "P"]);
    assert_eq!(g.text(), "cdef\n", "vim 9.1: small deletes do not go into \"1, so \"1P has nothing to paste");
}

// ------------------------------------------------ undo cursor (oracle Y1 / Z22)

/// **Finding 11 (P2)**: the undo cursor should land on the **first line of the undone change**; for
/// Ex range commands the engine treats the "begin_undo_group hint cursor" (= the cursor position
/// before the command) as the change position, so after undo the cursor stays on the pre-command
/// line instead of the deleted/changed line.
/// oracle Y1 (typeahead): on a 5-line buffer with the cursor on line 1, `u` after `:5d<CR>` →
/// line(".") = 5 (the deleted line); oracle Z22: after `G`, `:1,2s/x/Y/<CR>` then `u` → line(".") =
/// 1 (the first changed line).
#[test]
fn undo_cursor_lands_on_the_first_line_of_the_undone_ex_change() {
    let mut f = Fixture::at("l1\nl2\nl3\nl4\nl5\n", 0, 0);
    f.feed([":", "5", "d", "<CR>"]);
    assert_eq!(f.text(), "l1\nl2\nl3\nl4\n");
    f.feed(["u"]);
    assert_eq!(
        f.line(),
        4,
        "vim 9.1: after :5d, u lands the cursor on line 5 (the range line of the undone delete); the engine lands on the pre-command line 1"
    );

    let mut g = Fixture::at("x1\nx2\nx3\nx4\nx5\n", 4, 0);
    g.feed([":", "1", ",", "2", "s", "/", "x", "/", "Y", "/", "<CR>"]);
    g.feed(["u"]);
    assert_eq!(
        g.line(),
        0,
        "vim 9.1: after :1,2s, u lands the cursor on line 1 (the first changed line); the engine lands on line 5 where G was"
    );
}

// ------------------------------------------------ counted insert × C-u (oracle Y3)

/// **Finding 12 (P2)**: in a counted insert session, after `<C-u>` clears the typed text, the
/// remaining rounds continue with the **net text (empty)** — oracle Y3: on `zz`, `3iab<C-u><Esc>` →
/// buffer still "zz" (all three rounds net to empty; the repeats still fire but paste nothing). The
/// engine's C-u path does not trim `InsertRepeat`'s ledger, and the cursor returning to the anchor
/// meets the repeat condition again → it copies out the stale "ab" wholesale.
#[test]
fn counted_insert_ctrl_u_repeats_the_empty_net_text() {
    let mut f = Fixture::at("zz\n", 0, 0);
    f.feed(["3", "i"]);
    f.type_text("ab");
    f.feed_raw(Key::ctrl_char('u'));
    f.feed(["<Esc>"]);
    assert_eq!(
        f.text(),
        "zz\n",
        "vim 9.1: 3iab<C-u><Esc> nets to empty; the engine copied out the stale ab"
    );
}

// ------------------------------------------------ `.` of block-visual operators (vs the insert side of D11)

/// **Re-check 13 (expected to agree)**: after a block-visual **operator** change (`<C-v>jlx`), `.`
/// should replay the same-shaped block delete (vim's `.` redoes visual block changes; D11 only fixed
/// the block **insert** side).
#[test]
fn dot_after_block_visual_operator_redoes_the_block_delete() {
    let mut f = Fixture::at("aa\nbb\ncc\n", 0, 0);
    f.feed(["<C-v>", "j", "l", "x"]);
    assert_eq!(f.text(), "\n\ncc\n", "block delete of rows0-1 cols0-1");
    f.feed(["G", ".", "k"]);
    assert_eq!(
        f.text(),
        "\n\n\n",
        "vim 9.1: . replays the block-visual delete, also deleting the cc of the cc line"
    );
}
