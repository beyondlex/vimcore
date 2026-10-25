//! vimcore: a host-agnostic Vim engine.
//!
//! The engine knows nothing about gpui, windows, fonts or the system
//! clipboard. It operates on a buffer through the [`buffer::VimBufferMut`]
//! trait and surfaces side effects through the [`host::VimHost`] trait. Feed
//! it keys with [`state::VimState::handle_key`]; draw the cursor, selection
//! and highlights from the state it exposes.
//!
//! The layering (borrowed from IdeaVim's `vim-engine`):
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ your app (gpui / ratatui / crossterm / …)    │
//! ├──────────────────────────────────────────────┤
//! │ integration crate (one per frontend)         │  keystroke conversion,
//! │  · key interception + mode indicator helpers │  text-input bridging
//! ├──────────────────────────────────────────────┤
//! │ vimcore (this crate)                        │  pure engine:
//! │  · mode machine, key pipeline, tries         │  testable without a GUI
//! │  · motions, operators, objects, registers,   │
//! │    search, marks, undo semantics             │
//! ├──────────────────────────────────────────────┤
//! │ host traits: VimBuffer(Mut) + VimHost        │  implemented by your
//! └──────────────────────────────────────────────┘  buffer + widget
//! ```
//!
//! A VIEWING host (diff viewer, log pager) implements only [`buffer::VimBuffer`]
//! and [`host::VimHostRo`] and drives [`state::VimState::handle_key_ro`]: the
//! full motion / text-object / search / mark / scroll / yank surface works,
//! and editing commands are refused with vim's `E21` instead of needing a
//! writable buffer that does not exist. See `PLAN_READONLY.md`.

pub mod buffer;
pub mod cmdline;
pub mod config;
pub mod ex;
pub mod host;
pub mod insert_mode;
pub mod key;
pub mod keymap;
pub mod marks;
pub mod mode;
pub mod motions;
pub mod objects;
pub mod ops;
pub mod options;
pub mod registers;
pub mod search;
pub mod state;
pub mod tables;
pub mod tck;
pub mod word;

pub use buffer::{RoBuffer, VimBuffer, VimBufferMut};
pub use host::{MotionHook, RoHost, VimHost, VimHostRo};
pub use key::{Key, KeyKind, Modifiers};
pub use keymap::ModeClass;
pub use mode::{Mode, VisualKind};
pub use motions::{Motion, MotionKind, MotionResult};
pub use objects::TextObject;
pub use ops::Operator;
pub use registers::RegisterKind;
pub use state::{Ctx, CtxRo, KeyResult, VimState};

/// vim's `E21` — the message [`VimState`] reports when an edit is refused
/// because `'modifiable'` is off ([`VimState::set_modifiable`]).
pub use state::E21_MODIFIABLE_OFF;
