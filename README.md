# vimcore

A **host-agnostic Vim engine in pure Rust**: mode machine, motions,
operators, text objects, registers, search, marks, macros, `.` repeat,
Ex commands. Zero GUI, zero terminal dependencies (runtime deps are only
`regex` + `unicode-width`), wired into any Rust UI through two host
traits — gpui, ratatui, or your own ANSI renderer.

```
┌──────────────────────────────────────────────────────────────┐
│ Your app (gpui / ratatui / custom ANSI / ...)                │
├──────────────────────────────────────────────────────────────┤
│ Front-end integration crates (each owns a slice)             │
│  - gpui-vim      keystroke interception + mode UI            │
│  - crossterm-vim key translation + buffer + host side effects│
│  - ratatui-vim   state rendering layer                       │
├──────────────────────────────────────────────────────────────┤
│ vimcore (this crate)  pure engine:                           │
│  - mode machine - key pipeline - Trie command table          │
│  - motion - operator - text object                           │
│  - registers - search - marks - macros - `.` - Ex            │
├──────────────────────────────────────────────────────────────┤
│ Host traits: VimBuffer(Mut) + VimHost (you implement them)   │
└──────────────────────────────────────────────────────────────┘
```

The layered shape is inherited from IdeaVim's `vim-engine`: the engine
owns the cursor and modes; the host only implements buffer read/write
plus viewport/clipboard/undo hooks.

## Usage

```rust
use vimcore::buffer::VimBufferMut;
use vimcore::key::Key;
use vimcore::state::{Ctx, KeyResult, VimState};

let mut vim = VimState::new();
let mut buf = MyBuffer::default();   // implements VimBuffer + VimBufferMut
let mut host = MyHost::default();    // implements VimHost (viewport/clipboard/undo hooks)

let mut ctx = Ctx { buf: &mut buf, host: &mut host };
// one key per call; `Key::parse` handles a single keystroke
// (`vimcore::key::parse_keys("dw")` splits a sequence into keys)
match vim.handle_key(&mut ctx, Key::parse("d")) {
    KeyResult::Consumed => {}  // engine handled it; state and buffer updated
    KeyResult::Unknown => {}   // engine does not want this key; give it back to the host
}
```

Internally the engine uses UTF-8 byte offsets uniformly, and all state
needed for rendering (mode, cursor, visual selection, search highlight)
is readable.

## Read-only hosts (diff view / logs / previews)

Pure viewing hosts cannot provide `VimBufferMut`'s write primitives, yet
what they want is the same engine's motions / text objects / search /
marks / scrolling. The read-only path exists for exactly this:

```rust,ignore
use vimcore::buffer::VimBuffer;
use vimcore::host::VimHostRo;
use vimcore::state::{CtxRo, VimState};

impl VimBuffer for MyView { /* read-only access */ }
impl VimHostRo for MyHost { /* viewport / clipboard write / highlight / status messages */ }

let mut ctx = CtxRo { buf: &my_view, host: &mut my_host };
vim.handle_key_ro(&mut ctx, key);
```

- Editing commands are rejected with vim's **E21** (bell +
  `E21: Cannot make changes, 'modifiable' is off`); no panic and no
  silent no-op;
- `y` (yank), `m{a}`, search, `n`/`N`, scrolling, `:q` all work as usual;
- A host that wants to keep the full `VimHost` can also use the
  compatibility layer: `impl VimBufferMut for X {}` (zero methods, the
  write primitives reject by default) + `vim.set_modifiable(false)`;
- Host-side row models such as folding / synthetic line projection take
  over individual motions via `MotionHook` (`j`/`k` walk visible lines,
  `]c` jumps diff chunks); pending operators and visual extension come
  for free, and the folding concept stays out of the engine.

Details and vim 9.1 evidence: see `PLAN_READONLY.md` and the read-only
pipeline section of `NOTES.md`.

## Integration quality has a tested floor (TCK)

Hosts run the engine's built-in contract tests against their own buffer
/ host implementations, turning "integration quality relies on reading
docs" into "there is an executable acceptance":

```rust,ignore
#[test]
fn my_buffer_meets_engine_contract() {
    let mut buf = MyBuffer::default();
    vimcore::tck::buffer_edit_contract(&mut buf).unwrap();
    let mut host = MyHost::default();
    vimcore::tck::engine_smoke_contract(&mut buf, &mut host).unwrap();
}

// Read-only host: the 50 editing keys must be rejected with E21 and the buffer stay byte-identical
#[test]
fn my_readonly_view_meets_engine_contract() {
    let buf = MyView::new("alpha\nbeta\ngamma\n");
    vimcore::tck::readonly_pipeline_contract(&buf).unwrap();
}
```

## Actual consumers

- [gpui-vim](https://github.com/beyondlex/gpui_vim): GPUI integration (used by [PandaGit](https://github.com/beyondlex/pandagit))
- [crossterm-vim](https://github.com/beyondlex/crossterm_vim): general crossterm integration → ratatui-vim rendering layer (used by [jk](https://github.com/beyondlex/jks))

## Development

```bash
cargo test            # headless tests: full engine + TCK contracts + randomized fuzz
cargo run --release --example bench_probe   # performance probe for host implementations
```

`NOTES.md` records the review conclusions: fixed bugs (evidenced by vim
9.1 probes), known divergences from vim, and performance/UX notes —
worth reading through before integrating.

## Name history

`vim-core` → `gpui-vim-core` (2026-09-20, published to crates.io with
gpui_vim) → `vimcore` (2026-09-28, standalone repository). The engine
never belonged to any single front end, so the name no longer anchors to
gpui.

## License

MIT OR Apache-2.0
