//! The host-facing side-effect surface.
//!
//! The engine computes *what* to do; the host decides *how* it shows up:
//! scrolling, the system clipboard, search highlighting and the undo stack.
//!
//! Two host traits live here:
//!
//! - [`VimHost`] — the full surface, for an EDITING host.
//! - [`VimHostRo`] — the VIEWING half only, for a read-only host (diff
//!   viewer, log pager): no undo stack, no save, no buffer cycling, no
//!   clipboard reads. [`VimState::handle_key_ro`](crate::state::VimState::handle_key_ro)
//!   drives it through [`RoHost`], which adapts it back onto [`VimHost`] so
//!   the engine keeps ONE key pipeline (PLAN_READONLY P1-B).
//!
//! Editing hosts are unaffected: [`VimHost`] keeps every method it has
//! always had, and the read-only path is opt-in per call.

use crate::buffer::VimBuffer;
use crate::motions::{Motion, MotionResult};
use crate::state::VimState;
use std::ops::Range;

/// Where the requested line should land in the viewport (`zz`/`zt`/`zb`).
/// Plain cursor scrolls use [`ScrollAnchor::Cursor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAnchor {
    /// Just make the line visible (minimal scroll).
    Cursor,
    /// `zz` — centered.
    Center,
    /// `zt` — top of the viewport.
    Top,
    /// `zb` — bottom of the viewport.
    Bottom,
}

/// Opt-in hook that lets a host OWN individual motions.
///
/// A host with a projected view of the buffer — folded regions, a diff
/// viewer's unified synthetic rows — needs `j`/`k` to walk *visible* rows
/// and `]c`/`[c` to land on chunk boundaries. Rather than leaking a fold
/// model into the engine, the host answers "I know where this motion goes"
/// and returns the same [`MotionResult`] the engine would have built:
///
/// - `Some(result)` — the host handled it. The engine applies only its
///   generic bookkeeping (operator span, visual extension, jumplist).
/// - `None` — fall through to the built-in implementation.
///
/// The hook answers for every phase that resolves a motion: plain cursor
/// moves, operator-pending (`d{motion}`) and visual extension all route
/// through `Motion::target`, so one hook covers all three.
///
/// A host that does not implement it pays nothing: [`VimHost::motion_hook`]
/// (and [`VimHostRo::motion_hook`]) default to `None`.
///
/// Self-authored motions (`]c`) need no `Motion` variant of their own — the
/// host computes the target line from its own model and returns the result
/// directly.
pub trait MotionHook {
    /// `count` is the resolved count (an absent count arrives as 1).
    fn intercept_motion(
        &mut self,
        vim: &VimState,
        buf: &dyn VimBuffer,
        motion: Motion,
        count: usize,
    ) -> Option<MotionResult>;
}

/// Replay-safety declarations a host makes about its own side-effect
/// surface. Every flag defaults to `false`; a host opts in by overriding
/// [`VimHost::capabilities`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostCapabilities {
    /// `u` / `<C-r>` inside a macro (`@`) or `.` replay may call
    /// [`VimHost::undo`] / [`VimHost::redo`]. Without the flag the engine
    /// answers the replayed key itself with the "nothing to undo" bell and
    /// never calls the host: a host's undo usually answers from view state
    /// a replayed key never reaches (a view-local undo stack), so calling
    /// it anyway would make the same macro behave differently per host.
    pub macro_host_undo: bool,
}

/// Side-effect hooks the host must (or may) implement.
pub trait VimHost {
    /// Inclusive `(first_line, last_line)` of what is currently visible.
    fn viewport(&self) -> (usize, usize);

    /// Make sure `line` is visible.
    ///
    /// Must be a MINIMAL scroll: if the line is already on screen the
    /// implementation is a no-op, never a re-centering. `C-e`/`C-y` free
    /// scrolling depends on this — the engine scrolls the view first and
    /// then calls this only to keep the cursor in view; a host that jumps
    /// the line to the viewport edge would drag the cursor back on every
    /// keypress and silently disable free scrolling.
    fn scroll_to_line(&mut self, line: usize);

    /// Make `line` visible with a specific anchor (`zz`/`zt`/`zb`). The
    /// default forwards to [`VimHost::scroll_to_line`] for hosts that don't
    /// distinguish anchors.
    fn scroll_to_line_anchored(&mut self, line: usize, anchor: ScrollAnchor) {
        let _ = anchor;
        self.scroll_to_line(line);
    }

    /// Scroll the viewport by `lines` (positive = down) without moving the
    /// cursor (`C-e`/`C-y`). Default: no-op for hosts without free scrolling.
    fn scroll_lines(&mut self, lines: i32) {
        let _ = lines;
    }

    fn clipboard_write(&mut self, text: &str);
    fn clipboard_read(&self) -> Option<String>;

    /// Publish search matches for highlighting. `current` is the match the
    /// cursor sits on (rendered differently). An empty slice clears.
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>);

    /// Begin an undo group. The engine assigns a fresh monotonically
    /// increasing id per logical undo unit; edits sharing an id (an insert
    /// session) belong to the same group and the host must merge them.
    /// `cursor_offset` is the engine cursor *before* the group's first edit —
    /// the natural place to restore the cursor to on undo.
    ///
    /// CONTRACT (audit5 C-15): a group whose id differs from the currently
    /// open one is a FRESH change — implementations must clear their redo
    /// stack at this point (vim's undo is a tree: a new change after an
    /// undo invalidates `<C-r>`). The engine announces a group only through
    /// the edit funnels, never around its own undo/redo swaps.
    fn begin_undo_group(&mut self, id: u64, cursor_offset: usize);

    /// A logical undo unit completed (insert session exit, operator done,
    /// command done). Called by the engine right after `begin_undo_group`
    /// was announced and the unit's last edit landed — symmetric with
    /// [`VimHost::begin_undo_group`]. The host captures AFTER-text +
    /// AFTER-cursor from its own storage (it already saved BEFORE-text +
    /// BEFORE-cursor at `begin_undo_group`, keyed by `id`) and pushes ONE
    /// undo entry.
    ///
    /// Removing this signal forced every host to infer closure (typically
    /// "push when not in Insert/Replace after a key"), which let an
    /// edit-less `<Esc>` strand the snapshot until the next edit overwrote
    /// it (vim's `iZ<esc>u` reporting "Already at oldest change").
    ///
    /// `cursor_offset` is the engine cursor AFTER the unit's last edit —
    /// symmetric with [`VimHost::begin_undo_group`]'s pre-edit cursor. A
    /// host that pushes its undo entry HERE (instead of at its own flush)
    /// can build the complete entry: begin gave the before-text and the
    /// before-cursor, close gives the after-cursor, and its storage holds
    /// the after-text. Pushing at closure is what keeps a macro replay's
    /// history intact: a replayed key stream runs several commands inside
    /// ONE `handle_key` call, each closing its group — a host that waits
    /// for its flush loses every unit but the last (the staged slot is
    /// overwritten by the next group), and a replayed `u` lands on the
    /// wrong entry.
    fn end_undo_group(&mut self, _id: u64, _cursor_offset: usize) {}

    /// Undo the most recent group. Returns the cursor offset to restore,
    /// or `None` if there is nothing to undo.
    fn undo(&mut self) -> Option<usize>;

    /// The host's window width in display columns (`'columns'`). Only the
    /// screen-row motion family (`gm`/`gj`/`gk`) consults it; the default
    /// is vim's own 80-column fallback for heads without a real terminal.
    fn columns(&self) -> usize {
        80
    }

    /// Redo. Same contract as [`VimHost::undo`].
    fn redo(&mut self) -> Option<usize>;

    /// The buffer changed: repaint.
    fn changed(&mut self) {}

    /// Feedback for ignored keys (visual bell / flash). Optional.
    fn bell(&mut self) {}

    /// `:w` — persist the buffer. Hosts that don't persist may report the
    /// no-op through their own status channel.
    fn save(&mut self) {}

    /// `:q` — close the editor. The host decides whether that is allowed
    /// (e.g. prompting about unsaved changes is the host's call).
    fn request_close(&mut self) {}

    /// `:q`/`:q!`/`:wq`/`ZZ`/`ZQ` with the force flag: `forced == false`
    /// means a plain `:q`, which the host may REFUSE (vim's E37: No write
    /// since last change). The default keeps the old unconditional close.
    fn request_close_forced(&mut self, forced: bool) {
        let _ = forced;
        self.request_close();
    }

    /// Non-fatal feedback text for the status UI: `E486: Pattern not found`,
    /// substitution counts, unknown Ex commands. Optional (default no-op);
    /// errors that need a decision still go through `bell`.
    fn status_message(&mut self, message: &str) {
        let _ = message;
    }

    /// Declare replay-safety capabilities (all off by default). See
    /// [`HostCapabilities`].
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities::default()
    }

    /// The buffer's name for status UI (v1; `%` register support comes with
    /// expression registers). Optional.
    fn buffer_name(&self) -> &str {
        ""
    }

    /// `:bnext`/`:bprev`/`:bfirst`/`:blast` — the host owns the buffer list;
    /// it switches and returns false when there is nothing to switch to.
    fn cycle_buffer(&mut self, _forward: bool) -> bool {
        true
    }

    /// `:bfirst`/`:brewind` — jump to the FIRST buffer in the host's list.
    /// Returns false when there is nothing to switch to.
    fn first_buffer(&mut self) -> bool {
        true
    }

    /// `:blast` — jump to the LAST buffer in the host's list.
    fn last_buffer(&mut self) -> bool {
        true
    }

    /// Bridge for `:map <Leader>x :action SomeAction<CR>` — dispatch a
    /// HOST application action by id (IdeaVim's `:action` bridge).
    /// `strict` is true for host-specific config layers (a miss is worth
    /// reporting via `status_message`) and false for shared user layers
    /// (mappings aimed at other apps are expected to miss: ignore quietly).
    fn dispatch_host_action_hinted(&mut self, id: &str, strict: bool) {
        let _ = (id, strict);
    }

    /// Opt-in [`MotionHook`] for hosts with a projected view (folds, diff
    /// chunks). Default `None` = every motion takes the built-in path.
    fn motion_hook(&mut self) -> Option<&mut dyn MotionHook> {
        None
    }
}

/// The VIEWING half of [`VimHost`] — everything a read-only host needs to
/// drive [`VimState::handle_key_ro`](crate::state::VimState::handle_key_ro).
///
/// Deliberately a STANDALONE trait rather than a supertrait of [`VimHost`]:
/// a supertrait would force every existing editing host to add a second
/// `impl` block (and would let a forgotten `viewport` silently fall back to
/// a default). [`RoHost`] bridges the two instead.
///
/// What is NOT here, and why: `begin_undo_group`/`undo`/`redo` (no undo
/// stack), `save`/`cycle_buffer`/`first_buffer`/`last_buffer` (`:w` and
/// buffer rotation are meaningless without an editable buffer list),
/// `clipboard_read` (paste is an edit — `E21`).
pub trait VimHostRo {
    /// Inclusive `(first_line, last_line)` of what is currently visible.
    fn viewport(&self) -> (usize, usize);

    /// Make sure `line` is visible — a MINIMAL scroll, never a re-center
    /// (see [`VimHost::scroll_to_line`]).
    fn scroll_to_line(&mut self, line: usize);

    /// Make `line` visible with a specific anchor (`zz`/`zt`/`zb`).
    fn scroll_to_line_anchored(&mut self, line: usize, anchor: ScrollAnchor) {
        let _ = anchor;
        self.scroll_to_line(line);
    }

    /// Scroll the viewport by `lines` (positive = down) without moving the
    /// cursor (`C-e`/`C-y`).
    fn scroll_lines(&mut self, lines: i32) {
        let _ = lines;
    }

    fn clipboard_write(&mut self, text: &str);

    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>);

    /// The host's window width in display columns (`'columns'`).
    fn columns(&self) -> usize {
        80
    }

    fn changed(&mut self) {}

    /// Feedback for ignored keys (visual bell / flash).
    fn bell(&mut self) {}

    /// Non-fatal feedback text for the status UI.
    fn status_message(&mut self, message: &str) {
        let _ = message;
    }

    fn buffer_name(&self) -> &str {
        ""
    }

    /// `:q` — close the viewer.
    fn request_close(&mut self) {}

    /// `:q!`/`ZQ` with the force flag.
    fn request_close_forced(&mut self, forced: bool) {
        let _ = forced;
        self.request_close();
    }

    /// Opt-in [`MotionHook`] (folds, diff chunks). Default `None`.
    fn motion_hook(&mut self) -> Option<&mut dyn MotionHook> {
        None
    }
}

/// Adapts a [`VimHostRo`] onto [`VimHost`] so the read-only pipeline can
/// reuse the ONE key pipeline. Editing-side hooks are inert: `undo`/`redo`
/// report "nothing to undo", `clipboard_read` reports "nothing to paste",
/// `save` and buffer cycling do nothing.
pub struct RoHost<'a> {
    inner: &'a mut dyn VimHostRo,
}

impl<'a> RoHost<'a> {
    pub fn new(host: &'a mut dyn VimHostRo) -> Self {
        RoHost { inner: host }
    }
}

impl VimHost for RoHost<'_> {
    fn viewport(&self) -> (usize, usize) {
        self.inner.viewport()
    }
    fn scroll_to_line(&mut self, line: usize) {
        self.inner.scroll_to_line(line)
    }
    fn scroll_to_line_anchored(&mut self, line: usize, anchor: ScrollAnchor) {
        self.inner.scroll_to_line_anchored(line, anchor)
    }
    fn scroll_lines(&mut self, lines: i32) {
        self.inner.scroll_lines(lines)
    }
    fn columns(&self) -> usize {
        self.inner.columns()
    }
    fn clipboard_write(&mut self, text: &str) {
        self.inner.clipboard_write(text)
    }
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.inner.set_search_highlights(matches, current)
    }
    fn begin_undo_group(&mut self, _id: u64, _cursor_offset: usize) {}
    fn end_undo_group(&mut self, _id: u64, _cursor_offset: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {
        self.inner.changed()
    }
    fn bell(&mut self) {
        self.inner.bell()
    }
    fn save(&mut self) {}
    fn request_close(&mut self) {
        self.inner.request_close()
    }
    fn request_close_forced(&mut self, forced: bool) {
        self.inner.request_close_forced(forced)
    }
    fn status_message(&mut self, message: &str) {
        self.inner.status_message(message)
    }
    fn buffer_name(&self) -> &str {
        self.inner.buffer_name()
    }
    fn cycle_buffer(&mut self, _forward: bool) -> bool {
        false
    }
    fn first_buffer(&mut self) -> bool {
        false
    }
    fn last_buffer(&mut self) -> bool {
        false
    }
    fn dispatch_host_action_hinted(&mut self, id: &str, strict: bool) {
        let _ = (id, strict);
    }
    fn motion_hook(&mut self) -> Option<&mut dyn MotionHook> {
        self.inner.motion_hook()
    }
}
