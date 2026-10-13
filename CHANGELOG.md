# CHANGELOG

This file records only **host-facing API changes**. The itemized detail
of each round's semantic fixes lives in `NOTES.md` (organized by round,
each round with its own audit summary), which is authoritative before a
release.

The format follows [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
and the version numbers match `Cargo.toml`.

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
