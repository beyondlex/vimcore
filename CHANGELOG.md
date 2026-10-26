# CHANGELOG

This file records only **host-facing API changes**. The itemized detail
of each round's semantic fixes lives in `NOTES.md` (organized by round,
each round with its own audit summary), which is authoritative before a
release.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the version numbers match `Cargo.toml`.

## [0.5.0] - 2026-10-26

### Added

- **`VimState::pending_timeout(&mut Ctx)`** — vim's `'timeoutlen'` flush for
  held mapping prefixes (`<Leader>` waiting for its next key, insert-mode
  `j` of `jk`). The held keys execute AS TYPED: no further mapping
  resolution (the existing `no_remap_left` budget covers exactly the held
  segment), a digit arms the count, a builtin prefix (`g` of `gg`) stays
  armed, and a declined insert-mode printable comes back `Unknown` with the
  char parked for the host to place. The engine never measures time itself —
  the timer is the host's.
- **`VimState::has_pending_prefix()` / `pending_prefix_keys()`** — the
  timer-arming probe and the held keys for a status-line chord hint. False
  while a prompt is open (a mapping RHS entering the prompt keeps its
  queued continuation, and a settle must not steal prompt text).
- **`VimState::drop_pending_prefix()`** — discard the held keys without
  replaying (vim discarding pending input when the focus leaves a window);
  the host's cancel path for session end / pane switch.
- **`VimHostRo::dispatch_host_action_hinted`** (default: no-op) — the
  `:action` bridge now exists on the read-only half, and `RoHost` forwards
  it. An action-style mapping chord completes the same way on a viewer
  instead of dying there.

### Behavior

- Held keys are recorded exactly once, AT SETTLE TIME: the wait happens
  before the pipeline's record site, so macros and `.` learn the keys the
  user actually typed — no double capture, no missing capture.
- After a timeout the held prefix can no longer complete its mapping (vim:
  the builtin path is restored — `g` stays armed and `s` runs as a builtin).

## [0.4.1] - 2026-10-26

### Added

- **`VimState::exec_ex(&mut Ctx, line)`**: execute one Ex line outside the
  `:` prompt — the host-UI bridge the registry exists for. A host with its
  own command line parses nothing itself; the line takes the same dispatch
  (registered host commands first, then built-ins), with ranges, `!` and
  `|` handled by the engine.

## [0.4.0] - 2026-10-26

### Changed (breaking)

- **`VimHost::end_undo_group(id)` now takes the post-edit cursor:
  `end_undo_group(id, cursor_offset)`** — symmetric with
  `begin_undo_group(id, cursor_offset)`'s pre-edit cursor. A host that
  pushes its undo entry at closure (recommended — see below) can build the
  complete entry from what it already holds: begin gave before-text +
  before-cursor, close gives after-cursor, storage holds after-text.

### Why push at closure

A macro replay runs several commands inside ONE `handle_key` call, each
closing its group. A host that waits for its own flush loses every unit
but the last (the next `begin_undo_group` overwrites the staged
snapshot), and a replayed `u` lands on the wrong entry. Pushing in the
`end_undo_group` hook keeps the replay's history intact.

## [0.3.0] - 2026-10-26

The M1 engine prerequisites: shared session globals for multi-panel hosts,
the host-command Ex registry, and the MACRO_HOST_UNDO capability flag.

### Added

- **`SharedGlobals` + `VimState::share_globals(&SharedGlobals)` /
  `VimState::shared_globals()`**: several engine instances (one panel each
  — a merge view's editable Result plus read-only panes) can share ONE
  register file and macro table, matching vim's split of session-global
  registers and per-buffer undo. Undo, marks, jumps and search state stay
  per instance. `VimState::registers_snapshot()` copies the register file
  for render paths that must not hold the lock.
- **`VimState::register_ex(name, aliases, completion, handler)` +
  `VimState::ex_completions(line)`** (module `vimcore::ex`): host-registered
  Ex commands dispatch inside the engine's `:` pipeline BEFORE the
  built-in table — the engine hands the handler the parsed range, `!`
  flag and argument, and a `false` return fails the command like a
  built-in error. Name matching follows vim's boundary rule (`w` claims
  `:w` but not `:write`; a glued `s/a/b/` reaches a registration named
  `s` with its separator kept). See `src/ex.rs` for the exact rules.
- **`VimHost::capabilities() -> HostCapabilities`** (all flags default
  off) with the first flag **`macro_host_undo`**: `u`/`<C-r>` inside a
  macro or `.` replay now asks the host only when the host declared the
  flag; an undeclared host gets the plain "nothing to undo" bell without
  a host call. Direct (non-replay) undo is ungated.

### Changed (breaking)

- **`VimState::registers` is now `Arc<Mutex<Registers>>`** (was
  `Registers`). External readers lock it — `registers_snapshot()` copies
  for render paths — and internal engine code goes through private
  accessors. Everything else about the register semantics is unchanged.

## [0.2.3] - 2026-10-26

Undo groups now close with a deterministic engine signal instead of every
host inferring closure on its own ("push when not in Insert/Replace after a
key"). That inference let an edit-less `<Esc>` strand the host's snapshot
until the next edit overwrote it — vim's `iZ<esc>u` then reporting
"Already at oldest change".

### Added

- **`VimHost::end_undo_group(id)`** (default no-op): called by the engine
  when a logical undo unit completes (insert session exit, operator done,
  command done), symmetric with `begin_undo_group`. Only groups that
  actually announced (a group whose first real edit happened) are closed —
  an edit-less `i<Esc>` never announced, so no spurious signal. Hosts
  compiled against older 0.2.x keep working: the default is a no-op and
  the old inference stays theirs until they adopt the signal.
- Internally `end_edit` now takes `&mut Ctx` to reach the host
  (`pub(crate)` — not a public signature change).

## [0.2.2] - 2026-10-24

Repository-wide Chinese-to-English migration. **No API change**: every public
type, signature and behavior is identical to 0.2.1. Only comments, docs,
assert messages and git commit messages changed, so hosts upgrade with no code
edit.

### Added

- **`.cjk-allow` + `tools/check_cjk.sh`** (wired into a `cjk` CI workflow):
  fails when CJK text appears outside a per-file whitelist with a mandatory
  reason. Without this the migration would silently regress on the next
  Chinese comment.

### Changed

- All source comments, doc comments, assert messages and panic text are English;
  so are `NOTES.md`, `AGENTS.md`, `README.md`, `PLAN_READONLY.md` and this file.
- Git history rewritten to English messages. SHA values changed; the tree of
  every commit is byte-identical to before, so a tree diff against the
  pre-migration history shows no content change.
- `I18N_PLAN.md`, `.cjk-allow` and `tools/` are excluded from the published
  crate (`Cargo.toml` `exclude`): development artifacts, not API surface.

### Notes

- CJK test fixtures (~290 lines) are deliberately preserved. They measure
  wide-char display width and multibyte char-boundary behavior; substituting
  ASCII keeps the assertions green while destroying the coverage. The rule is
  recorded in `I18N_PLAN.md` section 1.1 and enforced file-by-file by
  `.cjk-allow`.

## [0.2.1] - 2026-10-13

One gap closed in the read-only pipeline: after the host swaps the text
underneath the engine (multi-view layouts switching pane/window), the
`hlsearch` highlights need to be realigned to the new buffer — until now
the only option was feeding a synthetic key.

### Added

- **`VimState::refresh_highlights_ro`**: the same wrapper as
  `handle_key_ro` (`RoBuffer` + `RoHost`, `modifiable` forced off),
  calling `republish_search_inner` internally. A no-op when there is no
  active search pattern.
- Regression test `tests/readonly_pipeline.rs`: what gets republished
  are the **coordinates of the new buffer** (not stale offsets), the
  buffer stays byte-for-byte unmodified, and no pattern is confirmed to
  be a no-op.

### Changed

- `Cargo.toml` version 0.2.0 → 0.2.1.

## [0.2.0] - 2026-10-12

The read-only pipeline and the host motion interception point —
`PLAN_READONLY.md` M1–M4. Motivation: pure viewing hosts such as diff /
logs / previews could not hook in (`Ctx` requires
`&mut dyn VimBufferMut`); previously the only options were faking the
write primitives or abandoning the whole engine.

### Added

- **Read-only pipeline**: `VimHostRo` (the viewing-side subset of
  `VimHost`'s hooks), `CtxRo { buf: &dyn VimBuffer, host: &mut dyn
  VimHostRo }`, `VimState::handle_key_ro`. The main pipeline is shared
  with `handle_key`; state.rs is not duplicated. The buffer enters by
  shared reference, so mutation is impossible by construction.
- **`MotionHook`** (a standalone opt-in trait) + the `VimHost::motion_hook()`
  accessor (default `None`): the host can take over individual motions
  (diff's `]c`/`[c`, fold-aware `j/k`); the insertion point is the
  outermost layer of `Motion::target`; pending operators and visual
  extension come for free.
- **`'modifiable'`**: `VimState::set_modifiable` / `is_modifiable`,
  `:set [no|inv]ma[?|!|&]` available. When `false`, editing commands are
  rejected at the execution point with vim 9.1's E21 semantics (bell +
  `status_message`, the key recorded as `Consumed`).
- **New tck contracts**: `tck::readonly_contract` (step A) and
  `tck::readonly_pipeline_contract` (step B) — the 41 editing keys are
  asserted to yield E21 + a byte-identical buffer + a healthy state
  machine after rejection; the 10 viewing keys must not false-positive.
- `RoBuffer` / `RoHost` adapters (`&dyn VimBuffer` → `VimBufferMut`,
  `&dyn VimHostRo` → `VimHost`) so hosts can integrate without changing
  their own types.

### Changed (**source-breaking**)

- The return type of `VimBufferMut::insert_text` / `delete_range` /
  `replace_range` changed from `()` to **`bool`**, and the **default
  implementation returns `false` (refuse)**. Migration for editing
  hosts is a mechanical substitution: add `-> true` to each method.
  Onboarding cost for read-only hosts becomes
  `impl VimBufferMut for X {}` (zero methods).
- `Cargo.toml` version 0.1.30 → 0.2.0.

### Design trade-offs (deviations from the plan)

- `VimHost` does **not** make `VimHostRo` a supertrait (it would force
  editing hosts to write an extra impl block, violating zero migration;
  and default-value hooks would silently hand the viewing side the
  wrong viewport) — the `RoHost` adapter was used instead.
- The buffer uses the `RoBuffer` adapter rather than the planned
  `BufferRef` enum — the enum approach would require changing 589
  references to `ctx.buf`, while the adapter approach leaves the main
  pipeline untouched.

Rationale and oracle evidence are recorded in the "read-only pipeline
(PLAN_READONLY P1/P2)" section of `NOTES.md`.

## [0.1.30] - 2026-10-11 and earlier

0.1.x (0.1.1 → 0.1.30) was round-by-round semantic alignment with vim
9.1: over thirty review/independent-audit rounds, 300+ cumulative
semantic fixes, all evidenced against `/usr/bin/vim` (9.1 patches
1-1752). Each round's fix list is in the corresponding round of
`NOTES.md`; tests land per round in `tests/audit*`,
`tests/parity_round*`, `tests/probe_round*`, `tests/fuzz_*`.

No host-facing breaking changes within 0.1.x (the host-side API is
stable through 0.1.30).
