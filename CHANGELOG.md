# CHANGELOG

This file records only **host-facing API changes**. The itemized detail
of each round's semantic fixes lives in `NOTES.md` (organized by round,
each round with its own audit summary), which is authoritative before a
release.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the version numbers match `Cargo.toml`.

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
