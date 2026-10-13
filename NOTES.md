# Review log (2026-09-29)

Conclusions from many rounds of code review: the bugs fixed (every
semantics-class one empirically verified against vim 9.1 probes, engine
invariant classes caught by fuzz), the known divergences from vim,
open suspicious points, and performance / UX notes. Trust the actual
code logic; the layering diagrams in README and `src/lib.rs` are written
host-agnostically.

The per-item oracle evidence archives of the four independent audits
(originally `BUG_AUDIT.md` / `BUG_AUDIT2.md` / `BUG_AUDIT3.md` /
`BUG_AUDIT4.md`) have been archived out of the working tree and are kept
permanently in git history, retrievable for instance with
`git show b52851d:BUG_AUDIT4.md` (`b52851d` is the round-4 commit; for
the other rounds use the commit of the corresponding round). This file
keeps a summary section per round (round / findings / fixes / key
points); the per-item evidence lives in git history.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Read-only pipeline (PLAN_READONLY P1/P2: M1–M4, 2026-10-12, 9 cases in `tests/readonly_pipeline.rs` + 5 in `src/tck.rs`)

The first real consumer is alacer's diff view (a read-only diff browser:
two panes + unified synthetic-line projection + folding), which until now
shipped a few hundred lines of hand-written read-only vim. This round
lands M1–M4 of `PLAN_READONLY.md` (M5 — the gpui-vim `VimEditorRo` and
the alacer migration — is cross-repo work, not part of here).

### Semantics (vim 9.1 oracle evidence: `/usr/bin/vim -Nu NONE -N -s`, sampled command by command)

Under `:set nomodifiable`:

| Key / command | Result |
|---|---|
| `x` `dw` `dd` `p` `J` `rZ` `~` `o` `O` `C` `S` `gUiw` `>G` `u` | **E21** |
| `:s/a/b/` `:d` `:j` `:1m2` `:pu` `:retab` `:ce` `:t 2` `:co 2` `:normal x` | **E21** |
| `yiw` `yy` `mA` `q` `ZZ` `:y` `:q` | **No E21** (legal) |

E21 text (copied verbatim): `E21: Cannot make changes, 'modifiable' is off`.

Two implementation choices settled by this:

1. **Operators reject at the execution point, not at keypress time.**
   vim's `d` merely suspends silently; E21 arrives together with the
   motion, so after `dw` the cursor has to stay where it was. The
   implementation lives in `complete_operator_with_span` /
   `apply_visual_operator` / `apply_block_operator`, not in the
   `CmdKind::Operator` branch of `execute_command` (the `is_edit()`
   verdict for `Operator` only serves those three visual-mode paths).
2. **`u` / `<C-r>` are edits as well**, rejected with E21 (not "bell when
   there is no history").

### APIs landed

- `VimBufferMut::insert_text/delete_range/replace_range` now return
  `bool`, **defaulting to `false`** (reject). Adopting a read-only host
  therefore costs just `impl VimBufferMut for X {}` (zero methods) +
  `vim.set_modifiable(false)`. **Source-breaking**: hosts that already
  implement editing must add `-> true` (mechanical substitution). The 5
  implementations on this machine were updated.
- `VimState::set_modifiable` / `is_modifiable`;
  `:set [no|inv]ma[?|!|&]` goes through a bypass in `ex_set`
  (`'modifiable'` is engine state, not part of `Options`).
- E21 is intercepted at four levels: the `is_edit()` precheck in
  `execute_command`, the three operator execution points, the direct
  `I`/`A` dispatch branches of visual block, and `ex_command_is_edit()`
  in Ex dispatch; plus a final backstop when a write primitive returns
  `false` (so a host that forgot `set_modifiable(false)` still does not
  silently no-op).
- `MotionHook` (a standalone opt-in trait) + the
  `VimHost(Ro)::motion_hook()` accessor (default `None`), hooked in at
  the outermost `Motion::target`.
- `VimHostRo` + `CtxRo` + `VimState::handle_key_ro`.

### Two deliberate departures from PLAN_READONLY (both recorded here)

1. **`VimHost` is not a supertrait of `VimHostRo`.** The plan said
   `VimHost: VimHostRo` + "editing hosts migrate for free", but a
   supertrait forces every existing editing host to add an
   `impl VimHostRo for X {}` block (that is not free migration), while
   defaulting things like `viewport` would let a host that omits them
   silently get the wrong viewport. Instead there is an **`RoHost`
   adapter**: `VimHostRo` is a standalone trait, `VimHost` loses no
   method, and `handle_key_ro` adapts a `VimHostRo` into a `VimHost` at
   the call site.
2. **The buffer also goes through an adapter (`RoBuffer`) rather than the
   plan's `BufferRef` enum.** The enum approach would require touching
   589 `ctx.buf` references (`state.rs` 274 + `cmdline.rs` 153 +
   `ops.rs` 124 + …); risk disproportionate to the payoff.
   `RoBuffer<'a>` wraps a `&dyn VimBuffer` into a `VimBufferMut` (write
   primitives use the rejecting default), constructed by
   `handle_key_ro` at the call site, and the main pipeline is untouched
   — the goal of "do not copy 6.4k lines of state.rs" is met the same
   way, with zero changes.

`handle_key_ro` forces `modifiable` to false for the duration of the
call and restores it on return (a host that set it elsewhere is not
clobbered by this call).

### Tests

- `src/tck.rs`: `readonly_contract` (step-A shape, buffer as
  `&mut impl VimBufferMut`) and `readonly_pipeline_contract` (step-B
  shape, buffer is `&dyn VimBuffer` — **mutation is impossible by
  construction**). 41 editing key bindings assert E21 + byte-identical
  buffer + motion/search/marks/yank still working after the rejection;
  10 viewing key bindings assert E21 must **not** be reported.
- `tests/readonly_pipeline.rs`: RO viewing capabilities, E21 rejection,
  key-by-key RW/RO equivalence replay (22 viewing key sequences,
  comparing cursor/mode/registers/highlights), the four `MotionHook`
  cases (plain movement / pending operator `dj` / visual extension `vj`
  / falling back to the built-in behaviour when not taken over), and the
  `:set noma` round trip.
- **fuzz RO configuration**: 8 seeds × 20 rounds × 60 steps, the
  buffer's write primitives are `panic!`, and the same key stream is fed
  to step A (`handle_key` + `set_modifiable(false)`) and step B
  (`handle_key_ro`) while asserting the two cursors match at every step.
  A single leak past the E21 gate turns the test red immediately — a
  strong assertion over the whole gate set.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 36: fuzz round36 memory blowup and two infinite-expansion fixes (2026-10-11, 5 cases in `tests/parity_round36.rs`)

On scene: running `target/debug/deps/fuzz_round36`, RSS went 5 GB → 9.4 GB
within 40 seconds, `footprint` 53.1 GB, and `sample`'s hot frame was
`VecDeque::truncate` (i.e. `pending_keys.clear()` only ran for the first
time after the safety valve had already collided). Not "the debug binary
is big" — the file is only 4 MB; at runtime the replay queue was eaten
through memory. After the fix the same target peaks at 8 MB and finishes
in 108 seconds.

Fixes (semantics-class ones all verified against vim 9.1 first; expect
PTY, `:let` + `reg_recording()` sampling on 2026-10-11):

| # | Symptom → rule |
|---|---|
| R36-1 | While playing, `q` used to start/stop recording and swallow its character argument → vim's `nv_record` gate is `reg_executing == 0` (`src/normal.c:7264`): during register execution `q` is **completely inert** — it does not start, does not stop, and does not consume the register name. Added `VimState::reg_executing`, set at the `MacroPlay` entry, reset at three places: the safety valve, `replay_error`, and queue drain |
| R36-2 | Keys played by `@` were fed into the macro being recorded → vim's replay goes through `ins_typebuf(..., nottyped = TRUE)` and `gotchars()` (`src/getchar.c:1418`) records only keys with `nottyped == FALSE`. Closed at the four sites `record_key`/`unrecord_key`/`record_typed_text`/cmdline recording by checking `reg_executing.is_none()`; the `.`/redo recording path is **not affected** (evidence: `qa@cxq` → `"a` is the three steps `@cx`, and replay still executes normally) |
| R36-3 | The replay budget pre-expanded the full `MAX_PIPELINE_STEPS` per call → a self-playing register (`"a` == `@a`) pushes another 100k keys for every `@` consumed, production far outpacing consumption. Now based on **queue headroom** `room = MAX_PIPELINE_STEPS - pending_keys.len()`, stopping playback early once exceeded instead of reaching the guard that discards everything |
| R36-4 | The register content of `:@` was itself another `:@…` → unbounded mutual recursion `execute_ex ↔ ex_at`, `fuzz_round17` `has overflowed its stack` (**note**: this predates round36; A/B against the HEAD working tree did not reproduce it, so it came from uncommitted changes). vim closes it off in `do_cmdline()`: `call_depth >= 200` → `emsg("E169: Command too recursive")` + `return FAIL` (`src/ex_docmd.c:707-737`). The engine adopts the same threshold with an `ex_depth` counter and `MAX_EX_DEPTH = 200`, splitting `execute_ex` into guard + `execute_ex_inner`. Evidence: `:let @b='@b'` + `:@b` → screen shows `E169: Command too recursive`, `v:errmsg` the same text, buffer unchanged |

### New known divergences (kept deliberately; integrators need to know)

- **`:@` does not strip a leading colon from the register content**: vim's
  `do_one_cmd()` starts with "Skip ':' and any white space"
  (`src/ex_docmd.c:2007-2012`), so `:let @a=':echo 7'` + `:@a` really
  prints `7` (PTY evidence, `v:errmsg` empty); the engine treats the whole
  line as a command name → `E492`. Unrelated to the three containment
  fixes of this round; the change surface is the dispatch prefix in
  `execute_ex_inner`, left as a standalone fix item.
- **The safety valve emits no message**: when the engine hits
  `MAX_PIPELINE_STEPS` it drops the queue silently; vim reports a full
  typebuf as `E74: Command too complex`. The guard counts **keys consumed**,
  so an explicit long play such as `1000000@a` gets truncated without a
  hint even when it is not self-recursive.

### Open items

- The early stop in `enqueue_replay` makes an over-long macro **play
  partially** (vim plays nothing at all once E74 fires). The difference is
  only in the truncated tail, and the fuzz side currently only requires
  "bounded and the buffer stays within range".

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 35: independent audit (7), area A closed out — audit8 comparison table converted to assertions + 7 fixes in Ex/`:s`/`:set`/insert indent (2026-10-10, 162 cases in `tests/audit8_a.rs`)

The material was a pure black-box differential: 161 (initial buffer,
**raw key bytes**) shapes fed identically to the engine and to
`/usr/bin/vim` (9.1 patches 1-1752, `-Nu NONE -N -i NONE -n`, prefixed
with `:set ic scs ts=4 tw=78 et` to pin the engine's deliberately
divergent defaults into oracle premises), then `:wq` and compare buffer
text byte by byte + compare the cursor byte position. Area = A (Ex
command line, `:s`, `:set`, insert indent). Way of closing out:
**the exploratory TSV scaffolding retires, the table becomes assertions**
— 151 cases went into `CASES` in `tests/audit8_a.rs` (one
`parity_with_vim_oracle` test runs them all, `feed` turning bytes back
into engine key names), with 11 additional targeted assertions for the fix
surface, pinning messages/text/cursor on both sides. Full
`cargo test --no-fail-fast`: **1127 cases (1063 non-`#[ignore]` + 64
open-item/tool probes) with 0 failures** (an actual counted number, not
the exit code of `| grep`). The sibling
`tests/audit7_b.rs` (the round-6 area-B motion/operator/visual
differential fuzz runner, `#[ignore]`, its two-sided sampling discipline
written in the file header) landed in the same batch — closing out it was
itself fixed in two places: the oracle side's `\<C-v>` notation switched to
raw bytes, the engine side's multi-character tokens switched to per-character
(see the discipline item below), so any difference count it reported
earlier is void.

Fixes (A-1…A-6 + the insert-indent group, all verified against 9.1):

| # | Symptom → rule |
|---|---|
| A-1 | The old `:set noic?` implementation **inverted** the display according to the `no` prefix. In vim `no{opt}?` is just another spelling of `{opt}?`, reporting the actual state (`:set ic` then `:set noic?` → `  ignorecase`) |
| A-2 | The old `:set all&` merely listed options like `:set all`. vim **resets everything to its default** (`ts=9` + `:set all&` → `tabstop=8`). Added `Options::reset_all`, which also clears `search.matches_generation` |
| A-3 | The old implementation still ran `:s/a/b/ e` (space before the flags after the third delimiter). vim's flag string must sit **flush against** the delimiter; once leading whitespace is present, the whole tail can only be a bare count, otherwise `E488: Trailing characters: e`. The flag whitelist is also tightened to `giIncelp#&r`, and any trailing junk raises E488 quoting the original text |
| A-4 | The old `:s/pat/repl/n` counted "all matches". vim counts **what the substitution would do**: without `g` at most 1 per line (on "aa" → `1 match on 1 line`); only `g` counts every occurrence. The two nouns in the message count independently: `{total} match{es} on {lines_with} line{s}` |
| A-5 | When the flags carried no `i`/`I` the old implementation unconditionally forced case-insensitive, so after `:set noic` a `:s/a/X/` (including the `n` branch) still ate `A` too. Now "flags only override; without a flag it follows `ignorecase`/`smartcase`" |
| A-6 | `:set notabstop?` reported unknown option. The `no` prefix is stripped for **value options** as well: with `ts=6` it prints `  tabstop=6` (in `describe` both the bool and the value branch must strip the prefix) |
| extra | Four insert-mode `<C-t>`/`<C-d>` issues: ① the old `old ± sw` is replaced by vim `get_new_sw_indent(round=TRUE)`'s **rounding to a shiftwidth multiple** (increase = next multiple; decrease = floor, and one more step down when already a multiple, hence 4→0 rather than the fake in-place success 4→4); ② indent width is converted via `'tabstop'` — `\t\t` is 8 columns at ts=4, the old `char_display_width` counted 2 columns, so `<C-t>` yielded 4 instead of vim's 12 (switched to `char_display_width_at`); ③ when the cursor lands **inside** the indent it moves with the indent's column shift and is clamped to the line start (`change_indent`'s vcol branch: `\t\tfoo` + `0i<C-t>X` → `    X        foo`, cursor byte 4); ④ `0<C-d>` goes through `INDENT_SET 0` — it deletes the typed `0` and **zeroes** the indent, it is not "subtract one shiftwidth" |

### The five sampling/differential disciplines caught this round (all documented, do not step on them again)

- **Ordering between `-s {scriptin}` and `+cmd`/`-c cmd`**: startup
  commands run first and typeahead is consumed afterwards, so `+':w! out'`
  samples the **initial buffer** — a whole batch of fake differentials
  saying "neither side changed" came from exactly this (this round's first
  `i0<C-d>` test showed the indent not budging at all). The correct
  posture: put the sampling into the typeahead too
  (`:redir! > f` / `:echo string(getline(1))` / `:redir END` / `:q!`). The
  previous round's version of this trap was `timeout` truncation; this
  time it is command order, with identical symptoms (both look like a
  no-op).
- **Pipes swallow cargo's exit code**: with `cargo test 2>&1 | grep …` the
  `$?` belongs to `grep`/`tail`, not cargo. This round that very mistake
  read "1 target failing" as "everything green" — you have to write to a
  file and then count (`cargo test --no-fail-fast > log 2>&1`, then
  `grep "^test result" log`). Two old pins caught in the same batch
  (`parity_round6::set_query_reports_current_value_without_changing_it`'s
  `:set noic?` → `noignorecase`, `parity_round13::probe_describe_garbage`'s
  `describe("notabstop") → None`) were precisely the old engine shapes
  overturned by A-1/A-6; corrected against the oracle with the evidence
  recorded in the tests (`execute('set noic?')` returns `  ignorecase`
  while `ic` is on; `execute('set notabstop?')` returns `  tabstop=6` with
  `ts=6`).
- **vim's `\<C-x>` notation does not expand inside `-s {scriptin}`**: the
  script file is a raw keystream, so `\<C-v>` is eaten as six ordinary
  characters (`audit7_b`'s oracle side was written that way, which produced
  a field of false positives across "block visual" and visual `gU` — the
  `\t`, `-- INSERT --`, `Error reading input` in vim's output are its
  autopsy report). Control keys must be raw bytes; arrow keys still need
  expect PTY.
- **Both sides of a differential must use the same key granularity**:
  `audit7_b`'s generator emitted multi-character tokens
  (`gg`/`ge`/`aw`/`iW`); vim consumes them character by character as a
  keystream, while the engine side's `Fixture::feed` goes through
  `Key::parse` — `Key::parse("aw")` yields `KeyKind::Named("aw")`, and an
  unknown name is swallowed outright. So the whole
  text object/motion **never happened** on the engine side while vim
  executed it normally, and the runner reported 20+ "differences" at once
  (the `gUaw` family — engine does nothing, vim uppercases — looked most
  like a real bug, and it was all this one). Only after switching to
  per-character expansion (`engine_tokens`) and rerunning do the numbers
  count. Same
  root cause as the `:set` lesson: **the number-one source of oracle false
  positives is the channel, not the engine**.
- **Printable characters in insert mode belong to the host's text channel**:
  the engine parks raw keys in `pending_unknown_chars` waiting for the host
  to land them (see the macOS note in `state.rs` and
  `take_pending_unknown_chars`), so `Fixture::feed(["0"])` in insert mode
  **drops the character entirely**. Tests must always type text via
  `type_text`; the command-line prompt is the opposite — there the
  characters are key events (`type_text` in prompt state feeds itself back
  into feed).

### Divergence changes

- Divergence #12's roster now also lists `:copy`/`:move` (the whole
  `:t`/`:co`/`:m` family is not implemented and falls through to E492),
  pointing at the red probes.

### Open items (scoped out of this round, not fixed)

- The whole `:copy`/`:move` family: the red probes
  `copy_command_missing`/`move_command_missing`/
  `move_cursor_lands_on_moved_line_first_non_blank` in `tests/audit6_a.rs`
  already carry the complete oracle matrix
  (`:2m0`→`[b,a,c,d]`, `:2t0`→`[b,a,b,c,d]`, `:1m2` cursor landing on the
  first non-blank of the moved line); implementing it turns them green.
- `:<C-h>d` (backspace on an empty command line): the `-s` channel and
  expect PTY give **contradictory** vim results (one rings the bell and
  rejects, the other eats the following characters); not characterized
  yet, so it is left out of `CASES`.
- 16 differences (10 text + 6 cursor) from the same batch of scaffolding
  (a TSV differential table, 328 cases, coverage pushed into
  motion/visual/undo/insert). The scaffolding itself retired with this
  round; the shapes can be reconstructed from the table
  (`initial @row:col` + keys; `«…»` is the host's text channel, equivalent
  to `type_text`). Only the **observation** is recorded per item, no
  conclusions:

  | id | initial @row:col | keys | engine | vim |
  |---|---|---|---|---|
  | cjk3 | `中文abc` @0:0 | `w` | cursor 8 | 6 (column 7) |
  | cjk4 | `中文abc` @0:0 | `dw` | `\n` (whole line emptied) | `abc\n` |
  | cjk5 | `中文abc` @0:0 | `diw` | `\n` | `abc\n` |
  | em5 | `👨\u{200d}👩x` @0:0 | `x` | `x\n` | `\u{200d}👩x\n` |
  | m20 | `abc def` @0:0 | `2g_` | cursor 6 | 0 |
  | m24 | `Aaa. Bbb. Ccc.` @8:0 | `(` | cursor 10 | 0 |
  | rr2 | `abcdef` @0:0 | `R«X»l«Y»<Esc>` | `XYcdef` | `XlYdef` |
  | rr4 | `abcdef` @0:0 | `R«XY»<Left>«Z»<Esc>` | `XZcdef` | `XYLeftZ` |
  | u4 | `a\nb\nc` @0:0 | `dddd u` | `b\nc` | `a\nb\nc` |
  | u5 | `abcdef` @0:0 | `xxu.` | `cdef` | `bcdef` |
  | vis11 | `abcdef` @0:0 | `vlly` | cursor 2 | 0 |
  | vis12 | `abcdef` @0:0 | `vllP` | `abcdef` (no paste) | `def` |
  | vis13 | `ab\ncd\nef` @0:0 | `VjP` | unchanged | `ef` |
  | blk5 | `abcd\nefgh` @0:0 | `<C-v>jlA«Z»<Esc>` | same text | cursor 2 vs 0 |
  | blk7 | `abcd\nefgh` @0:0 | `<C-v>jlc«Z»<Esc>` | same text | cursor 4 vs 0 |
  | insr5 | `abc` @0:0 | `i«X»<C-o>dw<Esc>` | `Xabc` | `X\n` |

  Before booking these, run them through the oracle discipline (the
  scaffold's `key_bytes` has been audited: `<C-v>`/`<Esc>`/`<BS>` and the
  like **do** expand to raw bytes, but arrow keys such as `<Left>`/`<Right>`
  fall back to `name.encode()` and are sent as literal text — the literal
  `Left` on `rr4`'s vim side is exactly that): `rr4` is void outright;
  `m20`/`m24`/`vis11`/`blk5`/`blk7` end with the vim-side cursor at byte 0,
  the shape of "the whole key sequence was never consumed" (vim stayed at
  the start) — rerun each case through expect PTY before booking. Among
  the genuine text differences
  (`cjk4`/`cjk5`/`em5`/`u4`/`u5`/`vis12`/`vis13`/`insr5`) the CJK word
  motions, visual paste, the delete after `<C-o>` and undo granularity are
  all fixable directions, but they were scoped out of this round and left
  untouched.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 34: independent audit (4), all 63 items fixed (2026-10-09, `tests/audit5_{a,b,c}.rs`)

3 parallel area agents (A=Ex, B=motion/objects, C=registers/visual/undo)
+ the main-session fuzz track (round35: randomly assembled Ex commands ×
unusual buffers, catching P0s). **This round is the first systematic port
of vim testdir behavioural assertions**; of 96 candidate shapes, 12 were
disproven on reverification. Highlights:

- New feature surface: the `|` command separator, the `:dl` family, `\\/`,
  `\\?`, `\\&` addresses, digits concatenated with a search address
  (`2/foo/`, the search anchored to the preceding address), the
  per-position skip semantics of `r<C-E>`/`r<C-Y>`, `I`/`A` in v/V, the
  keep_registers of visual `P`, the `<C-r>=` expression prompt on the
  command line, `"=<CR>` reusing the last expression, and the `r` flag of
  `:s`.
- Semantics fixes: the specialized join of `:s/\\n//`, `:sort n`
  putting non-numeric lines in front as a block, the fewer-lines message
  for `sort u`, `gg` flipping `g`, the `:s` count 0/overflow
  E939/E1510, delimiter E146/E10, `ts=0`/out-of-range E487/E474, the
  cursor of `r<C-E>` landing on the last processed position, forward
  search for block objects (the count-th unmatched opening bracket),
  cross-line inner block preserving newlines, whitespace handling of
  `a'`, sentence-object counting at boundary alternations, use_reg_one
  (`%`, backtick/single quote, search, section-boundary delete writing
  `"1`+`"-`), `'[`/`']` bookkeeping on four paths (put/undo/insert/c
  paths), gp at line level landing one line after the cursor, multi-line
  charwise p/P returning the cursor to the first character, `:retab`/`:ce`
  family leaving the cursor on the original character, operator exclusive
  rule (a) upward branch + rule 3 (inclusive first-non-blank shrink;
  MarkJump exempted per end_adjusted), `:marks` col switched to a 0-based
  byte column (previously display column +1; incidentally fixing a panic
  when rendering CJK selections), and gv selection-swap restoration after
  undo/redo (clamped version).
- Three old pins corrected (overturned by oracle reverification, evidence
  attached in the tests): for `r<C-E>` a neighbouring line being too short
  skips per position rather than cancelling the whole command; `:sort n`
  puts non-numeric lines in front rather than treating them as 0; `]`
  jumping has no fallback.
- Open item: B-17 (which side owns the trailing whitespace of the outer
  delete region in `at`; the `#[ignore]` probe stores the oracle matrix).
  Full suite: 1034 cases, 0 failures.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 33: round-32 audit residue cleared + all 55 independent-audit items fixed (2026-10-08, 60 cases in `tests/audit3_{a,b,c,d}.rs`)

The previous session had fixed about half of round 32 audit's 59 items (49
probes still red); this session first cleared the entire residue
(including the visual-block o/O rectangle model, the block-insert exit
cursor now stopping at the block's first line, undo restoring marks of
deleted lines, visual selections collapsing when their line is deleted, the
`g;` fresh semantics, etc.), then completed the third independent audit
with 4 parallel area agents (A=Ex, B=motion, C=visual,
D=registers/macros/undo/insert): **all 55 new findings fixed**, full suite
963 cases green. Highlights (including 7 open items):

- New features: the `!` filter operator (sh -c, stdin/stdout same as vim),
  `=` + `'lisp'` reindentation, `go`/`gm`/`gj`/`gk`, `z<CR>`/`z.`/`z-`,
  `<C-p>`/`<C-n>` on the command line, `:set all`/`:setglobal`,
  `i<C-t>`/`C-d`/`C-y`/`C-e`/`C-a`/`C-@`, `q"` and visual `@`, the block
  semantics of block `X`/`Y`/`C`/`S`, block `>` indentation, `{count}v`
  pre-selection, count repetition for block `I`/`A`, `"."` fidelity (empty
  session clears the ledger / C-w trimming / backspace replayed as \x08),
  undo restoring marks of deleted lines, visual selections collapsing with
  line deletion (column preserved), and unifying macros with registers
  (recording lands in a register, `"ayy` overwrites the macro, `"ap` pastes
  the macro; with write-generation synchronisation).
- Behaviour fixes: the `.` count-substitution model, `@:` honouring count,
  macro failure aborting replay (hard failure boundary), gv's cursor rules
  (CHARWISE reverse re-select: cursor at the start; LINEWISE/BLOCK return to
  the cursor end), Esc staying on the visual cursor end, the `g;` fresh
  semantics, `'>` resolving to the last selected character, undo's cursor
  landing on the first edited offset, `$j` landing on the last character, and
  a batch of P2/P3 in Ex/command line/retab/alignment (15 items in group A).
- Second relay batch (same day): the `]M`/`[M` landing points were derived
  as an **empirical rule** from the oracle discrimination matrix and
  implemented (the first bracket; if it is an opening bracket, its pair
  exists, and there is another bracket after the pair → land on the pair,
  otherwise land on the opening bracket; `]m`/`[m` = the first bracket of
  any kind at any column — the old claim "method brackets are not in column
  1" is disproven here); unterminated Ex search addresses (`:/pat` takes the
  whole line end as the pattern); `:reg` switched to vim's layout; the E
  message bell upgraded to a hard replay failure (error aborts, beep
  continues); added `tests/probe_round34.rs` and the macro×register fuzz
  `tests/fuzz_round33.rs`. Still open: the full TAB block-edge model (three
  new shapes of oracle data), the E4 mark after redo rewind, and named-key
  attrition in macro serialization (left as is). See the "Second relay
  batch" subsection of this round.


## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 31: all 50 independent-audit bugs fixed (2026-10-06, 50 regression cases added in `tests/audit_probe.rs`)

An external independent audit used "unexpected operations + unusual data"
as its attack surface and produced 50 findings (every expected semantic
was verified against the vim 9.1 typeahead oracle and deduplicated against
the 77 already-known divergences). All are fixed this round, the regression
suite is 50/50 green, and `cargo test` is green overall. The audit also
disproved 4 candidates (visual `n` not extending the selection, `dw` on an
empty line, the `2dw` word, `gqq` formatting only the current line — the
engine agrees with vim on all of them).

### Semantics fixes (oracle evidence)

- **A1** `motions.rs` ScreenTop/ScreenMiddle viewport arithmetic made
  saturating — a host returning extreme viewport values no longer panics.
- **B1** hex literal `a`-`f` digits enter the scan: `0x1f` with the cursor
  on `1` or `f` both increment to `0x20` (the old code only accepted `0-9`,
  so with the cursor on `f` the whole command was a no-op).
- **B2** forward search from a non-digit position recognises float tokens:
  `1.5` with the cursor on `.` + `C-a` → `2.5` (vim increments the float;
  the old code grabbed `5` and produced `1.6`).
- **C1-C4** the `:s` replacement side replaced by a full vim expander
  (`expand_vim_replacement`): `\0`-`\9` backreferences, `&` whole match,
  `~` reuse of the previous replacement (added bookkeeping in
  `cmdline.last_replacement` — mind the capture-before-overwrite order),
  the `\u \l \U \L \E \e` case modifiers, `\r` line break, `\n` NUL, `\t`,
  and the literals `\{sep}`/`\\`/`\&`/`\~`. `$` returns to being an
  ordinary character (vim's `\1` syntax replaces the old `$1` dialect —
  **the replacement side of known divergence #1 closes here**, the pattern
  side stays Rust regex).
- **D1-D3** three registers: `".` (most recent insertion, accumulated
  bookkeeping per insert session), `"%` (host file name, read directly in
  get_for_paste), `":` (most recent Ex command line, synchronised on
  Enter).
- **E1** bare `:sort` default range changed to the whole file (the old
  current-line default hit the single-line early exit and silently no-oped).
- **E2-E4** `:ce`/`:ri`/`:le` (alignment by display width, CJK counted as
  2).
- **E5** `:pu`/`:put [x] [!]`. **E6** `:retab [n] [!]` (measured with the
  old ts, re-laid out with the new one; et/`!` forces spaces).
- **E7** `:&&` (replay keeping flags), **E8/E9** `:undo`/`:redo` (reusing
  history_step), **E10** `g&` (whole-file replay keeping flags).
- **E11/E12** `:reg {name}` with an argument, `:delm[arks]` (including
  `a-z` ranges and `!`).
- **F1** the `'<,'>` address (the original audit item was a test artefact —
  the engine already supported it; promoted to a proper regression test);
  **F2** the `:/pat/,/pat/` search address (including empty-pattern reuse of
  `@/`, E486); **F3** `/pat/{offset}` (`e`/`s`/`b` + line shift, bookkeeping
  in `SearchState.offset`, `n`/`N` replaying it in sync).
- **G1/G3** visual-block virtual columns (`block_cursor_col`): `h`/`l`
  extend the block past short lines, `j`/`k` do not shrink it because of
  byte clamping; `block_row_range` anchors short lines at the **line end**
  (typed text lands at the end of the copy line rather than the start).
- **G2** visual `D` dispatched by kind: block = block column to each line's
  end, char/line = whole-line delete (vim's `v_D` semantics).
- **G4** the uppercase of `ß` is the single character `ẞ` (U+1E9E; re-proved
  against the oracle, vim 9.1 `~` behaves the same; the "SS" expectation in
  the old regression was wrong).
- **G5** charwise register × line-visual `p`: the pasted text replaces the
  selected lines as their content (the old code's bare insert swallowed the
  next line).
- **G6** `gp`/`gP` bound + `put_ex(leave_after)`: cursor stops after the
  pasted text.
- **H1-H3** the `i_CTRL-V` state machine (Single → any key literal;
  `u`/`U`/`x`/`o` + radix digit accumulation) and the `i_CTRL-K` digraph
  table (`ss`→`ß` and 70+ common pairs); the pending check runs before the
  Esc-exit (Esc inside a pending sequence does not leave insert).
- **I1** `'autoindent'` default changed to **noai** (vim's default; indent
  preservation for `cc`/`S`/`o`/`O`/`CR` now needs an explicit `:set ai` —
  the four old tests were given `:set ai` for this reason).
- **I2** `sw=0` = use `'tabstop'` (the old `.max(1)` produced one-column
  indents).
- **J1-J4** `<Insert>` enters insert mode; `ga`/`g8` info messages; `gq`
  (already implemented, promoted to a regression test).
- **K1** visual `g C-a` sequential increment (per line +count+i).
- **K2** insert `<S-Tab>` is a no-op (handle_key intercepts **before** the
  named→Char collapse and the shift stripping — the old path synthesised
  two spaces).
- **K3** cmdline `<C-r>{reg}` inserts register content.
- **K4** the `_` motion bound (count-1 lines down + first non-blank).
- **L1-L5** bell on failure paths: fewer than `Nr` characters in the line,
  `r` on an empty line, `<<` with no indent (both the double-keystroke and
  the LinewiseOp paths), `~` on an empty line, the `iw` family on an empty
  line (an empty selection now counts as failure).
- **M1** `]p`/`[p`/`]P`/`[P` indent-adjusted paste (shifted to the target
  line's indent, relative indentation preserved).

### Divergence changes

- **#1 replacement side closed**: replacement text now expands with vim
  semantics (`\1` references, `$` literal); the Rust regex dialect on the
  pattern side is unchanged.
- Added (minor): `C-v`-quoting a named key (arrow keys etc.) does not
  produce a termcap sequence and is treated as empty; the digraph table is a
  common subset rather than vim's full table.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 30 review additions (2026-10-06, `tests/probe_round30{,b}.rs` added, +30 targeted probes)

Themes of this round: **the "characters typed in this session" semantics of
insert `C-w`/`C-u`, Replace-mode restoration, backwards Ex ranges, and the
relationship between `.` and the Ex command line**. One more channel lesson
for the probe infrastructure: `vim -s` typeahead **does not recognise arrow
key escape sequences** — `\x1b[D` gets split into `Esc` (exits insert) +
`[D` (normal-mode `[D` jumping to the first macro definition, a no-op) +
`D`, so every conclusion drawn from a sequence containing arrow keys is
void; and **a sequence without a trailing `<Esc>` never reaches the disk**
(`:wq` did not run, the file on disk is the original, giving a fake "no
change"). The three early C-w probes of this round produced contradictory
results for exactly this reason and were all voided and rerun. Valid probes
now uniformly: no arrow keys, `<Esc>:wq` terminator, cross-validated from
several landing points.

### Semantics fixes (vim 9.1 typeahead probe evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **Backwards Ex ranges silently swapped** | The old `:5,1d` implementation executed after `a.min(b), a.max(b)` swapping — one transposed digit emptied the entire buffer with no feedback at all. vim asks interactively "Backwards range given, OK to swap?" and refuses in script mode (9.1 probe: not a single line deleted). Fix: `parse_range` reports `E16: Invalid range`, buffer untouched |
| 2 | **insert `C-w`/`C-u` ate pre-existing text before the insert start point** | vim's semantics are "delete the characters **typed** in this session" (`:h i_CTRL-U`: "Delete all entered characters before the cursor"): with `hello world`, inserting `ab` at column 2 + `C-u` → `hello world` (the engine's old implementation deleted to the line start → `llo world`); typing `X` inside a word + `C-w` deletes only `X` (the engine deleted the whole `abX` word); a second `C-w` at the typing start point is a **no-op** (the engine walked into pre-existing word). Added the `insert_typed_start` typing-origin anchor (cleared when the session enters, set while typing, re-anchored to the new line on `<CR>`, reset on vertical movement, relative adjustment in the three editing funnels + `refloor_stored_offsets` as a floor backstop); every deletion now goes through `insert_delete_typed_span`. A session with no input falls back to the line-start shape (the common `backspace=start` configuration shape) |
| 3 | **Replace-mode `C-w`/`C-u` did not restore overwritten characters** | vim: `RXX<C-w>` → `one two` (restores `on`, cursor lands at the typing start point; `RXX<C-w>X` → `Xne two`); `Rab cd<C-u>Z` → `Zbcdefg` (C-u restores everything, Z overwrites a); `Rab cd<C-w>Z` → `ab Zefg` (C-w deletes only the word `cd` and restores d/e — word boundary and restoration coexist). The engine's old implementation just did an ordinary delete (the `replace_overwritten` stack had a restoration path for BS only). `insert_delete_typed_span` now branches by Replace mode: stack entries inside the span are rebuilt from their original characters (they line up even when the original multibyte character has a different length), entries after the span shift by the length difference |
| 4 | **The Replace overwrite stack stored pre-replacement coordinates** | `R中a<BS>` restored nothing — the stack entries held the offsets from **before** the replacement (`中` at 0..3, `文` at 3..6) while after writing `ab` the typing actually sits at 0, 1, so BS's position match necessarily failed (round 14's probe matrix was all ASCII and never exposed it). Fix: store the **post**-replacement landing point (`at + the bytes already typed`), shared by the BS/C-w/C-u paths |
| 5 | **Round 9's "line-start `C-w`/`C-u` joins lines" was a bad probe** | Retyped via typeahead (probed twice: default bs and `bs=start`, same answer): at a line start with nothing typed, `C-w`/`C-u` are both no-ops — the "join" only happens when a deletion really crosses the line start (the `bs=eol` shape; the rare shape where a word is glued past the line boundary by a leading combining character). The engine's join branch was withdrawn (`C-u`'s dead `at > 0` path, `C-w`'s join branch kept only for cross-line word shapes); the two `parity_round9` tests now assert the verified behaviour |
| 6 | **`.` no longer replays the Ex command line** | vim's `.` replays only normal-mode changes (9.1 probe: after `x`, `:s/b/X/`, `j.` replays `x`; with `b` on the `/bxx` line, replaying `:s` would give `Xxx` — the actual result is `xx`, decisive). The engine's old implementation recorded the whole `:s/a/b/<CR>` keystroke sequence into `last_change` (a test in engine.rs pinned it). Fix: keys typed at the `:` prompt go only into macro capture (the register keeps the raw keys, `@a` still replays the Ex segment), not into `.` recording; `/` and `?` keep full recording (`.` replay of `d/foo<CR>` depends on the complete keystroke sequence); `begin_cmdline(':')` clears the optimistically recorded bare `:` from the pipeline — otherwise after an editing Ex command `last_change` would be a single colon and `.` would reopen the command-line prompt out of thin air (worse than the old behaviour, caught by a scratch probe) |

### New features

- **Ex `:&` and `:~`**: `:&` replays the last substitution without flags
  (`:h :&`; probe: after `:1,1s/a/B/g`, `:2,2&` replaces only the first
  match on line 2). `:~` rebuilds the command from the **current search
  pattern** + the original replacement string (`:h :~`; probe: after
  re-pointing with `/x`, `:3,3~` turns x into Q — the replacement text of
  the very first `:s`; flags are likewise not carried over). Unescaped
  delimiters inside the pattern get re-escaped, already-escaped sequences
  stay verbatim (a second escaping pass would turn `a\/b` into a literal
  backslash). With no previous substitution it reports E33. Before this,
  `&` only existed in its normal-mode form and the Ex side gave E492.

### Disproven during review, no change needed (saves a re-check next round)

- **`q1` recording into a numeric register**: vim documents
  `q{0-9a-zA-Z"}`, and the engine accepts 0-9 as well. Beware the
  criterion: recording **also executes** the keys (in `q1ix<Esc>q` the `x`
  still inserts), so validating a recording means replaying it (`@1`), not
  looking at the first execution.
- **Who replays `:s` under `.`**: see fix #6 — vim uses `&`/`:&` to repeat
  a substitution and `@:` to repeat an Ex command, so `:`'s responsibility
  narrows to ordinary changes.
- **Empty `/` + `<CR>` re-search**: the engine jumps from **after** the
  cursor (skipping a match under the cursor) and may wrap — same as vim (the
  `foo bar foo` sequence 8→0→8 agrees at all three steps).
- **`:0d` deleting the first line**, **`J` inserting a space at the end of a
  CJK line**, **`:.,+2d` deleting 3 lines** (+2 means "two more lines
  down") — the probe expectations were corrected; the engine already agreed
  with vim.
- **`$1` replacement expansion**: the group syntax of this engine's regex
  dialect (divergence #1); with no group, `$1y` looks up the capture group
  by the whole name, fails, and yields empty overall, while `$$` escapes a
  literal `$` — pinned by probes.
- **m1 illegal mark name**: the engine silently does not book it
  (`marks.set` accepts only letters and `^ .`), and vim 9.1 is equally
  silent (file unchanged) — consistent.
- **`q/` illegal register**: the engine rings the bell and does not hijack
  the following `/` (the shape already fixed in round 29); probes pin it
  additionally.

### Refactors (zero behaviour change)

- `insert_delete_typed_span` now contains the three delete shapes of C-w/C-u
  (whitespace-band sweep, word delete, line-start retreat), plus one site
  each for Replace restoration and for block-session copy shrinkage (only a
  pure tail trim touches `block.text`; when the character grid does not line
  up, the exit divergence guard takes over).
- `substitute_with_last_pattern` (the `:~` rebuild) and
  `substitute_without_flags` share `split_escaped_fields`.

### Known divergences (continuing the global numbering)

77. **`C-w`/`C-u` deleting the middle of what was typed inside a block-insert
    session** (after moving with arrow keys): the engine's fast path for
    shrinking the copy does not apply (not a pure tail), so the exit delta
    guard skips the copy — the typed line keeps its edit and the other lines
    are not copied. vim in this situation copies the deleted text as usual.
    Extremely rare (block insert × arrow keys × C-w combination), and the
    guard guarantees nothing gets corrupted.

### Performance notes

No hot-path changes. `insert_typed_start` is one Option write per key (the
insert typing path is branch-predictor friendly); `parse_range`'s backwards
check is O(1). bench_probe rerun: 900KB rescan 0.30ms per pass, 600KB `:%s`
6.2ms, 1MB single-line w-motion 117µs per key — the same order of magnitude
as the round 29 baseline.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 29 review additions (2026-10-06, `tests/probe_round29{,b,c,d}.rs` added, +35 targeted probes)

Themes of this round: **tag object semantics, how macro recording logs
keys expanded by mappings, and the empty-pattern incsearch preview**. The
probe infrastructure was upgraded again: round 28's PTY per-key delivery was
still unreliable for macro recording (`q`) — `q` behaves specially inside
`:normal`/`-es` (the register content gets polluted by the command text), so
this round switched to real typeahead via `vim -s scriptfile` (a key-sequence
file + in-script `writefile` sampling), where `q` recording works perfectly.
**Lesson recorded: any empirical work involving macro recording, mappings or
replay must go through `-s` typeahead; conclusions from `-es` are void and
must be re-verified** — an early false negative of round 28 ("vim also stores
the expanded keys") came from exactly this.

### Semantics fixes (vim 9.1 typeahead probe evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **The tag object's inner range swallowed the closing tag** | With `<div><p>x</p></div>` and the cursor on x, `dit` produced `<div><p></div>` — the inner range's end used `close_end` (after the closing tag's `>`), deleting `</p>` along with it. vim gives `<div><p></p></div>`. The inner end in `tag_range` is now the closing tag's own `<` (`close_start`). The existing test only pinned the invariant (no panic), never the semantics — the semantic surface of `it`/`at` is now a permanent probe resident |
| 2 | **Tag object containment: climbing the wrong level on an opening tag** | With the cursor on the `<` of `<p>` (col 5), `dit`: the old engine deleted `<p>x</p>` (climbed to the div level); vim deletes `x` (taking p's own block). On the closing-tag side vim is the opposite: with the cursor on the `<` of `</p>` it takes the **outer** level. The verified rule is `open_start <= offset < close_end` (dit/dat agree across all 12 cursor positions), relaxing the old strict `open_start < offset` by one step. The "does not select on an opening tag's `<`" entry in NOTES divergence #9 was thereby **voided and deleted** |
| 3 | **Macro/redo recorded the keys expanded by mappings** | `:nmap x dd` + `qaxjyyq`: vim's register a = `"xjyy"` (the raw keys); the engine recorded the expanded `xdd…` (recorded key by key in the pipeline), so macro length was counted after expansion and `@a` replay re-executed the expansion (one dd became several). Fix: at a mapping Match, record the LHS raw keys; added the `expansion_keys_left` counter so expanded keys skip recording in the pipeline (the Unknown unrecord path now also goes through the `recorded` boolean, and the counter reset is centralised across noremap / runaway guards / builtin parsing paths). Separate probes for `2.`, `2@a`, `u`, Named-key mappings and nested expansion |
| 4 | **The incsearch empty-pattern preview published zero-width matches** | After `/foo`, `<BS>`×3 empties the prompt: the empty regex matches with zero width at every byte position, and the old implementation published tens of thousands of `i..i` ranges to the host for rendering. vim's preview falls back to the highlight set of the last accepted search. The empty-pattern branch of `publish_incsearch` now uses the same recovery logic as `cancel_cmdline` |

### Disproven during review, no change needed (saves a re-check next round)

- **Worry about `.` pollution during a macro recording session**: after
  `qaxix<Esc>q`, does `last_change` carry the `q`/`a` prefix? Disproven —
  the `end_command` at the tail of `complete_char_arg` clears `recording`
  already when recording starts (`mutated=false` takes the else branch), so
  `last_change` cleanly holds only `[i, Text, Esc]`. Pinned by probe: `.`
  replay neither restarts a recording session nor rewrites macro content.
- **`<C-a>` on `-0`**: when the result is non-negative the sign is dropped
  giving `1`; vim does the same.
- **No valid digit after a `0x`/`0b` prefix**: with the cursor on the prefix
  `0` it increments as decimal (`0x tail` → `1x tail`); with the cursor on
  `x` it reports E18 (the 0 is before the cursor) — agrees with vim.
- **`:0d`** deletes the first line; **`~` on an empty line** is a silent
  no-op (typeahead shows vim agrees); **`v$d` on an empty line** deletes the
  empty line itself (the visual selection covers the newline); **insert
  `<C-r>` pasting a linewise register** inserts the text verbatim at the
  insertion point (the register's trailing `\n` splits lines) — all
  consistent.
- **The `x` after `.`**: after the replayed insert the cursor sits on the
  first character and `x` deletes it — same cursor landing point as vim.
- **`f<CR>` failure**: does not overwrite last_find, but **does not
  rewind** (`;` fails and rings the bell if it cannot find one at the line
  end); the engine agrees with vim (round 28 disproved "failure restores the
  old target", and the probe added this round pins the bell count too).

### New known divergences (continuing the global numbering)

75. **`@` replay when an insert mapping expands into printable characters**:
    after `:imap z xy`, the `z` inside a macro stores the raw key (correct),
    but on replay the expanded `x`/`y` go through the host's text-placement
    path and `record_typed_text` records the text one more time (the
    `replay_records` flag of `@` replay is on). `.` replay is unaffected
    (recording is off). A rare combination (macro × insert typing mapping);
    hosts can also self-check in the placement path.
76. **None.** (All this round's candidates — `~` on an empty line, radix
    boundaries and so on — verified consistent, no divergence entry added)

### Refactors (zero behaviour change)

- The `pending_visual_marks` write-back block was verbatim-duplicated in
  `exit_insert` and `finish_visual_op` → `write_pending_visual_marks` (the
  floor + lo..hi normalisation consolidated in one place; the peek logic of
  the linewise adjusted branch inlined as `matches!`).
- The case-mapping closure was duplicated between `ops::apply` and
  `apply_block_operator` → `case_mapped_text` (the Line/Block paths now
  share the ß↔SS multi-character mapping rule).
- The whitespace-line stripping inlined in `insert_text_at_cursor` → reuses
  `strip_ai_line` (single implementation of the did_ai strip for CR-split
  lines).
- The `numeric_sort_key` function body and its doc comment were mis-indented
  at column 0 → indentation corrected.
- The "mappings off while replaying" comment at `RepeatChange` was stale and
  has been corrected — replay passes through the mapping table as vim's
  semantics require (the raw keys are recorded and expanded again on replay).

### Performance notes

No hot-path changes. The macro recording path does one extra
`expansion_keys_left` counter comparison per key (branch-predictor
friendly); `publish_incsearch` skips a full-buffer scan for the empty pattern
(previously it scanned even for an empty regex).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 28 review additions (2026-10-06, regression tests merged into
`tests/review_regressions.rs`, +20 cases)

Themes of this round: **the registers and the `$` full path of visual block +
register hygiene for empty operators + `:s` flag validation**. Probe
infrastructure upgraded: `vim -es` cannot run visual mode (feedkeys is
swallowed under silent Ex too), so this round switched to a real PTY driven
by expect (keys delivered one by one at 0.35s intervals — a batched send
drops keys, so first validate the keys actually arrive against an `lld` shape
before trusting the result). The behaviour surface of block delete / block
change / block `$` / ordinary block put was re-checked against vim 9.1
throughout, catching nine deviations; two of them (the block-put merge model
and `$` blocks) were systematic misreadings left by earlier rounds' probes
"compressing the wrong shape".

### Semantics fixes (vim 9.1 PTY probe evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **Empty linewise deletes polluted the registers** | On an empty buffer the span of `dd`/`Vd`/`dip`/`dap`/`dG` is 0..0 linewise, and the old guard only blocked charwise — the empty delete went into the numbered ring + unnamed, so a later `p` opened a ghost empty line; vim writes nothing at all (`:reg` shows only "%"). **Yank is the opposite rule**: `yy`/`yip` on an empty buffer must store an empty line (the probe's `:reg` has an `l ""` entry, and `p` opens a new line) — this deliberate asymmetry nearly got "unified while we're at it". The empty-span guard in `delete_span` is now unconditional |
| 2 | **Visual block `$` (`:h v_$`) was not implemented** | `$` should extend the block to each line's end, and the anchor column drops out of the picture: probe `<C-v>jll$d` ≡ `<C-v>j$d` (both delete down to `['','','def']`). The old engine treated `$` as an ordinary motion and deleted only to the column where the cursor line's `$` sits (`"g line here"`). Added `block_dollar` (armed by the LineEnd motion, disarmed by any other motion; cleared at enter_visual/exit/restore/click), `BlockSpan::dollar` makes the line = its whole content; effective through the full d/y/c/I/A/p/r chain, `A` skips the append-column padding and yank skips the rectangle padding (`$`+A appends at each line's own end, probes `long line hereX`/`abcX`) |
| 3 | **Visual block d/c did not write registers** | After vim's `<C-v>jlld`, `p` pastes the deleted block back (`"0` untouched); block `c` likewise (probes `Xab`/`Xef`). The old engine's block-delete arm went straight to `edit_delete`, so `p` pasted a stale register. Added `store_block_rows` (raw lines joined, Blockwise kind, an explicit register prefix `"a` still honoured) |
| 4 | **Block yank padded short lines** | vim's block register lines are **not padded** (`:reg` strtrans evidence `"long\0abc"`, the short line `abc` verbatim); the width lives in the register type — padding is the put side's job (`put_blockwise` was always doing it). Removed the pad loop on the yank side; `:reg`/`"+` content now matches vim |
| 5 | **Visual block p had no swap semantics** | vim's `v_p` is a swap: `<C-v>jy` + `<C-v>jp` and then one more `p` pastes back **the block that was just deleted** (probes `aij`/`ekl`). The block arm now stores the deleted block into the delete destination via `store_delete(None, …)` |
| 6 | **Visual p overwrote the explicit paste register** | After `viw"ap`, vim still has `aaa` in `"a` and the replaced text went to the unnamed register (probe); the engine passed the paste register into `delete_span`, so `"a` got overwritten by the deleted text. Visual PutReplace (including line-visual `V"ap`) now passes `None` |
| 7 | **Block put dropped overflow rows** | When the register has more rows than the selection, vim merges the extra rows into the **existing lines below** the selection (inserted at the same column) and only opens new lines past the buffer end (probe `cee`); the engine discarded them outright |
| 8 | **The "push down" model of ordinary block put was wrong** | vim **merges**: `2G0p` → `babbb`/`cbccc` (register rows land at the same column of the existing line below, short rows padded with spaces first), `2G0P` → `abbbb`/`bcccc`; only past the last line does it open new lines (`ghab`/`␣␣cd` — at the buffer end the old model coincides with the merge model, and round15's probes happened to sit exactly on the last line). `put_blockwise` rewritten as merge model + tail overflow; `insert_block_row_at_column` handles the append boundary when column == line width (`offset_for_display_column` has **cursor landing** semantics and clamps back to the last character at a full column — that is why pasting `X<row>` onto an `X` line gives `<row>X`) |
| 9 | **Unknown `:s` flags were silently ignored** | `:s/a/X/z` still substituted (vim: `E488: Trailing characters: z`, buffer untouched) — a mistyped flag quietly changed the text. Flag characters are validated ⊆ {g,i,I,n,c,e,l,p,#,&}, anything else raises E488; while at it the `e` flag was implemented (no match no longer reports E486, which is `e`'s actual purpose in vim; the `n` branch synchronised) |

### Disproven during review, no change needed (saves a re-check next round)

- **The repeat target after a failed `fx`**: the probe (`ft` `fz` `;`
  stopping on the original column) shows vim also records the **failed
  character** into lastc — the engine's "record first, then move" agrees and
  is not a bug (we nearly "fixed" it as "failure should restore the old
  target").
- **The copy direction of `3Oab<Esc>`**: vim probe
  `['ab','ab','ab','orig']`, cursor on line 3 — `O`'s count copies also stack
  downwards and the cursor stops on the last copy, agreeing with the engine.
- **`yy` storing an empty line on an empty buffer**: see fix #1 — the
  "apparent pollution" on the yank side is vim's own behaviour.
- **`:1,2y 0` (count 0)**: count 0 is treated as absent, range unchanged; vim
  agrees (no E488).
- **expect batched send dropping keys**: not an engine problem, but recorded —
  a PTY probe must deliver keys one by one and first calibrate delivery with
  a known shape (`lld` deleting three columns) before trusting later results.
  Two early `$` probes of this round produced the false negative "vim also
  deletes only one column" for this reason.

### New known divergences (continuing the global numbering)

73. **`:s`'s `l`/`p`/`#`/`&` flags are accepted but have no effect** (in vim:
    list display / print the last replaced line / print with line numbers /
    reuse the previous flags). The no-UI divergence for `c` is documented in an
    earlier round. Validation lets them through (friendlier than an E488 false
    alarm); the effect side is for later.
74. **`gv` re-selection after visual block `$`**: `gv` restores via the
    pre-edit byte pair (line start to line start), and the `$` state itself
    does not survive with `'<`/`'>` — vim's gv re-selects a fixed-width block.
    Extremely rare (`$` block → operator → gv).

### Refactors (zero behaviour change)

- The count re-anchor triple in four places of `ex d/y/j/s` (first=last,
  last+=count-1, clamp both ends) → `reanchor_range`.
- The four identical cursor-anchored linewise spans in operator doubling / `S`
  / `Y` / the `guu` family → `linewise_span_at_cursor`.
- The deleted-row collection in `block_put_replace` reuses
  `store_block_rows`.
- The `Marks::for_each_pos` / `for_each_pos_clamped` double entry merged (the
  latter was just an `&mut` forwarding shell for the former).

### Performance notes

No hot-path changes: block yank does one less per-line padding (a small saving),
block delete collects one more pass of line text (O(block rows), same order of
magnitude); `:s` gains a flag-character scan (O(flag length)).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 27 review additions (2026-10-06, `tests/fuzz_round26.rs` added
(the two host-history-axis invariants, a second-scale permanent resident) + `tests/review_regressions.rs` (+10 cases))


Theme of this round: **the host history axis (undo/redo group ownership)**.
The engine delegates history to the host (`begin_undo_group`/`undo`/`redo`)
and only announces groups itself inside the `edit_*` funnels — the previous 25
fuzz rounds all checked state legality after a single editing step, and nobody
had ever taken a random key sequence and hammered `u` all the way to the
bottom. This round adds the two invariants only catchable that way:

  1. **Reversibility**: undo to the bottom then redo to the bottom returns the
     text byte for byte to its pre-undo state;
  2. **Completeness**: undoing to the bottom must equal the **initial
     buffer** — catches defects of the kind "some editing path forgot to go
     through the `edit_*` funnel and never entered any undo group" (an
     unopened edit is unundoable and gets lost silently between two `u`).

The fuzz_round26 key table mixes in IME composition replacements / host clicks
(the owns_group seam of `replace_range`), 24 seeds × 40 rounds × 60-200 steps,
0.2s under opt3. **Both invariants pass** — the history axis survived the
bombardment.

### Semantics fixes (vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **`gv` collapsed to zero width after a visual operator** | The operator of `vlld` pushed its own selection byte pair through `adjust_delete`, endpoints folding into the deletion point → `last_visual = (0,0)`; `gvd` became a silent no-op and even undo could not save it. vim's `'<`/`'>` are **row/column coordinates**: after `gg0vllld` they stay (1,1)-(1,3) — on the post-delete line `gv` re-selects the same-width span at the line start ("lo "), and after undo the same coordinate pair lands on the restored text so `gv` re-selects the original region (probe gvy evidence). Fix: extend the `pending_visual_marks` staging that the `c` path already had to **all operators** (d/y/~/r/p/block d/block y/block p), capturing the PRE-EDIT boundaries at entry and dispatching by category in `finish_visual_op` — char/block write the staged value (floored into the post-edit text), linewise keeps the current adjusted boundaries (vim's line delete folds the marks onto the following line, so round 7's `Vjd`→`:'<,'>d` semantics are unchanged). Along the way `resolve` gained a buf parameter: `'<`/`'>` floor to a character boundary when read |

### Disproven during review, no change needed (saves a re-check next round)

- **The illusion that `2u` "undoes only one step"**: after three `x` the text
  is `\nbbb…`, and going from there to `aa…` in one `2u` is two steps — the
  probe expectation had the snapshot order counted backwards. `3u` for three
  steps and `5u` undoing to the bottom with exactly one bell both agree with
  vim (the Undo arm does `for _ in 0..count` + a break at the edge).
- **The illusion that `u` does not ring the bell at the history edge**: the
  roundtrip probe once never terminated, but the cause was multi-character
  Named junk keys in the probe's key table (`"dd"` fed whole as a single key)
  leaving the engine at the cmdline prompt — every later `u` went into the
  prompt. The engine does ring the bell at the history edge (`history_step`
  returning false → bell + break).
- **The illusion that `i1<C-o>x2<Esc>` does nothing at all**: same cause —
  insert printable characters are handed to the host for placement per the
  contract (`Fixture::feed` does not simulate it), so the key stream only
  delivers control keys. `<C-o>` in insert mode is handed back to the host as
  `Unknown` (see divergence 72).
- **The `"0` semantics of `:y`**: `ex_yank_lines` goes through the same funnel
  as `ops::yank_span`, following the anonymous + `"0` double write, consistent
  with `yy`.
- **`g;` after `u`**: the changelist is shifted via
  `sanitize_stored_offsets`, landing correctly.
- **Fidelity of cursor restoration on undo**: the engine only announces
  `begin_undo_group(id, cursor)` at the first edit point as a hint; the host
  may (and should) store its own cursor per group — the harness using the
  hinted value already yields vim behaviour (`o<Esc>u` and friends are covered
  by existing parity tests).
- **`.` after `u`**: `last_change` is not touched by u, so `.` replays the
  undone change (same in vim).
- **`qA` append recording**: seeds from the existing content and lands back in
  the lowercase slot, `@a` replays the merged body.
- **The plan of storing `last_visual` as (line, column) was rejected**: halfway
  through the implementation it became clear that freezing row/column breaks
  the line-insert shift vim also has (`vll y` then `ggo<Esc>`; vim's lnum
  moves down with the inserted line, equivalent to the current byte adjustment
  — that is exactly what engine.rs's shift test pins). Byte storage + PRE-EDIT
  staging on the write side is the smaller correct fix (see fix table #1), and
  `resolve`'s buf parameter is kept as read-side defence.

### New known divergences (continuing the global numbering)

70. **Normal-mode `U` (line-wise undo) not implemented**: the host owns a
    linear undo stack, and rewinding a line requires a history lookup of "undo
    back to before that line's most recent change", which a linear stack cannot
    give — a cooperative host API is needed (e.g. `undo_line()`). Currently it
    rings the bell and leaves the cursor. Visual `U` (the uppercase operator) is
    unaffected.
71. **Undo tree / time navigation unsupported**: `g-`, `g+`, `:undo`,
    `:earlier`, `:later`, `:undolist` all E492/bell. Same family: history is
    delegated to the host's linear stack, so tree branching and timestamp
    navigation are out of the question. Integrators who need it should implement
    it on the host side (gpui-vim's undo tree is the precedent).
72. **Insert-mode `<C-o>` (temporary normal) not modelled**: same contract as
    printable characters — handed back to the host as `Unknown`, and the host may
    claim it. Implementing it engine-side would need the current insert session's
    undo group split and reopened (in vim's semantics a C-o command is its own
    block); the payoff/risk ratio does not justify it for now.

### Documentation corrections

- The "`:sort` unsupported" claim in the main divergence table #12 was stale
  (round 23 landed `ex_sort`, including E475/numeric sort/`u` dedup), so it was
  moved out of the "unsupported" list; `:g`/`:normal`/functions/autocmd stay.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 26 review additions (2026-10-05, regression tests merged into
`tests/review_regressions.rs`, +2 cases, 1 assertion updated)

Theme of this round: **a full-module code review pass** (no new fuzz round —
after twenty-five fix rounds this round switched to a reading perspective and
went file by file: buffer/key/keymap/marks/registers/search/options/mode/host/
word/motions/objects/ops/insert_mode/state/cmdline/tables/tck), clearing items
against the existing divergence list.

### Semantics fixes (found by reading the code + property-test evidence; the cluster×line boundary continues round24/25 semantics)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **A line-start / buffer-start ZWJ glues the following starter (prev/last mirror drift)** | `prev_grapheme_offset`'s "a starter's preceding ZWJ glues" rule did not check whether the ZWJ itself is glued: a ZWJ after `\n` or at the buffer start forms its own cluster in the forward scan, while the backward pass glued the starter onto it — the two directions gave **different cluster boundaries** for the same position (forward: `[ZWJ) [b)`; backward: `[ZWJ b)`). Visible consequence: on a `"ab\n\u{200D}b"` line, `$` parked the cursor on the invisible ZWJ (offset 3) and `x` deleted only the joiner, leaving a bare base character (after the fix `$` lands on b and `x` deletes b). `last_grapheme_start` (the `&str` mirror, the byte ledger of block-insert BS) had the same rule and got the same fix |

New **mirror-consistency property test**: for every cluster `[s,e)` cut by the
forward scan, backing up from `e` must return `s`; `last_grapheme_start(whole
string)` agrees with the buffer-side backward pass — nine adversarial corpora
(standalone mark / ZWJ hugging a line boundary, double ZWJ, emoji families
spanning lines, a lone ZWJ buffer). The old `zwj_clusters_never_span_newlines`
had one assertion freezing the pre-fix behaviour (a line-start ZWJ "forms its
own cluster" yet also glued the starter into it, contradicting the forward
pass); updated to the corrected invariant.

### Disproven during review, no change needed (saves a re-check next round)

- **`:set ts/sw=4` not vim's factory values** (ts=8/sw=8): a deliberate
  editor-friendly default (aligned in two places, options.rs `Default` +
  `VALUE_DEFAULTS`, with `ts&` resetting consistently); `>>` on `\t` indentation
  yielding 8 spaces is self-consistent with the engine's own default. Documented
  divergence surface (same family as divergence #2).
- **`prev_paragraph` not checking line 0**: the `line_start(0)` fallback is
  value-for-value identical to "stop on it if line 0 is empty", not a bug.
- **`:sort u` deduplicating before reversing**: the `!` flag means "reverse the
  comparison direction", deduplication is the same under both directions, so
  `dedup` then `reverse` is value-equivalent to vim's "sort descending, then
  dedup".
- **The double-ZWJ chain in `last_grapheme_start`**: in `"\nZWJZWJy"` the second
  ZWJ is glued into the first ZWJ's cluster (the forward pass likewise gives
  [1,8) as one cluster) — mirror-consistent after the fix, not a separate issue.
- **The out-of-bounds read in `match_bracket`'s `while o <= line_end`**: once
  `next_char_offset` passes the line end, `o > line_end` exits; at `\n`,
  `char_at` returns `Some('\n')` which is not a bracket, so no mis-pairing.
- **The `0b2` case in `increment_number_at_cursor`**: the `b` prefix-letter
  check requires a legal first digit (bin wants 0/1), so with `0b2` and the
  cursor on `b` the prefix arm does not fire and it falls back to decimal digit
  search — matching the vim probe the comment claims.
- **Macro/`.`/@ interleaving, `@:`, the redo register**: read through the shape
  matching of `bump_redo_register` (leading count, `"`, 1-8, trailing p/P);
  consistent with round15's probe semantics.

### Refactors (zero behaviour change)

- The E35/E486 miss branch of `cmdline::jump_to_current_match` was
  verbatim-duplicated in `state::report_search_miss` → reused.
- The shared body of the Undo/Redo arms in `execute_normal_cmd` (generation
  bump + refloor of stored offsets + cursor clamping + desired_col clearing) →
  extracted `history_step`.

### Documentation ledger cleared

- Divergence #10 (the `\\"` escaped-quote misjudgement) is marked fixed — the
  code switched to a backslash-run mod 2 rule in round 14 (round14 fix table
  #5); the summary table had simply not been updated.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 25 review additions (2026-10-05, regression tests merged into
`tests/review_regressions.rs` (+8 cases) + `tests/fuzz_round25.rs` added
(GB4 line-boundary×grapheme-cluster corpus hammered with all keys, carrying one
new engine invariant))

Theme of this round: **grapheme clusters × line boundaries (the UAX #29 GB4 hard
boundary)**. round24 established the "cursor never stops on a width-0 character"
invariant; this round fuzz_round25 adds a second one —

  **the normal-mode cursor never stops on `\n`** (except at the start of an
  empty line — an empty line has no character to stop on, and its line start is
  exactly the `\n` position).

The corpus deliberately targets shapes round24 did not have: standalone
mark/ZWJ in every arrangement hugging `\n` (line start / line end / alone on a
line / space followed by a mark). The two invariants together caught nine paths
(two of them reading candidates, seven pure fuzz-bombardment products), and a
delta-debugging minimizer (a throwaway script) shrank a 67-step sequence to 1-4
keys. After the stress version (1200 rounds × 150 steps × 14 corpora ≈ 2.5M
keys) passed, it converged to a 60-round permanent resident.

### Semantics fixes (fuzz-minimized evidence; no vim probe — the cluster×line boundary has no counterpart in vim's byte-level model, and the engine adopts the stronger self-consistency constraint of GB4 semantics, following the round24 precedent)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **`w`'s whitespace skip did not absorb continuation characters** | On `a \u{0301}bc`, `w` stopped on the mark (width 0) — round24 fixed `b` (the `is_cont` arm of `prev_word_start`) but missed the forward `w`. The mark adheres to the space and travels with it when whitespace is skipped (read + probe) |
| 2 | **Grapheme backtracking glued across `\n` (GB4)** | `prev/next_grapheme_offset` and `last_grapheme_start` glued a line-start standalone mark/ZWJ into `\n`'s cluster — `\n` is a hard boundary (GB4/GB5 outrank ZWJ/Extend joins). Consequence: after `w` reached the buffer end, clamp_cursor parked the cursor on the **previous line**'s newline (`x` joined lines). Probe: `prev_grapheme_offset(5) = 2` on `"ab\n\u{0301}"` |
| 3 | **insert `<Del>` deleted only the base character** | On `e\u{0301}`, Del left a bare mark (which then composed onto the preceding character) — round24's cluster semantics for BS were not mirrored into Del. Switched to whole-cluster deletion via `next_grapheme_offset` |
| 4 | **Multibyte `r` replacement underflowed on zero consumption** | When a ZWJ glued `\n`, advance_graphemes consumed 0 clusters and `start + 0 - ch.len_utf8()` underflowed usize — a debug panic (`r中` on the b of `"ab\u{200D}\ncd"`, the probe reproduced the panic). Fix: return early on zero cluster consumption + #2 removes the root cause |
| 5 | **Sentence objects collapsed to 0 when starting from a multibyte cursor character** | `(offset + 1)` in `sentence_range` landed inside a CJK cursor character, `prev_char_offset` returned None → `prev_sentence` fell to 0 — `dis` on `"Hi. 你好 ok"` with the cursor on 你 deleted "Hi." instead of its own sentence (probe evidence). Switched to taking a character boundary with `next_grapheme_offset` |
| 6 | **exit_insert's step-back landed on `\n`** | Under the folded-line model (the phantom-line folding of harness/tck), `ddo<Esc>` on `"ab\n\u{0301}"` stopped the cursor on the newline (offset_to_line attributed the last buffer position to the previous line, and the step-back guard misjudged). Fix: abandon the step-back when the candidate point is `\n`, preferring the last buffer position (= the start of the trailing empty line in vim's model) |
| 7 | **insert `<C-w>` joined lines and deleted an arbitrary byte** | When the cursor word's run started on the previous line (the line-start mark glued shape), the join branch deleted `at-1..at` = **any byte** before the cursor — a multibyte tail panics the host (fuzz evidence on `"b\n\u{301}d\n"`), ASCII deletes the wrong character without joining. Fix: delete the newline at `line_start-1` |
| 8 | **`class_at`'s backtracking had no GB4 stop point** | A line-start standalone mark glued to `\n` was classified Blank (classify('\n')) — contradicting its own documented "forms a degenerate run", so behaviour drifted (`e` on a mark degenerating etc.). Fix: stop immediately when backtracking hits `\n` |
| 9 | **J/gJ seams landing on `\n`/inside a cluster + cluster snapping of landing points consolidated** | When the next line is empty, `J` stopped at join_at = the leftover `\n`; the gJ seam stopped inside a cluster when exposing a mark; `2/\|<CR>`'s zero-width-pattern jump and `gv`'s stale endpoints had the same disease. Consolidation: `clamp_cursor` gained intra-cluster snapping (new helper `grapheme_cluster_start` — "the start of the cluster containing offset", as distinct from `prev_grapheme_offset`'s "the previous cluster"), covering all four landing points automatically |

### Disproven during review, no change needed (saves a re-check next round)

- **The whitespace classification of NBSP (U+00A0)** (round17 open item, settled
  with a vim 9.1 `-es` probe): for `w`/`daw` the engine agrees with vim (NBSP
  counts as whitespace and is skipped). vim itself is **inconsistent** — the `iW`
  object and `g_` treat NBSP as content (`yiW` takes the NBSP itself, `g_` stops
  on a trailing NBSP) while `w` skips it (mbyte.c's utf_class vs the object/`g_`
  checks are two different rules). The engine's single `is_whitespace` is
  self-consistent and matches the most-used `w` surface, so we do not chase it —
  see divergence 68.
- **The clamp mismatch of `replicate_count_insert`** (round17 open item):
  confirmed fixed by reading the code (clamp FIRST, both legs share the clamped
  copies), closed.
- **The Find/JumpMark arms of `complete_char_arg`** (round17 open item): the two
  arms now genuinely differ (Find records last_find, JumpMark reports E20), only
  the tails look similar, so not merged.
- **offset_to_line's folded-line model vs exit_insert**: see #6 — the engine
  being robust to both host line models is an existing contract (the tck phantom
  line tolerance); this round filled in exit_insert.
- **`line_count` folding `ab\n`** (the "no movement" illusion for `j`/`G` in the
  probes): the harness host's policy (a trailing `\n` folds the phantom line),
  not an engine bug; the engine only depends on the trait.

### New known divergences (continuing the global numbering)

68. **NBSP's object/g_ semantics**: vim has two whitespace checks internally
    (motion utf_class vs the `iW` object/`g_`), where NBSP is whitespace for `w`
    but content for `iW`/`g_`. The engine treats it uniformly as whitespace
    (is_whitespace), so `yiW` on an NBSP takes the next word rather than the
    NBSP itself. Rare surface (NBSP + cursor exactly on the NBSP + the W object).
69. **The cursor of `o<Esc>` under the folded-line model**: vim's line model
    stops at the start of the new empty line; a folded-model host sees the last
    buffer position (`cur == len`). The engine prioritises "never stop on `\n`"
    (#6), and `x` at that position is safe (a no-op).

### Performance notes

- bench_probe is level with the round23/24 baseline across the board: `w`
  6.3µs/key, 1MB single-line `w` 103µs, hlsearch rescan 0.25ms, `:%s` 5.3ms,
  `n` 3.8µs. The new hot-path branch this round is the cluster check in
  `clamp_cursor` (one width-table lookup; backtracking only for continuation
  characters) — text without marks/ZWJ pays zero extra, and the measurement is
  level.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 24 review additions (2026-10-04, regression tests merged into
`tests/review_regressions.rs` (+17 cases) + `tests/fuzz_round24.rs` added
(grapheme-cluster corpus hammered with all keys + visual/macro interleaving, carrying one new engine invariant))

Theme of this round: **the grapheme cluster across the whole chain**.
fuzz_round24 first wrote "the normal-mode cursor never lands on a
width-0 continuation character (combining char / VS16 / ZWJ)" as an executable
invariant — a width-0 character is a cluster continuation, so a cursor parked
there makes `x` split the cluster (deleting the base character and leaving a
bare mark, or vice versa). Around this invariant nine paths were fixed, all
verified through "write the probe first → minimize → locate in the code" (no vim
probe: vim's byte-level model does allow transient intermediate states around
composed characters, while the engine adopts the stronger self-consistency
constraint of "cluster atomicity").

### Semantics fixes (probe + fuzz-minimization evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **`prev_grapheme_offset` backtracked without looking at the character at the candidate position** | The old implementation checked the character *before* the candidate position to decide whether to keep going, so a trailing combining character (the `"e\u{0301}"` line end) returned the mark's own offset. Rewritten: if the character *at* the candidate position is width 0 / ZWJ, the cluster extends leftwards and backtracking continues; if the start character's predecessor is a ZWJ, glue it (family emoji mirroring the forward scan) |
| 2 | **The Esc step-back landed inside a cluster** | `exit_insert` used `prev_char_offset` to step back one slot: after typing `a\u{0301}`, Esc left the cursor on the mark and `x` deleted only the mark, leaving a bare `a`. Switched to `prev_grapheme_offset` (vim's delcombine=off default: a composed character is the deletion unit) |
| 3 | **The line-end landing points of `$`/`j`/`\|`/`g_` had the same disease** | `clamp_cursor`/`offset_for_display_column`/`Motion::LineEnd` all used `prev_char_offset` to take "the last character of the line", and `g_`'s non-blank scan accepted a mark as a landing point. All switched to the cluster start |
| 4 | **charwise `p` inserted text into the middle of a cluster** | "Insert after cursor" used `next_char_offset`, advancing one slot: pasting on `#`+VS16 put the text *between* the base character and the VS16 (`#n\u{fe0f}`) — the cluster was cleaved. Switched to `next_grapheme_offset`. The three `prev_char_offset` sites (visual paste landing point / visual object cursor / `gn` cursor) fixed the same way |
| 5 | **The `w`/`b`/`e` word scans were cluster-blind** | `class_at` classified a combining character as Punct: `w` on `"b\u{0301}"` stopped at the mark; `e`'s run end (`next_word_end`) landed on the mark; `b` treated the mark as the start of the previous run; `ge` reported the mark as a run endpoint. Fixes: continuation characters inherit their base character's class in `class_at` (`"base+marks"` is one unit); `e`'s landing point pulled back to the cluster start (`pull_off_continuation`); `b`/`w`'s whitespace skip merges marks into their whitespace cluster; `ge`'s backward scan skips continuation characters |
| 6 | **Block delete orphaned marks** | The unit tail in `block_row_range` covered only the base character: block `d` deleted `a` and left U+0301, which then composed onto the preceding character. The unit tail now extends past trailing continuation characters |
| 7 | **Visual `r` padded by character count** | The padding count for `中文ab`→`----` was computed in chars, so a cluster (base+mark) counted as two. Added `buffer::grapheme_count` counting clusters (vim semantics: a composed character is one replacement unit) |
| 8 | **`*` searched out a mark on a whitespace-only line** | The forward scan for "next non-blank" used is_whitespace: a mark is not whitespace → it searched out an invisible character and parked the cursor there. The forward scan now skips continuation characters |
| 9 | **`^`/`gg` stopped on a line-start mark** | Both `first_non_blank`'s landing point and its whitespace-line fallback worked per character, so a line-start mark (`\u{0301}abc`) or a mark after indentation was stopped on directly. The landing point now skips width 0; the whitespace-line fallback first strips the trailing continuation run and then takes the last visible character |

### Supporting infrastructure

- `buffer::last_grapheme_start(&str)`: the `&str` mirror of
  `prev_grapheme_offset`, used by the `block.text` byte ledger of block-insert
  sessions to trim by cluster (when BS deletes a whole cluster,
  `typed_end`/`text` must shrink in sync so that the exit-side
  delta==text.len() copy invariant still holds).
- `tests/fuzz_round24.rs`: 11 cluster corpora (decomposed combining
  characters / VS16 / ZWJ families / skin-tone modifiers / full-width space /
  line-start mark) × 100-120 steps of key bombardment + visual/macro
  interleaving. The single legal exception: a standalone mark whose base
  character was deleted (its own cluster with no base in the line to attach to,
  allowed only in its line-start form). The delta-debugging minimizer (a
  throwaway script, not committed) shrank a 96-step failing sequence to the 4
  keys `v i w y`, pointing straight at the visual object cursor landing point.

### Disproven during review, no change needed (saves a re-check next round)

- **`dw`/`cw`/`diw`/`J`/`:s` (including cluster patterns and cluster
  replacement strings)/`R`/`~`/block-insert CJK alignment/`gq` CJK
  reflow** — the targeted probes all matched expectations (clusters take part in
  operators and Ex as whole units). The bizarre `:s` output in the first probe
  batch was the probe itself forgetting to feed `:` (`%s/ab/XY` was eaten in
  normal mode: `s` deleted a character and entered insert, `<CR>` made a
  newline), not an engine problem.
- **`Put`'s block register / linewise branches**, `replace_overwritten` stack
  (Replace mode BS mirrors its own single-character records, cluster semantics
  self-consistent).
- **Zero performance regression**: bench_probe level with the round23 baseline
  item by item (w 6.4µs/key, 1MB single-line w 163µs, hlsearch rescan 0.35ms,
  `:%s` 6.6ms, `n` 3.7µs).
  `class_at`'s base-character backtracking only triggers when continuation
  characters are present, so mark-free text keeps the original fast path.

### New known divergences (continuing the global numbering)

66. **Grapheme-cluster semantics are overall stronger than vim's byte-level
    model**: vim (without `delcombine`) allows byte-level transient intermediate
    states inside composed characters (`ta` can park the cursor *inside* a
    composed character); the engine always clamps to the cluster start. The
    user-visible difference is limited to rare arrangements such as "line-end
    combining characters / ZWJ families", and the engine's behaviour (atomic
    clusters) matches rendering intuition better.
67. **A mark-only line (only `\u{0301}` left after the base character was
    deleted)**: `w`'s whitespace skip passes over it and lands on the next
    visible character, and `^`'s landing point skips it too — vim's byte-level
    model would stop on the mark. Stopping on an invisible character has no
    rendering meaning, so the engine takes the visible-semantic rule.

### Performance notes

- The only new hot-path branch this round is `class_at`'s continuation-character
  check (one width-table lookup) and `is_cont` (same line); text without
  mark/ZWJ pays zero extra. bench_probe level with the round23 baseline (see
  above).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 23 review additions (2026-10-04, regression tests merged into
`tests/review_regressions.rs` (+7 cases) + `tests/fuzz_round23.rs` added
(the hostile-input surface of `:set`/`:sort` arguments + random interleaving of host events (IME commit/undo/macro replay) + the `is_idle`×mode contract))

Process of this round: read all of `src/` (key shapes → prompts → cmdline →
editing paths → host event surface) → settled the two hostile-input probe
batches (`-es` scripts + Python PTY byte level) → fixes. New tooling shape:
`pty.fork` with raw keys fed in, replacing the `:normal!`/execute probes (the
latter get polluted by Ex parsing under `-es`; `m<CR>` once falsely reported
mode=c).

### Semantics fixes (probe + vim 9.1 evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **IME commits split the insert session's undo group (data-loss severity)** | `replace_range` (the IME composition commit path) unconditionally called `end_edit()` — every commit mid-session became its own group, so `i<Esc>u` undid only the last commit (vim keeps one undo block for the whole session). Fix: close the group only when this call opened it itself; REPL-style out-of-session replacements still open and close on their own and never hang |
| 2 | **`floor_to_char_boundary` treated the buffer end as mid-character** | `char_at(len)` returning None was taken as "not a boundary" and floored to the last character's start — a host `replace_range(5..7)` on a 7-byte buffer only edited 5..6 and swallowed the last character. Fix: `offset == len` is always a boundary and returns early; the floor contract in parity_round5 updated to the new semantics |
| 3 | **Enter was unconditionally mapped to `\n` for every char-arg command** | `f<CR>` failed yet overwrote `last_find` (the next `;` searched for a newline — vim does not update on failure); `'<CR>` produced E20 text containing a bare newline. PTY evidence: vim consumes Enter only for `r<CR>` (splitting the line) and `vllr<CR>` (splitting per character). Fix: map it only for Replace/VisualReplace, everything else takes the failure path (bell + `last_find` preserved) |
| 4 | **`:set ts=` reported E518 (Unknown option)** | vim 9.1 `-es` evidence: `E521: Number required after =: ts=`. A known name with an illegal value goes to E521; E518 stays reserved for unknown option names |
| 5 | **`:sort z` sorted silently** | PTY evidence: vim reports `E475: Invalid argument: z` and leaves the buffer alone. Fix: flag whitelist `!iun`, everything else (including vim's valid but unimplemented `x/o/b/f/l`) rejected with E475 — a loud divergence beats a silent mis-sort |
| 6 | **`:sort n` (numeric sort) was missing** | New implementation: the first decimal number in the line is the key (a leading `-` counts as the sign), lines without a number count as 0, stable (equal values keep the original order), and `u` dedups by sort key (vim's uniq shares the sort comparator) |
| 7 | **`is_idle` returned true while a prompt was open** | Hosts use it as the key-interception gate (VimEdit local undo), so literal text inside the prompt (the `u` of `:ru`) got hijacked. Fix: always false in cmdline modes; insert still counts as idle (text input is the host's job) |
| 8 | **The generic `'x` mark branch lacked the `'>` exclusive-end correction** | The exact `'>` branch resolves `line(off-1)`, while a mangled token (`'<,'>'<,'>` double prefill) took the generic branch and landed on the next line. Along the way: the E20 text now quotes the mark name parsed from the token rather than the garbage tail of the raw token |

### UX improvements

- The error semantics of `:set`/`:sort` moved back from "wrong class" to vim's
  two error codes (E521/E475), so the host status bar's report no longer
  misleads (an illegal value ≠ a nonexistent option).
- Illegal mark/find arguments such as `m<CR>` now ring the bell instead of being
  silently ignored (same shape as failed `r` arguments).
- The trailing-whitespace normalisation in the prompt's C-w became `trim_end()`
  (a half-finished item from a stash; Tab never actually reached the prompt
  buffer — the comment was rewritten to the genuinely reachable surface).

### Disproven during review, no change needed (saves a re-check next round)

- **The counting semantics of `3/pat`**: the engine's `N/pat ≡ Nn` (wrap
  included: on a 3-match buffer `3/foo` falls back to the 1st match) — vim 9.1
  PTY agreed across all seven shapes, and round14's implementation was already
  right.
- **The register of `dd` on an empty buffer / empty line**: the engine stores an
  empty linewise entry — vim too (the unchanged SENTINEL in the first probe
  batch was the illusion of a 0-line buffer; on `['']`, dd leaves getreg as
  `"\n"`, rendered by strtrans as `^@`). Register rotation happens as well.
- **`v r<CR>`**: vim replaces each selected character with one newline (PTY:
  `vllr<CR>` on "abcdef" → three empty lines + "def") — the engine's per-character
  repeat agrees.
- **`S`/`cc` entering insert on an empty buffer, and `:d` ringing the bell on an
  empty buffer** — both agree with vim.
- **The huge count macro `9999@x`**: the pipeline guard converges within 1.3ms,
  no hang.

### New known divergences (continuing the global numbering)

64. **IME commits (`replace_range`) are not recorded for `.` replay**: a
    host-driven raw replacement has no key/text form to record — the replay
    contains only the typed Text steps (probe: the session landed "hello阿", and
    `.` replay gives "helloab"). vim replays the final landed text form.
65. **`:sort x/o/b/f/l` (hex/octal/binary/float/locale) rejected with E475**: vim
    implements these flags; the engine rejects them loudly rather than silently
    mis-sorting lexicographically.

### Performance notes

- bench_probe rerun + **same-machine control against the baseline commit
  (e96d0bb)**: `w` 6.4µs/key vs baseline 6.2µs, 1MB single-line `w` 167µs vs
  163µs, hlsearch rescan 0.30-0.35ms, `:%s` 6.1ms, `n` 3.6µs — the deviation
  from round22's record (2.3µs/103µs) shows up on the baseline too, so it is
  machine-state drift rather than a regression this round (the only
  hot-path-adjacent change this round is the early return in
  `floor_to_char_boundary`, removing work rather than adding it).


## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 22 review additions (2026-10-04, regression tests merged into
`tests/review_regressions.rs` (+8 cases) + `tests/fuzz_round21.rs` added
(`:s` replacement-string escape surface × 6 buffers × 9 patterns' line-addressing contract + CJK search-motion targeting + NUL addressing + `&` replay single-shot)

Process of this round: read all of `src/` (key shapes → prompts → cmdline →
editing paths) → settle the hostile-input probes → vim 9.1 `-es` byte-level
evidence → fixes. New tooling shape: `vim -es` script mode replacing
interactive PTY (`printf script | vim -N -es -u rc` + `writefile` capturing
v:errmsg/cursor/buffer bytes), which is more stable than the PTY for evidencing
the Ex semantics surface.

### Semantics fixes (probe + vim 9.1 evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **The `<C-Space>` mapping hijacked the plain space key (data-corruption severity)** | `parse_angle`'s `space`/`lt`/`bar` branches unconditionally did `Key::char(..)` and dropped modifiers — `:nmap <C-Space> ix` got stored as a plain space, so pressing space triggered it. vim's `:nmap` listing evidences that `<C-Space>` and `<Space>` are two keys. Fix: modifiers travel with the key |
| 2 | **Single-character Named ctrl keys were dead keys** | `Key::ctrl("w")` (the gpui shape) was not equivalent to `Key::ctrl_char('w')` (the parse shape) — insert's `<C-w>` chord and the whole ctrl command table only recognised the Char form, so the Named form silently became Unknown. Fix: `handle_key` collapses single-character Named keys into Char (modifiers preserved); incidentally letting ctrl+Named("space") hit a `<C-Space>` mapping |
| 3 | **`3/pat` prompt huge count overflowed and panicked** | The prompt's `search_count` was the only count entry bypassing `take_total_count`'s one-billion cap; twenty 9s then `/pat<CR>` overflowed the u64 addition in `jump_to_match` — a debug panic (silent wrong jump in release). Fix: take the step count modulo the match count first, so the sum can never go out of range |
| 4 | **`\r`/`\n` in `:s` replacement strings passed through literally** | vim 9.1 byte-level probe: `\r` is a newline (the `%s/,/,\r/g` idiom), `\n` is NUL (`fX\x00Yo bar`); the engine wrote both into the buffer as literals. Fix: `unescape_replacement` gained the two mappings, `\\r` stays literal |
| 5 | **The cursor landing point of line-splitting replacements** | After `\r` landed, the cursor was still parked using the **pre-split** line numbers. vim evidenced three shapes: `%s/a/\r/` on "aaa\nbbb" lands on the "aa" line, the g-form lands on line 4, `2s/a/\r/g` lands on the last line of the split region. Fix: shift down by the number of new lines inside the replacement slot |
| 6 | **The range stored for `&` / bare `:s` replay** | state.rs's old comment asserted "ranges behave as typed" — vim 9.1 probes overturned it: `:1s/a/B/` + `+` + `&` acts on **line 2**, not the stored line 1. The old engine's `&` retyped absolute line numbers, and a bare `:s` gave E492 outright once a range had been stored (`strip_prefix('s')` cannot get through "1,2"). Fix: strip the range prefix before replay (the typed range still applies) |
| 7 | **Trailing digits in `:sort` were silently accepted** | vim 9.1: `:1,3sort 3` gives E488 and leaves the buffer alone; the engine sorted silently. Fix: digits anywhere in args → E488 and refuse to execute |

### UX improvements

- `:set` unknown options changed from a mute bell to **E518: Unknown option:
  {name}** (wording evidenced by vim; subsequent items are not executed).
- `:s` illegal patterns changed from a mute bell to publishing the compilation
  error text (not impersonating vim's E486 — the legacy engine tolerates an
  unterminated `[`, and the RE2 dialect can only honestly report a compile
  failure; refines divergence #1).
- The `DOT_TEXT_MARKER` comparison on the `.` replay hot path moved from
  `KeyKind::Named(String construction)` to `matches!` — one String allocation
  per key removed from the pipeline.

### Comment/format corrections (guarding against future mis-fixes)

- The old `GoToFilePercent` comment in `motions.rs` claimed "the percentage
  formula needs the original count" — by construction that variant only carries
  typed-1 and the formula never looks at count.
- The misaligned indentation of `ex_join` in `cmdline.rs` (a legacy leftover);
  round21/regression code aligned to rustfmt. Deliberately **not** running fmt
  repo-wide: 20+ rounds of hand-set style, and a repo-wide churn would drown
  blame.

### Disproven during review, no change needed (saves a re-check next round)

- **f/t arguments go through the mapping table**: while `f` waits for its
  argument, a `:nmap x l` mapping still expands — vim does too ("mapping `;`
  breaks `f;`" is community common knowledge), so no no_remap is needed.
- **The existing `&` regression tests** (`ampersand_repeat_drops_substitute_flags`
  and friends) all store rangeless commands, so range stripping never touches
  them.
- **An explicit `"1dd` does not rotate the numeric ring** (`store('1')` writes
  directly) — vim same.
- **The in-line byte indexing of `increment_number_at_cursor`**: everything is
  based on line-slice relative offsets, so multibyte lines are safe (radix
  ambiguities like `0b2`+C-a are as deep as vim's internal rules, already pinned
  by a comment and not chased).

### New known divergences (continuing the global numbering)

62. **The `<S-Space>` mapping form is still dead**: on the delivery side
    shift+space is normalised to a plain space (same family as `<S-1>`, which is
    keyboard-layout dependent). `<C-Space>`/`<M-Space>` were fixed this round.
63. **The error wording for an illegal regex**: vim's legacy engine tolerates
    more syntax (an unterminated `[` reports E486 and stores the pattern); the
    engine (RE2) reports `Invalid pattern: …` and stores nothing (`&` does not
    replay a broken command).

### Performance notes

- bench_probe full rerun: `w` 2.3µs/key (round21 recorded 5.9µs, including
  machine jitter), 1MB single-line `w` 103µs, hlsearch rescan 0.25ms, `:%s`
  5.4ms, `n` 3.7µs — level with or slightly better than the round18-21 baseline.
  All of this round's changes are on the prompt/Ex surface, not the per-key hot
  path (the only hot-path change is DOT_TEXT_MARKER's de-allocation).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 20 addendum (2026-10-03, fuzz round20)

`tests/fuzz_round20.rs`: the command-line surface (`:` `/` `?`) switched from
"whole-string named keys" to burst input — "enter the prompt → feed the hostile
alphabet character by character → finish with Enter/Esc" — and added a rendering
contract invariant: **the highlight ranges published to the host must be
addressable in their entirety**. It caught the rendering violation where the
E35 early-exit path (`/ab<C-u><CR>`) failed to clear the incsearch preview, and
fixed it. Round 21 additionally cleaned up that file's three clippy warnings
(manual_is_multiple_of).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 21 review additions (2026-10-04, regression tests merged into
`tests/review_regressions.rs` (+15 cases); no new fuzz file — this round's
abnormal surface is covered by the existing round20 cmdline bombardment plus
targeted regressions)

Process of this round: read all of `src/` (list candidates from the code → settle
with PTY probes → fix), then a second pass reviewing the new surface
(search-motion × macros/`.`/cross-line/multibyte/CJK). One methodological gain:
**probe the probe's own assertions first** — all 4 of the second pass's
"failures" were the probe miscomputing its assertion while the engine agreed with
vim (the visual selection includes the cursor character, a backwards exclusive
keeps the cursor character, `/aa` skips the match under the cursor itself, the
append typing landing point); pin the probe correctly before talking about
overturning anything.

### Semantics fixes (PTY byte-level probe evidence)

| # | Problem | Evidence / root cause |
|---|------|-----------|
| 1 | **`d` + `/`: the operator dangled across the prompt (data-loss severity)** | After the old engine suspended on `d`, `/` still opened the prompt as usual and Enter performed a pure jump while the operator stayed **untouched** — the next motion key after the search accidentally fired `d<motion>` (probe: `j` → `dj` emptied the whole buffer). The fix is the new feature described below: the prompt is now itself the motion |
| 2 | **`:marks` listed `^` twice** | `exit_insert` wrote both the offsets table (`set('^')`) and the engine-tracked `last_insert_exit`, and the listing's append branch deduplicated only `.` while appending `^` unconditionally |
| 3 | **`iw` at the end of a CJK line misjudged the whole line** | The text-object probe's `line_end - 1` was a raw byte step: a multibyte tail character landed mid-character → `class_at` read None → the empty-line branch fired. Fix: the probe floors to a character boundary, and the whole run scan is anchored at the clamped position (the old code only clamped the classification probe while the scan still started from the raw offset, which could produce a range crossing the line) |
| 4 | **`:1j`'s no-op on the last line left the cursor at col 0** | The no-seam path left the cursor at the `line_start(first)` set before the join, whereas both the success path and the empty `:5` command land on the first non-blank — the vim 9.1 probe (`:1j` on `"    abc"` then `x` deleting `a`) confirms first non-blank. Also correcting round19's record: what it wrote as "same as `:5`" did not hold on indented lines; only this round did it truly align |
| 5 | **`<S-a>`-style mappings never fired** | `handle_key` dropped the shift flag for pure character keys while `parse_angle` kept `shift+Char('a')` — the delivered shape could never match. vim 9.1 probe: the mapping listing for `:nmap <S-x> ihello` shows `n  X  ihello` (vim normalises too). Fix: shift-only + ASCII letter → the uppercase letter; non-letters (`<S-1>`, keyboard-layout dependent) stay as they are |
| 6 | **`d/foo` silently no-oped when the match was only at the cursor** | On the wrap fallback to a self-match the span came out empty — the engine did nothing and said nothing; the vim probe gives the wrap message + bell. Fix: treat `target == origin` as a motion failure, ring the bell + abandon the operator |

### New feature: search-as-motion (`d/pat`・`c?pat`・`gu/pat`・`v/pat`)

When `/`/`?` opens the prompt under an operator or in visual mode it is recorded
in `search_motion`, and Enter settles it with vim's semantics (all PTY probes
aligned):

- **Under an operator**: completes the operator with the exclusive span
  `[cursor, match_start)` (`d/bar<CR>` on `"foo bar\nbaz"` → `"bar\nbaz"`); counts
  merge (`2d/foo` ≡ `d2/foo`); a miss reports E486 and **abandons the operator**;
  Esc cancels silently (vim's clearop); wrap semantics identical to `n`.
- **In visual**: the cursor jumps to the match start, the selection extends (the
  anchor does not move), and visual **stays active** (`v/bar<CR>` highlights
  `"foo "`, selection including the cursor character — vim probe
  `v/bar<CR>x` → `"ar\nbaz"`).
- **Got for free**: `.` replay (the whole prompt keystrokes are recorded, so
  after `d/foo<CR>` `.`/`2.` repeat correctly), macro record/replay
  (`qb d/bar<CR> q` is six steps, `@b` gives the same shape), `y/pat` leaving the
  cursor alone (yank does not move the cursor, vim same), cross-line spans, and
  the whole `gu`/`gU`/`g~`/`c` operator family.

Before the fix, `/` in visual mode was a dead key that rang the bell — it became
usable in the same pass.

### Disproven during review, no change needed (saves a re-check next round)

- **Search-motion inside macro replay**: the first probe's "`@b` replay deleted
  nothing" was the probe's own fixture bug (the macro lives inside one `VimState`,
  and switching fixture loses it) — replaying on the same fixture after resetting
  the buffer is byte-correct. Macros do not cross `VimState`; this had never been
  pinned, so it went into the regressions this round.
- **The cursor character under backwards exclusive**: `$d?def<CR>` on
  `"abc def ghi"` → `"abc i\n"` — it deletes `[match_start, cursor)` and keeps the
  cursor character. The engine and vim agree byte for byte (the first pass's
  comment misread it as "including the cursor character"; corrected).
- **The selection after `v/pat` includes the cursor character**: `d` deletes
  `"foo b"` leaving `"ar"` — that is visual semantics, the vim probe agrees.
- **`/aa` skipping the match under the cursor**, **`v` inside `qa`**
  **(visual is recordable)**, **the `.` repeat of an `R` session**,
  **`i<C-r>/` (search register insert)**, **7 odd ranges such as `:+0d`, `:;d`,
  `:1;3d`** — all consistent with vim/documentation.

### Comment corrections (guarding against future mis-fixes)

- `objects.rs word_range`'s old comment claimed "never INTO a trailing
  multi-byte char" while `line_end - 1` did exactly that — comment and code
  rewritten together.
- The round20 test `angle_keys_accept_lowercase…` in `review_regressions` pinned
  the dead mapping behaviour where `<s-x>` kept shift — corrected per the vim
  probe (normalised to `X`).

### New known divergences (continuing the global numbering)

59. **The wrap hint message of search-motion**: vim shows "search hit BOTTOM,
    continuing at TOP"; the engine has no channel for that message (the bell
    semantics are already aligned — a failure always rings; a successful wrap
    rings on neither side).
60. **autoindent expansion in `i<C-r>{reg}`**: the engine expands indentation per
    line through `insert_text_at_cursor`; whether vim inserts literally was not
    probed (multiline register + ai scenario, very narrow).
61. **The cursor on an equal-address no-op such as `:2,2j`**: the engine's early
    return leaves the cursor alone; whether vim drops it onto the address line was
    not probed (`:1j`'s landing point is evidenced and aligned, the
    equal-double-address form was not followed up).

### Performance notes

- bench_probe full rerun: `w` 5.9µs/key, 1MB single-line `w` 120µs, hlsearch
  rescan 0.25ms, `:%s` 5.5ms, `n` 3.8µs — level with or slightly better than the
  round18/19 baseline (all of this round's changes are on the prompt path, not the
  per-key hot path).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 19 review additions (2026-10-03, regression tests merged into
`tests/review_regressions.rs` (+8 cases), round13's two reasoning pins corrected per the new probes; fuzz adds `tests/fuzz_round19.rs` (115k steps + targeted bombardment of the D/C shape truth table))

This round read all of `src/` in a single thread (list candidates from the code →
probe evidence → fix). The biggest methodological gain was **typing the probe
channels**: the `-es` channel distorts arrow keys (triggering cmdline
backtracking), visual mode (`v` has no effect, already recorded in round8) and
multi-digit counts (`99D` swallows whole key groups), while **a PTY channel
started with `script` works for all of it** — this round's D/C line model was
pinned down with it. One reverse lesson: **the leading `0a` group in xxd is "the
empty line after clearing", not "the line-start character was deleted"** — one
misreading flip-flopped the conclusion about D's line-start shape across three
rounds, and it only converged at last by repeating the same shape 3 times and
cross-checking against a different buffer length.

### Semantics fixes (PTY byte-level probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **`D`/`C` swallow the file eol on the last line** | `abc\ndef\n` with `D` on the last line → vim file `abc\nde\n`; the old engine model pulled `line_range(last).end` (including the trailing `\n`) into the span — on the host the trailing `\n` *is* the file eol, so bytes were silently dropped. count=1 at line start is the same class: the line is cleared but kept (`"\nb\nc\n"`; the old model only coincidentally agreed on a sole line). The "covered lines vanish entirely" behaviour of count≥2 at line start *is* vim's (`2D`@a/b/c → `"c\n"`, `99D` at line start of a two-line buffer → empty buffer), but the old model misjudged that too — **it left a phantom empty first line**. The final model in one sentence: count≥2 and starting at line start → `line_range(last).end`; everything else → `line_end(last)`. `C` does not take the whole-line branch (at line start `C` clears the line and enters insert, probe `"z\nb\nc\n"`; `2C` at line start → `['new','cccc','dddd']`) |
| 2 | **`Nr<CR>` produces N newlines** | `:h r` states it plainly: "5r<CR> replaces five characters with a single line break" (PTY byte probe: `3r<CR>` on abcdef → one empty line + def). The old engine implementation pushed per character. The autoindent details, derived from the documented model, turn out to be exactly equivalent to a bare `\n` (the indent stays on the first line; the ai indent the new line never typed is stripped by Esc). **Also corrects round18's probe misreading of "3r<CR> three newlines"** (back then it probably only verified the engine's own behaviour) |
| 3 | **round13's two "PROBE" pins were really inference pins** | What `probe_count_d_shapes` pinned — `3D` at line start → `"\ndddd\n"`, and `99D` deleting the trailing eol along with them — both contradict the PTY measurement; the comments said PROBE but a real probe was never run (the `-es` channel distorts the gg+col0 combination, and at the time it was most likely silently swallowed). Corrected per the new probe, with the reason noted (same lesson as round8's `O` probe misreading: a pin can freeze a misreading for three rounds) |

### New features

- **`r<C-E>`/`r<C-Y>`**: the replacement character is taken from the same
  **display column** of the line below/above (`:h r`; `10r<C-E>` copies the 10
  characters below; wide characters align by display column, mirroring
  `i_CTRL-E`). If either side is out of range (neighbouring line too short / the
  cursor line has too few characters / no neighbouring line) the whole command
  cancels and rings the bell — the same cancel semantics as `3rx` leaving two
  characters. Visual `r<C-E>` is not supported (the old path rings the bell;
  kept as is).

### UX improvements

- **`'z`/`` `z `` on an unset mark now reports the E20 text**
  (`E20: Mark 'z not set`, same wording and same channel as the Ex-side `:'a`)
  instead of only a mute bell.
- **incsearch's current highlight changed to "the first match from the cursor"**
  (the one Enter would jump to; the old implementation always marked the
  buffer's first match, which made the preview misleading when the cursor was
  below it).
- **`:j` on the last line (no seam) is a pure no-op**: it no longer bumps — the
  changelist / `.` mark are not polluted by an empty operation (aligned with the
  no-op discipline of the operator family); the cursor still lands on the range's
  address line (same shape as the empty `:5` command; vim's behaviour there is
  unprobed, see the open item below).

### Comment corrections (guarding against future mis-fixes)

- `complete_char_arg`'s non-printable-argument branch: the comment claimed
  "swallowed, keep waiting" (round3/round7 UX notes recorded it the same way),
  but **the code has been "cancel + bell" since day one**. PTY evidence shows vim
  doing the same (after `r<Up>`, `x` runs as a new command: `llr<Up>x` → `abdef`)
  — the behaviour was right, the documentation wrong for nine rounds; this round
  fixed the comment. No test had ever pinned this branch's behaviour; fuzz
  round19's char-arg × cancel-key interleaving surface now covers it.

### Disproven during review, no change needed (saves a re-check next round)

- **The count-leak hypothesis for `2qa…q`**: reading the code suggested that
  after `2qa` the count leaks into the first command inside the recording (vim
  discards a count typed before `q`). Measured: the count is correctly cleared by
  the shared `end_command()` at the tail of `complete_char_arg` (`2qaxq` deletes
  one character, same as vim) — it just relies on the shared tail path rather
  than on explicit semantics. Behaviour is right, no change needed.
- **`f<CR>`**: targeting `\n` must fail within a line → bell, consistent with vim.
- **Where `vy` leaves the cursor**: the engine keeps it on the movable end —
  round5 already disproved "vim leaves it at the selection start" by probe, and
  this round does not reopen it.

### Engine behaviour notes caught by fuzz (not bugs)

- **In insert mode the cursor may sit at line_end and at the buffer's phantom
  last position** (after typing in append, after `<Del>` joins lines): the
  addressability invariants (≤len, char boundary) hold, but "never past line
  end" only holds for normal/visual mode — fuzz round19's invariant check passes
  once relaxed per mode. Host rendering just has to go through `offset_to_line`
  (clamped to the last line).
- **fuzz's buffer reset brought inside the contract**: swapping text wholesale is
  only legal while the engine is idle and in Normal (a real host swaps text
  through the undo/redo paths, which sanitize on the engine side; swapping bare
  mid-session violates the host contract and blew up a false positive of cursor
  8 > len 3 on the very first round).

### New known divergences (continuing the global numbering)

55. **Register shape for `D` with count≥2 at line start**: the whole-lines-vanish
    path uses a charwise span (the register receives charwise text containing
    `\n`); vim's register content/type for the same shape is unprobed (under PTY
    the `:reg` surface is too narrow). A very narrow surface; open item.
56. **`r<CR>` inside indentation**: the engine replaces the character with a bare
    `\n` (the indent stays on the first line); vim's "delete the character then
    `i<CR><Esc>`" indentation-truncation rule for a newline in the middle of the
    indent is unprobed.
57. **The cursor after a `:j` no-op**: [closed in round 21] the vim 9.1 probe
    confirmed it lands on the first non-blank of the address line (`x` after
    `:1j` on `"    abc"` deletes `a`), and the engine now matches; the
    equal-two-address form (`:2,2j`) remains an open item (see 61).
58. **The anchor for visual `J` with a count**: `:h v_J` says the count is counted
    from "the last highlighted line"; the engine counts from the first line of
    the selection. Neither `-es` nor PTY can reliably drive visual + count
    combinations, so the probe left it undecided and the current behaviour stays
    as an open item.

### Open items (read-code candidates, carried forward)

- **`publish_incsearch`'s `current`**: changed to "the first match from the
  cursor"; but whether the preview before Enter should also move the **cursor**
  (vim's incsearch jump preview) is a much bigger piece, so it still only emits
  highlights (a host may implement that itself).
- **`:set` partly succeeding and partly failing**: in `ic wrap`, after `ic` takes
  effect `wrap` fails and returns early, skipping the search-rule re-publish.
  Impact ≈ 0; open item.
- **The sentence motion `(` one position after end-of-sentence punctuation**:
  whether it lands at the start of the previous sentence or of the current one —
  the difference between the engine model and vim is already covered by earlier
  divergences #8/#46, so it was not dug into further.

### Performance notes

- Full re-run of bench_probe: `w` 6.5µs/key, `w` on a 1MB single line 164µs,
  hlsearch rescan 0.35ms, `:%s` 6.5ms, `n` 3.6µs — on par with the round18
  baseline (every change this round is in span endpoint computation, not a hot
  path).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 18 review additions (2026-10-03, regression tests merged into
`tests/review_regressions.rs` (+6 cases), +1 inline config unit test; fuzz reuses
round17's 16-seed suite — no new invariant machinery this round)

This round's order of work: first bring clippy to zero (**the repository was not
clippy-green before this** — 1 deny-level `never_loop` error sitting on top, 4
warnings scattered over lib/tests), then a full read-through plus a batch of
abnormal-operation / abnormal-data probes (the recursive self-recording macro
`qa@aq`, the `99999999@q` budget, insert `<C-r>` with an empty register,
`:%s/^/x/` on an empty buffer, `dip`/`guu` on an empty buffer, `x`/`~` on an emoji
family cluster, `viwr<CR>`, `ci(` with unbalanced parens `( ( )` … all passed),
and finally caught three same-root bugs on the navigation-key paths.

### Semantics fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **Arrow keys leaking into the `.` replay record** | `navigation_key` bypassed `end_command`, so `<Down>` lingered in an uncommitted recording and was baked into the next change command's `last_change` — `<Down>x` followed by `.` replayed as "move down + delete char". vim's redo buffer never contains a pure motion. Fix: the motion branch now finishes through `end_command()` (visual's accumulating-recording branch is unaffected) |
| 2 | **`<Del>` could not be repeated with `.`** | Same root: the Del branch never committed a change record, so `.` replayed the command from two steps back; and the lingering Del key polluted later records the same way. Fix: same as above, through `end_command()` |
| 3 | **`d<Down>` dangling did nothing** | With an operator pending the arrow keys moved the cursor directly (`d` silently cancelled, the next key rang the bell). vim's `d<Down>` = `dj`. Fix: while an operator is pending, route through `execute_command(CmdKind::Motion)` — count merging (`2d3<Down>` equals `d6<Down>`), linewise span and the failure bell all reuse the existing pipeline |
| 4 | **`let mapleader = "="` silently dropped** | `split('=').nth(1)` took the fragment between the first two `=` (` "`), which trims to empty, so the leader kept the default backslash without a trace (`ignored` did not count it either). Fix: `split_once` takes the complete tail after the first `=` |

### Disproven during review, no change needed (saves a re-check next round)

- **`ci"` on a single-quote line** (an odd-length tail of positions): `chunks(2)`
  drops the odd tail and the fallback takes the first pair in the line — the
  probe rang the bell correctly. This round also rewrote the fallback loop as
  `first_chunk::<2>()` (the original form was clippy's `never_loop`: the for loop
  body always breaks and never iterates a second time; behaviour equivalent, a
  pure shape fix).
- **`ci(` on unbalanced parens `( ( )`**: the inner pairing is correct (probe).
- **The `X` after a `ci"` bell**: reading the code I misjudged it as "no
  operation"; what it actually deletes is the quote itself (the cursor sits one
  position after the quote, so `X` deletes backwards) — probe disproved the
  candidate.
- **The abnormal-data / abnormal-operation surface**: `3r<CR>` (a
  three-newline replacement), `viwr<CR>`, `m1` (a digit mark silently ignored),
  `A<C-r>"` after `yy`, `:1,2d 0` (count 0 = no count), the whole empty-buffer
  family — all probes passed, nothing new to carry.

### UX notes

- `d<Down>`/`d<Up>`/`d<Home>` and the like (operator × navigation key) work for
  the first time; `<Del>` has joined the `.` system. What changes feel most is
  that arrow keys no longer "stick" into the replay of the next change — before
  this, any sequence of "move with an arrow key, then change, then `.`" replayed
  one action too many.
- `let mapleader = "="` (and any value containing `=`) in the rc file no longer
  fails silently.

### Performance notes

- bench_probe re-run: `w`/`n`/`:%s`/`gq` on par with the existing baseline; the
  233µs/key for 100× `x` is the naive String host's O(n) line lookup + memmove
  (an existing NOTES conclusion), not an engine regression. No new hot paths.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 17 review additions (2026-10-03, regression tests in
`tests/parity_round17.rs` (23 cases), fuzz in `tests/fuzz_round17.rs`
(115k steps + host-event injection + huge-count text-object budget assertions))

This round read the code in three parallel tracks (state / ops·motions·objects /
cmdline·insert·search) plus a case-by-case probe re-check. **Two read-code
candidates were disproved by byte-level probes**, the key methodological lesson of
this round: ① `g_` on a whitespace-only line — the candidate claimed the engine's
Inclusive(line_start) was a bug ("vim deletes only up to the cursor"), but vim 9.1
`dg_` on `   ` deletes 1/2/3 characters from c1/c2/c3 respectively, so **the
engine was correct as it stood** and the candidate's probe key sequence was
wrong; the change was rolled back halfway when the probe matrix stopped it, and
the behaviour is now pinned in a regression test to prevent a repeat. ② The `2>>`
"count = a multiple of sw" hypothesis — the count is really the **line count**
(`2>>` = shift two lines); the first probe buffer had only 1 line, which is why it
looked like a no-op.

### Semantics fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **The `q` that stops a macro left a `.` recording behind** | `J` after `qaxq`: the stop branch returned early without committing the recording, so the leftover `q` was baked into the next change's `last_change` — `.` replaying MacroRecord **wedged the engine in a recording state that never closes** (every later key swallowed). Fix: the stop branch clears `recording`/`recording_mutated`/`register`. The `"a` register-prefix leak on the same path is cleared too |
| 2 | **`@a` suppressed redo commits, so `.` repeated the pre-macro change** | vim's redo model: macro keys count as typed keys; after `qaxq~0@a.` the `.` repeats the `x` inside the macro (probe ['def']). The engine's `replaying` flag suppressed `@`'s recording/commit across the board. Added `replay_records`: `.`'s own replay stays suppressed (no self-expansion) while `@`'s replay records normally — the last change inside the macro becomes `last_change`. Text steps on the replay path (insert sessions inside the macro) record along too, otherwise `.` replays an empty insert |
| 3 | **`"+` register writes went to a dead slot** | `"+yy` landed in the named slot while `get_for_paste` only read the host clipboard — after a yank, `"+p` pasted **stale external content** and the roundtrip never held. Fix: the three register funnels (delete_span/yank_span/blockwise yank) explicitly push `clipboard_write`; on the read side, an empty host clipboard falls back to the named-slot mirror (hosts without a clipboard keep the roundtrip) |
| 4 | **A count before a text object was silently dropped** | `d2aw` equalled `daw` (`objects::range` took no count parameter). Fix: **rescan the object repeatedly from span.end** — this one model reproduces all of vim's count behaviour at once: `v2aw`="foo bar ", `v2i"` two quote pairs, `v2i(` climbing to the outer block (the inner block's end lands on the outer closing paren; cross it and rescan), `2iw`="foo " (word + trailing whitespace run), `3iw`="foo bar" — even vim's own iw counting quirk is matched exactly |
| 5 | **Whole-surface whitespace attribution in the sentence objects `is`/`as`** | `is` swallowed the trailing whitespace (vim 9.1's `dis` stops at the end-of-sentence punctuation; `"Aaa. Bbb."` leaves `" Bbb."`); `as` was missing two rules: trailing whitespace is **capped at the paragraph boundary (empty line)**, and the buffer's **final newline is not trailing whitespace** (`das` on the last sentence takes the leading fallback and the buffer stays a single line). The leading fallback does not cross line start (`["Aaa.","","Bbb."]` with the cursor on Bbb deletes only the sentence — byte-by-byte probe ['Aaa.', '', '']) |
| 6 | **insert `<C-w>` on in-line whitespace peeled 1 byte at a time and joined lines wrongly** | When everything from line start to the cursor was whitespace, `prev_word_start` crossed the line and fell into the join branch, deleting `at-1..at` (the last space). vim 9.1: one stroke deletes the whole run from line start to the cursor (the auto-indent-clearing idiom, `o` + `C-w` clears the line); joining happens only at col 1. Fix: delete the all-whitespace prefix as one span (no join inside a line, safe for block sessions) |
| 7 | **`<<`/`>>` byte-stripping model vs vim's column model** | The old model treated a tab as a whole sw unit and always inserted a bare tab on right shift — wrong for ts≠sw, mixed indentation and noet (probe matrix: under noet ts=8 sw=4, `<<` on `\tx` → vim `    x`, `\t\tx` → `\t    x`; `>>` at col5 → `\t `). Fix: move the indent **display column** ± sw and re-express it per tabstop (whole tabs + a space remainder; et → all spaces); truly empty lines are skipped, whitespace-only lines participate (`<<` clears, `>>` appends — vim does the same) |
| 8 | **A bare Ex `+`/`-` offset reported E1247** | `:+d`/`:-d`/`:5+d`/`:.-d` — vim's `:h :range`: "an omitted number means 1". round15's E1247 fix (the number-overflow direction) accidentally hit the empty-number case. Fix: `end == 0`, i.e. a bare sign, takes 1; the overflow path still keeps E1247 |
| 9 | **`:s`/`:d` trailing count missing** (`:s`) / no numeric boundary (`:d`/`:y`/`:j`) | `:s/a/X/ 3` = 3 lines counted from the range's last line (`:1,2s/a/X/2` = lines 2-3); `boundary_cmd` did not accept a numeric boundary, so `:d2`/`:j2` gave E492. The count semantics of `:d`/`:y` were already in place (an earlier round); `:s` now completes the same rule |
| 10 | **The `:nohlse` family of prefixes was missing** | vim's abbreviation rules cover every unambiguous prefix of `nohlsearch`; the old table stopped at `nohls` |

### Disproven during review, no change needed (saves a re-check next round)

- **`g_` on a whitespace-only line**: see the top of this section —
  Inclusive(line_start) *is* vim's behaviour.
- **The landing column of `N%`/`1%`**: `apply_motion_result` rewrites it to the
  first non-blank across the board, same as vim.
- **`ip`/`ap` treating blank lines as paragraph boundaries**: an explicit exception
  clause in vim's docs; round15's fix was right.
- **`ci"` with the cursor on the closing quote**: vim's "search from start of line"
  takes the containing pair; the engine agrees.
- **User mappings on the key a char-argument is waiting for**: after
  `:nmap x ihello`, `rx` replaces the literal `x` (probe); the engine does not
  remap char-arg arguments — behaviour matches, no guard needed.
- **Sentence objects with the cursor in the gap between sentences / on a blank
  line**: vim degrades (on a blank line `dis` deletes the blank line itself; with
  the cursor on trailing whitespace it deletes only the whitespace); the engine
  does not reproduce this — see divergence #46.

### Engine robustness (read-code + probe)

- **The climb-and-merge behind a text-object count**: for an inner block (`i(`), a
  rescan whose probe point lands on a closing paren of the same level would make
  zero progress; rescanning across the closing paren plus a min/max merge (both
  start and end) makes `v2i(`/`v2a(` match vim's "climb a level" semantics.
- **The `replay_records` lifecycle**: both exits — pipeline-guard overflow and
  drain — reset it in step, and nesting (`.` inside a macro) keeps the outer
  macro's recording alive.

### New known divergences (continuing the global numbering)

46. **No reproduction of the sentence objects' degenerate case in the gap between
    sentences**: vim's `is`/`as` on a blank line / on trailing whitespace has
    quirks (on a blank line `dis` deletes the blank line itself; with the cursor on
    whitespace `dis` deletes only the whitespace). The engine returns safe
    endpoints for those cursor positions (no-op or a sentence-level span).
47. **`dis` on an indented first sentence deletes one extra leading whitespace**:
    vim keeps 1 indent space (`dis` on `"  Aaa. Bbb."` → `" Bbb."`); the engine
    strips all leading whitespace → `"Bbb."`. Visible only when the first sentence
    is indented.
48. **The `:s` completion message format**: the engine says `{n} substitutions`;
    vim says `{n} substitutions on {m} lines`. Wording only.
49. **Commands that reject a range ignore the range**: `:1,2set ts=8`/`:1,2reg`
    execute normally; vim reports E481 No range allowed (the `:w` family's
    dropping is already #45).
50. **The `\` separator of `:s`**: vim rejects it (E10 family); the engine treats it
    as an ordinary separator, falls into escape folding and rings a mute bell when
    the regex fails to compile. `"` and `|` are in the same family.
51. **Ex literal-address overflow**: `:<21-digit number>d` in vim wraps around on
    overflow and then deletes line 1 (probe); the engine gives E16 and does nothing.
    The engine's behaviour is safer; not chased.
52. **The semantics of `v2i"`**: vim's 2i" equals one complete quoted string
    (quotes included); the engine's repeated scan covers two pairs. A very narrow
    surface: quote object + count>1.
53. **The `.` after `:@:`**: the vim probe shows `.` as a no-op (does `@:` reset
    the redo buffer?); the engine's `.` replays that `:s`. An extreme surface,
    pending a probe to settle it.
54. **A `so3`-style digit directly after sort**: vim E488; the engine treats the `2`
    as a flag and ignores it silently. sort has no count semantics, so it is
    harmless drift.

### Open items (read-code candidates, probes not conclusive, carried forward)

- **Unicode whitespace classification**: `char_class` in `word.rs` uses
  `char::is_whitespace` — NBSP/U+2028 count as whitespace; vim's motion whitespace
  class is ASCII only. Trigger surface: `w`/`iw` on `"a\u{a0}b"`. Needs a set of
  NBSP probes to establish vim's behaviour before deciding.
- **The duplicated `CmdKind` check for the key a `char_arg` waits on**: the
  Find/JumpMark arms of `complete_char_arg` are near copies (a smell carried
  forward, no behavioural difference).
- **The clamp mismatch in `replicate_count_insert`**: the span uses the un-clamped
  copies while the replacement text uses the clamped ones — reachable only with
  "more than 8MB typed in a single R session + count≥3", unreachable in practice;
  both legs should share one copies value (carried forward as a consistency
  cleanup).

### UX notes

- `:+`/`:-` now move the address line by line like vim (the sister path of the
  single-address line jump `:5`); on failure the E16 text lands in the status bar
  instead of a mute bell.
- `"+p` is usable for the first time: the host-clipboard roundtrip in gpui-vim /
  crossterm-vim no longer depends on the host bridging `"*` itself.
- `<<`/`>>` in a noet + tab-indented codebase no longer "shrink the more you
  shift" — the tabstop-aware re-expression was the single change with the biggest
  effect on real-project feel this round.

### Performance notes

- `object_span_count`'s repeated scan advances at least one character each time
  (the `next.end > range.end || next.start < range.start` guard), and a huge count
  (10⁹) breaks as soon as the span touches the end of the buffer — fuzz's budget
  assertion pins the termination.
- `shift_line` is O(indent width) per line (not counting the body text), the same
  order as the old implementation; the re-expression allocates one string, no
  measurable regression.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 16 review additions (2026-10-02, regression tests in
`tests/parity_round16.rs` (17 cases), fuzz in `tests/fuzz_round16.rs`
(115k steps + host-event injection with 1/8 probability per step + a huge-count empty-copy budget assertion))

This round read the code on a single track (re-reading all of `src/`) plus probes.
Two read-code candidates were **disproved** by probes and needed no fix: `ge`
landing at the word start when no word end is findable (vim lands the same way,
P1/P2/Q7 — `prev_word_end`'s scan-from-the-start behaviour happened to be
correct); `~`/`x` counts crossing lines (vim stops counting at line end and the
engine's limit guard was already right, P6-P9). fuzz's **host-event injection**
(drag-select / click / IME text / IME commit delivered in any mode) was the
biggest gain of this round: on its very first run it caught two host-path
invariant violations that code reading had never surfaced.

### Semantics fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **The count repeat of empty linewise text was missing entirely** | `3o<Esc>` opened only 1 line (it gave up as soon as `rep.text` was empty); vim 9.1 opens count lines (probes R1/R7). The copy unit is now a pure newline (an empty copy **carries no indent** — vim strips the indent from an ai line nobody typed on); empty charwise sessions (`3i/3A/3R`) stay a no-op (Q3-Q5). The navigation guard moved from `insert_change_pos` (the first typed position, None for an empty session) to `InsertRepeat.anchor` (the session start position), so empty sessions get the same arrow-drift protection |
| 2 | **The whole autoindent empty-line lifecycle (vim's `did_ai`) was missing** | The engine had no rule for vim's "purely indented line nobody typed on": `o<Esc>` kept the indent (vim keeps a truly empty line, S1/S6), `o<CR><Esc>` kept both lines (vim empties both, S3), `o<BS>` deleted one character (vim erases the whole run in one stroke, S5), `cc<Esc>` kept the indent (vim leaves it empty, S8). Added a session-level `insert_did_ai`: set when o/O opens a line, when a linewise change restores the indent, and when `<CR>` splits a line carrying ai (the set points are all **after** `begin_insert` — the session start clears the flag); any typing that is not a newline clears it (S4: type something then delete it all and the indent stays). The three paths where it takes effect: Esc strips the current all-whitespace line (the cursor lands at line start), CR strips the stale indent of the line being split before splitting it, BS deletes the whole run in one stroke at line end |

### Follow-ups to the semantics fixes (ordering interaction, caught by regression)

- The stripping originally sat **before** replicate: once the cursor had been moved
  to line start, the `anchor` guard skipped the whole `3o<Esc>` copy. Replicate now
  runs first (a non-empty copy reads the typed line's indent — probe R2's `3ofoo`
  gives three indented copies; the anchor check needs an unmoved cursor), and the
  stripping moved after it and now **covers both the typed line and the cursor
  line** — count replication carries the cursor onto the last copy, and the opened
  line's indent still has to be stripped (R1). Lines above, once stripped, shift
  the cursor by the number of bytes deleted.

### Engine invariant bugs caught by fuzz (host-event injection, 2)

| # | Problem | Root cause |
|---|---------|------------|
| 1 | **`replace_range` passed a non-boundary range straight through + cursor-delta saturation** | The IME commit-replacement path wrote the host's range into the buffer as is — a host converting from UTF-16 coordinates can hand over a mid-char byte offset, and a bare write panics the host. Also: the cursor adjustment `text.len().saturating_sub(range.len())` clamped the **negative delta of a shortening** replacement to 0, leaving the cursor past the new end of the buffer. Fix: floor the range to a char boundary first (the same defence as `set_cursor_offset`); the delta is signed now; the exit floors at the end |
| 2 | **`edit_insert` never called `visual_anchor`/`cmdline_visual`** | Those two bare fields live outside marks and missed `adjust_insert`'s relative shift — a host IME insertion while a visual selection was alive left the anchor inside a multi-byte character of the newly inserted text (the delete/replace funnels finish up via `refloor_stored_offsets`, insert had been overlooked). Added the relative shift + the same refloor |

### Engine robustness (host-API guards, not probes)

- **`set_visual_range` tore the mode machine apart during an insert session / while
  a prompt was active**: a host drag-select rewrote `self.mode` wholesale to Visual
  while `insert_session`/cmdline state was still alive — Esc then went through
  `exit_visual`, but the undo group and `last_insert_exit` never closed. Drag-select
  is now accepted only in Normal/Visual.
- **`set_cursor_offset` moved the buffer cursor while a prompt was active**: the Ex
  range's default address reads this offset, and vim ignores clicks while the
  cmdline is open — the same ignore added.

### UX improvements

- **Failed delete/join families ring the bell** (same as vim): `x` on an empty line,
  `X` at line start, `D` on an empty line, `J`/`gJ` at EOF were all silent in the old
  implementation. `s` rings on a partial failure of its delete half but still enters
  insert as usual; `join_lines` now returns the number executed — `3J` at EOF-1
  rings for the seams it cannot reach, `2J` (a single seam) succeeds without a bell
  (probe R3 shows no bell, pinned along the way).
- **showcmd now includes the char-arg command letter**: while waiting for the
  argument of `r`/`f`/`t`/`F`/`T`/`m`/`q`/`@`/`'`/`` ` `` the old implementation
  displayed nothing (vim shows the partial command, e.g. `4r`). Added
  `CharArgCmd::pending_key`, displayed together with the argument once it is known.

### Performance fixes

| # | Problem | Fix |
|---|---------|-----|
| P1 | **`:sort` reallocated the sort key on every comparison** | `sort_by_key(lower(s))` did one `to_lowercase` allocation per comparison. Changed to decorate-sort-undecorate: the key is computed once per line and the dedup compares against the same key column (semantics unchanged from round6's probe conclusion) |

### Dead-code cleanup and comment corrections

- An unused `addressable` helper in `tests/fuzz_round15.rs` and one `mut` in
  `parity_round12` (two compiler warnings).
- `last_change` field comment corrected: "visual changes cannot be replayed (v1)"
  had been untrue since round8 (`finish_visual_op` records it, so `.` can replay
  it); rewritten to match reality.
- `exit_insert`'s block-copy delta stays isize until the comparison (usize boxing let
  a net-delete session's negative delta wrap into a huge positive value, safe only
  because it "can never equal text.len()"; now explicitly checked for negative); the
  local `sw` in `insert_tab` was really a tabstop, renamed.

### Read-code candidates disproved/closed by probes (saves a re-check next round)

- **`ge`/`gE` landing at the word start when no word end is findable**: code reading
  inferred it should fail with a bell; probes P1/Q7 showed vim lands on `'a` the same
  way (`prev_word_end` scans from one position before the cursor and, having
  exhausted the line, falls back to 0 — behaviour that happens to match vim). The
  engine was fine; closed.
- **`~`/`x` counts crossing lines**: reading suspected `3~`/`2x` should cross lines;
  probes P6-P9 showed vim stops counting at line end — the engine's grapheme/limit
  guard was already correct; closed.
- **`2J` at EOF-1**: at one point I wanted to add a bell "after a partial join";
  probe R3 showed `2J` only ever has one seam, so no failure and no bell. Modelling
  it as `requested = count.max(2)-1` makes it naturally correct.

### New known divergences (continuing the global numbering)

45. **Ranged `:w`/`:q`/`:wq`/`:x` drop the range**: `:2,5w` saves the whole file as
    usual. vim's `:[range]w` writes part of the file; the host `save()` hook has no
    range parameter, so integrators need to know (silently accepting vs vim's
    partial write/error).

### Still unresolved (updated)

- **The insert `<C-r>` special-register surface** (`C-r .`/`C-r C-w`/`C-r %`…):
  unchanged (carried forward since round15).
- **`display_column` O(line bytes) per key**: unchanged at 0.2 (carried since
  round15).
- **The two separate `set` parsers in config.rs and ex_set**: unchanged (carried
  since round14).
- **tck gaps**: the minimal-scroll assertion for scroll_to_line, an IME round-trip
  smoke test, the phantom-line check on an empty buffer — all still open.
- **`:s` silently ignoring unknown flags** (besides `e` there are `l`/`p`/`#` and
  others): folded into divergence #42's flag-subset surface, not tracked separately.

### UX notes (this round)

- fuzz round16's host-event injection turned "the host calls the API at the wrong
  moment" into a standing stress test: the four event kinds (drag-select / click /
  IME text / IME commit) are delivered randomly across insert/cmdline/visual/normal,
  and both real host-path bugs were caught on its first run — the injection surface
  keeps expanding next round.
- Ringing on failed deletes makes "the key did nothing" perceivable (the bell channel
  of gpui-vim/crossterm-vim had never fired for this command family before).
- fuzz round16 has the same budget as round15 (16 seeds × 60 rounds × 120 steps); the
  full suite is 522 cases (round15 baseline 503).

### Performance notes (re-checked this round)

- Full bench_probe re-run, item by item consistent with the round14 record: `w`
  6.5µs/key, `w` on a 1MB single line 176µs, hlsearch rescan 0.30ms, `:%s` 6.1ms, `n`
  3.6µs/key — `edit_insert`'s new refloor tail-scan (O(stored-offset count), a cost
  the x-delete path had long been paying) shows no measurable regression; ex_sort's
  decorate-sort sits outside the bench measurement points (the probe has no sort
  line), and by construction its magnitude is a monotone improvement.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 15 review additions (2026-10-02, regression tests in
`tests/parity_round15.rs` (16 cases), fuzz in `tests/fuzz_round15.rs` (115k
steps + huge-count termination assertion))

This round three independent code-reading agents swept the domains
cmdline/config/options/keymap/key, ops/motions/objects/word/search and
insert/registers/marks/buffer/host/tck, producing about 25 candidates in total.
**Every semantics fix was verified with an engine probe before changing it** — three
of them were disproved or rewritten: the position-stack mismatch for multi-byte BS
in Replace mode (measured correct; the code-reading inference was wrong), the `d{`
reverse "deletes all three lines" assertion (vim keeps `bbb`; the agent had wrongly
transplanted the `d}` conclusion to `d{`), and the real symptom of `:s` replay
corruption differing from the prediction (the actual root cause was a missing
replacement escape on the *first* execution, not the replay splitting). Two earlier
fixes were **overturned** by new probes: the reference host's mid-char guard in
`offset_to_line` (round13 #26) turned out to be an identity function, and the `:s`
escape-separator fix (round14 #20) covered only the pattern side.

### Semantics fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **The whole replacement-escape surface of `:s` was missing** | `:s/a/b\/c/` gave `b\/c` (vim: `b/c`), `:s/a/b\\c/` gave `b\\c` (vim: `b\c`). round14 #20 fixed only the pattern-side splitting; the replacement went through regex expand (where a backslash is not special). Now `unescape_replacement` folds `\{sep}`/`\\` and leaves unknown escapes as they are |
| 2 | **`&`/a bare `:s` replayed a corrupted replacement** | The rebuild used a naive `split(sep)`: the fields of `s/a/b\/c/` were chopped up, so the replay result differed from the first execution (`:h :&` requires the same replacement string). Replay now goes through the shared `split_escaped_fields` |
| 3 | **An out-of-range address offset `unwrap_or(0)` silently deleted the wrong line** | `:1+<21 digit 9>d`: the parse overflowed into +0 → line 1 deleted. vim 9.1 reports **E1247** in both directions and does not execute (same for the negative side). `with_offset` now returns `Result` |
| 4 | **`}`/`{` treated whitespace-only lines as paragraph boundaries** | vim recognises **truly empty lines** only (probe: on `aaa/␢␢␢/bbb`, `d}` deletes all three lines and `{` lands on line 1). `next_paragraph`/`prev_paragraph` switched to `is_empty_line`; `d{` in reverse keeping `bbb` pinned down with it (the agent's original "deletes all three lines" assertion was overturned by the probe) |
| 5 | **`J` on an empty line inserted an extra space** | `J` on `["","def"]` gave `" def"` (vim: `"def"`). No separator when either side of the seam is empty; same for `["def",""]` |
| 6 | **`ci"` rang the bell when the cursor was past the line's closing quote** | With the last pair's closing quote taken as the left fallback there is no right partner left → no-op. vim falls back to **the first pair in the line** (changing `hi` at the end of `say "hi" then "bye" end`). A dangling left quote (no right partner) takes the same fallback |
| 7 | **`N%` was one line off vim's file percentage** | The engine floored `count*total/100` (0-based); vim's nv_percent is 1-based **ceil** `(count*total+99)/100`. `2%`@200 lines used to land on line 5 (vim: line 4). `count>100` is rejected as in vim |
| 8 | **Typed `1%` was treated as bare `%` (bracket matching)** | Once the count collapses, 1 is indistinguishable from "no count" — following the `1G`→`gg` rewrite precedent, added `Motion::GoToFilePercent`. On a 200-line buffer `1%` used to ring the bell in place (vim: line 2) |
| 9 | **`"_yy` polluted the unnamed register** | store_yank fell through and re-pointed `last` unconditionally: after `"_yy`, `p` pasted the yanked text (vim 9.1: the paste yields nothing). `"_dd` was already correct (its store returns early) — the asymmetry was the tell |
| 10 | **`3R{text}<Esc>` dropped the count** | Replace had been excluded from count-repeat. vim repeats the whole input and **overwrites by character count** (`3Rab` on `abcdefghij` → `abababghij`). Added a Replace arm to the exit path (walks characters across the span; appending once line end clamps); sessions containing newlines are conservatively skipped |
| 11 | **The redo-register was missing** | After `"1P`, `.` re-pasted the same register. vim's `.` increments the register number around the numbered ring (`:h redo-register`: `dd dd "1P .` restores both lines). `bump_redo_register` recognises the `[count]"{1-8}[count]p|P` shape, rewrites it and writes back into last_change |
| 12 | **The canonical ctrl form of `<C-h>` was swallowed** | The prompt and insert mode accepted only the raw `\x08` byte, so a gpui-style `Key::ctrl_char('h')` silently did nothing (the comment claimed it "folds to Backspace"). Both sites now fold it to BS; vim binds C-h ≡ BS |
| 13 | **No `'a`/`` `a `` in visual mode** | Single-key trie Miss: bell + clear pending. In vim the cursor jumps to the mark and the selection follows. Added: the visual path now goes through CharArgCmd::JumpMark → goto_motion |
| 14 | **`:s` flag `e` semantics inverted** (recorded, not fixed) | With no match in the line, `:s/z/q/e` makes the engine report E486 while vim stays silent because of the `e` flag. Part of the missing flag subset, but it changes the error behaviour, so it went into the divergence table as #42 |
| 15 | **config did not recognise `setl[ocal]`/`setglobal`** | The rc line `setlocal ts=4` silently ended up in `ignored` (the runtime `:setl` was already supported since round14). The config side now covers all four spellings |
| 16 | **search_rule's alias table invented `isc` and omitted `scs`** | `:set scs` changed the matching rules but never invalidated the highlight cache. Invented entry dropped, real aliases added |
| 17 | **The `:brew` prefix gave E492** | vim's abbreviation rules allow every unambiguous prefix (a leftover from round14 #23's fix surface); `:br`/`:bre` stay excluded because of the `:break` ambiguity |
| 18 | **A block register's empty last line was swallowed by put** | The yank side did `join("\n")` with no terminator, yet an empty last line produces a trailing `\n`; the put side's `trim_end_matches('\n')` ate the line boundary along with it. Changed to a plain split (reachable through the host block API) |

### Outside the engine: a real bug in the reference host (caught by the new contract)

- **The tck reference host's `offset_to_line` mid-char guard was an identity
  function**: `offset.min(ceil_char_boundary(offset))` returns the offset itself for
  a mid-char offset (ceil ≥ offset), so it still sliced `[..mid]` and panicked — the
  problem round13 #26 claimed to have fixed was in fact still there, because the
  contract had never tested a mid-char offset. Now floored to the character start.
  buffer_read_contract gained multi-byte midpoint assertions (the tolerance surface
  of char_at/offset_to_line/prev_char_offset), and `offset_to_line(len)` accepts both
  the "clamp to the last line" and the "phantom line number" implementations (the
  engine tolerates either; the tests host and the tck host each pick a side).

### Engine invariant bugs caught by fuzz

- None new (this round's incremental fuzz key table landed entirely on already-fixed
  paths; huge-count termination is asserted through a dedicated budget test). The
  possibly-stale `last_matches` invariant keeps round13's constraint (staleness
  allowed while live update is off, rescan before consumption).

### Performance fixes

| # | Problem | Fix |
|---|---------|-----|
| P1 | **Huge counts spinning on motion fixed points** | `999999999e` ran a full 10⁹ whole-word scans at the buffer's last character (the sister implementation WordEndBack has a `next == o break` guard, WordEnd was missing it); the `}`/`)`/`{`/`(`/`b`/`w` family each did an O(line scan) per step at the endpoints. All got fixed-point guards and stop at the first step that makes no progress (same semantics as vim's motion loop). Returns in <150µs |
| P2 | **tck reference host's `line_range` did three full-text scans per call** | Guard + total + enumeration, one O(text) pass each, and the trait-default line_start/line_end/line_content go through it on every action. Merged into one pass (a host copying the reference implementation saves 2 O(full-text) passes per key) |
| P3 | **`tag_range` quadratic on a bare trailing `<`** | On a template fragment without `>`, every `<` rescanned the whole rest of the text. Break as soon as find returns None (nothing further can close it) |

### Dead-code cleanup

- The tail of parse_angle in `key.rs` rebuilt a Key field by field from itself (its
  comment described the old implementation); `keymap::Trie::is_empty` (zero callers
  repo-wide, following round14's criterion); the unreachable `till` branch at the end
  of `find_from` (till always returns above); two `vim.search.forward = forward`
  assignments in search.rs (set_pattern_inner already assigns the same value).
- The verbatim duplicated line-break merge condition of `daw`/`caw` extracted into
  `merges_following_line_break`; `:marks`' double `items()` call merged into one
  collection pass; `:y`'s pointless destructuring.

### Comment corrections (guarding against future mis-fixes)

- `next_word_end` in `word.rs` used to be commented as "no-op / motion failure" at
  its fixed point — measured, `ye`/`de` **deleting that very character on the last
  word's character is exactly vim's behaviour** (round12's wording nearly tempted
  this round into turning it into stuck()). Reworded as "fixed-point semantics +
  caller guard".
- The preserve_inner comment of `adjust_replace` in `marks.rs` said
  "column-preserving" while it actually preserves the **byte** position (for an
  equal-length replacement); the division of labour with refloor is now spelled out.
- The `scroll_to_line` contract in `host.rs` gained "must scroll minimally": the free
  scrolling of C-e/C-y relies on a self-disciplined host implementation, and a
  re-centring implementation would silently disable it (a UX gap, feeding the
  INTEGRATION surface). The matching tck contract assertion is still missing (open).

### Read-code candidates disproved/closed by probes (saves a re-check next round)

- **The Replace multi-byte BS position-stack mismatch** (agent, high confidence):
  `R中<BS>` measured on `ax`/`xyz` restores correctly — the equal-width byte-grid
  redraw plus the offset shift is already self-consistent; closed.
- **`d{` in reverse deleting all three lines**: vim keeps `bbb` (the agent had wrongly
  transplanted the `d}` probe conclusion).
- **A huge negative offset deleting a line**: `with_offset`'s saturating_sub would let
  `:5-<huge>d` land within 0, but it already reported E16 because the base was out of
  range — after unifying on E1247 this round both directions are explicit.

### New known divergences (continuing the global numbering)

42. **The `:s` `e` flag flips the error behaviour**: in vim `e` swallows E486; the
    engine drops the flag silently and still reports E486. Like the `c` flag (#40) it
    belongs to the flag subset, but what `e` changes is the observable error rather
    than the execution semantics.
43. **`3R` sessions containing newlines are not repeated**: the repeat of
    `3Rab<CR>cd<Esc>` is not implemented (both the span and the seam line model assume
    single-line typing); vim repeats the newlines along with it. Conservatively
    skipped.
44. **The put path for block-register text still uses `trim_end_matches` for the
    combination of linewise/charwise registers with visual-block replace** (state.rs
    line 3711): vim's exact semantics for a linewise register with several empty lines
    on that path is unprobed, so the old behaviour stands.

### Still unresolved (updated)

- **The insert `<C-r>` special-register surface** (`C-r .`/`C-r C-w`/`C-r %`…):
  currently inserts nothing silently with no E353 feedback; `%` is already carried as
  an open item in host.rs. It involves last-insert text tracking and cursor-word
  reading on the cmdline side, left to a dedicated effort.
- **`display_column` O(line bytes) per key**: with `desired_col == None`, `j`/`k`
  recompute character by character from the line start every time (degrading in the
  1MB single-line scenario). The fix needs to carry the previous column through
  vertical moves or cache per line, which touches the `apply_motion_result` surface;
  left for 0.2.
- **The semantics of `:set sw=0`/`ts=0`**: vim's `sw=0` means "follow the tabstop" and
  `ts=0` should report E487; the engine has no lower bound. Same batch as #41
  (negative `so`), left for the 0.2 breaking window.
- **`:set ts=4 ?` (a value form plus a space and `?`)**: the engine reports an error
  and drops the assignment; vim's tolerance of the mixed form is unprobed.
- **tck gaps**: no executable assertion for scroll_to_line's minimal scroll; no smoke
  coverage for the IME return path (take_pending_unknown_chars); the empty-buffer
  phantom-line check is still skipped (when line_count==1).
- The two separate `set` parsers in config.rs and ex_set (round14's open item stands —
  this round the config side also gained setlocal/setglobal, so the divergence
  narrowed further, but extracting a shared function is still to do).

### UX notes (this round)

- `<C-h>` was a dead "pressed, nothing happens" key on gpui-style hosts (the canonical
  ctrl form); both delivery forms now fold to BS alike.
- visual mode `'a`/`` `a `` went from "bell + drop pending" to a usable selection jump.
- A mistyped huge count (`999999999e`) went from an effective hang to an instant
  return.
- Out-of-range Ex addresses now report vim's E1247/E16 without touching the buffer
  instead of "delete the wrong line / clamp and execute" — the error-message channel
  (status_message) lets the host's status bar show the reason.

### Performance notes (re-checked this round)

- bench_probe was not re-run (this round's changes are not on the motion/search
  hot-path measurement points: the fixed-point guards only engage at endpoints, the
  tck reference host is not in the bench path, and tag_range is a text-object slow
  path). round14's conclusion (absolute values mean something only within the same
  run) stands.
- fuzz round15 has the same budget as round14 (16 seeds × 60 rounds × 120 steps); the
  full suite is 503 cases (round14 baseline 486).

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 14 review additions (2026-10-02, regression tests in
`tests/parity_round14.rs` (48 cases), fuzz in `tests/fuzz_round14.rs`)

This round three independent code-reading agents re-swept all of state.rs,
cmdline/config/options/keymap and ops/motions/objects/word/search/registers/marks/
insert_mode, producing about 40 candidates; every semantics fix was first verified
against local vim 9.1 (three batched probe-script rounds; the conclusions are in each
entry's parentheses). Two assertions from earlier rounds were **overturned and
corrected** by probes (round8's block `O` probe misreading, round13's endpoint for
`:1,2d 3x`), one round12 open item was disproved and closed (`:s//` empty pattern),
and the till family uncovered a **previously unknown real divergence** (the operator
span one character short — the old `dto` assertion gave "lo world" while vim gives
"o world").

### Semantics fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **The till repeat `;`/`,` falsely "succeeded" in place** | `t2;` rescanned from the cursor and immediately re-hit the same target: moved=true with the offset unmoved and no bell. Now the adjacent target is skipped once and the repeat then advances per lookup (`t3;;` hops once per `;`, `t3` followed by `2;` advances only one — probes A2/A4/A6/Q5) |
| 2 | **A fresh `Nt{x}` counted lookup attempts instead of targets** | `2t3` used to re-hit the same target and land on the first one; vim counts **distinct targets** (the adjacent target counts as the first: probes A1/A5/A9 all land on the second 3) |
| 3 | **The till operator span was one character short** | The cursor's stop position was used directly as the span endpoint; vim's span extends to **the target position** (`dto` gives "o world", not "lo world"; `d2t3` deletes up to before the second 3). MotionResult gained `till_target`, span_from_motion aligned |
| 4 | **Yank polluted the changelist / `.` mark** | The operator-completion path called `bump()` unconditionally: `yw`/`yi(` showed up in `:changes` and `g;` landed at the yank. Probe Q2: vim's `:changes` has the same number of entries before and after yiw. Changed to `bump_if_edited` (zero-span operators benefit in the same stroke) |
| 5 | **`ci"`'s escape test looked at a single preceding backslash only** | The closing quote of `"a\"` was misjudged as escaped and the whole line lost its pairing. Now by backslash-run length mod 2 (probes P20/Q8) |
| 6 | **`ap` on the last paragraph (no trailing blank line) left the blank line before it** | `:h ap`: with no trailing whitespace the leading one is included. Probe P19: [para1,"",para2] with `3Gdap` → [para1] |
| 7 | **`"ax`/`"aX`/`"as`/`"<Del>` dropped the register prefix** | They wrote only to the unnamed register. Probes P10/P10b/P10c: vim's @a receives the deleted text. `delete_chars` gained a register parameter |
| 8 | **The `""` prefix froze the numbered ring** | `""dd` wrote only unnamed. Probe P18: vim's `""` ≡ no prefix (`""dd` rotates as usual). Normalised to None |
| 9 | **Blockwise visual `O` had the direction inverted** | The probe10 baked into round8's test was a misreading; three independent probes agree: the cursor **stays on its own line** and switches to the block's other column corner, the anchor mirrors and the rectangle is unchanged (anchor(0,0)+cursor(1,2) → cursor(1,0); in this geometry probe cur=3:2). A single-line block does a real horizontal corner swap ((0,2)→(0,0)), where it had been a no-op |
| 10 | **Char/line visual `O` rang the bell** | Probes Q3/Q4: in vim it equals `o` (endpoints mirrored, no bell) |
| 11 | **A mapping hijacked the continuation key of a partial command** | After `:nnoremap j gj`, typing `gj`: the engine let the continuation key `j` enter the mapping table, the expanded replay hit the same mapping again → the runaway fuse blew and gj stopped working entirely; with `:nnoremap j G`, G jumped to the last line. Probe P6: vim's continuation key completes the builtin directly (cur=2:1). mapping_step gained a cmd_seq guard — mappings only take effect at command boundaries |
| 12 | **The no_remap budget leaked across guards** | After the pipeline guard / maxmapdepth cleared the queue, the leftover budget let later keys bypass mappings silently. Now zeroed whenever the queue is cleared |
| 13 | **MAX_MAP_DEPTH=100 was below vim's default** | vim's maxmapdepth=1000; legitimate hundred-level expansion chains had been misjudged as runaway. Aligned to 1000 |
| 14 | **The count of `N:` leaked** | After `3:<CR>`, `j` jumped 3 lines; `3:` left the cursor unmoved. Probe P7: vim turns the count into a range and prefills `.,.+2` (`3:d<CR>` deletes 3 lines). The range is now prefilled |
| 15 | **The count of `N/pat` was discarded** | count=1 had been hard-coded. Probe P8b: vim's `3/foo` jumps to the 3rd match. A cancelled search drops the count (probe P9: after `2/x<Esc>`, `x` deletes only 1 character); visual `:` likewise |
| 16 | **Trailing garbage after `:d`/`:y`/`:j` executed silently** | `:1,2d 3x` deleted the range; `:d a b` took only a. Probes P13/P54/B4: vim reports E488 and does not execute (round13's comment described the right behaviour but `unwrap_or(0)` never implemented it). The `|` separator also lands on E488 (`:h :bar`) |
| 17 | **`:d!`/`:y!` were silently accepted** | Probe P16: vim reports E477 No ! allowed. The message carries the complete original line (`E477: ...: 1,2y!`) |
| 18 | **Out-of-range addresses were clamped instead of reported** | `:1000000d`/`:2,99999d`/`:.+99d`/a bare `:99999999` used to clamp to the last line and execute as usual. Probes P15/P50: vim reports E16 and changes nothing. Address 0/negative is still legal and acts on line 1 (probes B1/B3) |
| 19 | **`:y a3` lost the count in the packed form** | The register token took only its first character. Probe P17: vim's `a3` = register a + 3 lines. Shared token parsing (register + a pure-digit tail) |
| 20 | **`:s` had no `\/` escaped separator** | `:s/a\/b/x/` parsed into pieces. Probe P26: vim matches the literal `a/b`. Splitting now happens on unescaped separators, leaving the backslash inside the pattern |
| 21 | **`:s`'s `n` flag was ignored and the destructive substitution ran** | `:%s/foo//n` really deleted. Probe P14: vim reports "2 matches on 2 lines" and leaves the buffer alone |
| 22 | **The `:set` surface was missing** | The `:se` abbreviation, `:setl[ocal]` (single buffer = :set), `:set ts&`/`ts&vim` resetting to the default, the `:set ts ?` space-questionmark query, a bare `:set ts` query — all used to bell and abort the whole line (probes P27/P29/B8/Q16) |
| 23 | **`:bn`/`:bp` accepted only the exact spelling** | `:bne` gave E492. vim's abbreviation rules allow every prefix |
| 24 | **config: `let mapleader2` hijacked the leader** | strip_prefix swallowed the whole line, so an unrelated variable silently re-bound `<Leader>` and did not even land in `ignored`. Added a word-boundary check |
| 25 | **config: TAB-separated set/source lines fell into ignored** | The map family had been fixed for TAB long ago; set/source were missed |
| 26 | **config: rc `set` comments/queries produced garbage** | `set ts=4 " note` produced On("\"")/On("note") that inflated `ignored`; `set ic?` produced junk tokens. Comments now truncate, queries are dropped; added `Setting::Reset` (`set ts&`) |
| 27 | **`<LocalLeader>` mappings died silently** | A leftover Named("localleader") marker key that the keyboard can never produce. Now resolved through `let maplocalleader` (default `\`) |
| 28 | **Replace BS was a bare LIFO** | BS then `<Right>` then BS wrote a character belonging to a later position at the cursor (it once produced 'ayy') and the stack stayed permanently misaligned. Entries now carry the buffer offset: the stack top is restored only when it is exactly the backspace position (an appended character deletes instead); on a mismatch the whole key is a no-op — consistent with the probe matrices Q8a-e/P23a-d; the newline-split mismatch (round9 open item) closed along the way |
| 29 | **count-repeat insert dropped the count under ai** | `3ifoo<CR>bar<Esc>`'s exit invariant compared against the original text length; indentation made the buffer grow past it → silently abandoned. Probe P24: vim gives three foo/bar groups. The expanded text is now the copy unit |
| 30 | **`*` rang on a non-word character and referenced a stale pattern** | Probe Q1: vim searches the **literal** of that character (pattern = the escaped single character). An all-whitespace cursor line now reports E348 (the old behaviour reported E486 referencing an unrelated old pattern) |
| 31 | **changelist's dedup branch did not advance change_pos** | When the new edit landed exactly on the newest entry the pointer stayed behind, so later `g,` walked from the wrong position |
| 32 | **u/`<C-r>` did not clear desired_col** | A stale vertical target column was carried into `j` after undo (same handling as JumpBackward/OlderChange) |
| 33 | **`replace_range` left a dangling undo group** | The pub host API, called outside an insert session, never closed open_undo: the next edit merged into the same undo group and one `u` undid two steps |

### Dead-code cleanup

- `word::is_non_blank`, `keymap::Trie::is_prefix` (zero callers repo-wide);
- the always-`Some` branch of `search::compile` (the signature now returns a builder),
  the dead `SearchState.last_index` state (written, never read);
- the unreachable `guu`/`gUU`/`g~~`/`gqq` rows in tables.rs plus all 4-key doubling rows
  (the second key of `gu` already Hits and cmd_seq is cleared, so a longer sequence can
  never accumulate in the trie — doubling goes through the operator-pending path, which
  fuzz_round10 had been covering all along) and the normal `<Del>` row (the navigation
  interceptor runs before the trie, and visual maps it to d too);
- the unused `_buf` parameter of `ops::span_from_object`.

### Open items disproved/closed by probes

- **`:s//x/`'s empty pattern reusing "the previous substitute pattern" (round12 open
  item)**: not true — probes P25b/P25d: vim uses the **search** pattern (after `s/b/B/`,
  `s//Q/` replaces b with Q). The engine's behaviour is already correct; the open item
  is closed.
- **`m^`/`m.` settable but unresolvable**: probes P22/P48: vim also accepts `m^`
  silently and `` `^ `` still points at internal state — the engine's observable
  behaviour matches vim's; closed.
- **`gv` with no previous selection (round11 open item)**: implemented in round12/13
  (`NormalCmd::RestoreVisual` + `marks.last_visual` shifted along with edits); this
  round's fuzz confirmed the path is robust, so the record is closed.

### New known divergences (continuing the global numbering)

38. **Recursive mappings do not produce E223 through a builtin prefix**: with
    `:map j gj` (recursive) and typing `j`, vim expands recursively until E223; because
    of the cmd_seq guard (fix #11) the engine lets `j` complete the builtin directly
    once the expansion's `g` is assembled — gj runs once, no recursion. On pathological
    configurations the engine is more permissive.
39. **Multi-step BS+move sequences in Replace**: probe matrices 9/10 agree with the
    engine; P23e (`Rab<BS><Right><BS>` giving 'abyz' while the engine gives 'axyz')
    shows an extra stack interaction in vim's Right/BS combination after a restore,
    which is not modelled.
40. **`:s`'s interactive confirmation flag `c` is unsupported**: there is no host
    confirmation UI; it now silently performs every substitution as if `c` were absent
    (vim asks one by one). It still changes the buffer, so integrators need to know.
41. **`:set so=-1` is still rejected**: the scrolloff field is usize (downstream
    gpui-vim/crossterm-vim read it directly), and changing it to i64 is a breaking
    change; vim allows -1 (the centre-the-cursor semantics). Left for the 0.2 breaking
    window.

### Still unresolved (updated)

- config.rs and ex_set **are still two separate implementations** of set-syntax parsing
  (this round completed the comment/query/reset surface on the config side, narrowing
  the divergence; extracting a shared parse function is still to do).
- The cmdline prompt has no cursor model, `|` inside `:s` arguments is still read
  literally (`:h :bar` semantics are that way anyway; kept), splitting `state.rs` into
  files (now 3900+ lines), the tab display width, and the exact `p` semantics of visual
  charwise + a linewise register — all unchanged.

### UX notes (this round)

- Error messages aligned with vim's error-code family: E488 (trailing argument), E477
  (stray !), E16 (out-of-range), E348 (no word to search) — the host status bar can now
  show the same wording as vim, and the error for input like `:5>` was corrected from a
  misleading E16 to the honest E492/E16 combination.
- `3:` now prefills the `.,.+2` range (same as vim), visible in the prompt and still
  editable.
- parity_round8's two O tests and three assertions from round3/round13 were rewritten
  per the new probes with the reason for the correction noted (round8's probe10 was a
  probe misreading that the tests then froze for three rounds).
- Half of fuzz round14's seeds pre-register `nnoremap j gj`, so the cmd_seq guard and the
  mapping-expansion replay stay under continuous random-sequence bombardment.

### Performance notes (re-checked this round)

- Full bench_probe re-run, **item by item identical to the pre-fix baseline commit** (w
  6.0 vs 6.3µs/key, a 1MB line 173 vs 186µs, hlsearch rescan 0.30 vs 0.30ms, `:%s` 6.1
  vs 5.8ms, `n` 3.6 vs 3.7µs) — this round's changes to the motion/search hot paths (the
  find_from rewrite, the `*` chain) show no measurable regression. Against the 2.1µs
  recorded in round13's NOTES the whole curve drifts about 3×, reproducible within the
  same commit, judged to be a machine-state difference (absolute values are only
  meaningful within the same run; round13's numbers are void).
- fuzz round14 has the same budget as round13 (16 seeds × 60 rounds × 120 steps) and the
  same runtime.

## 0⁺⁺⁺⁺⁺⁺⁺⁺⁺ — Round 13 review additions (2026-10-02, regression tests in
`tests/parity_round13.rs` (30 cases), fuzz in `tests/fuzz_round13.rs`)

This round three independent code-reading agents swept cmdline/config,
ops/motions/objects/word and insert_mode/keymap/tables/tck, plus my own read of
state.rs' whole pipeline; fuzz for the first time put host mouse events
(`set_cursor_offset` clicks, `set_visual_range` drag-selects), uppercase marks and
the macro character surface into its key table — all five new bugs were caught by
this round's new fuzz invariants. Every semantic claim was first re-checked against
local vim 9.1 (including the 5-probe set on block-insert BS, which overturned an
expectation recorded in round8).

### Engine invariant bugs caught by fuzz (5+1)

| # | Problem | Root cause |
|---|---------|------------|
| 1 | **A block copy landed inside a multi-byte character** (host insert_str panic) | The leftover count-repeat from `3o` inserted 83 bytes first at block-session `exit_insert`; the block copy's offsets were shifted only by the typed line's delta and missed those 83 bytes. Fixed on three layers: `begin_insert` clears the leftover repeat; `replicate_count_insert` bails out for block sessions; copy offsets finally floor onto a character boundary |
| 2 | **A host click in visual mode collapsed the selection** | `set_cursor_offset` also moved the anchor to the click point, making the selection zero-width so `d` deleted only one character. Now, per `:h visual-use`, only the cursor moves and the active selection is kept in sync |
| 3 | **The `cmdline_visual` snapshot leaked** | Close paths other than executing or cancelling visual `:` (e.g. exiting indirectly during a `@:` replay) never consumed the snapshot, so the host saw a stale frozen selection through `visual_selection`. The snapshot is now consumed only by the prompt, plus a sweep at the end of `handle_key` |
| 4 | **`visual_selection` reported the live anchor while in cmdline mode** | It should report the pair frozen when the prompt opened (the live anchor may already have been broken by later edits); the frozen pair is now part of the exit cleanup |
| 5 | **`marks.active_visual` was not adjusted by edits** | The active selection was not in `for_each_pos`'s walk — after an Ex command changed or deleted text during visual `:` it had not shifted at all and `parse_range` received a mid-character offset |
| 6 | **`refresh_highlights` was dead code** | The large-file host API was short-circuited by its own `hlsearch_live_update` guard: with the per-edit rescan off, an explicit refresh was always a no-op and the match cache never updated. The guard now stays only on the per-edit path |

### Read-code + fuzz fixes (all vim 9.1 probe evidence)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 7 | **`~` deleted combining marks and the ZWJ family tail** | It consumed the whole grapheme cluster but wrote back only the mapped base character — `e`+U+0301 became `E` (3 chars → 2) and an emoji family lost members. The cluster's tail slice is now carried back verbatim; `gu`/`gU` were already correct (the asymmetry is gone) |
| 8 | **Linewise paste dropped trailing empty lines** | `trim_end_matches('\n')` swallowed every separating newline: register `"a\n\n` (lines a, "") pasted a single line. Changed to `strip_suffix`, which consumes exactly one |
| 9 | **visual Y/D/X/C/S used charwise semantics** | Probes: v_D/v_X delete the covered **lines**, v_Y yanks linewise, v_C=v_S change the line (v_x/v_s stay charwise). Added `VisualCmd::LinewiseOp` over the covered-line span |
| 10 | **Block `A` did not pad the shorter lines** | Probe `"123456"/"12"` at block columns 0-2 → `"123 X456"/"12  X"`: short lines are padded to **the append column** (col_hi, the exclusive endpoint); the old filter only padded lines ending before col_lo. Also fixed the subtraction overflow caused by `then_some`'s eager evaluation |
| 11 | **`ap` swallowed only one blank line** | Made directional: from a text line it swallows **all** following consecutive blank lines; from a blank line it swallows **the whole next paragraph** (not its trailing blanks) — all three probe shapes now align |
| 12 | **Block-session BS/divergence semantics were the inverse of vim's** | 5 probes: BS deletes the last typed character only at the end of typing (a BS mid-typing does nothing and never nibbles at line content); a divergent session whose typing was deleted by `<Del>`/`<C-w>` is **not replicated**. Exit-side delta guard (typed-line increment ≠ copy length → skip replication) + BS position limit. round8's "divergent sessions still replicate" expectation was corrected per the probes |
| 13 | **`:1,2d 3x` deleted the wrong set of lines** | The trailing-argument garbage `unwrap_or(1)` re-anchored the typed range to the last line. 0 = no count |
| 14 | **`:w!`/`:wq!`/`:x!`/`:noh!` gave E492** | The bang forms were missing (one of the most-typed command families); `:x`'s "do not write when unmodified" is left to the host (a documented contract) |
| 15 | **Ex abbreviations missing** | `:de/:del/:dele/:delet`, `:ya/:yan`, `:jo/:joi`, `:su…:substitute` (a static spelling table; `:ju` is a different command, see divergence #26) |
| 16 | **A no-op overwrote the `.` replay record** | `begin_edit` speculatively set `recording_mutated`: after `x` on an empty line or `p` with an empty register, `.` replayed the no-op instead of the last real change. The set point moved inside the three `edit_*` funnels |
| 17 | **The count survived visual `3"<Esc>`** | The Esc check moved before the `"` register prefix (normal mode's ordering was already correct) |
| 18 | **The count leaked for `q`/`q{reg}`** | Starting or stopping a recording did not clear the count, so `x` after `2q…q` deleted two characters |
| 19 | **`q/` hijacked the macro slot** | `q{reg}` now takes only a-zA-Z0-9; `qA` appends to the recording (vim semantics; the old version opened a separate A slot) |
| 20 | **`:5 +2d` gave E16 / the mid-range `%` semantics were inverted** | Whitespace inside an address is skipped (as in vim's get_address); `%` in the middle of a range expands to `1,$` (the old version treated it as the cursor line) |
| 21 | **A line-leading `"` comment gave E492; `:set ts=4 " note` mis-reported** | Whole-line comments are skipped silently; `:set` stops parsing arguments at `"` |
| 22 | **An insert mapping prefix swallowed characters** | With `:imap jk <Esc>`, typing `jx`: `j` was answered Consumed by the mapping prefix and, once `x` broke the prefix, the single-slot IOU held only `x` — `j` was lost from the buffer. `pending_unknown_char` became the queue `take_pending_unknown_chars()` (the single-character spelling kept) |
| 23 | **The cmdline prompt lacked editing keys** | C-h normalised to C-BS; C-w deletes a word, C-u clears the line (the complete editing model of a cmdline window is still out of scope, see the open items); history de-duplication is now non-adjacent (vim moves the older duplicate to the end) |
| 24 | **`o`/`O`/`cc`/`S` ignored noautoindent** | Indent copying/preservation is now gated by the `autoindent` option (the engine's default ai=true had masked this) |
| 25 | **`let g:mapleader` silently failed; TAB-separated mappings fell into ignored** | The `g:` scope spelling works; both the map command boundary and the lhs/rhs split accept TAB |
| 26 | **The TCK reference implementation's offset_to_line panicked on mid-char slicing** | A host copying the reference implementation verbatim crashed on ordinary offsets; guard added, plus a new "phantom line tolerance" contract clause (`line_range(line_count)` returns `len..len`) |
| 27 | **map_depth ratcheted across calls** | The early returns of Wait/Done never reset it: after a few dozen keys a self-referencing mapping raised a false runaway and wiped the freshly typed text |

### Review findings disproved (probe vs code-reading inference)

- **`:actions x` mis-routed into the `:action` bridge**: not true — after
  `strip_prefix("action")` the boundary filter requires a leading space, `"s x"` does
  not match, and it goes to E492.
- **Block `A` should also pad a line that "ends exactly at col_hi"**: not true — when
  the line width = col_hi (exclusive) no padding is needed and `range.end` is the
  append column.

### New known divergences (continuing the global numbering)

35. **Blockwise visual `l` does not go past line end**: vim's block can enter virtual
    columns (on a short line `2l` pushes the block's right edge to col 2; the probe
    `<C-v>j2lA_X` on `"123456"/"12"` pads to col 2 and appends at col 3), while the
    engine's `l` stops at line end — this needs virtual-column block geometry and is
    deferred.
36. **Left/right arrow keys do not cross lines in insert mode**: vim's `<Left>`@col0
    goes to the previous line's end; the engine stays in place (which also keeps block
    sessions safer).
37. **The cmdline prompt has no cursor model**: Left/Right/Insert/Tab completion are not
    implemented; the buffer is only append/pop. C-w/C-u/C-h and the up/down history
    cover everyday use. `:ju[mp]` still see #26.


## 0⁺⁺⁺⁺⁺⁺⁺⁺ — Round 12 review additions (2026-10-01, regression tests in
`tests/parity_round12.rs` (37 cases), fuzz in `tests/fuzz_round12.rs`)

This round ran on two tracks: two independent code-reading agents swept
`cmdline.rs`/`options.rs`/`config.rs` and
`word.rs`/`objects.rs`/`motions.rs`/`search.rs`/`registers.rs`/`marks.rs`/`insert_mode.rs`;
every claim was first re-checked against local vim 9.1 and then verified on the
engine — **a fix went in only when both legs agreed** — about 1/3 of the agents'
code-reading inferences were disproved by probes (see the end of this section).
fuzz's key table covers every surface fixed this round, with a new 600-round
random-concatenation bombardment of Ex range garbage arguments.

### New fixes (all vim 9.1 probe evidence unless marked fuzz / closing an open item)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **The Ex range scanner panicked on multi-byte input** (user-triggerable) | `:'中d`/`:,'中d`: the scanner's `i += 2` cut 中 (3 bytes) in the middle of a byte and `line.split_at` panicked. It now advances per character; an invalid mark name reports `E78: Unknown mark` (same as vim) |
| 2 | **The address offset parsed only its first segment** | `:5+2+1d` should delete line 8; the engine deleted line 5. The offset chain now accumulates (`with_offset` loops over the `+n`/`-n` segments) |
| 3 | **`w`/`b` parked on whitespace-only lines** | Probe: on `abc␤   ␤def␤`, `w` lands on the `d` of def — vim stops only on **truly empty** lines. `is_empty_line` replaced the `line_is_blank` test |
| 4 | **`e` returned `buf.len()` at the last word's end** | `ye` swallowed the trailing newline (the register got `"c\n` and `p` split a line). It now returns in place (motion-failure semantics), so `ye` gives `"c` |
| 5 | **`d$`/`dg_` swallowed the newline on an empty line** | On an empty line the Inclusive endpoint is the `\n` position, so extending by 1 byte merged two lines (probe: it should be a no-op). Empty and whitespace-only lines now take an Exclusive empty span; plus a general guard on the ops side: an Inclusive endpoint never swallows a `\n` |
| 6 | **`aw` on whitespace took only the whitespace run** | Probe matrix: `yaw` between words = `"   bar"`, `daw` deletes it leaving `"foo"`, on a whitespace/empty line the object crosses lines to the next word, and `daw` also deletes the next line's newline (3 lines become 1). `blank_run_plus_next_word` now carries all these shapes, with line merging on the Delete/Change side |
| 7 | **`d-`/`d+` deleted the current line at the buffer edges** | Probes: `d-` on the first line and `d+` on the last are both no-ops — with no adjacent line the motion must fail; the original saturating handling absorbed the out-of-range value and the linewise span then deleted wrongly |
| 8 | **`2*` jumped only one match** | The count was hard-coded to 1 and never passed into `jump_to_match`. `2*` now lands on the second next match (probe col 9) |
| 9 | **`2gN` repeated itself** (confirmed by reading the code) | The backward iteration set `from = range.start`, which always contains itself, so it re-selected the same match forever. From the second step on it is strict (skipping the containing preference) + wrapscan added (`2gN` on the first match wraps to the last) |
| 10 | **Between two quote pairs, `ci"` opened the next complete pair** | Probe: with the cursor on `then` in `say "hi" then "bye"`, `ci"X` → `say "hi"X"bye"` — **the left quote acts as the opening quote** paired with the next one; only when there is no quote on the left does it take the first pair on the right. The old implementation blanked out bye |
| 11 | **Replaying a ranged `:s` lost the range** | `:3,4s` (empty-argument replay) landed on the cursor line — visual's prefilled `:'<,'>s` is a high-frequency path. Replay now goes through `ex_substitute`, carrying the given range |
| 12 | **`:s` did not update the search state** | Probe: after `:s/x/Y/`, `@/` = `x`, `n` searches for `x` and hlsearch highlights `x`. `set_pattern` is now synced after the edit |
| 13 | **`:2,2j` wrongly joined two lines** | vim's `:h :j`: a range whose two addresses are equal does nothing (a single address `:2j`/a bare `:j` still joins the next line). `parse_range` now reports the address count so this can be decided |
| 14 | **`:d _` polluted the numbered registers** | The blackhole register was discarded by the argument parser while its content still went into `"1`. Special register names (`_ + " . - : % # *`) are now recognised and `_` goes to the blackhole |
| 15 | **An explicit count=1 was treated as no count** | `:1,2d 1`/`:1,2y 1` should take only line 2 (EX_COUNT re-anchoring); the `count > 1` test became `count >= 1` (three places: d/y/j) |
| 16 | **`:set ic/hls` did not refresh existing highlights** | Probe: after `:set noic` the hlsearch match set re-renders immediately under the new rules. Search-class options are recognised and the active pattern re-published |
| 17 | **`/^$` always reported E486** | The search path filtered out every zero-width match. They are kept now (the `ggn` probe lands at the empty line's start); only the phantom position after the trailing newline is dropped |
| 18 | **`zz`/`zt`/`zb` were missing in visual mode** (closing round11's open item) | Registered in the visual trie: anchored scroll reporting + **the selection is preserved** (vim's behaviour) |
| 19 | **`:marks` listed `.` twice** | The `.` set by `m.` and the last_change entry each produced a row. Deduplicated |
| 20 | **`:5>` reported a misleading E16** | `>` was removed from the range alphabet (`'>` still resolves through the mark branch), so `:5>` now honestly reports E492 (the shift command is not implemented) |

### Code-reading suspicions disproved by probes (so they are not re-checked next round)

- `w` landing on the `buf.len()` phantom position in a trailing-newline buffer: the
  existing clamp was already correct (the v1 assertion passed).
- `N` jumping from the middle of a match to the current match's start: the engine agrees
  with vim (the `rposition` semantics are right).
- A single-line `daw` between words deleting only the whitespace run: that same-line
  shape **was** indeed wrong (see fix 6), but the agent's claimed "vim gives `foobar`" probe
  result is actually `foo` — fixed per the probe in the end.
- vim's E486 for `:/^$`: a double-escaping bug in the probe (the `@/` actually held
  `^\$`), not vim's behaviour.

### New known divergences (continuing the global numbering)

33. **Ex commands such as `:>`/`:m`/`:t`/`:co`/`:k` are missing** → E492. Ex
    move/copy/indent are everyday vim commands; this round corrected the pseudo-E16 for
    `:5>` into an honest E492.
34. **Huge address offsets saturate**: `:1+99999999999999999999` saturates to the last
    line in the engine; vim reports an E16-class error (overflow detection). (**closed in
    round15**: measured, vim reports E1247 and does not execute; round12 #2's offset-chain
    rewrite had turned this path into `unwrap_or(0)` — worse than the "saturation"
    recorded here, since it deleted the wrong line. Both directions now give an explicit
    E1247.)

### Still unresolved (recorded this round, unchanged)

- `:s//x/`'s empty pattern reuses "the previous search pattern"; vim prefers "the
  previous substitute pattern" and falls back to the search pattern only when there is
  none (the engine keeps a single last search).
- The `\/` separator escape does not exist: `:s/a\/b/x/` parses into pieces (a failure in
  the parsing layer).
- `:set so=-1`: scrolloff is the only one of the four numeric options that may legally be
  negative, and `parse::<usize>` rejects it. The `:se` abbreviation, `:setlocal` and
  `:set ts&` are missing too.
- Browsing the cmdline history does not re-issue incsearch; the `|` command separator is
  silently swallowed by the argument parsers of `:s`/`:d`/`:y`/`:j` (`:set`/`w`/`q` at
  least report an error).
- `:d abc` takes only the first character as the register and swallows the trailing
  garbage silently (vim reports E354/E488).
- `config.rs` and `ex_set` parse the same `set` syntax with contradictory precedence (the
  `no` vs `=` priority differs between the two implementations); a shared parse function
  should be extracted.
- `search::compile` always returns `Some` (a dead branch) and `SearchState.last_index` is
  dead state — low-cost cleanup items.
- The ambiguous leading numeric argument of `:d`/`:y` = register (round10 open item)
  partly converged: the explicit count semantics are aligned, but the argument surface of
  `:'a,'bd 1` still needs separating.
- `gv` with no previous selection (round11), the cmdline history limit (round11),
  splitting `state.rs` into files (round9, now 3700+ lines), the Replace BS restore stack
  (round9), the tab display width (round7) — all unchanged.

### UX notes (this round)

- **The vim 9.1 probe methodology** (to reuse directly next round): under `-es`, when
  `-c` runs, the cursor starts at the **last line's start**; inside `:normal` a failing
  key (such as `j` on the last line) **aborts the rest of the sequence** — so positioning
  is always made explicit with `execute "normal! gg..."`; a `\` inside
  `execute "..."` needs double escaping (this round's `/^$` probe was wrong for one round
  because of it); results go through `writefile(getline(...))` rather than `:%p` (the
  latter is disturbed by buffer state).
- fuzz round12's key table grew to ~150 keys following "this round's fix surface"; added
  `ex_range_garbage_never_panics`: 600 rounds of random address/offset/separator/command
  concatenation, where only failure — never a panic — is allowed.
- The `2gN` regression assertion uses `marks.active_visual()` (for a single-character
  match, `visual_selection` with anchor==cursor is a degenerate shape).

### Performance notes (re-checked this round)

- Full bench_probe re-run: `w` 2.1µs/key (11KB), `w` on a 1MB single line 107µs/key,
  hlsearch rescan 0.25ms/round (900KB/10k matches), `:%s` on 600KB 5.3ms, `n`
  repeat-jumps 3.7µs/key (ropey) — the same order as round11 or slightly better (this
  round's changes are off the hot paths).
- After `all_matches` kept zero-width matches, the match count for patterns like `/^` is
  still capped by `take(10_000)`, so incsearch's per-key cost is unchanged.

## 0⁺⁺⁺⁺⁺⁺⁺ — Round 11 review additions (2026-10-01, regression tests merged into
`tests/review_regressions.rs`, fuzz in `tests/fuzz_round11.rs`)

This round read all of `src/` through, listing suspects from the code → verifying them
against vim 9.1 → fixing; fuzz's key table gained the mark-operator surface and this
round's fixed paths, and a 512-seed offline bombardment caught one search-cache
violation. Two code-reading suspicions were **disproved / unobservable** by probes and
left unchanged: single-line block `O` (vim's "diagonal, same column" is an identity when
the rows are equal — a block operation's column range does not depend on the anchor
direction; the selection-extension probe gave identical results in both shapes); `<C-a>`
on a leading-zero `077` → `0100` (the engine was already right, with the width growing
naturally).

### New fixes (1-5 and 8, the semantics class, each first run through a vim 9.1 probe; 6-7 were caught by fuzz)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **A bare command with a range put the cursor on the wrong line** | Probe: `:2,5<CR>` lands the cursor on line 5 (the range's **last address**); the engine used to land on `first`, so `:2,5` landed on line 1. The `:%<CR>` family is the same (it should land on the last line) |
| 2 | **A multi-address range was not truncated** | Probe: `:1,2,3d` on ['a','b','c','d'] deletes lines 2-3 — vim keeps only the **last two** addresses; the engine kept first-and-last and deleted 1-3 |
| 3 | **Empty addresses were skipped** | Probe: `:,3d` on line 2 deletes 2-3 and `:2,d` on line 3 deletes 2-3 — an empty address defaults to **the current line**; the engine skipped the empty segment, so both commands deleted only a single line |
| 4 | **`&`/a bare `:s` replay kept the flags** | Probe: after `s/a/B/g`, both `&` and `:s` on "xaxax" replace only the first match — `g` does **not** carry into the replay (`:h :&`). The engine replayed the whole command line verbatim. The trailing flag segment is now stripped before replay |
| 5 | **Mark jumps as operator targets were missing entirely** | `d'a`/`` d`a ``/`y'a`/`c'a` did nothing silently and the dangling operator polluted the next key (the JumpMark arm of `complete_char_arg` only moved the cursor). Now it mirrors the Find arm: under an operator, `Motion::MarkJump` takes the span through `span_from_motion` (`'a` line-wise across to the mark's line, `` `a `` character-wise exclusive, direction-independent; all probes aligned) |
| 6 | **Stale search cache for the block-insert pad edit** (fuzz round11) | Block-select `A` padding short lines with spaces went through `edit_insert` but never re-ran the hlsearch scan, so `last_matches` stayed at pre-edit offsets; on multi-byte text the mid-char case appeared (after the pad shifted the "b中" line, the cached end=20 fell inside 中). Added `republish_search`, under the same rule as the other edit paths |
| 7 | **Stale cache on the IME commit-replacement path** | `VimState::replace_range` (the host committing composed text) did not republish after editing. Filled in the same way |
| 8 | **`<C-a>` with the cursor on a radix-prefix letter produced garbage** | Probe: on `0x1f` with the cursor on `x`, C-a → `0x20` (vim treats the prefix letter as "the number is at the cursor"); the engine treated `x` as ordinary text and grabbed the bare digit run, assembling `00x2f`. The prefix-letter path now anchors back to the leading `0` and requires legal digits to follow (the `b` of `0backup` does not count as a digit; vim leaves it alone) |

### New known divergences (continuing the global numbering)

32. **The reversed range `:5,2d`**: the engine silently swaps first and last; vim
    interactively prompts "Backwards range given, OK to swap (y/n)" and refuses to
    execute under `-es`. The engine has no y/n interaction channel, so it takes the swap
    semantics (the same trade-off as IdeaVim).

### Still unresolved (recorded this round, unchanged)

- `gv` with no previous selection: the engine stays silent. vim most likely reports an
  error (under `-es` `mode()` is distorted, so the exact error code could not be
  probed); whether to add the E448 text waits for a reliable probe.
- visual mode `zz`/`zt`/`zb`: the engine's trie has no such row → bell and cancel
  pending; vim scrolls and preserves the selection. A small gap: the three rows need
  registering in the visual trie.
- The cmdline history has no limit (vim has the 'history' limit, default 10000).
  Session-level memory grows slowly (one command text per entry); adding the option plus
  truncation is low-value, deferred.
- The leading numeric argument of `:d`/`:y` (round10 open item), splitting `state.rs`
  into files (round9 open item, now 3700+ lines), the tab display width (round7 open
  item) — all unchanged.

### UX notes (this round)

- fuzz round11's key table is ~200 keys, extended along "this round's fix surface + the
  mark-operator surface"; besides the invariants it now bombards changelist/jumplist
  stepping (`g;`×5 + `<C-i>`×3 interleaved every 7 steps) — the floor guards of both
  lists have had standing pressure ever since.
- If `m.` has set the `.` mark, `:marks` lists `.` twice (one row from items() and one
  from last_change). Purely cosmetic, deferred.

### Performance notes (re-checked this round)

- Full bench_probe re-run: `w` 3.3µs/key (11KB), hlsearch rescan 0.30ms/round, `:%s` on
  600KB 5.0ms, `n` repeat-jumps 3.7µs/key (ropey) — the same order as round10. The new
  pad-republish only triggers on the block-select `A` short-line padding path, off the
  hot paths.

## 0⁺⁺⁺⁺⁺⁺ — Round 10 review additions (2026-10-01, regression tests in `tests/review_regressions.rs`,
fuzz in `tests/fuzz_round10.rs`)

This round read all of `src/` through: list suspects from the code → verify against vim
9.1 → fix. fuzz's key table grew a lot (history browsing, the full text-object set,
count-insert, `:set` options, indent counts, insert `<C-r>`, IME injection containing
newlines) and gained a `search.last_matches` addressability invariant; its very first
run caught one stale-cache violation. Two code-reading suspicions were **disproved** by
probes and left unchanged: the range semantics of `&` (`last_substitute` already stores
the command body with the range stripped, so both `&` and a bare `:s` land on the
current line; the vim probe `ba|bb|ba` agrees with the engine); and "a macro register is
cleared when its replay contains `q`" (a constructed probe proved it unreachable — as
soon as the expanded `q` executes it stops the recording and is popped, so a
reproducible `q` can never be recorded inside a macro, same as vim).

### New fixes (1-2 and 4-5, the semantics class, each first run through a vim 9.1 probe; 3 was caught by fuzz)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **count-repeat insert had no allocation cap** | `99999999i` + long text + `<Esc>`: `replicate_count_insert` did not guard `text.repeat(copies)`, so one small session could generate a GB-scale insertion at exit (the register-paste path had long had a 16MB line of defence). It now goes through the same `clamped_repeat_count`; the linewise branch also moved from a per-iteration `format!` loop (O(n²)) to a `repeat` of the unit string |
| 2 | **No cap on huge `:set ts/sw=` values** | tabstop/shiftwidth feed indent synthesis of the `" ".repeat(n)` shape (insert Tab, `>>`), so `:set sw=99999999999` plus one indent is a GB-scale allocation. Numeric options are now clamped to 1e6 at the `set_value` boundary (vim's limit is 2^31-1, divergence 31) |
| 3 | **Stale search cache after insert-mode edits** (fuzz round10) | `<Del>`/`<BS>`/`<C-w>`/`<C-u>` edited the buffer without re-running the hlsearch scan, so `last_matches` kept the pre-edit offsets and a deletion could land exactly inside a multi-byte character (after deleting 'a' from "a中b" the cached 1..4 pointed inside 中). `cancel_cmdline` published that cache verbatim to the host — a host doing `offset_to_line` on the highlights then panicked (the same exposure class as round9's `insert_change_pos`). Added `republish_search` on the four paths, same rule as the IME typing path |
| 4 | **Zero-width replacements were broken altogether** | `:s/^/>/`, `:s/$/</`, `:s/a*/-/` changed nothing silently and falsely reported E486 — empty matches were skipped by the "do not count, do not replace" policy (a round7 open item). Verified against vim: `:s/^/>/` inserts at line start (an idiom), `:s/$/</` appends at line end, a non-global `x*` matches vim. Empty matches are now counted and expanded normally. The remainder is in divergence 30 |
| 5 | **A range with an unset mark reported E16** | Verified against vim: `:'<,'>d` (no prior visual) → `E20: Mark '< not set`; named marks behave the same. `parse_range` now separates "range syntax error (E16)" from "mark not set (E20, with the mark name)" |

### New features

- **`VimState::macro_len(reg)`**: a read interface for the step count of a macro
  register (status bar / test observation; macros are stored as a step list and are
  invisible through the text-register channel).

### New known divergences (continuing the global numbering)

30. **Line-end advancement for a zero-width match with the `g` flag**: vim's
    `:s/x*/-/g` on "xabcx" → `-a-b-c-`, and `:s/a*/-/g` on "bbb" → `-b-b-b` — it does not
    take "the last empty match at the line-end position"; rust regex produces one extra
    empty match at line end, so the tail gains one more replacement character
    (`-a-b-c--`/`-b-b-b-`). `^`/`$`/non-global all agree. Aligning would need a
    hand-written replacement scan with its own advancement rule; the cost outweighs the
    benefit.
31. **The numeric-option cap is 1e6** (`ts`/`sw`/`tw`/`so`): vim's cap is 2^31-1. A
    defensive tightening (fix 2); a 1e6 column already far exceeds real use.

### Still unresolved (recorded this round, unchanged)

- `g&` (the global repeat of `:s`; vim replays over the `%` range) is not implemented.
- When the first argument of `:d`/`:y` is a number it is parsed as COUNT (`:y 1` = count
  1); vim also accepts a digit as a legal register name (`:1y 2` writes into `"2`). An
  extreme edge; aligning requires distinguishing "a lone numeric argument = count, an
  explicit second numeric argument = register", deferred.
- `publish_incsearch`'s per-key whole-buffer scan is not governed by
  `set_hlsearch_live_update(false)` (that switch belongs to the edit paths); hosts with
  huge files should `set nois` themselves.
- The Replace-mode BS restore stack's position mismatch after a newline split (round9
  open item) and splitting `state.rs` into files (round9 open item) — both unchanged.
- The tab display width being counted as 1 (round7 open item) unchanged.

### UX notes (this round)

- fuzz round10's key table is ~150 keys: the new paths were filled along two lines,
  "this round's fix surface + the long-tail command surface"; after adding the
  `last_matches` invariant, keeping the search cache in sync with the text has had
  standing pressure ever since.
- `Fixture::feed`'s per-key semantics tripped me again while writing the E20 test (`'<`
  in normal mode is a mark jump; you have to enter `:` first) — the helper's comment
  already says so, noting it once more.

### Performance notes (re-checked this round)

- The four insert edit paths now do one extra hlsearch rescan per key (same rule as the
  typing path); hosts with huge files take the `set_hlsearch_live_update(false)` +
  `refresh_highlights` escape hatch, so the order of magnitude is unchanged.
- bench numbers are in the same order as round9 (this round's changes are not on bench
  hot paths).

## 0⁺⁺⁺⁺⁺ — Round 9 review additions (2026-10-01, regression tests in `tests/parity_round9.rs`,
fuzz in `tests/fuzz_round9.rs`)

This round read all of `src/` through — list suspects from the code → verify against
vim 9.1 → fix; then extended fuzz's key table to this round's fixed paths
(`<C-w>`/`3D`/`3C`/`r<CR>`/`<Tab>`/the full `:substitute` spelling/`:2j 3`/`:bfirst`) and
re-bombarded it offline (256 seeds × 300 rounds × 200 steps), catching one engine
invariant violation. Two code-reading suspicions were **disproved** by probes and left
unchanged: `:s/foo/` (a trailing separator with an empty replacement) — the engine
already replaced with the empty string (same as vim); and the cursor landing of `2rx` on
a multi-byte line (both end up on the last replaced character).

### New fixes (1-6, the semantics class, each first run through a vim 9.1 probe; 7 was caught by fuzz)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **`gq` doubled empty/whitespace-only lines** | Probe: `ggVGgq` on ["","",""] — vim keeps all three lines 1:1; the engine did an empty-paragraph flush for every blank line (an empty flush still emitted one line-terminating `\n`), turning 3 lines into 7; a whitespace-only line `"   "` additionally collapsed to empty (vim keeps it verbatim). `format_lines` now flushes only non-empty paragraphs and keeps separator lines verbatim |
| 2 | **`3D`/`3C` ignored the line count** | Probes: at (1,2), `3D` on a 4-line buffer → ['a','dddd']; `99D` on a 2-line buffer → ['a']. The semantics are "delete [count] lines, at least to the line end": the covered lines vanish entirely and the last covered line keeps its own terminating `\n` (it goes too only when the count touches the buffer's last line). `D`/`C` share `delete_to_end_span` |
| 3 | **insert `Tab` aligned by byte column** | Probe: a Tab at the end of a '中文' line (expandtab ts=4) — vim pads 4 spaces (filling to a multiple of ts by display column) while the engine, using byte column 6%4, padded only 2. Now uses `display_column` |
| 4 | **insert `<C-w>`/`<C-u>` did not join lines at line start** | Probes: at line start, `i<C-w>x<Esc>`/`i<C-u>x<Esc>` on ['aaaa','bbbb'] → ['aaaaxbbbb'] (only the newline is deleted). The old behaviour clamped at line start and did nothing (`aaaa\nxbbbb`). Joining inside a block-insert session rings the bell and is refused (the copy-offset assumption is that every line is independent; same family as round8's line-start BS) |
| 5 | **Ex commands recorded the changelist at the old cursor** | `:s`/`:d`/`:sort`/`:j` moved the cursor only after `bump()`, so the `.` mark and the changelist captured the position before the command (probe: after `:4s`, `g;` should land on line 4). Cursor placement moved uniformly before `bump`. Connected: the changelist's two boundary messages are now directional (E662 at the oldest end, E663 at the newest; the old behaviour reported E662 for both) |
| 6 | **`:s/foo` with no replacement rang the bell** | Probe: `:s/foo` (no trailing separator) equals `:s/foo//` — delete the match. The old behaviour belled and did nothing. The same family for block-select `p` when the register's lines run out: vim deletes the covered region directly (a 2-line register pasted into a 3-line selection → the third line is empty); the old behaviour repeated the register's last line |
| 7 | **`insert_change_pos` mid-char panic** (fuzz) | The first-typed position of an insert session was a bare offset that the edit funnels never adjusted; after a mid-session BS joined lines and moved the text, `exit_insert` recorded an offset inside a multi-byte character into the changelist, and when `:marks` listed the `.` row the host's `offset_to_line` panicked. Now floored to the current text at exit |

### New features

- **`:{last}j {count}`**: joins count lines starting from the range's last line (probe
  `:2j 3` joins lines 2-4); the old behaviour ignored the count and joined only two
  lines.
- **The full `:[range]s[ubstitute]` spelling**: normalised to the `s` spelling (vim's word
  alias); a word that is not a `substitute` prefix still goes to E492.
- **`:bfirst`/`:brewind`/`:blast`** (`:bf`/`:bl`): `VimHost` gained the
  `first_buffer`/`last_buffer` hooks (successful by default; a host that refuses rings
  the bell). Closes the first item of round8's "still unresolved".
- **`r<CR>` now lands the cursor at the start of the new next line** (probe: `r<CR>` on
  'abc' → (2,1)); the old behaviour clamped back onto the previous line. The feature
  itself had existed for several rounds; this fixes the landing.

### Still unresolved (recorded this round, unchanged)

- The Replace-mode BS restore stack mismatches positions **after a newline split**:
  typing `\n` mid-`R` and then BS leaves the overwritten-character stack out of sync with
  the real position (the engine restores by character stack, vim by position). A rare
  path that needs the stack turned into (offset, char); deferred.
- Deeper differences from vim in behaviour such as block-register plain `p` padding short
  lines and block-insert `I` inserting at the line end of short lines (vim grows/shrinks
  the block session) keep the v1 trade-off — vertical movement inside a block session is
  still refused with a bell, see round8's notes.
- `state.rs` at 3600+ lines: the module is sectioned clearly by "pipeline / edit funnels /
  normal / visual / char-arg", but any longer and it should be split into files (to do
  alongside the next big change).

### UX notes (this round)

- `Fixture::feed` (test domain) follows `Key::parse`'s semantics "one item = one key": a
  multi-character bare string (such as "up") is a Named key, so an Ex command line has to
  be fed character by character — stepped on once while writing tests, now spelled out in
  the helper's comment.
- `bench_probe`'s own `line_range` implementation does two `split('\n').count()` passes
  per call — waste in the probe itself, unrelated to the engine (it does not change the
  direction of the measurement conclusions).

### Performance notes (re-checked this round)

- Full bench_probe re-run: `w` 2.5µs/key (11KB), hlsearch rescan 0.30ms/round, `:%s` on
  600KB 5.9ms, `n` repeat-jumps 3.7µs/key (ropey) — the same order as round8. The first
  run's `w` 6.3µs was warm-up noise and fell back on re-run.

## 0⁺⁺⁺⁺ — Round 8 review additions (2026-09-30, regression tests in `tests/parity_round8.rs`,
fuzz in `tests/fuzz_round8.rs`)

This round read all of `src/` through — list suspects from the code → verify against vim
9.1 → fix; then extended fuzz's key table to this round's fixed paths and injected host
clicks, and a 64-seed × 150-round × 200-step offline bombardment caught three engine
invariant violations. Two code-reading suspicions were **disproved** by probes and left
unchanged: for a `:j` no-op at EOF vim also records a changelist + undo entry (the
engine's `bump()` actually matches); and after `dip` following a single blank line vim
really does write the blank line into the `"` register.

### New fixes (1-4, the semantics class, each first run through a vim 9.1 probe; 5-7 were caught by fuzz)

| # | Problem | Evidence / root cause |
|---|---------|-----------------------|
| 1 | **`gq` charwise duplicated the text before the cursor** | Probe: with the cursor on 'bb', `gq}` — vim re-formats the whole line starting at column 0; the engine's replacement span started at `span.start` (mid-line) while the reflowed text contained the whole line → "aaaa " appeared twice. `format_lines` now anchors the start at the first line's start (the same as charwise visual `v$gq`) |
| 2 | **`.` replay was polluted by an Ex command that changed nothing** | Probe: `:2` `x` `gg` `.` — vim replays only `x` ("one"→"ne"); the engine had left `:2<CR>` in the recording, so `.` became "jump to L2 then delete". The enter branch of `cmdline_key` now decides by `edit_generation` after `execute_ex`: no change (`:5`/`:noh`/`:reg`/E492/…) discards the recording; changing commands (`:s`/`:d`/`:j`/`:sort`) have already committed inside execute_ex |
| 3 | **`Vp` linewise paste damaged a surviving indent line** | Probe: `"XY` (linewise) + `V` selecting 'abc' + `p` leaves the surviving ' ghi' intact. The engine's `PutReplace` linewise branch inserted at the cursor parked by delete_span (first_non_blank, possibly mid-line) → `" XY"` with the indent lost. Line-level selections now insert from `span.start` (line start); the combination of a charwise selection + a linewise register is still open (see round7) |
| 4 | **An invalid `:s` pattern overwrote `last_substitute`** | A `:s/(/x/` that failed to compile had been recorded into `last_substitute` before `build()`, so `&` would replay a command that never succeeded. It is now recorded only after a successful build |
| 5 | **`gv` restored a mid-char anchor** (fuzz) | `last_visual` existed in two copies: `marks.last_visual` (read by `'<`/`'>`) and the engine-side `VimState.last_visual` (read by `gv`, carrying the kind). Round7 #11's refloor covered only the marks copy — after an equal-length replacement redrew the byte grid, the engine copy stayed inside a character and `gv` restored an illegal selection. `refloor_stored_offsets`/`sanitize_stored_offsets` now floor the engine copy too, and the `RestoreVisual` read path adds another defensive layer. **The root cause was then removed by refactoring**: `Marks::last_visual` became the single source of truth carrying the kind, and `gv`/`'<`/`'>`/`:marks` all read it; the hand-written engine-copy adjustment loop in the three edit funnels was deleted outright (`marks.adjust_*` covers it naturally), so this whole class of dual-copy desync bugs no longer exists |
| 6 | **Three cross-line holes in a block-insert session** (fuzz) | Round7 had locked only vertical movement / carriage return; a line-start BS joining lines, `<Del>` deleting `\n`, and IME text containing `\n` wrecked the copy offsets just the same (reproduced by fuzz: an insert beyond the end of the buffer). All three now ring the bell and are refused. Fixed along the way: `record_typed_text` pushed the content into `block.text` **before** `insert_text_at_cursor` refused it — so a refused newline still took part in the exit replication; appends now happen only once the insert has landed |
| 7 | **A host click flung the cursor away inside a block session** (fuzz) | Once click injection moved the cursor to another line, the copy-offset assumption (all typing lands on the typed line) broke. `set_cursor_offset`/`set_visual_range` now ignore movement during a block session — a real gpui/crossterm host clicked while typing would do the same |

### New features

- **Block-select `u`/`U`/`~`** (the same as `gu`/`gU`/`g~`): case flipping per line, with
  the cursor at the block's start (vim 9.1 probe: `<C-v>jllU` → ABC/DEF, cursor line 1
  col 1). The old behaviour rang the bell. Multi-byte lines resolve the block range by
  display column and the replacement goes through `edit_replace`.
- **Block-select `O`**: swaps the line ends while each side keeps its column — the
  selection rectangle is unchanged on screen, and the cursor lands on the anchor row at
  the cursor's column (vim 9.1 probe: anchor(2,2) cursor(3,3) → after `O`, cursor(2,3)).
  `O` in char/line mode rings the bell (vim supports it only for blockwise). The existing
  `o` = diagonal corner swap is unchanged.
- **`:marks` lists `.` and `^`**: the last change position and the last insert-exit
  position (vim lists them).
- **`:bN`**: vim's alias for `:bprev`.

### Still unresolved (recorded this round, unchanged)

- ~~`:bfirst`/`:blast` unsupported~~ (**supported in round9**: `VimHost` gained the
  `first_buffer`/`last_buffer` hooks).
- The exact `p` semantics of a visual **charwise** selection + a linewise register are
  still open (left over from round7; what this round aligned was V/V line-level
  selections, which the probes made clear).
- `parse_range` saturates **out-of-range positive line numbers** (`:5,10y` on a 3-line
  buffer) to the boundary while vim reports E16 — the same family as the negative-line
  divergence recorded in round7, harmless.

### UX notes (this round)

- The block-insert session's cross-line blocklist is complete from here on: vertical
  movement, carriage return, line-start BS, `<Del>` on a newline, text containing `\n`,
  host click/drag — all refuse with a bell or are ignored.
- `:reg` rendering dropped a redundant whole-text `replace` computation (twice per
  register → once).

### Performance notes (re-checked this round)

- Full bench_probe re-run: `w` 2.1µs/key (11KB), hlsearch rescan 0.25ms/round, `:%s` on
  600KB 5.0ms, `n` repeat-jumps 3.7µs/key (ropey) — the same order as round7.
- The bulk of `w` at 109µs/key on a 1MB single line is the naive String host's O(n)
  `offset_to_line` scan (3-4 calls per key); on a ropey host it is 3.7µs — no new hot
  spot on the engine side; `x` at 232µs/key is still dominated by the host's String
  memmove.
- The new last_visual dual-copy refloor is O(stored-offset count) per delete/replace,
  invisible at bench magnitudes.

## 0⁺⁺⁺ — Round 7 review additions (2026-09-30, regression tests in `tests/parity_round7.rs`,
fuzz in `tests/fuzz_round7.rs`)

This round read all of `src/` end to end: read the code to list suspicious points →
verify with vim 9.1 probes → fix. Two read-code suspicions were **falsified** and left
unchanged: `Options::describe("notabstop")` correctly returns None (the `no` prefix does
not wrongly swallow a numeric option); and both `:sort` on a single-line range and
`:sort u` on a single-line range are no-ops under vim, matching the engine's behavior
apart from its existing `!unique` guard.

### New fixes (every semantic case ran a vim 9.1 probe first; fuzz cases note how they were caught)

| # | Problem | Evidence/root cause |
|---|------|-----------|
| 1 | **`:'<,'>` reads a stale range after a visual operator** | Probe reproduction: after `Vjd`, `:'<,'>d` empties the buffer. `marks.active_visual` (the live selection) is not cleared once `finish_visual_op` commits it, and `parse_range`'s `'<` resolution reads it **first** — so every `'<` range command after a visual operator hits a stale offset. Fix: `finish_visual_op` clears the live selection |
| 2 | **Visual text-object extension does not update the live range** | Probe reproduction: after a cross-line `vi(`, `:s/ba/X/` only replaced the collapsed line. In visual mode only motion extension updates `active_visual`; object extension (`viw`/`vi(`) was missed. Fix: sync after object extension |
| 3 | **`gv` breaks after visual `c` / block `I`/`A`/`c`** | vim probe: after `viwcX<Esc>`, `gv` reselects the pre-edit byte range (`'<` stays (1,1), `'>` stays (1,5)); after block I it restores the original block. The engine's insert session bypasses `finish_visual_op`, so `last_visual` is never updated → gv misses. Added a `pending_visual_marks` staging slot: the three visual→insert entry points record the selection before the change, and `exit_insert` writes it back after the session ends (floored to the current text) |
| 4 | **Cancelling the `/` prompt leaks the highlight when `hlsearch=false`** | hlsearch off + incsearch on: after Esc, `cancel_cmdline` replaces the preview highlight with `last_matches` (the previous pattern's matches), leaving a permanent highlight. Fix: respect hlsearch on cancel |
| 5 | **`g<Esc>` beeps wrongly; the count of `3"<Esc>` survives** | normal_key's Esc check ran after the trie walk: `g<Esc>` took the trie-miss retry path and beeped; the register-pending branch swallowed Esc while the count survived, so the following `dd` deleted 3 lines. Fix: move the Esc check ahead of all pending states |
| 6 | **`:sort u` on a single-line range creates a phantom undo group** | A single line has no consecutive duplicates to dedup; the old `!unique` guard missed the `u` form — a no-change replace still announced an undo group, wasting one `u`. Fix: single line is always a no-op |
| 7 | **`'<`/`'>` rewritten after visual `:`** (closes divergence #14) | `:s` places the cursor at the first non-blank of the line; rewriting by "anchor..post-execution cursor" collapsed the selection and broke gv's second-pass semantics. Fix: `cmdline_visual` records the cursor at prompt time, and `close_visual_after_cmdline` commits using the prompt-time range (vim: keeps the executed range) |
| 8 | **`gn` leaves the cursor mid-char on a multi-byte match** (fuzz-caught) | The byte arithmetic `cursor = range.end - 1` lands inside a character when the match ends with a multi-byte character (e.g. `中`); the host's `offset_to_line`/`slice` panic outright. Same pattern in `state.rs` and `motions.rs`; switched to `prev_char_offset` to take the start of the last character |
| 9 | **`db` usize underflow panic at line start** (fuzz-caught) | The column-1 rule only modelled downward line crossing; when `db` crosses upward, `target_line - 1` underflows. vim probe: `db` at line start = linewise delete of the **previous line** (['abc','def'] → ['def']). Added the mirrored branch (covering target_line..=start_line-1) |
| 10 | **Stale cursor offset in the block-insert session** (fuzz-caught) | After `exit_insert`'s line replication inserts above the cursor, the cursor's byte offset did not shift with it — the logical position landed before `text.len()` bytes (mid-char on multi-byte text). Fix: shift the cursor in sync when the insertion point is above it. Side effect: in a block session an insert Enter can move the cursor to another line, breaking the replication assumption — blocked the same way as vertical motion (beep) |
| 11 | **Stored-offset addressability gap** (fuzz-caught) | Two forms escaped the relative adjustment: when a deletion touches the buffer end, offsets run past the new length (after emptying the buffer `last_visual` is out of range); an equal-length replacement (`gJ`'s `\n`→space) redraws the byte grid and leaves interior marks inside a multi-byte character. `edit_delete`/`edit_replace` added `refloor_stored_offsets` — all stored offsets (marks/changes/jumps/last_visual) floored+clamped back into the current text |

### New features

- **`:d {register}`**: the register argument of `:[range]d[elete] [x] [count]` landed
  (`:1,2d a` stores into `"a`, same as vim); argument parsing shares
  `parse_reg_count` with `:y`; without a register it still uses the numbered ring/`"-`
  path. Known divergence #7 is thereby closed.

### New known divergences (continuing the previous table's numbering)

29. After a block-insert session the cursor rests at the typed line's insert position
    (the logical position corrected by the replication shift); vim rests at the **top**
    line's insert column (probe `x<C-v>jjI#<Esc>` → [1,1]). The behavioral difference
    does not affect the text, so not pursued for now.

### Unresolved (recorded this round, unchanged)

- ~~**`:s` empty match**~~ **fixed in Round 10**: the vim probe overturned the hold
  (`:s/^/>/` inserting at line start is a vim idiom); empty matches now expand and count
  as usual; the residual (the `g` flag advancing at line end) is logged as divergence 30.
- `parse_range` saturates negative line numbers (`:-5`) to 0; vim reports E16. Harmless divergence.
- `set_cursor_offset` in visual mode moves the anchor to the click point (the selection
  collapses to zero width but stays in visual) — needs the consumer to confirm intent,
  the previous round's hold is maintained.
- Tab display width is accounted as 1 (a tabstop-aware display column would ripple through
  the whole chain); the previous round's hold is maintained.
- Loose argument validation for `set`/`mark` etc.: `m<Space>` is silently ignored (vim
  E355), `q:` records into register `:` instead of opening the command window. Harmless, deferred.

### Experience notes (added this round)

- `Esc` is now normal mode's highest-priority "cancel everything" key: any pending state
  (count/register prefix/operator/multi-key sequence) plus the highlight are cleared together,
  consistent with vim.
- The three read-only feedback channels `:reg`/`:marks`/`:set` (introduced in Round 6) remain in
  effect; `:d a`, completed this round, lets deletions also go into a named register, visible in `:reg`.

### Performance notes (re-checked this round)

- Full bench_probe rerun: `n` repeated jump 3.6µs/key, hlsearch rescan 0.30ms/pass
  (900KB/10k matches, better than the 0.75ms recorded in Round 6), `:%s` on 600KB 5.6ms.
  The bulk of the 232µs/key for 100x `x` deletes is the O(n) memmove of the host String
  buffer (the bench's `B` host `delete_range` is `String::replace_range`); no new hot spot
  on the engine side. The per-edit O(number of stored offsets) cost added by
  refloor_stored_offsets is invisible at this magnitude.

## 0⁺⁺ — Round 6 review additions (2026-09-30, regression tests in `tests/parity_round6.rs`)

This round read all of `src/` end to end (~8k lines), first reading the code to list
suspicious points, then verifying each case with vim 9.1 probes. Two read-code suspicions
were **falsified** by probes (the behavior was already correct, unchanged): `5s` on a line
with too few characters likewise only deletes to line end then enters insert in vim (same
clamping, not "cancel the whole thing"); and `:sort u` without `i` does not case-fold dedup
(`[foo,FOO,bar]` → all kept, byte order), the engine's original dedup semantics being
already right — folding is needed only when `i` participates.

### New fixes (every semantic case ran a vim 9.1 probe first)

| # | Problem | Evidence/root cause |
|---|------|-----------|
| 1 | **Visual `<Del>` deletes only the one character at the cursor** | vim probe `viw<Del>` deletes the whole selection (`hello world` → `hello `). `navigation_key` intercepts `Named("delete")` before the command table, so in visual mode it steals the command table's `<Del>`→`Operator(Delete)` row. Fix: in visual mode let that key fall through to the command table |
| 2 | **`2d3w` deletes 23 words** (count concatenated across the operator) | vim probe: 6 words (2×3 multiplied). When the operator key is pressed the prefix count stays in `count`, and the motion's digits keep appending via `×10+d`. The `op_count` field was previously dead state (nobody wrote it) — now, in the operator arm, `op_count = count.take()` and `take_total_count` multiplies. Consequently prefix counts on line-level double-writes like `2gUU` all go through this path |
| 3 | **`3S` clears only one line** | vim probe: `3S` clears lines 2-4 (['l1','X','l5']). `SubstituteLine` ignored count. Fix: take count, expand the line range, then use the existing linewise Change |
| 4 | **Linewise `p` lands the cursor on the original line when the last line has no trailing newline** | vim probe `yy p` on "abc" lands on line 2. The `p` "open a new line after the last" branch inserts at `buf.len()`, so the insert position points at the newly added separator `\n` (still the old line) and `offset_to_line(insert_at)` is off by one. Fix: +1 the cursor position in that branch |
| 5 | **`:1,2d 3` deletes L1-L4** (count widened the range) | vim probe deletes L2-L4: `{count}` is counted from the range's **last** line (help: "starting with the LAST line in [range]"); `:1,2y 3` likewise picks up L2-L4. Both `:d`/`:y` changed to `first = range.last` when count>1 |
| 6 | **`:sort iu` does not case-fold dedup** | vim probe `[foo,FOO,bar]` → `[bar,foo]` (after a stable i-sort the earlier-ordered variant survives, exactly matching Rust's stable sort + `dedup_by(lower)`; `u` without `i` stays case-sensitive) |
| 7 | **`:j` cursor lands at the seam** | vim probe: `:1,2j` on `['    aaaa','bbbb']` puts the cursor at col 5 (first non-blank of the merged line); bare `:j` col 1; only normal-mode `J` lands at the seam (control probe col 9). ex_join now lands at `first_non_blank(first)` |
| 8 | **Replace-mode BS at offset 0 swallows a stack item** (read-code find, not a probe round) | The position guard comes first: pop only when `at > 0`. The old behavior wasted one overwrite-stack item on a BS at buffer start, so a subsequent BS restored the wrong character (test: `R ab Home BS End BS` → `ay` instead of `ax`) |

### New features

- **`:set name?`**: report the current value rendered like vim (`ignorecase` /
  `noignorecase` / `tabstop=4`); the `no`-prefixed spelling is queryable (`noic?`); a
  query does not change the value. Bare **`:set`** lists every option line by line (the
  old implementation only beeped). `Options::describe` / `describe_all` are the new
  read-only query surface.
- **`:reg[isters]`**: lists non-empty registers line by line (unnamed first, named in
  order; `^J` denotes a newline, overly long content elided). **`:marks`**: named marks
  plus the `<` `'` special marks (`mark  line  col  text`). `Registers::items` added.
  Both go through the existing `status_message` channel, visible to the host with zero changes.

### New known divergences (continuing the previous table's numbering)

26. `:ju` (ambiguous join abbreviation) reports E492 instead of joining (`:j` and `:join` work).
27. `:sort`'s `u` dedup representative may differ from vim at the edges of "multiple
    variants + non-stable sort" — aligned under stable sort (see fix #6); only extreme inputs are in doubt.
28. Replace-mode BS's restore stack is modelled as "per-character LIFO": after a vertical
    move within the session, BS applies the previous line's restore item to the current
    line (vim's restore history includes line moves). Aligning needs a position-aware stack; deferred.

### Unresolved (recorded this round, unchanged)

- The `p` semantics when **the block register's line count and the selection's line count
  differ**: under `-es` the vim probe's block selection is distorted (the method used across
  four rounds failed to reproduce this round), so whether `rows.get(i).or(last)` or vim's
  "cyclically repeat the register lines" is right or wrong is not empirically verified; left alone for now.
- Whether the per-position expansion of **`:s` empty match** (e.g. `s/x*/-/`) matches vim's
  advancement rule is not probed; Rust regex `replace_all` expands each empty match once per
  position, while vim has its own skip rule. Rare, deferred.
- `parse_range` saturates negative line numbers (`:-5`) to 0; vim reports E16. Harmless divergence.

## 0⁺ — Round 5 review additions (2026-09-30, regression tests in `tests/parity_round5.rs`)

This round ran "probe-first": every semantic change was first verified with a vim 9.1
headless script (`-es` + `normal!`, note that in this mode the cursor initially lands on
the last line, so probes must explicitly anchor with `gg`) before touching the engine. Two
Round-4 conclusions were **overturned** after re-checking with probes: after `visual y`
vim's cursor **stays put** (it remains on the movable end — the old behavior was already
correct; Round 4's record had no such issue, but this round briefly changed it wrongly and
rolled back); and ex_set's parsing of `:set number` was actually correct (`number` does not
start with `no`; the first read-code judgment was a misread, falsified by probes).

### New fixes (every semantic case ran a vim 9.1 probe first)

| # | Problem | Evidence/root cause |
|---|------|-----------|
| 1 | **Visual indent operators ignore count**: `Vj3>` shifts only one level | vim probe sw=4 expandtab: `Vj3>` → 12 spaces, `Vj2>` → 8. count applies only to indent operators (`Vj3d` still deletes the selection once). The implementation parses the line range only once and loops `shift_line` directly — reusing `ops::apply` would reuse the old byte span, and after the leading indent is inserted the span drifts, so trailing lines miss the shift (measured: l1 shifted 3 times, l2 shifted 1 time) |
| 2 | **Unmapped visual keys don't clear count**: `V 3 & j` jumps three lines | vim cancels the whole pending. Both Miss branches (the cmd_seq continue branch + the single-key branch) added `reset_pending` |
| 3 | **No-op commands pollute the changelist**: after `x` on an empty line the second `g;` lands on a ghost change point | vim probe `jx gg x g;g;` stops on the real change line both times. `x`/`X`/`s`/`S`/`p`/`P`/`J`/`gJ`/`~`/`<Del>`/`r`/visual `r`/linewise operators all switched to `bump_if_edited` (decides a true edit by `edit_generation`) |
| 4 | **A mapping completing mid-expansion-queue was treated as a literal**: with `:imap a <Esc>` + `:imap q ax`, typing `q` — `a` did not trigger | `Trie::get` Missed the whole queue when "the prefix is already a leaf but more keys hang off it". Added `longest_terminal_leaf_prefix`: only a complete sequence that is exactly a leaf hits and consumes the prefix; a hit with children still goes through the existing builtin-vs-mapping resolution (that trade-off has a dedicated anti-regression test). Self-referential mappings are bounded by MAX_MAP_DEPTH |
| 5 | **`/<CR>` silent with no previous pattern**, **`:s//x/` with no previous pattern only beeped** | vim probe `/<CR>` → v:errmsg = E35. Both added `E35: No previous regular expression` + bell |
| 6 | **`:1y a 2` ignored count** | vim probe: the digits after the register are a count (two lines into `"a`). `:y` argument parsing changed to "first numeric arg = count; letter = register + optional second count" |
| 7 | **The TCK reference implementation violated its own `char_at` contract**: bare slices panic at non-boundary offsets | The engine's `floor_to_char_boundary` deliberately probes non-boundaries; the host copying the reference implementation would take a panic from a legal path (`"中文"` offset 1 reproduced in practice). Added an `is_char_boundary` guard, and promoted it from the test module to a public `tck::TckStrBuf` (hosts can reuse it directly) |
| 8 | **Visual `p` with the selection at line end inserts before the line-end character** (found by the downstream crossterm-vim integration test, not a probe round) | `PutReplace`'s insert position used `cursor.offset`, while `delete_span` parks the cursor via `clamp_cursor` — clamp is a "parking" semantic, pulling the offset back to the last character when it lands at line end. When span.start is exactly at line end (`v$`/`viw`) the paste is one byte early: on `"hello world"`, `yiw w viwp` → `"hellohello "`. The insert position now takes the deletion's real start `span.start`, leaving clamp only for the final cursor parking; regression test `visual_put_selection_at_line_end_inserts_at_span_start` (three forms: `v$`/`viw`/single-line whole word). Why the engine's own tests didn't catch it: existing visual put cases had span.start at line start, so the `end > line_start` guard made clamp an identity |

### New features

- **`:{range}sor[t][!] [i] [u]`**: line sort. `!` reverses, `i` ignores case, `u` dedups
  after sorting; the newline at the range end is preserved faithfully. A single-line range
  is a no-op (same as vim). The old path fed `:sort` into the substitute parser and only
  beeped. Flags like `n`/`x`/`o` are ignored.
- **`:{range}j[oin][!]`**: line join. A single-line range joins the next line (bare `:j`
  has this semantic); `!` joins verbatim (≈ `gJ`). Reuses `ops::join_lines`' separator logic.
- **`&` and bare `:s`**: repeat the last substitute command (`Cmdline::last_substitute`
  records on successful parse; after E486 replay the same command and report the same
  error); report vim's E33 when there is no previous.
- **Removed dead code `CmdKind::mutates()`**: after the lazy undo-group rework nobody calls
  it, and its documented purpose no longer exists (not referenced by the gpui_vim /
  crossterm_vim downstream).

### New known divergences (continuing the previous table's numbering)

23. Visual indent count only recognizes the **leading** form (`3>`); `>3` is likewise
    ignored in vim, so it is consistent — but in `Vj>3` the lingering count becomes the
    next motion's count (same in vim, not a divergence, noted for the record).
24. `:sort` does not support the `n` (numeric)/`x`/`o`/`b` flags nor `{pattern}` truncation.
25. `:j`'s count form (`:j 3`) is not implemented (use `:j` on `:,+1j` instead).

### Unresolved (recorded this round, unchanged)

- **`set_cursor_offset` moves the anchor to the click point in visual mode**: the selection
  collapses to zero width but stays in Visual mode — neither like vim's "click exits visual"
  nor like "drag to extend". Needs the consumer (gpui-vim's mouse path) to confirm intent
  before settling the semantics.
- The exact semantics of **visual `p` with a linewise register** (the edge case where the
  selection is replaced by line-level text) gave unclear vim probe results; both behaviors
  are in doubt, left alone for now.
- **Tab display width accounted as 1** (`char_display_width` returns 1 for control
  characters): j/k column preservation, `|`, and block selection on lines containing Tabs
  diverge from terminal rendering. A real fix needs a tabstop-aware display column, rippling
  through the whole `display_column` chain; not done this round.
- **`all_matches`' 10_000-match truncation**: beyond that, `n` only cycles within the
  truncated set. Huge-file + wide-match scenarios need the host to self-manage via the
  existing `set_hlsearch_live_update(false)` mechanism; the truncation value is left unchanged for now.

## 0 — Round 4 review additions (same day, regression tests in `tests/parity_round4.rs`)

Probe method upgraded: vim 9.1 verified case by case with `-es` scripts (note that under
`-es` setline does not enter the undo tree and the search-prompt behavior is distorted —
undo-class probes used the "count undo passes" method, search-class used buffer-result
comparison).

### New fixes (every semantic case ran a vim 9.1 probe first)

| # | Problem | Evidence/root cause |
|---|------|-----------|
| 1 | **`dw` wrongly deletes the line on a whitespace-only line**: `dw` on `"  \nbar"` deleted the newline too | vim probe `dw@0` → `['', 'bar']` (keeps the newline). The linewise promotion only holds for **truly empty** lines; a whitespace-only line goes through the "single w cross-line clamp" = span to line end. Related correction: a w landing on the buffer-end phantom position deletes the last line's newline too as exclusive-linewise (vim `d2w@blank` → `['AAAA']`) |
| 2 | **`^`/`gg`/`G` land on the `\n` of a whitespace-only line** | vim lands on the last space (col=3 on `"   "`). The old `first_non_blank` returned line end, so with the cursor after the `\n`, `diw`/`dw` swallowed the newline and joined lines |
| 3 | **`I` inserted at the wrong position on a whitespace line** | vim `"   "` + IZ → `"   Z"` (line end); `i` at `^` is before the last space (`"  Z "`) |
| 4 | **Phantom undo groups**: `x`/`X` on an empty line, `J` at EOF, `i<Esc>`, `p` with an empty register, `r<Esc>` each consumed one `u` | vim probe (undo-count method): none of these no-ops build an undo entry. Fixed to lazy groups: `open_undo_group` only records the id; the three `edit_*` funnels call `begin_undo_group` only before the first real edit (cursor still at the pre-edit position) |
| 5 | **Empty span polluted the register**: `dw` at buffer/line end wrote an empty string into `"-` | vim probe: `ci(` on `()` leaves unnamed unchanged. `delete_span`/`yank_span` return directly for an empty charwise span |
| 6 | **`<C-a>` broke hex/octal/binary**: `0x1f`+1 → `2x1f` | vim nrformats defaults bin,octal,hex: `007`+2=`011`, `077`+3=`0102`, `0x1f`+2=`0x21`, `0XAB`+1=`0XAC`, `0b101`+2=`0b111`, `0x10`-1=`0x0f` (zero-padded width), `010`-1=`007`. Prefix case preserved; decimal not zero-padded (`0099`+2=`101`, same in vim); a negative sign folds in directly (`ab-99` → `ab-98`, same in vim) |
| 7 | **Visual `p` with an empty register deleted the selection but didn't paste** | vim E353 and the selection stays intact. Now it beeps and keeps the selection; normal `p` with an empty register also beeps (was silent before) |
| 8 | **Block-insert session offset drift** (fuzz caught a host slice panic): a page key during the session moved the typing point to another line, and the replication offset landed mid-character | Two-layer fix: block sessions in insert mode block vertical motion (vim allows it and extends block semantics, which this engine does not model — rejected with a beep); `BlockInsert` now records the typed line's line number + line length, shifts by exact per-line-length increments, and BS retroactively shrinks the replicated text |
| 9 | **Cursor landed on a phantom position**: `w` on a word line landed at len (past the last character) | vim probe `wx` on "abc" deletes c. `clamp_cursor` unified to "if the line has content, land on the last character"; host click/drag, undo restore, and g;/C-o/'' entries all route through it. Accompanying: the clamp at the end of `apply` only applies to block cursors (after cw/cc enters insert the line-end offset is legal, and `viwc`'s `.` replay depends on it) |

### New features

- **`gn`/`gN`**: select a search match (the match at the cursor, else the next/previous).
  A normal key enters visual and selects; under an operator (`dgn`/`cgn`/`ygn`) the span is
  exactly the match itself, not dragging in the gap between the cursor and the match — the
  `cgn` + `.` replace-one-by-one workflow works (vim probe: three `b` each turned into `X`).
  count follows vim's `2gn` ≙ select after `2n`.
- **`:d [count]`**: aligned with `:y` (`:2d 3` deletes lines 2-4); the old implementation ignored it.
- **`<C-a>`/`<C-x>` radix support** (see fix #6).

### New known divergences (continuing the previous table's numbering)

19. `<C-a>` saturates large numbers at i64::MAX (vim probe 26 9s → u64::MAX wraparound).
20. `gn` with no pattern reports E35 + bell (vim opens the search prompt).
21. Vertical motion is rejected within a block-insert session (vim allows it and extends block semantics).
22. `/` in visual mode does not extend the selection (vim does); insert `<C-o>{cmd}` not implemented.

## 1. Round 3 review additions (earlier the same day, regression tests in `tests/parity_round3.rs`)

### New fixes (every semantic case ran a vim 9.1 probe first)

| # | Problem | Evidence/root cause |
|---|------|-----------|
| 1 | **`cw`/`cW` swallowed words across lines**: `cw` on the single-character word `b` deleted `b\nc`; `cw` on a line-end word ate into the next line | The old implementation rewrote the motion as `ce`, bypassing all of `dw`'s cross-line special cases. The correct rule is "**`dw`'s span with trailing whitespace removed**" — six probes (`cw` on a single-character word, `c2w`, `cW`, punctuation, tab, `cw` on a line-end word) all align only under this rule (`ce` itself does cross lines: `ce` on the b of `a|b|c` changes `b\nc`, so it cannot be relied on) |
| 2 | Mid-sequence bell on `gugu`/`gUgU`/`g~g~`/`gqgq` | Doubling completes via the "drop the first key and refeed" path; the trie miss of `g` beeped by mistake. Now silent only when the refed key completes a doubling (genuine failures like `dgx` still beep, same as vim) |
| 3 | `''`/`` `` ``/`'.` had nowhere to jump | Added `marks.last_jump` (maintained by `record_jump`) and the implicit-name branch of `resolve`; `g;`/`'.` aligned with vim (insert sessions see #4) |
| 4 | **changelist / jumplist did not shift with edits**: `g;`/`C-o` landed on stale offsets | The three `edit_*` wrappers now adjust both lists under the same rules as marks; after host undo/redo a full floor via `sanitize_stored_offsets` runs (relative adjustment is out of the question) |
| 5 | A pure `i` session never entered the changelist (`g;`/`'.` missed the typing) | vim probe: `g;` = the exact position of the first typed byte, `'.` = the first non-blank of that line. `insert_text_at_cursor` records the first input position and `exit_insert` commits it; `o`+Esc also counts as a change |
| 6 | `p`/`P` with an empty linewise register was a no-op | vim probe: insert one empty line. Only triggers on genuinely empty text such as `yy` on a single-empty-line buffer |
| 7 | Search failure beeped without a message | `n`/`N`/`*`/`/⏎` now report `E486: Pattern not found` / `E35: No previous regular expression` |
| 8 | Backspace on an empty prompt closed `/`/`:` | vim probe (`mode()` stays `c`): keep the prompt open |
| 9 | `99999999.`/giant-count macros queued hundreds of millions of keys (a measured 5-minute run) | The replay queue is prebuilt; the count is now clamped to the pipeline guardrail's 100k-step budget; rounds beyond the budget are simply not queued |
| 10 | `99999999p` = register × count bytes of allocation (OOM) | Pasted bytes capped at 16MB (`registers::clamped_repeat` / `ops::clamped_repeat_count`), on both normal `p` and visual `p` |
| 11 | `last_visual` out of range after a visual operator deleted its own selection (fuzz caught `0..17` stored on an empty buffer) | `finish_visual_op`/`exit_visual` now use `clamped_visual_bounds` (floor+min); the `unwrap_or(hi+1)` fallback at the buffer end changed to `hi` |
| 12 | fuzz's multi-char key items (`dd`/`iw`/`<C-a>`) were merged by `Key::parse` into one junk Named key, effectively no-ops | fuzz now materializes sequences via `parse_key_sequence`; added mark/last_visual boundary invariants; rounds 200→500 |

### New features

- **visual `r{char}`**: charwise pads by the number of covered characters, linewise by each
  line's character count (`中文ab` → `----`, by characters not display width), blockwise by
  each line's covered segment; the cursor lands at the selection start (probe).
- **`:{range}y[ank] [x] [count]`**: linewise yank into a register (the numeric argument is a
  line count), symmetric with `:d`.
- **Count-repeated insert**: `3ifoo<Esc>` → `foofoofoo`, `3ofoo<Esc>` opens three lines,
  `2a!` doubles. The cursor rests on the last character of the final copy (probe: column
  matches), a single undo group, and `.` replays the count together (the count keys are
  recorded; replay goes through the same EnterInsert path). Conservative semantics: copy
  only when the cursor is still at the typing end and the text is single-line; the count of
  c/s belongs to the motion; Replace/block-select do not participate.

### New known divergences (continuing the previous table's numbering)

16. Count-repeated insert only covers "pure typing until Esc"; for sessions that used arrow
    keys/Enter the replay semantics are not aligned with vim (no duplication).
17. `:s` flags such as `c`/`n`/`e` are silently ignored (`i`/`I`/`g` are honored).
18. Block-select `$` column extension (a ragged block reaching each line's own line end)
    is not implemented.

## 2. Bugs fixed (rounds 1-3)

Every entry has a corresponding regression test; semantic fixes first ran probes on vim 9.1
for oracle evidence, then changed the engine.

| # | Problem | Root cause | Evidence method |
|---|------|------|----------|
| 1 | `dw`/`cw` joined lines when crossing into indentation | The w-operator cross-line rule only covered "cursor on a line-end word"; when the landing point was not in column 1 the column-1 rule did not fire and the span swallowed the newline+indent | 8 vim probes (incl. d2w emptying the current line rather than deleting it, d3w joining, and the two empty-line `dw` branches no-op/delete-line) |
| 2 | `2N`/`3N` moved in the forward direction | The backward count used `start_index + (count-1)`, reversing the direction | vim probe: on the 5th match 1N→4, 2N→3, 3N→2 |
| 3 | `:s` left the cursor on a stale offset | It recorded the pre-replacement match byte offsets, invalidated once earlier line replacements changed line lengths (landing mid-character on multibyte text) | vim probe: cursor = last replaced line + first non-blank column, independent of the match position inside the line |
| 4 | `gv` collapsed a restored forward selection to a single character | `exit_visual` parked the cursor at the selection start and then re-read the selection | engine test (forward `v3l` → `gv` leaves only 1 character) |
| 5 | Lowercase modifiers like `<c-a>` parsed as `Named("c-a")` | Inside angle brackets only the uppercase `C-` modifier prefix was recognized | vim accepts `<c-a>` mappings; both spellings are common in rc files |
| 6 | `*` on a word-less line silently re-jumped with the old pattern | `search_word_under_cursor` failed without signal and `jump_to_match` kept using the previous pattern | engine test (cursor unchanged, highlights unchanged) |
| 7 | Exclusive end `c+1` byte arithmetic of the visual range | Landing inside a multibyte cursor character → `'<`/`'>` and the `:'<,'>` range panicked on strict hosts | random-key fuzz (ZWJ/CJK characters) |
| 8 | Linewise span used `end-1` to take the last line | With no final newline and a multibyte last character (buffer `"中"`), `end-1` landed mid-character | fuzz + harness panic |
| 9 | Equal-length replacement left mark/change-list/jump-list offsets mid-character | `adjust_replace`'s preserve_inner kept relative bytes; the change/jump lists were never adjusted with edits | fuzz; all external offset entry points now uniformly `floor_to_char_boundary` |
| 10 | Giant count panics | `count*10+d` usize overflow; `take_total_count` product overflow; `:5+<huge>` addition overflow | fuzz/boundary tests; saturating arithmetic + 1e9 cap |
| 11 | Block-select `A` did not pad short lines with spaces | Appended directly at the line end | vim probe: block columns 3-5, line `ab` → `ab   X` |
| 12 | `o`/`O` converted tab indentation to spaces | Rebuilt the indent with `" ".repeat(indent)` | engine test; changed to copy the original indentation as a byte slice |
| 13 | `S` panicked on a newline-less multibyte last line | Same as #8 (the Change/Indent/Format branches of `apply`) | fuzz |
| 14 | Legitimate large-count macros were silently dropped | Pipeline guardrail of 500 steps; `10000@a` (five-key macro) = 50k steps | limit raised to 100k; runaway macros still stop at a bounded cut |
| 15 | Key sequences with an unclosed `<` swallowed the tail | Without '>' `parse_key_sequence` merged all remaining characters into one Named key | fell back to per-character literals |

## 3. Behaviors confirmed to need **no change** after vim 9.1 oracle evidence

- `>>`/`<<` do nothing on empty lines (vim also skips empty lines).
- `G`/`gg`/`H`/`M`/`L` land on the first non-blank column (visual mode included) and do
  not preserve the column.
- `<Space>` is exactly `l`: a standalone motion — no move at line end, no line crossing.
  This round registered it into the command table (the old implementation beeped).
- Block-select `I` on short lines: inserts at the line end without padding spaces (consistent
  with vim; the comment was already correct before).
- `cw` on whitespace: an ordinary w span (multiple spaces skipped together), not a single character.

## 4. New features (rounds 1-3)

- **Block-register `p`/`P` in normal mode**: the first line lands at the target column
  (`p` = cursor column + 1, short lines padded with spaces); the remaining lines become new
  lines below padded with spaces to the same column, without shoving lines down and merging
  them; count repeats horizontally. The old implementation flattened block text into a
  character stream. The cursor lands at the first line's insertion point (aligned by vim 9.1
  probe).
- **`r<CR>`**: replaces the current character with a newline (vim's line-split semantics);
  the old implementation beeped.
- **`<Space>`** = `l` (see above).

## 5. Known divergences (deliberate or deferred; integrators should know)

1. **Regex dialect**: search/`:s` use Rust `regex` (RE2 semantics) and replacements use
   `$1` instead of vim's `\1`; `\%V` is unsupported and lookbehind-like assertions follow
   RE2 rules.
2. **`ignorecase` defaults to true** (vim defaults to false), `smartcase` true.
   Better default ergonomics for CJK/code search, but different from vim's factory values.
3. **`textwidth` defaults to 78** (vim defaults to 0 = no auto-wrap); insert does not
   auto-wrap, `gq` reformats by textwidth.
4. **The `scrolloff` option is stored but not read by the engine**: the viewport belongs
   to the host; the engine only reports the target line via `VimHost::scroll_to_line`.
   Hosts may implement scrolloff themselves.
5. **The type of the `"+` clipboard register is guessed**: text containing `\n` is pasted
   as linewise. Host clipboards carry no charwise/linewise metadata, so multi-line charwise
   text gets pasted by line. If the host can provide metadata, extend `get_for_paste`.
6. **`;` and `,` are equivalent inside `:` ranges** (vim's `;` lands the cursor on each
   intermediate address in turn).
7. ~~The named-register argument of `:d` is ignored~~ (**fixed in Round 7**: `:1,2d a`
   stores into `"a`).
8. **Sentence motions recognize only the single characters `.!?`**, no `...` or
   line-follow rules (declared in the `Motion::SentenceNext` comment).
9. ~~tag objects (`it`/`at`) do not select when the cursor sits on the opening tag's `<`~~
   (**fixed in Round 29**: containment changed to `open_start <= offset < close_end`,
   aligned with the 12-position vim 9.1 probe matrix — on the opening tag take this block
   itself, on the closing tag's `<` take the outer one; the inner endpoint was corrected in
   sync to the closing tag's own `<`. `<`/`>` inside attribute values still interfering with
   parsing remains the status quo).
10. ~~`\\"` escaped quotes~~ (**fixed in Round 14**: `quote_positions` decides by backslash
    run length mod 2; the quote after `\\"` now pairs correctly as a literal backslash + a
    real quote).
11. ~~insert-mode `<C-w>`/`<C-u>` at line start did not delete the newline~~ (**fixed in
    Round 9**: at line start `<C-w>`/`<C-u>` join with the previous line; rejected with a
    beep inside block-insert sessions).
12. **`:g`, `:normal`, functions/autocmd are unsupported**, routed to E492 or the rc
    `ignored` collection (the same trade-off as IdeaVim). (`:sort` was implemented in
    Round 23: `ex_sort`, incl. E475/numeric sort/`u` dedup/`!` reversal.) **The whole
    `:copy`/`:move` family (`:t`/`:co`/`:m`) is also unimplemented** — the sibling line
    commands `:d`/`:y`/`:j`/`:pu`/`:s` all exist, so missing these two is an odd gap
    (carried forward since Round 35; the oracle matrix already lives in the red probes
    `copy_command_missing`/`move_command_missing`/
    `move_cursor_lands_on_moved_line_first_non_blank` in `tests/audit6_a.rs` — implementing
    turns them green).
13. **`""yy` lands in `"0`**: `""` is the unnamed register, so a yank must write `"0`,
    matching vim; but `""dd` does not enter the numbered ring (vim neither).
14. ~~after visual `:` runs, `'<`/`'>` were rewritten as "anchor..current cursor"~~
    (**fixed in Round 7**: the selection range at prompt time is committed, restoring `gv`'s
    second-pass semantics).
15. **Block copy pasted via `o`/`A` into insert does not handle multi-line text**: when the
    block-inserted copy contains `\n`, every line gets pasted in (no host-visible breakage,
    but the semantics are not aligned with vim).

## 6. Performance notes

- **`[profile.test] opt-level = 3`** (Round 13): the fuzz suite's self-recursive macro
  (`qa…@a…q` then `@a`) saturates the 100k-key pipeline budget; an opt1 debug build took
  ~78s; opt3 compresses it back to seconds. The engine's own 100k-key budget is untouched
  (that is the formal worst-case promise to hosts).
- fuzz_round13's key table carries a 4KB buffer reset: yy+p loops can double the buffer
  into megabytes, making the per-step whole-file scan quadratic; every bug this fuzz caught
  reproduces on a few-KB buffer.

- **Full hlsearch re-scan per edit** is the default behavior (`hlsearch_live_update`),
  O(whole-file scan) per edit. Large-file hosts should
  `set_hlsearch_live_update(false)` + periodically `refresh_highlights`
  (bench_probe: rescanning 900KB/10k matches is ~0.75ms).
- `all_matches` copies the whole text once via `slice(0..len)` before scanning: measured
  ~20000× faster than walking line-by-line through the trait (the measurement conclusion is
  kept in the code comment), at the cost of one whole-text memcpy per scan — for ropey
  hosts this can be amortized by throttling via `refresh_highlights`.
- `n`/`N` use the cached match table when nothing changed (`matches_generation`), O(1)/key.
- Random-key fuzz (200 rounds × 160 keys) is now a permanent resident test,
  `fuzz_random_key_sequences_hold_invariants`, covering the invariants:
  no panics, cursor always at a character boundary, line count ≥ 1.

## 7. Experience notes

- `*` with no word now beeps and leaves the cursor untouched (the old implementation
  silently re-jumped once with the old pattern — an "intuition-defying UX point" rather
  than a crash).
- Arrow keys after `r`: a non-printable argument is swallowed with a beep, waiting for the
  next key (same as vim).
- `q` beeps before recording starts; `@@` beeps when there is no previous register; both
  have corresponding tests.
- Unknown Ex commands report `E492` text + bell; an `:action` miss routes by strict/lenient
  (the host decides whether to prompt).
- The three read-only `:set` forms (Round 6): `name?` queries one setting, bare `:set`
  lists all, `:reg[isters]`/`:marks` lists — previously `:set` without arguments just
  beeped and register/mark state was entirely invisible to users. Feedback goes uniformly
  through `status_message`, zero host changes.
- C-w/C-u/C-h/Up-Down history on the command-line prompt (Round 13): `:fo<C-w>` deletes a
  word, `:<C-u>` clears the line — muscle memory trained by rc files; before this,
  everything except characters/Enter/Esc/backspace/Up-Down was silently swallowed.
- Ex abbreviations (Round 13): vim spellings like `:de`, `:su/…/` work directly, no more E492.
- Visual click following (Round 13): hosts forwarding clicks in visual mode no longer
  collapse the selection; a click outside a drag selection extends it like vim.
