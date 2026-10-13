# PLAN: read-only pipeline and host motion interception point

> Drafted: 2026-10-12. Motivated by the first real consumer — alacer's diff
> view (read-only diff browser: two panes + unified synthesized-line
> projection + folding).
> Every "current state" excerpt in this document was verified against the
> source on main at the time of writing.

## 1. Motivation

vimcore's editing capabilities (insert session, undo groups, ops) are consumed
together with its mode machine / motions / text objects / search / marks /
scrolling: the single entry point of the key pipeline requires a **mutable**
buffer, which locks out purely read-only hosts (diff/log/preview-style
viewers) — they would have to either fake the write primitives of
`VimBufferMut` or abandon the whole engine.

alacer currently uses a hand-written read-only vim of several hundred lines in
its diff view (`diff_view/vim_motion.rs`), covering only a small corner of
motions (h/l/w/e, f/F/t/T, quote/bracket text objects, scrolling, counts),
with semantic parity against vimcore maintained by manual alignment. Once the
read-only pipeline lands, it should migrate entirely to vimcore.

## 2. Current state and hard constraints

```rust
// state.rs
pub struct Ctx<'a> {
    pub buf: &'a mut dyn VimBufferMut,   // ← hard constraint: a read-only host cannot produce this
    pub host: &'a mut dyn VimHost,
}
pub fn handle_key(&mut self, ctx: &mut Ctx, key: Key) -> KeyResult; // Consumed | Unknown

// buffer.rs
pub trait VimBuffer { len / line_count / char_at / prev_char_offset
                      / line_range / offset_to_line / slice … }      // read-only, sufficient
pub trait VimBufferMut: VimBuffer {
    fn insert_text(&mut self, offset: usize, text: &str);           // must implement
    fn delete_range(&mut self, range: Range<usize>);                // must implement
}

// host.rs —— the existing VimHost hooks split into two groups:
//   "editing session" vs "pure viewing":
//   viewing side: viewport / scroll_to_line(_anchored) / scroll_lines /
//           clipboard_write / set_search_highlights / changed / bell /
//           status_message / request_close(_forced) / buffer_name
//   editing side: begin_undo_group / undo / redo / save /
//           cycle_buffer / first_buffer / last_buffer / clipboard_read
//           (paste is editing; :w and buffer cycling are meaningless for a read-only host)
```

The call sites of the write primitives are concentrated in ops.rs (operator
execution) and insert_mode.rs; the mode machine / motions / objects / search /
marks themselves do not write to the buffer — this is the structural basis
that makes the "read-only pipeline" viable.

## 3. Goals / non-goals

**Goals**
1. A read-only host only needs `impl VimBuffer` + a slimmed-down host trait to
   get full normal/visual motion, text object, search, mark, scrolling, and
   register-yank capability;
2. Editing keys are rejected in read-only mode with vim's E21 semantics (bell +
   status_message, no panic, no silent no-op);
3. The host can take over individual motions (diff's `]c`/`[c` chunk jumps,
   fold-aware line movement);
4. Zero breakage of existing APIs — everything is incremental.

**Non-goals**
- No "line-index model" refactor within this plan (the engine keeps the byte-offset
  cursor model; folding is handled by the host through the interception point);
- No changing frontend assumptions outside the gpui integration layer
  (ratatui/crossterm hosts benefit equally).

## 4. Design

### P1 — read-only pipeline

**Step A (compatibility layer, first, no breaking change)**

- Make the two write primitives of `VimBufferMut` **optional**:
  ```rust
  fn insert_text(&mut self, offset: usize, text: &str) -> bool { false }
  fn delete_range(&mut self, range: Range<usize>) -> bool { false }
  ```
  Returning `false` = rejection (the default implementation rejects). Editing
  hosts implement as before and return `true`; when a primitive returns
  `false`, the apply path in ops.rs takes the "unmodified" branch (keeping
  cursor/register state consistent — equivalent to vim's E21: the change did
  not happen).
- Add `modifiable: bool` to `VimState` (default `true`; mirroring vim's
  `'modifiable'`). When `false`, editing commands (operator execution,
  i/a/I/o/O entering insert, direct edits like `~`/`x`, `:s`, `:!` filter,
  `.` repeat-edit command, paste) are rejected early at the **command
  execution point**: `host.bell()` + `host.status_message("E21: Cannot make
  changes, 'modifiable' is off")`, and the key is recorded as `Consumed`
  (consistent with vim — a rejection is still "handled by the engine" and must
  not leak to the host for secondary processing).
- The adoption cost for a read-only host becomes: `impl VimBuffer` +
  `impl VimBufferMut {}` (zero methods, all default-reject) +
  `vim.set_modifiable(false)`.

**Step B (type-layer split, the 0.2 API evolution)**

- Add `VimHostRo`: the existing `VimHost` minus the editing-side hooks (§2
  grouping), with the current `VimHost: VimHostRo` (supertrait) so editing
  hosts migrate with zero changes.
- Add `CtxRo<'a> { buf: &'a dyn VimBuffer, host: &'a mut dyn VimHostRo }` and
  `VimState::handle_key_ro(&mut self, ctx: &mut CtxRo, key: Key) -> KeyResult`.
- Implementation path (without duplicating the 6.4k-line state.rs): internally
  replace `Ctx`'s `buf` with `BufferRef<'a>`
  (`enum { Ro(&dyn VimBuffer), Rw(&mut dyn VimBufferMut) }`, text-reading
  methods forward to both, write primitives implemented only in the Rw
  branch); `handle_key` constructs the Rw view, `handle_key_ro` constructs the
  Ro view, and the main pipeline is shared. The `modifiable=false` rejection
  check and the unreachability of write primitives under the Ro view form a
  double safeguard.
- marks/jumplist are naturally safe in read-only mode: the buffer never
  changes, so offsets never become stale.

**Semantic alignment checklist (part of P1 acceptance)**

| Category | Behavior in read-only mode |
|---|---|
| motions (h/l/w/e/f/F/t/T/%/G/gg/{count}) | normal |
| text objects (iw/a"/i( etc. within visual) | normal |
| operator `y` (yank) | normal, writes register + `clipboard_write` |
| operators `d/c/x/s/p/…`, direct edits like `~` | rejected with E21, no state drift |
| `v`/`V`/`gv`, `/`?n/N`, search-highlight publishing | normal |
| `m{a}` marks, `` `{a} `` jumps | normal |
| scrolling (zz/zt/zb/C-d/C-u/C-e/C-y) | normal |
| insert family (i/a/I/o/O, R) | rejected with E21 (does not enter insert) |
| `:s`, `:%!`, `.` (edit repeat), undo/redo | E21 / silent rejection (same as vim: undo with no history is a bell) |
| `:q`/`:q!`/`ZZ` | goes through `request_close(_forced)` (equally legitimate for a viewer) |

### P2 — host motion interception point

Addresses the needs of "fold / synthesized-line projection" hosts (alacer diff
view: `j/k` must walk **visible lines**, `]c`/`[c` jump by chunk; `z a`
fold-toggle is host-side anyway):

```rust
// VimHostRo (or a separate trait — see "decision points")
/// Host answers first: Some = this motion was handled (the engine only
/// consumes count and mode-state side effects); None = hand back to the
/// engine's built-in implementation.
fn intercept_motion(
    &mut self, vim: &VimState, motion: Motion, count: usize,
) -> Option<MotionResult>;
```

- Insertion point: the outermost layer of `Motion::target()`. Inside the host
  interceptor, "skip hidden lines" or "jump to the next diff chunk" can be
  freely implemented, returning a `MotionResult` of the same type as the
  engine's (including Exclusive/Inclusive/Linewise and stuck semantics);
  operator-pending / visual extension come for free.
- This is far less invasive than an "engine-built-in visible-line model"
  (adding `is_line_visible()` to `VimBuffer` and changing Up/Down semantics),
  and it does not leak the folding concept into the engine. The latter is
  listed as a long-term option, not part of this plan.

**Decision points (settle before implementation)**
1. Should `intercept_motion` live on `VimHostRo` or on a separate opt-in trait
   `MotionHook`? Leaning toward a separate trait: doesn't pollute ordinary
   hosts, and unimplemented means zero overhead.
2. The `Motion` received at the interception point is the raw enum value; how
   are host-invented motions (`]c`) expressed? Leaning toward: the host
   directly returns a `MotionResult` in terms of "previous/next target line"
   inside the interceptor — no enum extension needed; the enum stays the
   built-in motion set.

### P3 — gpui-vim integration layer + alacer migration verification

- Add `VimEditorRo` to gpui-vim (read-only variant: key interception + mode
  badge, no text-input bridge, no undo reconciliation).
- alacer diff view migration (the first real consumer, serving as acceptance):
  - Delete the motions/find/text-object/scrolling/count implementation in
    `vim_motion.rs`, replacing it with vimcore RO;
  - Keep host-side: dual-pane focus and synchronized scrolling, folding,
    unified line projection, `z a`, `]c` (via `intercept_motion`);
  - Re-audit the differences between the `[diff]` keymap table and merge's
    (which keys can be merged).

## 5. Test plan

- **New tck contract probe**: `readonly_contract(buf, vim)` — feed the full
  editing keymap table under `modifiable=false` and assert: buffer unchanged
  byte-for-byte, E21 status message returned, subsequent motions/searches
  still work normally (rejection does not pollute the state machine).
- **Motions equivalence**: replay the same batches of tck key sequences
  through the RW and RO pipelines respectively, with cursor/highlights/
  registers equal one by one.
- **Fuzz**: add an RO configuration to the existing round suites; the host is
  a probe implementation that panics on any write primitive — a single
  reaching of a write primitive turns red. The fuzz `!` safety valve
  (round20) becomes a naturally stronger assertion in RO mode.
- **Interception point**: unit tests for the host interceptor returning stuck
  / jump; selection-extension tests for intercepted motions in visual mode.

## 6. Compatibility

- Step A: the `VimBufferMut` write-primitive signature changes from `()` to
  `bool` — source-level breaking for existing editing hosts, but a mechanical
  substitution (add `-> true`). Do it within 0.1.x with a changelog note; or
  first add `insert_text_checked`/`delete_range_checked` as a transition and
  mark the old methods `#[deprecated]`. Pick one at implementation time.
- Step B: purely incremental; `VimHost: VimHostRo` guarantees zero migration
  for editing hosts.
- semver: Step A handled within 0.1.x's breaking-change budget; Step B ships
  with 0.2.

## 7. Milestones

| Milestone | Content | Size |
|---|---|---|
| M1 | Step A: write primitives default-reject + `modifiable` + E21 + semantic alignment checklist all green | 1 PR, medium |
| M2 | tck/fuzz RO contract + RW/RO equivalence replay | 1 PR, medium |
| M3 | `intercept_motion` (separate trait) + tests | 1 PR, small |
| M4 | Step B: `VimHostRo`/`CtxRo`/`handle_key_ro` (0.2) | 1–2 PRs, large (touches Ctx construction) |
| M5 | gpui-vim `VimEditorRo` + alacer diff view migration acceptance | cross-repo |

After M1–M3, alacer can migrate immediately (the Step A compatibility layer +
interception point already satisfy all diff view requirements); M4 is an API
tidiness step and does not block consumers.

## 8. Implementation status (2026-10-12)

| Milestone | Status | Delivered |
|---|---|---|
| M1 | ✅ | `VimBufferMut` write primitives return `bool` and default-reject; `VimState::modifiable`; five E21 gates; `:set [no]ma` |
| M2 | ✅ | `tck::readonly_contract` / `tck::readonly_pipeline_contract`; `tests/readonly_pipeline.rs` (RW/RO equivalence replay + panic-on-write fuzz) |
| M3 | ✅ | `MotionHook` + `VimHost(Ro)::motion_hook()`, insertion point `Motion::target` |
| M4 | ✅ | `VimHostRo` / `CtxRo` / `handle_key_ro` (`RoHost` + `RoBuffer` adapters, main pipeline untouched) |
| M5 | ✅ cross-repo | gpui-vim-x 0.2.0 released (pager migrated to the RO pipeline, 40/40 green); alacer converged on registry 0.2.0 and completed the diff view migration (§9, all three phases landed, 773/773 green) |
| M6 | ✅ cross-repo | alacer merge view read-only panes (Local/Remote) migrated to the RO pipeline, Result stays RW; `is_write_key` interception table + write-then-rollback mechanism deleted (§10, 778/778 green) |

The version bumps to **0.2.0**, driven by M1's source-level breakage (write
primitive signature) and M4's 0.2 API evolution.

The two intentional deviations of the implementation from this document
(`VimHost` does not become a supertrait of `VimHostRo`; the buffer uses
adapters instead of a `BufferRef` enum) are recorded in `NOTES.md` together
with their rationale.

## 9. M5 implementation design: alacer diff view migration (finalized 2026-10-12)

### 9.1 Baseline assessment (before migration)

- `crates/alacer-ui/src/diff_view/vim_motion.rs`: **1226 lines** of
  hand-written read-only vim: motions (w/b/e, f/F/t/T, quoted/bracket text
  object) + `vim_key` dispatch (h l ^ _ $ w b e f F t T i a n N / v V y Y
  g G z{zt,zb,a} 0-9 counts).
- Host state (`diff_view/mod.rs`): `vim_col`/`vim_mode`/`vim_anchor`/
  `vim_count`/`pending_yank`/`pending_g`/`pending_z`/`pending_find`/
  `pending_text_obj`; coordinates are (row, grapheme-col).
- Host-side privileges: `/` search (IME + incremental highlighting, the
  `find_input` pipeline), `n`/`N` (find_goto), `z a` (toggle_fold_all),
  `]c`/`[c` (keymap actions NextChange/PrevChange), j/k (keymap actions),
  C-o/C-i (host jumplist recording (pane, row, col) — the pane-side
  information vimcore's offset-only jumplist lacks), mouse drag selection
  (pane_mouse).
- **The reference implementation lives in this repo**: `merge_view/vim.rs` is
  already a resident RW vimcore integration (`SharedLines` + prefix-sum
  cache, `MergeVimHost: VimHost`, `offset_rc`/`rc_offset` conversion,
  `vim_frame_sync`/`vim_sync_pane`/`vim_selection`). The diff view migration
  follows this blueprint, in RO form.
- Pane `pane.lines: Arc<Vec<String>>` (code_pane.rs:573) — the buffer adapter
  gets a zero-copy snapshot just by cloning the Arc.

### 9.2 Three adapters (new file `diff_view/vim_ro.rs`)

1. **`DiffVimRoBuf(Arc<Vec<String>>)`**: implements only `VimBuffer` (line
   access + `len`) and **does not implement `VimBufferMut`** — the write path
   is unreachable at the type level; `VimState` sets
   `set_modifiable(false)`, editing keys go through E21.
2. **`DiffVimRoHost`**: implements `VimHostRo` — `viewport` (reads the pane's
   visible line range), `scroll_to_line(_anchored)`, `clipboard_write`,
   `status_message` (→ `flash_status`; E21 passes through here),
   `set_search_highlights`, `columns`. **Deferred-apply mode**: while the
   engine is calling, these effects are collected, then applied to DiffView
   after `handle_key_ro` returns (gpui's `cx.write_to_clipboard`/`cx.notify`
   both happen after the call), avoiding the borrow conflict between
   `&mut DiffView` and Ctx.
3. **Cursor authority belongs to the engine**: all host-side cursor changes
   (j/k, mouse, ]c/[c, n/N, gg/G, minimap click) write through
   `vim_set_cursor_rc(row, col)`; rendering reads only the engine
   (`vim.cursor.offset` / `vim.mode` / `vim.visual_anchor`). Same model as
   merge view.

### 9.3 Key partition: what the engine takes, what the host keeps

| Key | Owner | Rationale |
|---|---|---|
| h l ^ _ $ w b e f F t T v V y Y counts gg/G/{N}G | **engine** | vimcore implements all of it, oracle-aligned |
| d/c/s/S/C/o/O/i/a/I/A/R/~ x X p P J u C-r . etc. | **engine** (→ E21) | rejected with `modifiable` off + `status_message` passthrough |
| zz/zt/zb | host (as-is) | same Chord as `z a`; the engine's z prefix would swallow `z a` |
| `z a` | host | fold toggle — a concept the engine doesn't have |
| `/` `n` `N` `?` | host | the existing find pipeline (IME/incremental highlighting) is far richer than the engine's search |
| `i` `a` | **host intercepts, then delegates to engine text-object** | the engine's i/a are insert/append (E21 under RO), which would kill vi"/va" |
| j k ]c [c C-o C-i mouse | host (keymap/existing channels) | writes through the engine cursor; C-o/C-i use the host jumplist (incl. the pane side) |

**Two deviations from the plan (with rationale)**:
- **`]c`/`[c` do not go through `MotionHook`**: they are keymap actions, not
  engine motions; making the engine handle them would require adding an
  engine-side motion variant for "jump to next change" — `d]c` is meaningless
  in the read-only scenario, not worth it. MotionHook stays reserved for
  future RW consumers.
- **The z chord as a whole stays host-side**: see the table above; the
  engine's z prefix conflicts with `z a`.

### 9.4 Deletions and test migration

- **Delete**: `vim_word_forward/back/end`, `vim_find`, `quoted_span`,
  `block_span`, the motion dispatch of `vim_key`, the five pending fields
  (yank/g/z/find/text_obj — keep find/text_obj only if the host retains
  text-objects; see the i/a row of 9.3).
- **Keep**: `vim_set_caret*`/`vim_move` (for write-through), `yank_*` (keep
  if the host yank channel beyond n/N is retained, otherwise delete),
  `vim_scroll_cursor_anchored`, the find pipeline, `toggle_fold_all`,
  `vim_goto_line` (for ]c/[c landing).
- **Tests**: change the `vim_key` calls in `tests/z_anchor.rs` and
  `tests/model.rs` to the new entry point; motion-level semantics belong to
  vimcore (oracle-aligned + tck); the alacer side tests only the integration
  surface: cursor write-through, selection rendering, yank to clipboard, E21
  flash, host-retained keys.

### 9.5 Phasing

1. **Parallel integration**: write `vim_ro.rs` fresh; switch the motion branch
   of `vim_key` to call the engine; intercept host-privileged keys in place —
   behavior unchanged, side-by-side comparison possible.
2. **Deletion**: hand-written motion code + mod.rs fields; render/mouse
   switched to read the engine; tests rewritten.
3. **Wrap-up**: the E21 wording of `flash_status`, cross-check the vim_keys
   list in the help panel, commit separately once `cargo test -p alacer-ui`
   is fully green.

## 10. M6 implementation design: alacer merge view read-only panes migrated to the RO pipeline (finalized and completed 2026-10-13)

### 10.1 Key difference from diff view (§9): hybrid, not wholesale

merge view is a three-pane RW resident integration (Local / Result / Remote),
and **Result is the manual-edit escape hatch for conflict resolution — it must
stay RW**. So M6's scope is:

| Pane | Path | Writes |
|---|---|---|
| Result | `Ctx` + `MergeVimBuffer` (RW, unchanged) | real editing |
| Local / Remote | `CtxRo` + `LineBufView` → `handle_key_ro` | E21 inside the engine |

### 10.2 The old mechanism being replaced (and its hole)

The old design for read-only panes was an **interception table +
write-then-rollback**: the `is_write_key` table blocked single-key write
operations; whatever slipped through (chords like `gI`) really wrote into
SharedLines via the RW path, and `vim_flush` detected the buffer Arc had
changed and rolled back + flashed READONLY_MSG. Under the new design writes
are **impossible by construction** (`RoBuffer` rejects writes, `RoHost`
neuters the editing-side hooks); the interception table and the rollback
branch go away together (the safety net on the chord path is kept).

### 10.3 Integration shape

- `MergeVimHost` **directly** implements `VimHostRo` (the collector fields are
  shared with the `VimHost` impl; `vim_flush` lands the same set of effects
  for both paths) — no second adapter struct needed; both `VimHostRo` and
  `VimHost` calls go through trait objects, so there is no method-name
  ambiguity.
- `vim_feed_engine_ro`: still `vim_buffer.0.set(before)` before feeding
  (keeping the Arc fast path for cursor conversion consistent with the clamp
  path), but the buffer is presented to `CtxRo` directly as `&dyn VimBuffer`;
  the RW storage is not touched.
- The routing point sits where the old interception table was in
  `vim_process_key`; **mapped chord keys keep the RW path** (deliberately:
  action-type mappings must be completable in read-only panes, while `RoHost`
  neuters the host-action bridge); leader-abort replay is also routed per
  pane.

### 10.4 Engine behavior verified along the way (vim 9.1 oracle + engine probes)

| Key family | vim 9.1 (`:set noma`) | Engine | Disposition |
|---|---|---|---|
| `i`/`I`/`a`/`A`/`o`/`O`/`gi`/`R` | E21 at entry | rejected at entry (locked since M1) | consistent |
| `gI`, visual/block `I`/`A` | **silently enters insert**, E21 on the first write | rejected at entry | engine is stricter (in the safe direction); tck `EDIT_KEYS` extended with a lock + a comment records the divergence |

vim's behavior for `gI` (silent entry) is a vim quirk; the engine rejects
earlier, and read-only panes hold the visual selection, giving a better
experience. Under `CtxRo` the write primitives are unreachable at the type
level, so there is no path of "let it into insert and then corrupt the
buffer".

### 10.5 Results

- alacer: 5 integration tests in `vim_ro_panes.rs` (E21 flash + buffer
  unchanged + cursor no drift / visual edits rejected with selection retained
  / navigation + search + yank + `"+y` clipboard / Result's writable path
  unaffected / leader chords still complete in read-only panes); full
  `cargo test --workspace` green, clippy clean.
- vimcore: `tck.rs` EDIT_KEYS extended with 8 insert-family corner cases +
  `readonly_pipeline.rs` locks the post-deviation mode semantics; 1161/1161
  green.

## 11. Engine signal loop-closure: search-highlight rendering + visual bell (completed 2026-10-13)

The two known gaps remaining after closing out M1–M6, closed this round:

### 11.1 Engine search highlights into rendering (diff view)

`*`/`#` movement always took effect immediately, but the
`set_search_highlights` publication in diff view was only collected and
cleared after every key feed (old lines 312–315 of `vim_ro.rs`), never
rendered. Now:

- `vim_apply_engine_search_hits` (diff view) converts engine byte ranges into
  the focused pane's `search_hits` — isomorphic with merge view's
  `vim_publish_highlights` (cross-line hits clamped to the start line),
  hooking into CodePane's existing golden-highlight canvas + line-highlight
  cache, with zero new rendering machinery;
- engine search **takes over host find state** (find_matches/find_current
  rebuilt, find_query yields): in vim semantics `*` supersedes the previous
  search, so `n`/`N` navigate the new hits rather than stale host `/`
  results;
- only the focused pane is drawn (vim's `*` searches only the current
  buffer); when the host `/` opens a search, `clear_search_hits` resets
  `vim_hits_pane` — the two search systems never stack;
- **re-scan on pane switch**: the engine buffer changes text with focus, so an
  old publication carries old coordinates. vimcore 0.2.1 adds
  `refresh_highlights_ro` (the same layer of RoBuffer/RoHost wrapping as
  `handle_key_ro`); the host re-scans and re-publishes against the new buffer
  on the first key feed after a switch — incidentally eliminating the "`n`
  yanks the cursor back to the old pane" bug. Remaining small gap: if the
  previous publication was empty (pattern had no hits), switching does not
  re-scan until the next search action (`vim_hits_pane` being None means no
  record).

### 11.2 Bell actually forwarded + visual bell

The engine's `bell()` has an empty default impl on `VimHostRo`, and both hosts
fell into "discarded":

- diff view used to **synthesize** two hand-written-era messages in `vim_key`
  ("No matching bracket found" / "No quoted string on this line") — the real
  vim 9.1 behavior is that a failed action only rings the bell with **no
  message text** (evidence in the §9 side-by-side comparison). The synthesis
  logic was deleted along with `pending_target`/`block_pair`;
- both hosts implement `bell()` → collect → flush triggers a **visual bell**:
  a ~150ms whole-screen flash (a 12%-opacity foreground-color overlay,
  isomorphic to the terminal visualbell), with a generation mechanism so old
  timers cannot clear a newer bell;
- E21 goes through `status_message` (toast text) **plus** bell (flash) —
  vim's errors are message + beep; the two coexist, not mutually exclusive.

### 11.3 Release and verification

- vimcore **0.2.1** published to crates.io (`refresh_highlights_ro`, a new
  API, backward compatible); alacer's lock follows.
- alacer: 782/782 green (5 new tests: `*` highlighting + n/N takeover,
  re-scan on pane switch, esc clears highlights, bell-without-message in
  merge RO panes, E21 message + bell); clippy clean.
