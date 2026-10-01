//! Engine state and the key-handling pipeline.
//!
//! The pipeline mirrors IdeaVim's `KeyHandler` consumer chain, simplified for
//! a synchronous engine:
//!
//! ```text
//! mapping queue → char-argument → count → register → operator-pending
//!   → command trie (per phase) → fall back to the host (Unknown)
//! ```
//!
//! `KeyResult::Unknown` is the contract with the host: an unknown key is fed
//! back to gpui's normal key handling (keymap bindings, IME, ...).

use crate::buffer::{clamp_cursor, VimBuffer, VimBufferMut};
use crate::cmdline::Cmdline;
use crate::host::{ScrollAnchor, VimHost};
use crate::key::{Key, KeyKind, Modifiers};
use crate::keymap::{self, Keymaps, MappingMatch};
use crate::marks::Marks;
use crate::mode::{Mode, VisualKind};
use crate::motions::Motion;
use crate::ops::{self, Operator};
use crate::options::Options;
use crate::registers::Registers;
use crate::search::SearchState;
use crate::tables::{mapping_class_for, CmdKind, CommandTables, NormalCmd, Phase, VisualCmd};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ops::Range;

/// Buffer + host pair threaded through all engine calls.
pub struct Ctx<'a> {
    pub buf: &'a mut dyn VimBufferMut,
    pub host: &'a mut dyn VimHost,
}

/// What happened to a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    /// The engine used (or deliberately swallowed) the key.
    Consumed,
    /// The engine does not handle this key: the host should process it.
    Unknown,
}

/// How an insert session was started (drives cursor placement + `Esc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertKind {
    Insert,              // i
    Append,              // a
    InsertFirstNonBlank, // I
    AppendLineEnd,       // A
    OpenLine {
        below: bool,
    }, // o / O
    InsertAtColumnZero,  // gI
    /// gi: insert where the last insert session ended (`^` mark), or at the
    /// cursor when there is none.
    LastInsertExit, // gi
    Change,              // c / s / S / C
    Replace,             // R: overwrite instead of insert
}

/// Pending multi-row insert of a visual-block `I`/`A`/`c`: on exit the
/// typed text is replicated onto every other row (single undo group).
#[derive(Debug)]
struct BlockInsert {
    /// (insert offset, row's line index) for the non-cursor rows, both taken
    /// at session start. The offset already accounts for a block `c`
    /// deletion; the line index decides which rows shift with the typing.
    rows: Vec<(usize, usize)>,
    /// Line index the typing happens on (session start; vertical motions are
    /// locked out mid-session, so this stays the typing row throughout).
    typing_line: usize,
    /// Byte length of the typing row at session start. The exact per-row
    /// shift on exit is this row's length DELTA — typing, backspace and
    /// autoindent newlines all fold into it (an estimate from the typed
    /// text's length drifted when the session edited the row otherwise, and
    /// replication offsets landed mid-character).
    typing_line_len: usize,
    text: String,
    /// Cursor offset right after the last appended chunk; a backspace
    /// landing here undoes typed text and shrinks `text` to match.
    typed_end: Option<usize>,
}

/// Pending count-repeat of a plain insert session (`3ifoo<Esc>` types foo
/// three times): the count, whether the session opened a LINE (`o`/`O` so
/// each copy gets its own line), and the text typed so far. Sessions that
/// navigated (arrows/backspace/enter) or typed multi-line text do not
/// replicate. `.` repeats one copy (documented divergence from vim).
#[derive(Debug)]
struct InsertRepeat {
    count: usize,
    linewise: bool,
    text: String,
}

/// Synthetic pending-key marker for a recorded [`RecordedStep::Text`]: when
/// the `.` replay reaches it, the stashed text is applied through
/// `insert_text_at_cursor` instead of the key pipeline. Not producible by
/// `Key::parse`, so it can never collide with real keys or mappings.
pub(crate) const DOT_TEXT_MARKER: &str = "\u{0}dot-text";

/// Safety valve for the pending-key pipeline: one keystroke may legitimately
/// enqueue thousands of keys (a mapping RHS, a replayed macro — `10000@a`
/// with a five-key macro is 50k), but a live lock (e.g. a macro that expands
/// to itself) must not hang the host. On trip the queue is dropped.
const MAX_PIPELINE_STEPS: usize = 100_000;

/// Depth limit for nested mapping expansions. A `:map x y` + `:map y x` pair
/// would otherwise ping-pong forever; vim errors out the same way.
const MAX_MAP_DEPTH: usize = 1_000; // vim 9.1 default maxmapdepth=1000

/// Cap for the jumplist (`C-o`/`C-i` history) and the changelist (`g;`/`g,`).
const LIST_LIMIT: usize = 100;

/// One recorded step of the last change, for `.` repeat. Typed text is
/// recorded as [`RecordedStep::Text`] (it never goes through the key
/// pipeline — on macOS it arrives via the IME — so replay must not feed it
/// back as keys either, or the host would insert it a second time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecordedStep {
    Key(Key),
    Text(String),
}

/// State collected while a char-argument command waits for its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharArgCmd {
    Find {
        forward: bool,
        till: bool,
    },
    Replace,
    MarkSet,
    JumpMark {
        linewise: bool,
    },
    /// `q{reg}` / the trailing `q` that stops recording.
    MacroRecord,
    /// `@{reg}` / `@@`.
    MacroPlay,
    /// Visual `r{char}`: replace the whole selection with one char.
    VisualReplace,
}

#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub offset: usize,
    /// Visual column (bytes from line start) kept across vertical moves,
    /// like vim's `wv_col`. `None` = derive from offset.
    pub desired_col: Option<usize>,
}

/// Marker for an in-progress insert session. The session's undo group lives
/// in `open_undo` and its exit point in `marks.last_insert_exit`; the engine
/// only needs to know *whether* a session is open (exit behavior is currently
/// uniform across insert kinds).
#[derive(Clone, Copy, Debug)]
pub(crate) struct InsertSession;

/// The vim engine. Hosts embed one per buffer/editor.
pub struct VimState {
    pub mode: Mode,
    pub cursor: Cursor,
    pub(crate) visual_anchor: Option<usize>,

    // pending command assembly
    count: Option<usize>,
    register: Option<char>,
    register_pending: bool,
    op: Option<Operator>,
    op_count: Option<usize>,
    cmd_seq: Vec<Key>,
    char_arg_cmd: Option<CharArgCmd>,

    pub(crate) char_arg: Option<char>,
    pub(crate) last_find: Option<(char, bool, bool)>,

    pending_keys: VecDeque<Key>,
    map_depth: usize,

    /// `.` repeat: the last change as replayable steps, plus the in-progress
    /// recording. Text typed during an insert session is recorded as
    /// [`RecordedStep::Text`]. Visual-mode changes are not repeatable (v1).
    last_change: Vec<RecordedStep>,
    recording: Vec<RecordedStep>,
    recording_mutated: bool,
    recording_blocked: bool,
    /// Set while `.` replays: keys flow through the pipeline but recording
    /// and committing are suppressed so `last_change` stays put.
    replaying: bool,
    replay_texts: VecDeque<String>,
    /// Hosts suppress recording while IME composition previews mutate the
    /// buffer; only the committed text becomes part of a `.` repeat.
    recording_suppressed: bool,

    /// Macro registers (`q`/`@`). Stored as recorded steps (not register
    /// text): the notation round-trip through `Key::parse` is lossy for
    /// named keys, and typed text never flows through the key pipeline.
    macros: HashMap<char, Vec<RecordedStep>>,
    /// Active `q` recording: the register and the steps captured so far.
    macro_capture: Option<(char, Vec<RecordedStep>)>,
    /// Last register executed with `@` (for `@@`).
    last_macro_played: Option<char>,

    /// Remaining keys of a `:noremap` expansion: the mapping table is not
    /// consulted while these are consumed (no recursive remap).
    no_remap_left: usize,
    /// A user mapping just expanded: the queued expansion belongs to the
    /// mapping, so a cmdline entry inside it must not break the queue early.
    expanding_mapping: bool,
    /// Changelist (`g;`/`g,`): positions of recent changes, newest last;
    /// `change_pos` indexes the current entry.
    changes: Vec<usize>,
    change_pos: usize,
    /// For the `gq`/`gw` spellings of Operator::Format: the trigger letter
    /// (`q` or `w`) to match in the linewise doubling.
    format_trigger: Option<char>,
    /// Visual state while the visual `:` prompt is open: (kind, anchor,
    /// cursor-at-prompt). Cancel restores the selection intact; on execute
    /// the `'<`/`'>` marks are written from the PROMPT-TIME range — vim
    /// keeps the executed range, not "anchor..post-command cursor".
    pub(crate) cmdline_visual: Option<(crate::mode::VisualKind, usize, usize)>,
    /// `:action <unknown-id>` is silently ignored instead of reported.
    /// Hosts sharing one rc file across apps set this while applying the
    /// user layer (mappings aimed at other apps are expected to miss).
    pub(crate) lenient_actions: bool,
    /// Whether every edit re-runs the search highlight scan (default). Huge
    /// file hosts turn this off and refresh on their own schedule.
    hlsearch_live_update: bool,
    /// Visual-block `I`/`A`/`c`: the rows (insert offsets, descending) that
    /// receive the typed text when the session exits, and the text typed on
    /// the cursor row so far.
    block_insert: Option<BlockInsert>,
    /// Pending count-repeat of a plain insert session (see [`InsertRepeat`]).
    insert_repeat: Option<InsertRepeat>,

    /// Jumplist (`C-o`/`C-i`): visited positions, `jump_pos` = index of the
    /// current entry. Jump motions and search execution append the origin
    /// and destination; forward entries are truncated on a new jump.
    jumps: Vec<usize>,
    jump_pos: usize,

    pub(crate) insert_session: Option<InsertSession>,
    pub(crate) insert_register_pending: bool,
    /// First buffer position typed into during the current insert session.
    /// Plain `i`-sessions run no command `bump`, so this is what puts them
    /// on the changelist (vim's `g;` lands on the first inserted char).
    insert_change_pos: Option<usize>,

    pub options: Options,
    pub registers: Registers,
    pub marks: Marks,
    pub search: SearchState,
    pub cmdline: Cmdline,
    pub keymaps: Keymaps,
    tables: CommandTables,

    /// Replace (`R`) mode: characters the typing overwrote, newest last.
    /// `None` entries mark text typed PAST the line end (no original to
    /// restore). Backspace pops this stack and restores, like vim.
    pub(crate) replace_overwritten: Vec<Option<char>>,

    /// Pre-edit selection bounds of a visual change that opened the current
    /// insert session (`viwc`, visual-block `I`/`A`/`c`). vim re-selects the
    /// ORIGINAL selection with `gv` after such a change (9.1 probe:
    /// `viwcX<Esc>gv` spans the original byte range; block `I` the original
    /// block) — the stash is written back unadjusted in `exit_insert`,
    /// because the session's edits would otherwise shift/collapse it.
    pending_visual_marks: Option<(usize, usize, VisualKind)>,

    undo_seq: u64,
    open_undo: Option<u64>,
    /// Whether `open_undo` has been announced to the host (lazy group
    /// opening — see `flush_undo_group`).
    undo_group_announced: bool,
    /// Bumped by every buffer mutation (the `edit_*` funnels, engine-driven
    /// undo/redo, option changes through `:set`). Lets `n`/`N` tell whether
    /// the cached match list still describes the current text.
    pub(crate) edit_generation: u64,
    /// Platform plumbing (gpui-vim): printable chars the key interceptor
    /// declined in insert mode. On Linux/Windows the platform delivers that
    /// same char again through the text-input path, where it must be placed
    /// without re-running the pipeline. A QUEUE, not a slot: a mapping
    /// prefix can hold several printables in one `handle_key` call
    /// (`:imap jk <Esc>` then typing `jx` — the `j` was answered Consumed
    /// while it was a mapping prefix, and the `x` breaks the prefix in the
    /// same walk; a single Option overwrote the `j` and the character was
    /// lost from the buffer).
    pub(crate) pending_unknown_chars: Vec<char>,
}

impl Default for VimState {
    fn default() -> Self {
        Self::new()
    }
}

impl VimState {
    pub fn new() -> Self {
        VimState {
            mode: Mode::Normal,
            cursor: Cursor {
                offset: 0,
                desired_col: None,
            },
            visual_anchor: None,
            count: None,
            register: None,
            register_pending: false,
            op: None,
            op_count: None,
            cmd_seq: Vec::new(),
            char_arg_cmd: None,
            char_arg: None,
            last_find: None,
            pending_keys: VecDeque::new(),
            map_depth: 0,
            last_change: Vec::new(),
            recording: Vec::new(),
            recording_mutated: false,
            recording_blocked: false,
            replaying: false,
            replay_texts: VecDeque::new(),
            recording_suppressed: false,
            macros: HashMap::new(),
            macro_capture: None,
            last_macro_played: None,
            no_remap_left: 0,
            expanding_mapping: false,
            lenient_actions: false,
            hlsearch_live_update: true,
            changes: Vec::new(),
            change_pos: 0,
            format_trigger: None,
            cmdline_visual: None,
            block_insert: None,
            insert_repeat: None,
            jumps: Vec::new(),
            jump_pos: 0,
            insert_session: None,
            insert_register_pending: false,
            insert_change_pos: None,
            options: Options::default(),
            registers: Registers::default(),
            marks: Marks::default(),
            search: SearchState::default(),
            cmdline: Cmdline::default(),
            keymaps: Keymaps::default(),
            tables: CommandTables::build(),
            replace_overwritten: Vec::new(),
            pending_visual_marks: None,
            undo_seq: 0,
            open_undo: None,
            undo_group_announced: false,
            edit_generation: 0,
            pending_unknown_chars: Vec::new(),
        }
    }

    // ---- read API for hosts ------------------------------------------------

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor_offset(&self) -> usize {
        self.cursor.offset
    }

    /// `(anchor, cursor, kind)` while in visual mode (raw, unnormalized).
    pub fn visual_selection(&self) -> Option<(usize, usize, VisualKind)> {
        match self.mode {
            Mode::Visual { kind } => Some((self.visual_anchor?, self.cursor.offset, kind)),
            // the visual `:` prompt: the selection is the FROZEN prompt-time
            // range. The live `visual_anchor` may already be stale here (it
            // survives the prompt while other code moves the cursor), and
            // reporting it handed hosts a mid-character anchor (fuzz round
            // 13) — the snapshot pair is floored by the edit funnels instead.
            Mode::CommandLine { .. } => {
                let (kind, anchor, prompt_cursor) = self.cmdline_visual?;
                Some((anchor, prompt_cursor, kind))
            }
            // outside visual there is no selection (a stale anchor must not
            // keep rendering one)
            _ => None,
        }
    }

    /// Status-bar mode text (`-- INSERT --` etc.).
    pub fn mode_indicator(&self) -> &'static str {
        self.mode.indicator()
    }

    /// Pending-key display for `showcmd` (e.g. `"3d"` while typing `3dd`).
    pub fn showcmd(&self) -> String {
        if !self.options.showcmd {
            return String::new();
        }
        let mut s = String::new();
        if let Some(r) = self.register {
            s.push('"');
            s.push(r);
        }
        if let Some(c) = self.count.or(self.op_count) {
            s.push_str(&c.to_string());
        }
        if let Some(op) = self.op {
            s.push_str(op_keys(op));
        }
        for key in &self.cmd_seq {
            s.push_str(&key.notation());
        }
        if self.char_arg_cmd.is_some() {
            s.push_str(
                self.char_arg
                    .map(|c| c.to_string())
                    .as_deref()
                    .unwrap_or(""),
            );
        }
        if matches!(self.mode, Mode::Visual { .. }) {
            if let Some((_, _, kind)) = self.visual_selection() {
                s.push_str(kind.indicator());
            }
        }
        if s.is_empty() {
            return String::new();
        }
        s
    }

    /// Cursor width hint: block in normal/visual modes, bar in insert.
    pub fn cursor_is_block(&self) -> bool {
        !matches!(self.mode, Mode::Insert | Mode::Replace)
    }

    /// True when no command input is partially assembled: no pending count,
    /// register prefix, operator, char argument, multi-key sequence or
    /// queued keys. Hosts that route some keys AROUND the engine (e.g.
    /// `VimEdit`'s local undo stack) must only intercept when this is true —
    /// an `u` arriving while `g` is pending belongs to `gu`, and hijacking
    /// it breaks `gu`/`guw`/`guu`.
    pub fn is_idle(&self) -> bool {
        self.count.is_none()
            && self.register.is_none()
            && !self.register_pending
            && self.op.is_none()
            && self.op_count.is_none()
            && self.char_arg_cmd.is_none()
            && self.cmd_seq.is_empty()
            && self.pending_keys.is_empty()
    }

    pub fn options_mut(&mut self) -> &mut Options {
        &mut self.options
    }

    pub fn keymaps_mut(&mut self) -> &mut Keymaps {
        &mut self.keymaps
    }

    /// The register currently recording a macro (`q`), for host status UI.
    pub fn macro_recording(&self) -> Option<char> {
        self.macro_capture.as_ref().map(|(reg, _)| *reg)
    }

    /// Number of recorded steps in a macro register (0 when unset). Status
    /// UI / test observability: macros live as step lists, not text.
    pub fn macro_len(&self, reg: char) -> usize {
        self.macros.get(&reg).map_or(0, Vec::len)
    }

    /// True while a visual-block `I`/`A`/`c` session is gathering text for
    /// multi-row replication (insert mode locks vertical motions then).
    pub(crate) fn in_block_insert(&self) -> bool {
        self.block_insert.is_some()
    }

    /// The end offset of the block session's typed text, if any (for the
    /// block-session BS rule: only the last typed char deletes).
    pub(crate) fn block_typed_end(&self) -> Option<usize> {
        self.block_insert.as_ref().and_then(|b| b.typed_end)
    }

    /// Remember where the block session's typed text ends (its cursor after
    /// an append), so a backspace there can shrink the replica text.
    pub(crate) fn block_note_typed_end(&mut self, at: usize) {
        if let Some(block) = &mut self.block_insert {
            block.typed_end = Some(at);
        }
    }

    /// Backspace undo for a block session: when the cursor is exactly at the
    /// end of the appended text, drop its last char from the replica. `at`
    /// is the cursor offset BEFORE the deletion.
    pub(crate) fn block_backspace_undo(&mut self, at: usize) {
        let Some(block) = &mut self.block_insert else {
            return;
        };
        if block.typed_end == Some(at) {
            if let Some(c) = block.text.chars().last() {
                block.text.truncate(block.text.len() - c.len_utf8());
                block.typed_end = Some(at - c.len_utf8());
            }
        }
    }

    /// Host-side recording suppression: while set, text placed into the
    /// buffer is NOT recorded for `.` repeat. Wrap IME composition preview
    /// mutations (the raw pinyin) with this; only the committed text should
    /// be repeatable.
    pub fn set_recording_suppressed(&mut self, suppressed: bool) {
        self.recording_suppressed = suppressed;
    }

    /// Platform plumbing (see [`VimState::take_pending_unknown_chars`]).
    pub fn set_pending_unknown_char(&mut self, c: Option<char>) {
        self.pending_unknown_chars = c.into_iter().collect();
    }

    /// Platform plumbing: printable chars the key interceptor declined in
    /// insert mode and which the platform will deliver again through the
    /// text-input path, in order. Drained by the host as the platform
    /// replays them.
    pub fn take_pending_unknown_chars(&mut self) -> Vec<char> {
        std::mem::take(&mut self.pending_unknown_chars)
    }

    /// Single-char spelling of [`VimState::take_pending_unknown_chars`] for
    /// hosts that place one char per event.
    pub fn take_pending_unknown_char(&mut self) -> Option<char> {
        if self.pending_unknown_chars.is_empty() {
            None
        } else {
            Some(self.pending_unknown_chars.remove(0))
        }
    }

    /// Column helper for vertical motions.
    pub(crate) fn desired_column(&self, buf: &dyn VimBuffer) -> usize {
        match self.cursor.desired_col {
            Some(col) => col,
            None => crate::buffer::display_column(buf, self.cursor.offset),
        }
    }

    // ---- top-level entry ---------------------------------------------------

    /// Feed one keystroke into the engine.
    pub fn handle_key(&mut self, ctx: &mut Ctx, mut key: Key) -> KeyResult {
        // an escape is an escape no matter which modifiers the platform
        // layered on top of it (hyper-key taps like caps-lock→Esc mappings
        // can release their modifiers in the same event). This runs *before*
        // the Cmd passthrough so `<D-Esc>` still exits insert mode.
        if matches!(&key.kind, KeyKind::Named(name) if name == "escape") {
            key.modifiers = Modifiers::NONE;
        }

        // Space arrives under two spellings — `Named("space")` from the gpui
        // key path and `Char(' ')` from the text-input path (and from every
        // `<Space>` mapping, since parse_angle canonicalizes to Char).
        // Canonicalize so mapping lookups and char-argument commands see one
        // key no matter which path delivered it.
        if matches!(&key.kind, KeyKind::Named(name) if name == "space") && key.modifiers.is_plain()
        {
            key.kind = KeyKind::Char(' ');
        }

        // A printable key's shift flag is redundant: the character itself
        // already encodes it (macOS hands over `I` as shift+i with key_char
        // "I", `$` as shift+4 with key_char "$"), while commands and mappings
        // are declared as plain chars (`Key::parse("I")`, `Key::parse("$")`).
        // Drop the flag so shifted keys hit the same command-table entries —
        // otherwise every uppercase letter and shifted punctuation misses.
        if key.modifiers.shift && key.modifiers.is_plain() && matches!(key.kind, KeyKind::Char(_)) {
            key.modifiers.shift = false;
        }

        // other host command chords (Cmd-…) always pass through
        if key.modifiers.platform {
            return KeyResult::Unknown;
        }

        if matches!(self.mode, Mode::CommandLine { .. }) {
            // the pipeline loop never runs in cmdline mode — record here so
            // `.` can replay Ex commands typed into the prompt
            if !self.replaying {
                self.record_key(&key);
            }
            return self.cmdline_key(ctx, key);
        }

        self.pending_keys.push_back(key.clone());
        let mut guard = 0usize;
        let mut any_unknown = false;

        while let Some(front) = self.pending_keys.front().cloned() {
            guard += 1;
            if guard > MAX_PIPELINE_STEPS {
                self.pending_keys.clear();
                // a stale no-remap budget would silently bypass mapping
                // resolution for every LATER keystroke until it drained
                self.no_remap_left = 0;
                self.reset_pending();
                self.discard_change_record();
                self.replaying = false;
                self.replay_texts.clear();
                return KeyResult::Consumed;
            }

            match self.mapping_step(ctx) {
                MappingStep::Expanded => continue,
                // both end the pipeline with the queue preserved for the next
                // keystroke (a mapping or builtin still waiting for more keys).
                // map_depth resets WITH the loop: the counter only guards
                // runaway expansion WITHIN one walk — letting it persist
                // across calls ratcheted it toward a false runaway abort that
                // wiped freshly typed text
                MappingStep::Wait | MappingStep::Done => {
                    self.map_depth = 0;
                    return KeyResult::Consumed;
                }
                MappingStep::FallThrough => {}
            }

            self.pending_keys.pop_front();
            // replayed text is applied inline, not through the key pipeline
            if front.kind == KeyKind::Named(DOT_TEXT_MARKER.to_owned()) {
                if let Some(text) = self.replay_texts.pop_front() {
                    self.insert_text_at_cursor(ctx, &text);
                }
                continue;
            }
            // record the key optimistically; keys the engine DECLINES
            // (insert-mode printables on macOS: the host places that text
            // itself) are popped again below — the text placement records
            // the Text step instead, exactly once. Otherwise every typed
            // char would replay twice.
            if !self.replaying {
                self.record_key(&front);
            }
            let outcome = self.process_key(ctx, front);
            if !self.replaying && outcome == ProcessOutcome::Unknown {
                self.unrecord_key();
            }
            match outcome {
                ProcessOutcome::Consumed => {}
                ProcessOutcome::Unknown => any_unknown = true,
                ProcessOutcome::Feed(keys) => {
                    for key in keys.into_iter().rev() {
                        self.pending_keys.push_front(key);
                    }
                }
            }
            // a mapping RHS that enters cmdline mode must keep processing
            // its queued keys (`:action Foo<CR>` as a mapping RHS)
            if matches!(self.mode, Mode::CommandLine { .. })
                && !self.replaying
                && !self.expanding_mapping
            {
                break;
            }
        }
        self.map_depth = 0;
        if self.pending_keys.is_empty() && self.replaying {
            self.replaying = false;
            self.expanding_mapping = false;
            self.replay_texts.clear();
            self.recording.clear();
            self.recording_mutated = false;
        }
        if self.pending_keys.is_empty() {
            self.expanding_mapping = false;
        }
        // A visual `:` snapshot is consumed by its prompt's execute (marks
        // written from the prompt-time range) or cancel (selection restored).
        // Any leftover while the prompt is GONE means a close path bypassed
        // both — keeping it would let `visual_selection` report a stale frozen
        // selection to the host, so drop it here (the sweep runs after every
        // keystroke; a live prompt leaves its snapshot untouched above).
        if !matches!(self.mode, Mode::CommandLine { .. }) {
            self.cmdline_visual = None;
        }
        if any_unknown {
            KeyResult::Unknown
        } else {
            KeyResult::Consumed
        }
    }

    /// Try to resolve the queue front through the user's mappings.
    ///
    /// Skipped while an operator is pending (vim uses `:omap` there, which
    /// is not supported yet) and while a `:noremap` expansion is being
    /// consumed (no recursive remap).
    fn mapping_step(&mut self, ctx: &mut Ctx) -> MappingStep {
        if self.no_remap_left > 0 {
            self.no_remap_left -= 1;
            return MappingStep::FallThrough;
        }
        if self.op.is_some() {
            return MappingStep::FallThrough;
        }
        // A partial builtin command is being assembled (`cmd_seq` holds the
        // `g` of `gj`): vim routes the continuation key straight into the
        // command — mappings are only consulted BETWEEN commands, never for
        // a partial command's next key (probe 9.1: `:nnoremap j G` + typed
        // `gj` runs the builtin `gj`; the mapped `G` does not fire).
        if !self.cmd_seq.is_empty() {
            return MappingStep::FallThrough;
        }
        let class = mapping_class_for(self.mode);
        // Waiting keeps the key IN the queue: insert-mode printables are
        // declined by the pipeline and delivered by the host later, so
        // popping them here would lose them.
        let contiguous: &[Key] = self.pending_keys.make_contiguous();
        match keymap::lookup(self.keymaps.table(class), contiguous) {
            MappingMatch::Match {
                used,
                expansion,
                noremap,
            } => {
                self.pending_keys.drain(..used);
                if noremap {
                    self.no_remap_left = expansion.len();
                }
                self.expanding_mapping = true;
                for key in expansion.into_iter().rev() {
                    self.pending_keys.push_front(key);
                }
                self.map_depth += 1;
                if self.map_depth > MAX_MAP_DEPTH {
                    self.pending_keys.clear();
                    self.no_remap_left = 0;
                    self.reset_pending();
                    ctx.host.bell();
                    return MappingStep::Done;
                }
                MappingStep::Expanded
            }
            // a longer mapping may still follow — BUT if the built-in
            // command trie already resolves the combined input, prefer it:
            // `gg` must fire on the second press even when a `gt` mapping
            // exists (vim resolves the moment the input stops being a
            // mapping prefix)
            MappingMatch::Waiting => {
                if matches!(self.mode, Mode::Insert | Mode::Replace) {
                    return MappingStep::Wait;
                }
                let mut combined: Vec<Key> = self.cmd_seq.clone();
                combined.extend(self.pending_keys.iter().cloned());
                let phase = match self.mode {
                    Mode::Visual { .. } => Phase::Visual,
                    _ => Phase::Normal,
                };
                let builtin = self.tables.trie(phase).get(&combined);
                if !matches!(builtin, keymap::Walk::Hit(_)) {
                    // no builtin for the combined input either — the queued
                    // keys must stay for the mapping to complete (e.g.
                    // `<Leader>a`, ambiguous on BOTH sides)
                    return MappingStep::Wait;
                }
                // `,` is both a complete builtin (repeat-find reverse) and a
                // live mapping prefix (`<Leader>d` with the default
                // mapleader): vim without 'timeout' keeps waiting while the
                // combined input can still grow into a mapping — fire the
                // builtin only when no mapping can extend it.
                let mapping_can_extend = match self.keymaps.table(class) {
                    Some(table) => matches!(table.get(&combined), keymap::Walk::Pending),
                    None => false,
                };
                if mapping_can_extend {
                    return MappingStep::Wait;
                }
                let keymap::Walk::Hit(kind) = builtin else {
                    unreachable!("checked Hit above");
                };
                let kind = *kind;
                let queued: Vec<Key> = self.pending_keys.drain(..).collect();
                self.cmd_seq.clear();
                if !self.replaying {
                    for k in &queued {
                        self.record_key(k);
                    }
                }
                // a resolved command consumes cleanly; Feed can't occur for
                // a terminal trie hit
                let _ = self.execute_command(ctx, kind);
                MappingStep::Done
            }
            MappingMatch::None => MappingStep::FallThrough,
        }
    }

    /// Push one key onto the `.` recording and the active macro capture.
    /// Callers gate on `replaying`: a replay must not append to the
    /// recording (its steps come FROM the recording).
    fn record_key(&mut self, key: &Key) {
        self.recording.push(RecordedStep::Key(key.clone()));
        if let Some((_, keys)) = &mut self.macro_capture {
            keys.push(RecordedStep::Key(key.clone()));
        }
    }

    /// Undo [`VimState::record_key`] when the engine declines the key.
    fn unrecord_key(&mut self) {
        self.recording.pop();
        if let Some((_, keys)) = &mut self.macro_capture {
            keys.pop();
        }
    }

    /// Enter replay mode (`.` repeat / `@` macro): pipeline results are no
    /// longer recorded, and the stale recording accumulated so far is
    /// dropped (the `.` key itself must not become the first step of the
    /// next change).
    fn begin_replay(&mut self) {
        // replay through the pipeline with `.` recording suppressed; the
        // pipeline guard handles recursive macros
        self.replaying = true;
        self.recording.clear();
        self.recording_mutated = false;
    }

    /// Queue `steps` for replay, `count` times. Plain keys re-enter the key
    /// pipeline; recorded text is applied inline via the synthetic
    /// [`DOT_TEXT_MARKER`] key.
    ///
    /// The queue is built EAGERLY, so `count` is clamped to the pipeline
    /// guard's budget (`count × steps` keys would otherwise be pre-allocated
    /// up front — `99999999.` meant gigabytes of queue before the guard ever
    /// saw a step). Beyond the budget the replay stops early instead of
    /// tripping the lose-everything guard.
    fn enqueue_replay(&mut self, steps: &[RecordedStep], count: usize) {
        let rounds = (MAX_PIPELINE_STEPS / steps.len().max(1)).max(1);
        for _ in 0..count.min(rounds) {
            for step in steps {
                match step {
                    RecordedStep::Key(key) => self.pending_keys.push_back(key.clone()),
                    RecordedStep::Text(text) => {
                        self.replay_texts.push_back(text.clone());
                        self.pending_keys.push_back(Key::named(DOT_TEXT_MARKER));
                    }
                }
            }
        }
    }

    fn process_key(&mut self, ctx: &mut Ctx, key: Key) -> ProcessOutcome {
        match self.mode {
            Mode::Normal => self.normal_key(ctx, key),
            Mode::Visual { .. } => self.visual_key(ctx, key),
            Mode::Insert | Mode::Replace => self.insert_key(ctx, key),
            Mode::CommandLine { .. } => {
                self.cmdline_key(ctx, key);
                ProcessOutcome::Consumed
            }
        }
    }

    fn reset_pending(&mut self) {
        self.count = None;
        self.register = None;
        self.register_pending = false;
        self.op = None;
        self.op_count = None;
        self.cmd_seq.clear();
        self.char_arg_cmd = None;
        self.char_arg = None;
    }

    // ---- undo grouping -----------------------------------------------------

    /// Open a new undo group unless one is already open for the current
    /// logical command, and return its id.
    ///
    /// The host hook does NOT fire here: groups get opened speculatively by
    /// whole command families, several of which turn out to be no-ops (`x` on
    /// an empty line, `J` at EOF, `i<Esc>`, `p` with an empty register, a
    /// failed `r`). A host snapshot for a group that never edits would
    /// consume a real undo step — vim's `u` skips its own no-ops (probe:
    /// `x` on an empty line leaves the undo count untouched). The id is just
    /// recorded; [`VimState::flush_undo_group`] announces it lazily, right
    /// before the group's FIRST actual edit, when the cursor is still
    /// pre-edit as the `begin_undo_group` contract requires.
    fn open_undo_group(&mut self) -> u64 {
        match self.open_undo {
            Some(id) => id,
            None => {
                self.undo_seq += 1;
                let id = self.undo_seq;
                self.open_undo = Some(id);
                id
            }
        }
    }

    /// Announce the pending group to the host (once). Called by the three
    /// `edit_*` funnels — the only paths that actually mutate the buffer.
    fn flush_undo_group(&mut self, ctx: &mut Ctx) {
        if let Some(id) = self.open_undo {
            if !self.undo_group_announced {
                self.undo_group_announced = true;
                ctx.host.begin_undo_group(id, self.cursor.offset);
            }
        }
    }

    /// Open (or reuse) the undo group for the current logical command.
    ///
    /// Deliberately does NOT touch `recording_mutated`: whole command
    /// families open the group speculatively and several turn out to be
    /// no-ops (`x` on an empty line, `p` with an empty register). Marking
    /// the recording mutated here let a no-op command REPLACE `last_change`
    /// — `.` then replayed the no-op instead of the previous real change
    /// (vim's redo record is only written by actual edits). The three
    /// `edit_*` funnels set the flag when bytes actually moved.
    pub(crate) fn begin_edit(&mut self) {
        self.open_undo_group();
    }

    /// Close any open undo group (end of a logical command or insert session).
    pub(crate) fn end_edit(&mut self) {
        self.open_undo = None;
        self.undo_group_announced = false;
    }

    /// `C-a`/`C-x`: find the number at or after the cursor on this line and
    /// add `delta` to it, cursor on the last digit of the result. Returns
    /// false when no number is found.
    ///
    /// Radix handling follows vim's default `nrformats=bin,octal,hex`:
    /// `0b101`+2 → `0b111`, `077`+3 → `0102` (octal), `0x1f`+2 → `0x21`,
    /// `0XAB`+1 → `0XAC` (prefix and digit case preserved). Decimal literals
    /// with leading zeros are NOT zero-padded (`0099`+2 → `101`, vim same);
    /// a `0`-prefixed run containing 8/9 is decimal, not octal.
    fn increment_number_at_cursor(&mut self, ctx: &mut Ctx, delta: i64) -> bool {
        let line = ctx.buf.offset_to_line(self.cursor.offset);
        let start = ctx.buf.line_start(line);
        let end = ctx.buf.line_end(line);
        let text = ctx.buf.slice(start..end);
        let bytes = text.as_bytes();

        // cursor column (bytes) within the line
        let cur = self.cursor.offset - start;
        // "at or after the cursor": a digit UNDER the cursor means THAT
        // number — walk back to the run's first digit, or `129` with the
        // cursor on `9` would only read the `9` (129+1 turning into 1210).
        // A cursor parked at the line end sits VISUALLY on the last char,
        // so a line ending in a digit counts as "on" it too. A non-digit
        // under the cursor starts the search forward; numbers BEFORE the
        // cursor are ignored (vim reports E18 instead).
        let on_digit = bytes.get(cur).is_some_and(|b| b.is_ascii_digit())
            || (cur >= text.len() && text.ends_with(|c: char| c.is_ascii_digit()));
        let run_start = if on_digit {
            let mut i = cur.min(text.len().saturating_sub(1));
            while i > 0 && bytes[i - 1].is_ascii_digit() {
                i -= 1;
            }
            i
        } else if cur < text.len()
            && matches!(bytes[cur], b'x' | b'X' | b'b' | b'B')
            && cur > 0
            && bytes[cur - 1] == b'0'
            && bytes.get(cur + 1).is_some_and(|&b| match bytes[cur] {
                // the prefix only counts when a valid first digit follows
                // (vim 9.1: cursor on the `b` of `0backup` finds NO number)
                b'b' | b'B' => b == b'0' || b == b'1',
                _ => b.is_ascii_hexdigit(),
            })
        {
            // a radix-prefix LETTER counts as "on the number" (vim 9.1
            // probe: cursor on the `x` of `0x1f` increments the whole
            // literal to `0x20`; treating `x` as plain text grabbed the
            // bare digit run `1` and spliced garbage around it). Anchor
            // the run at the prefix's own `0` — the run=="0" branch below
            // re-detects the radix and covers the full literal.
            cur - 1
        } else {
            match (cur..text.len()).find(|&i| bytes[i].is_ascii_digit()) {
                Some(i) => i,
                None => return false,
            }
        };
        let mut run_end = run_start;
        while run_end < text.len() && bytes[run_end].is_ascii_digit() {
            run_end += 1;
        }
        let run = &text[run_start..run_end];

        // radix detection (see the doc comment): digits may sit AFTER a
        // 0x/0X/0b/0B prefix, or the cursor may be ON the prefix's own `0`
        fn is_hex(b: u8) -> bool {
            b.is_ascii_hexdigit()
        }
        fn is_bin(b: u8) -> bool {
            b == b'0' || b == b'1'
        }
        fn is_oct(b: u8) -> bool {
            (b'0'..=b'7').contains(&b)
        }
        let (radix, prefix_start, digits_start, num_end) = if run_start >= 2 && run_end > run_start
        {
            let prefix = &bytes[run_start - 2..run_start];
            let (r, valid): (u32, fn(u8) -> bool) = match prefix {
                [b'0', b'x' | b'X'] => (16, is_hex),
                [b'0', b'b' | b'B'] => (2, is_bin),
                _ => (10, is_oct),
            };
            if r != 10 && run.bytes().all(valid) {
                (r, run_start - 2, run_start, run_end)
            } else {
                (10, run_start, run_start, run_end)
            }
        } else {
            (10, run_start, run_start, run_end)
        };
        if radix == 10 && run == "0" {
            if let Some(&p) = bytes.get(run_start + 1) {
                let (r, valid): (u32, fn(u8) -> bool) = match p {
                    b'x' | b'X' => (16, is_hex),
                    b'b' | b'B' => (2, is_bin),
                    _ => (10, is_oct),
                };
                let mut e = run_start + 2;
                while e < text.len() && valid(bytes[e]) {
                    e += 1;
                }
                if r != 10 && e > run_start + 2 {
                    return self.replace_number(
                        ctx,
                        start,
                        text,
                        run_start,
                        run_start,
                        run_start + 2,
                        e,
                        r,
                        delta,
                    );
                }
            }
        }
        // decimal (possibly octal): a leading 0 with only 0-7 digits is octal
        let radix =
            if radix == 10 && run.len() > 1 && run.starts_with('0') && run.bytes().all(is_oct) {
                8
            } else {
                radix
            };
        // a directly attached minus is part of the number (vim: `-99` + 1
        // turns into `-98`, `ab-99` with the cursor before it increments to
        // `-98` as well)
        let tok_start = if prefix_start > 0 && bytes[prefix_start - 1] == b'-' {
            prefix_start - 1
        } else {
            prefix_start
        };
        self.replace_number(
            ctx,
            start,
            text,
            tok_start,
            prefix_start,
            digits_start,
            num_end,
            radix,
            delta,
        )
    }

    /// Rewrite the number token `[tok_start, num_end)` (line-relative) with
    /// `digits[digits_start..num_end]` parsed in `radix`, plus `delta`.
    /// Formats bin/octal/hex with the original prefix and digit case.
    #[allow(clippy::too_many_arguments)]
    fn replace_number(
        &mut self,
        ctx: &mut Ctx,
        line_start: usize,
        text: String,
        tok_start: usize,
        prefix_start: usize,
        digits_start: usize,
        num_end: usize,
        radix: u32,
        delta: i64,
    ) -> bool {
        let negative = text[tok_start..].starts_with('-');
        let digits = &text[digits_start..num_end];
        // unrepresentable literals saturate (vim errors instead; saturation
        // at least keeps the sign from flipping through wrap)
        let magnitude = i128::from_str_radix(digits, radix)
            .ok()
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(i64::MAX);
        let new_value = if negative {
            (-magnitude).saturating_add(delta)
        } else {
            magnitude.saturating_add(delta)
        };
        let (sign, magnitude) = if new_value < 0 {
            ("-", new_value.unsigned_abs())
        } else {
            ("", new_value.unsigned_abs())
        };
        // bin/octal/hex results keep the ORIGINAL digit width by zero-padding
        // (`0x10`-1 → `0x0f`, vim probe); decimal never pads (`0099`+2 → `101`)
        let width = num_end - digits_start;
        let pad = |body: String, width: usize| format!("{body:0>width$}");
        let body = match radix {
            16 => {
                let lower = !text[digits_start..num_end]
                    .bytes()
                    .any(|b| b.is_ascii_uppercase());
                let prefix = &text[prefix_start..prefix_start + 2]; // "0x"/"0X"
                let mut body = prefix.to_owned();
                let digits = pad(format!("{magnitude:x}"), width);
                if lower {
                    body.push_str(&digits);
                } else {
                    body.push_str(&digits.to_uppercase());
                }
                body
            }
            // octal re-adds its marker `0`, padding covers the digits after
            // it (`077`+3 → `0102`, `010`-1 → `007` — vim probes)
            8 => format!("0{}", pad(format!("{magnitude:o}"), width - 1)),
            2 => {
                let prefix = &text[prefix_start..prefix_start + 2]; // "0b"/"0B"
                format!("{prefix}{}", pad(format!("{magnitude:b}"), width))
            }
            _ => magnitude.to_string(),
        };
        let new_text = format!("{sign}{body}");

        self.begin_edit();
        let abs = line_start + tok_start;
        self.edit_replace(ctx, abs..line_start + num_end, &new_text);
        self.bump(ctx);
        // cursor on the last digit of the new number
        self.cursor.offset = (abs + new_text.len() - 1).max(abs);
        self.cursor.desired_col = None;
        self.end_edit();
        true
    }

    pub(crate) fn bump(&mut self, ctx: &mut Ctx) {
        self.record_change_position(self.cursor.offset);
        self.republish_search(ctx);
        ctx.host.changed();
    }

    /// [`VimState::bump`], but only when the buffer actually changed since
    /// `gen_before` (an `edit_*` funnel ran). Commands that can no-op (`x` on
    /// an empty line, `p` with an empty register, `J` at EOF, `~` past the
    /// line end, visual `<` with no indent to remove) must not feed the
    /// changelist: vim 9.1 skips its own no-ops (`g;` after a no-op `x` walks
    /// straight past it, probe `jx gg x g;g;` stays on the changed line).
    pub(crate) fn bump_if_edited(&mut self, ctx: &mut Ctx, gen_before: u64) {
        if self.edit_generation != gen_before {
            self.bump(ctx);
        }
    }

    /// Record `offset` as the latest change position: the `.` mark and the
    /// changelist entry (`g;`/`g,`), deduping consecutive repeats.
    fn record_change_position(&mut self, offset: usize) {
        self.marks.last_change = Some(offset);
        if self.changes.last() != Some(&offset) {
            self.changes.truncate(self.change_pos + 1);
            self.changes.push(offset);
            if self.changes.len() > LIST_LIMIT {
                self.changes.remove(0);
            }
            self.change_pos = self.changes.len() - 1;
        }
    }

    /// Buffer edits shift the byte offsets behind any published highlights;
    /// recompute and republish so `hlsearch` visuals follow the text. No-op
    /// while nothing is published (`:noh`, no pattern) — editing must not
    /// revive cleared highlights.
    pub(crate) fn republish_search(&mut self, ctx: &mut Ctx) {
        // hosts embedding the engine in huge-file editors can turn this off
        // (set_hlsearch_live_update(false)) and call refresh_highlights on
        // their own schedule (e.g. 150ms after the last edit)
        if !self.hlsearch_live_update {
            return;
        }
        self.republish_search_inner(ctx);
    }

    /// The scan-and-publish body shared by the per-edit path and the host's
    /// explicit [`VimState::refresh_highlights`] — the `hlsearch_live_update`
    /// guard lives ONLY in `republish_search`: putting it here silently
    /// no-op'd the explicit refresh forever (a host with live updates off
    /// never got a fresh match list — fuzz round 13, huge-file host mode).
    fn republish_search_inner(&mut self, ctx: &mut Ctx) {
        if !self.options.hlsearch {
            return;
        }
        // NOTE: an empty last_matches cache must NOT early-return here —
        // after a deletion removes the last match, later edits have to
        // re-scan to notice the pattern matching again.
        let Some(pattern) = self.search.pattern.clone() else {
            return;
        };
        let matches = crate::search::all_matches(self, ctx.buf, &pattern);
        if matches.is_empty() {
            self.search.last_matches.clear();
            self.search.last_index = None;
            ctx.host.set_search_highlights(&[], None);
            return;
        }
        let current = matches
            .iter()
            .find(|m| m.start >= self.cursor.offset)
            .or_else(|| matches.last())
            .cloned();
        let index = current
            .as_ref()
            .and_then(|c| matches.iter().position(|m| m == c));
        self.search.last_matches = matches;
        self.search.matches_generation = Some(self.edit_generation);
        self.search.last_index = index;
        let matches = &self.search.last_matches;
        ctx.host.set_search_highlights(matches, current);
    }

    // ---- buffer edits (the ONLY mutation paths; keep marks in sync) -------

    /// After the host swapped the text underneath the engine (undo/redo),
    /// every stored byte offset can be stale: past the end, or inside a
    /// multi-byte char of the NEW text. Floor them onto the nearest surviving
    /// boundary so later reads (`gv`, `'>`, `g;`, `C-o`) stay addressable —
    /// consumers floor individually, but a bounded mark is worth more than a
    /// clamped read.
    fn sanitize_stored_offsets(&mut self, buf: &dyn VimBuffer) {
        let floor = |off: usize| crate::buffer::floor_to_char_boundary(buf, off);
        self.marks.floor_all(&floor);
        for pos in self.changes.iter_mut() {
            *pos = floor(*pos);
        }
        for pos in self.jumps.iter_mut() {
            *pos = floor(*pos);
        }
        // the live selection survives a host undo (`u` right after `gv:`'s
        // cmdline closes back into visual) — same invariant as the edits
        if let Some(anchor) = self.visual_anchor.as_mut() {
            *anchor = floor(*anchor);
        }
        if let Some((_, a, c)) = self.cmdline_visual.as_mut() {
            *a = floor(*a);
            *c = floor(*c);
        }
    }

    /// Apply a mark-style offset adjustment to the changelist / jumplist.
    /// Both lists store raw byte offsets like the marks do, so they need the
    /// same shifting when the text moves under them (vim adjusts its
    /// jumplist and changelist on every edit; a list that stayed stale made
    /// `g;`/`C-o` land at pre-edit positions).
    fn adjust_positions(positions: &mut [usize], adjust: impl Fn(usize) -> usize) {
        for pos in positions.iter_mut() {
            *pos = adjust(*pos);
        }
    }

    /// All engine buffer mutations go through these three wrappers so marks
    /// (`a-z`, `^ . < >`) and the last-visual span shift with the text. Never
    /// call `ctx.buf.insert_text/delete_range/replace_range` directly.
    pub(crate) fn edit_insert(&mut self, ctx: &mut Ctx, at: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        if !self.replaying {
            self.recording_mutated = true;
        }
        self.flush_undo_group(ctx);
        ctx.buf.insert_text(at, text);
        let len = text.len();
        self.marks.adjust_insert(at, len);
        Self::adjust_positions(&mut self.changes, |p| if p > at { p + len } else { p });
        Self::adjust_positions(&mut self.jumps, |p| if p > at { p + len } else { p });
        self.edit_generation += 1;
    }

    pub(crate) fn edit_delete(&mut self, ctx: &mut Ctx, range: Range<usize>) {
        if range.start >= range.end {
            return;
        }
        if !self.replaying {
            self.recording_mutated = true;
        }
        self.flush_undo_group(ctx);
        ctx.buf.delete_range(range.clone());
        self.marks.adjust_delete(range.clone());
        let (start, end) = (range.start, range.end);
        let adjust = |p: usize| {
            if p >= end {
                p - (end - start)
            } else if p > start {
                start
            } else {
                p
            }
        };
        Self::adjust_positions(&mut self.changes, adjust);
        Self::adjust_positions(&mut self.jumps, adjust);
        self.edit_generation += 1;
        // Relative shifts keep offsets on char boundaries of the OLD text,
        // but two shapes can still leave them unaddressable: a deletion
        // reaching the buffer end shifts offsets past the NEW end, and an
        // equal-length replace redraws the byte grid under inner offsets
        // (`gJ`'s `\n`→space swap strands a mark mid-`中`). Floor+clamp the
        // stored offsets onto the current text — the engine's standing
        // addressability invariant (fuzz-enforced).
        self.refloor_stored_offsets(ctx);
    }

    /// Floor every stored byte offset onto the nearest char boundary of the
    /// CURRENT text (also caps at the buffer end). Used by the edit funnels
    /// after the relative mark adjustment.
    ///
    /// `visual_anchor` / `cmdline_visual` live OUTSIDE the marks map (raw
    /// fields) and so miss the marks' relative shift; they still must obey
    /// the standing addressability invariant — an Ex command run from the
    /// visual `:` prompt (`gv:` then `@:`) rewrites the text while the live
    /// selection survives, and a stale anchor resumed mid-character made
    /// host renders panic (fuzz round 13).
    fn refloor_stored_offsets(&mut self, ctx: &Ctx) {
        let buf: &dyn crate::buffer::VimBuffer = ctx.buf;
        let floor = |p: &mut usize| {
            *p = crate::buffer::floor_to_char_boundary(buf, *p);
        };
        self.marks.for_each_pos_clamped(floor);
        for pos in self.changes.iter_mut() {
            floor(pos);
        }
        for pos in self.jumps.iter_mut() {
            floor(pos);
        }
        if let Some(anchor) = self.visual_anchor.as_mut() {
            floor(anchor);
        }
        if let Some((_, a, c)) = self.cmdline_visual.as_mut() {
            floor(a);
            floor(c);
        }
    }

    pub(crate) fn edit_replace(&mut self, ctx: &mut Ctx, range: Range<usize>, text: &str) {
        if range.start >= range.end {
            return self.edit_insert(ctx, range.start, text);
        }
        if !self.replaying {
            self.recording_mutated = true;
        }
        self.flush_undo_group(ctx);
        ctx.buf.replace_range(range.clone(), text);
        let new_len = text.len();
        self.marks.adjust_replace(range.clone(), new_len);
        // same inner-preservation rule as marks: an equal-length replace
        // keeps inner offsets valid (same char positions), a length-changing
        // one collapses them onto the range start
        let delta = new_len as isize - range.len() as isize;
        let preserve_inner = new_len == range.len();
        let (start, end) = (range.start, range.end);
        let adjust = move |p: usize| {
            if p >= end {
                (p as isize + delta).max(0) as usize
            } else if p > start && !preserve_inner {
                start
            } else {
                p
            }
        };
        Self::adjust_positions(&mut self.changes, adjust);
        Self::adjust_positions(&mut self.jumps, adjust);
        self.edit_generation += 1;
        self.refloor_stored_offsets(ctx);
    }

    // ---- movement ------------------------------------------------------------

    pub(crate) fn apply_motion_result(
        &mut self,
        ctx: &mut Ctx,
        motion: Motion,
        result: crate::motions::MotionResult,
    ) {
        // the goal column must be read BEFORE the cursor lands: once the
        // cursor sits on a wide char that clamped it, the derived column
        // would be the clamped one, not the original (vim's wv_col)
        let preserves_column = matches!(
            motion,
            Motion::Up
                | Motion::Down
                | Motion::ScrollHalfDown
                | Motion::ScrollHalfUp
                | Motion::PageUp
                | Motion::PageDown
        );
        let desired = if preserves_column {
            Some(self.desired_column(ctx.buf))
        } else {
            None
        };
        self.cursor.offset = clamp_cursor(ctx.buf, result.offset);
        if result.kind == crate::motions::MotionKind::Linewise {
            if preserves_column {
                let desired = desired.unwrap();
                self.cursor.desired_col = Some(desired);
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                self.cursor.offset =
                    crate::buffer::offset_for_display_column(ctx.buf, line, desired);
            } else {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                self.cursor.offset = ctx.buf.first_non_blank(line);
                self.cursor.desired_col = None;
            }
        } else if !preserves_column {
            self.cursor.desired_col = None;
        }
        if matches!(self.mode, Mode::Visual { .. }) {
            if let Some(anchor) = self.visual_anchor {
                let (a, c) = (
                    anchor.min(self.cursor.offset),
                    anchor.max(self.cursor.offset),
                );
                // the exclusive end is one CHAR past the cursor — `c + 1`
                // bytes would land inside a multi-byte cursor char and blow
                // up `'<`/`'>` range resolution later
                let end = crate::buffer::next_grapheme_offset(ctx.buf, c).unwrap_or(c);
                self.marks.active_visual = Some((a, end));
            }
        }
        let line = ctx.buf.offset_to_line(self.cursor.offset);
        ctx.host.scroll_to_line(line);
    }

    pub(crate) fn goto_motion(&mut self, ctx: &mut Ctx, motion: Motion, count: usize) -> bool {
        let origin = self.cursor.offset;
        let result = motion.target(self, ctx, count);
        if !result.moved {
            return false;
        }
        self.apply_motion_result(ctx, motion, result);
        if motion.is_jump() {
            self.record_jump(origin, self.cursor.offset);
        }
        true
    }

    /// Append a jump (origin -> dest) to the jumplist, discarding any
    /// forward entries (like stepping back then jumping anew in a browser).
    /// Also re-points the jump-context mark (`''`/`` `` ``) at the origin,
    /// like vim.
    pub(crate) fn record_jump(&mut self, origin: usize, dest: usize) {
        self.marks.last_jump = Some(origin);
        if self.jump_pos + 1 < self.jumps.len() {
            self.jumps.truncate(self.jump_pos + 1);
        }
        if self.jumps.last() != Some(&origin) {
            self.jumps.push(origin);
        }
        if self.jumps.last() != Some(&dest) {
            self.jumps.push(dest);
        }
        while self.jumps.len() > LIST_LIMIT {
            self.jumps.remove(0);
        }
        self.jump_pos = self.jumps.len() - 1;
    }

    // ---- insert sessions -------------------------------------------------------

    pub(crate) fn begin_insert(&mut self, kind: InsertKind) {
        // Reuse an open undo group when one exists: the change family (c/s/S/C)
        // deletes the span through a group that is already open, and the
        // deletion + subsequent typing must undo as ONE step. Without this the
        // host would snapshot between deletion and typing, so the first `u`
        // only undid the typing and a second one was needed for the deletion.
        self.open_undo_group();
        self.insert_session = Some(InsertSession);
        // a NEW session supersedes any count-repeat left over from a previous
        // one (`3o` still open when a visual-block `I`/`A`/`c` starts): the
        // stale repeat fired inside the block session's exit_insert, inserting
        // its copies AFTER the replica row offsets were recorded and desyncing
        // the block replication into mid-character inserts (fuzz round 13).
        // execute_command re-arms the repeat AFTER start_insert returns, so a
        // plain `3i` keeps its count.
        self.insert_repeat = None;
        self.mode = if kind == InsertKind::Replace {
            Mode::Replace
        } else {
            Mode::Insert
        };
        self.cursor.desired_col = None;
    }

    /// Count-repeat insert on session exit: `3ifoo<Esc>` typed "foo" once,
    /// this appends two more copies. Only plain type-then-escape sessions
    /// replicate: the cursor must still sit at the end of the typed text and
    /// the text must be single-line (arrows/backspace/enter set no flag but
    /// move the cursor or add `\n`, both of which bail out). Linewise
    /// (`3ofoo<Esc>`) copies get their own lines below, each with the opened
    /// line's indent.
    fn replicate_count_insert(&mut self, ctx: &mut Ctx) {
        let Some(rep) = self.insert_repeat.take() else {
            return;
        };
        // a block session never carries a count-repeat (mutually exclusive by
        // construction); if one is still armed here the two replication passes
        // would fight over the same exit — bail instead
        if self.block_insert.is_some() {
            return;
        }
        if rep.text.is_empty() || rep.text.contains('\n') {
            return;
        }
        let Some(start) = self.insert_change_pos else {
            return;
        };
        if self.cursor.offset != start + rep.text.len() {
            return; // the session moved around: no replication
        }
        let copies = rep.count - 1;
        if rep.linewise {
            let line = ctx.buf.offset_to_line(self.cursor.offset);
            let ls = ctx.buf.line_start(line);
            let indent_str = ctx.buf.slice(ls..ls + ctx.buf.line_indent(line).0);
            let at = ctx.buf.line_end(line);
            // one unit per copy; the repeat count is byte-capped like the
            // paste path (`clamped_repeat_count`) — `99999999o` + typed text
            // must not turn a small session into a multi-gigabyte insert
            let unit = format!("\n{indent_str}{}", rep.text);
            let copies = crate::ops::clamped_repeat_count(unit.len(), copies);
            let extra = unit.repeat(copies);
            let extra_len = extra.len();
            self.edit_insert(ctx, at, &extra);
            self.cursor.offset = at + extra_len;
        } else {
            let copies = crate::ops::clamped_repeat_count(rep.text.len(), copies);
            let extra = rep.text.repeat(copies);
            let len = extra.len();
            self.edit_insert(ctx, self.cursor.offset, &extra);
            self.cursor.offset += len;
        }
        self.cursor.desired_col = None;
    }

    pub(crate) fn exit_insert(&mut self, ctx: &mut Ctx) {
        // count-repeat insert (`3ifoo<Esc>`): replicate the typed text while
        // the session's undo group is open and BEFORE the exit cursor
        // step-back, so the cursor lands one left of the LAST copy (vim)
        self.replicate_count_insert(ctx);
        // back one char unless at the line start (Replace mode too: vim
        // leaves the cursor on the last replaced character)
        let line_start = ctx
            .buf
            .line_start(ctx.buf.offset_to_line(self.cursor.offset));
        if self.cursor.offset > line_start {
            if let Some(prev) = ctx.buf.prev_char_offset(self.cursor.offset) {
                self.cursor.offset = prev.max(line_start);
            }
        }
        self.marks.last_insert_exit = Some(self.cursor.offset);
        self.marks.set('^', self.cursor.offset);
        // visual-block I/A/c: replicate the typed text onto the other rows
        // while the session's undo group is still open (one `u` restores
        // all). Rows BELOW the typing row shift by that row's exact byte
        // delta (see `BlockInsert::typing_line_len`); rows above don't move.
        if let Some(block) = self.block_insert.take() {
            // The replica model's own premise: the typing row grew by EXACTLY
            // `block.text.len()` bytes (the same delta the row offsets below
            // are shifted by). A session that navigated (<Left>+BS, <C-w>)
            // edited the row in ways `block.text` didn't track — delta and
            // text disagree — and replicating it gave the OTHER rows text the
            // typing row no longer corresponds to. On divergence keep the
            // typing row's edits and skip replication.
            let cur_len =
                ctx.buf.line_end(block.typing_line) - ctx.buf.line_start(block.typing_line);
            let delta = (cur_len as isize - block.typing_line_len as isize) as usize;
            let pure_typing = delta == block.text.len();
            if !block.text.is_empty() && pure_typing {
                let shift = delta as isize;
                let mut rows: Vec<usize> = block
                    .rows
                    .into_iter()
                    .map(|(adjusted, line)| {
                        if line > block.typing_line {
                            (adjusted as isize + shift).max(0) as usize
                        } else {
                            adjusted
                        }
                    })
                    .collect();
                rows.sort_unstable_by(|a, b| b.cmp(a)); // bottom-up inserts
                for offset in rows {
                    // the delta model assumes ONLY typing happened on the
                    // session's typing row; any other length change above a
                    // row (a stale count-repeat, a host-side rewrite) leaves
                    // the shifted offset unaligned — floor it so a replica
                    // lands at worst one char off, never mid-character (the
                    // buffer insert itself would panic the host)
                    let offset = crate::buffer::floor_to_char_boundary(ctx.buf, offset);
                    self.edit_insert(ctx, offset, &block.text);
                    // a row ABOVE the cursor shifts everything below it, the
                    // cursor's byte offset included: edit_insert adjusts
                    // marks/jumplist but the cursor is caller-managed, and a
                    // stale offset points `text.len()` bytes early — mid-
                    // character on multi-byte text (fuzz-caught)
                    if offset < self.cursor.offset {
                        self.cursor.offset += block.text.len();
                    }
                }
            }
        }
        self.commit_change_record();
        // a plain insert session never ran a command `bump`: its first typed
        // position is the changelist entry (vim probe: `i`-typing at col 2
        // then `g;` lands on that exact offset). The position was taken at
        // the session's FIRST typed char and is NOT adjusted by the edit
        // funnels — a mid-session line join (BS at line start, <C-w>) shifts
        // the text under it, so floor it onto the CURRENT text (fuzz round
        // 9: a stale entry landed mid-multibyte-char and `:marks`'s `.` line
        // panicked in the host's offset_to_line)
        if let Some(pos) = self.insert_change_pos.take() {
            let pos = crate::buffer::floor_to_char_boundary(ctx.buf, pos);
            self.record_change_position(pos);
        }
        self.insert_session = None;
        // the live `'<`/`'>` range is dead once insert mode ends: a session
        // entered from visual (`viwc`, block `I`) leaves offsets pointing at
        // pre-edit bytes, and `parse_range` must fall back to the `'<`/`'>`
        // marks instead of preferring the stale live range
        self.marks.active_visual = None;
        // gv after a visual change: vim re-selects the PRE-EDIT selection
        // bounds (9.1 probe: `viwcX<Esc>gv` spans the original byte range,
        // block `I` the original block) — write the stash here, after the
        // session's edits, exactly as vim's marks survive them. The raw
        // pre-edit offsets must still be floored onto the post-edit text:
        // the session may have changed byte lengths mid-range (insert-BS
        // line joins, <C-w>...), and stored offsets stay addressable — the
        // engine's standing invariant (fuzz-enforced), same tradeoff as
        // `sanitize_stored_offsets` after a host undo.
        if let Some((lo, hi, kind)) = self.pending_visual_marks.take() {
            let floor = |off: usize| crate::buffer::floor_to_char_boundary(ctx.buf, off);
            let (lo, hi) = (floor(lo), floor(hi));
            self.marks.last_visual = Some((lo.min(hi), hi.max(lo), kind));
        }
        self.republish_search(ctx);
        self.end_edit();
        self.mode = Mode::Normal;
        self.insert_register_pending = false;
        ctx.host.changed();
    }

    /// Insert `text` at the cursor (the IME path, and internal insert helpers).
    ///
    /// Only meaningful in insert/replace mode. Newlines get autoindent.
    pub fn insert_text_at_cursor(&mut self, ctx: &mut Ctx, text: &str) {
        if !matches!(self.mode, Mode::Insert | Mode::Replace) || text.is_empty() {
            return;
        }
        // a newline inside a block session splits the typing row the same way
        // a locked vertical move would (replica offsets assume one line per
        // row) — reject like the <CR> lock
        if text.contains('\n') && self.in_block_insert() {
            ctx.host.bell();
            return;
        }
        // the replica text follows what ACTUALLY landed in the buffer
        // (rejected text must not reach the replication on exit)
        if let Some(block) = &mut self.block_insert {
            block.text.push_str(text);
        }
        self.begin_edit();
        let line_start = ctx
            .buf
            .line_start(ctx.buf.offset_to_line(self.cursor.offset));
        let indent_chars = ctx
            .buf
            .slice(line_start..line_start + self.current_line_indent(ctx));
        let at = self.cursor.offset;
        if self.insert_change_pos.is_none() {
            self.insert_change_pos = Some(at);
        }
        if let Some(rep) = &mut self.insert_repeat {
            rep.text.push_str(text);
        }
        let expanded = if self.options.autoindent && !indent_chars.is_empty() {
            text.replace('\n', &format!("\n{indent_chars}"))
        } else {
            text.to_owned()
        };
        if self.mode == Mode::Replace {
            // overwrite up to the text length, then insert the remainder.
            // Each overwritten char is stashed so Backspace can restore it
            // (vim's Replace-mode BS); chars appended past the line end
            // record `None` — backspacing over them plain-deletes.
            let mut end = at;
            let line_end = ctx.buf.line_end(ctx.buf.offset_to_line(at));
            let mut overwritten = 0usize;
            for _ in 0..expanded.chars().count() {
                match ctx.buf.next_char_offset(end) {
                    Some(next) if next <= line_end => {
                        self.replace_overwritten.push(ctx.buf.char_at(end));
                        end = next;
                        overwritten += 1;
                    }
                    _ => break,
                }
            }
            let appended = expanded.chars().count().saturating_sub(overwritten);
            self.replace_overwritten
                .extend(std::iter::repeat_n(None, appended));
            self.edit_replace(ctx, at..end.min(line_end.max(at)), &expanded);
        } else {
            self.edit_insert(ctx, at, &expanded);
        }
        self.cursor.offset = at + expanded.len();
        self.block_note_typed_end(self.cursor.offset);
        self.republish_search(ctx);
        ctx.host.changed();
    }

    /// Whether each edit re-runs the hlsearch scan (default true). Huge-file
    /// hosts set false and call [`VimState::refresh_highlights`] on their own
    /// schedule (debounce/interval), keeping per-keystroke cost O(edit).
    pub fn set_hlsearch_live_update(&mut self, live: bool) {
        self.hlsearch_live_update = live;
    }

    /// Re-run the highlight scan and publish to the host (for hosts that
    /// disabled [`VimState::set_hlsearch_live_update`]).
    pub fn refresh_highlights(&mut self, ctx: &mut Ctx) {
        self.republish_search_inner(ctx);
    }

    /// While set, `:action <unknown-id>` misses are silently ignored
    /// (host chooses whether the bridge reports; shared rc files contain
    /// mappings aimed at other apps).
    pub fn set_lenient_actions(&mut self, lenient: bool) {
        self.lenient_actions = lenient;
    }

    pub fn lenient_actions(&self) -> bool {
        self.lenient_actions
    }

    /// Apply a parsed user config (`~/.vimcorerc` style): options via the
    /// `:set` machinery, mappings into the per-mode mapping tables.
    pub fn apply_config(&mut self, config: &crate::config::Config) -> crate::config::ConfigStats {
        let mut stats = crate::config::ConfigStats::default();
        for setting in &config.settings {
            let ok = match setting {
                crate::config::Setting::On(name) => self.options.set_boolean(name, true),
                crate::config::Setting::Off(name) => self.options.set_boolean(name, false),
                crate::config::Setting::Toggle(name) => match self.options.bool_option(name) {
                    Some(current) => self.options.set_boolean(name, !current),
                    None => false,
                },
                crate::config::Setting::Value(name, value) => self.options.set_value(name, value),
            };
            if ok {
                stats.options += 1;
            } else {
                stats.ignored += 1;
            }
        }
        for mapping in &config.mappings {
            self.keymaps.map(
                mapping.class,
                &mapping.lhs,
                mapping.rhs.clone(),
                mapping.noremap,
            );
            stats.mappings += 1;
        }
        stats.ignored += config.ignored.len();
        stats
    }

    /// Host-side entry for typed/composed text (the IME placement path).
    /// Records the text as ONE Text step for `.`/macro replay — the key
    /// pipeline declines these chars, so recording them as keys too would
    /// double every typed character on replay.
    ///
    /// NOTE: the block-insert replica text is NOT appended here — the text
    /// may still be REJECTED by [`VimState::insert_text_at_cursor`] (a `\n`
    /// mid-block-session), and the replica must only carry text that actually
    /// landed in the buffer. `insert_text_at_cursor` appends on success.
    pub fn record_typed_text(&mut self, text: &str) {
        if text.is_empty() || self.replaying || self.recording_suppressed {
            return;
        }
        if self.insert_session.is_some() {
            match self.recording.last_mut() {
                Some(RecordedStep::Text(existing)) => existing.push_str(text),
                _ => self.recording.push(RecordedStep::Text(text.to_owned())),
            }
            if let Some((_, keys)) = &mut self.macro_capture {
                keys.push(RecordedStep::Text(text.to_owned()));
            }
        }
    }

    fn current_line_indent(&self, ctx: &Ctx) -> usize {
        let line = ctx.buf.offset_to_line(self.cursor.offset);
        let (indent, _) = ctx.buf.line_indent(line);
        indent
    }

    /// Replace an arbitrary range (IME committed composition text).
    pub fn replace_range(&mut self, ctx: &mut Ctx, range: Range<usize>, text: &str) {
        self.begin_edit();
        self.edit_replace(ctx, range.clone(), text);
        // place the cursor at the end of the replacement when it touches it
        if range.contains(&self.cursor.offset) || self.cursor.offset == range.end {
            self.cursor.offset = range.start + text.len();
        } else if self.cursor.offset > range.end {
            self.cursor.offset += text.len().saturating_sub(range.len());
        }
        // committed text is a real edit: keep the published highlights and
        // the match cache in step with the buffer, like every other path
        self.republish_search(ctx);
        ctx.host.changed();
    }

    /// Host-initiated cursor move (e.g. a mouse click). `offset` may be any
    /// byte position — a host translating a click can easily land inside a
    /// multi-byte char — so it is floored to a char boundary first; handing
    /// a mid-char offset onward would panic in `offset_to_line`.
    ///
    /// IGNORED while a visual-block insert session is open: a click mid-
    /// session would move the typing point to another row, and the replica
    /// offsets assume every keystroke lands on the session's typing row
    /// (vertical moves are locked out for the same reason — fuzz round 8
    /// caught the replication then writing past the buffer end).
    pub fn set_cursor_offset(&mut self, buf: &dyn VimBuffer, offset: usize) {
        if self.in_block_insert() {
            return;
        }
        let offset = crate::buffer::floor_to_char_boundary(buf, offset);
        let offset = clamp_cursor(buf, offset);
        self.cursor.offset = offset;
        self.cursor.desired_col = None;
        // In visual mode a click moves the CURSOR only: vim's selection
        // follows while the anchor stays put (`:h visual-use`, mouse drag
        // aside — hosts use set_visual_range for that). Overwriting the
        // anchor collapsed the selection to zero width, so a later `d`
        // deleted a single char instead of the dragged-out range.
        if matches!(self.mode, Mode::Visual { .. }) {
            if let Some(anchor) = self.visual_anchor {
                let (lo, hi) = (anchor.min(offset), anchor.max(offset));
                let end = crate::buffer::next_grapheme_offset(buf, hi).unwrap_or(hi);
                self.marks.active_visual = Some((lo, end));
            }
        }
    }

    /// Host-initiated visual selection (e.g. a mouse drag). Both ends are
    /// floored to char boundaries, like [`VimState::set_cursor_offset`].
    /// Ignored mid-block-insert-session, same as clicks.
    pub fn set_visual_range(&mut self, buf: &dyn VimBuffer, anchor: usize, cursor: usize) {
        if self.in_block_insert() {
            return;
        }
        let anchor = crate::buffer::floor_to_char_boundary(buf, anchor);
        let cursor = crate::buffer::floor_to_char_boundary(buf, cursor);
        self.visual_anchor = Some(clamp_cursor(buf, anchor));
        self.cursor.offset = clamp_cursor(buf, cursor);
        if !matches!(self.mode, Mode::Visual { .. }) {
            self.mode = Mode::Visual {
                kind: VisualKind::Char,
            };
        }
    }

    // ---- visual helpers ---------------------------------------------------------

    pub(crate) fn enter_visual(&mut self, kind: VisualKind) {
        self.visual_anchor = Some(self.cursor.offset);
        self.mode = Mode::Visual { kind };
        self.marks.active_visual = Some((self.cursor.offset, self.cursor.offset));
    }

    pub(crate) fn exit_visual(&mut self, ctx: &mut Ctx) {
        // capture the span BEFORE moving the cursor: the cursor lands on the
        // selection start, and re-reading the selection after that collapse
        // forward selections (cursor right of anchor) to a single char,
        // breaking `gv`
        if let Some((lo, hi, kind)) = self.clamped_visual_bounds(ctx.buf) {
            let end = ctx.buf.next_char_offset(hi).unwrap_or(hi);
            self.marks.last_visual = Some((lo, end, kind));
            self.cursor.offset = lo;
        }
        self.visual_anchor = None;
        self.marks.active_visual = None;
        self.mode = Mode::Normal;
        self.discard_change_record();
        ctx.host.changed();
    }

    /// Restore the cursor to a visual range start (used after visual ops).
    /// Row-wise application of an operator over a visual block. Only
    /// Delete / Yank / Change are supported in v1 (other operators bell).
    fn apply_block_operator(&mut self, ctx: &mut Ctx, op: Operator) {
        let Some(block) = ops::span_from_visual_block(self, ctx.buf) else {
            ctx.host.bell();
            return;
        };
        match op {
            Operator::Delete => {
                self.begin_edit();
                let adjusted = self.delete_block_rows(ctx, &block.rows);
                self.end_edit();
                self.bump(ctx);
                self.reset_pending();
                self.cursor.offset = clamp_cursor(ctx.buf, adjusted.first().copied().unwrap_or(0));
                self.cursor.desired_col = None;
                self.finish_visual_op(ctx);
            }
            Operator::Yank => {
                let width = block.col_hi.saturating_sub(block.col_lo);
                let mut texts = Vec::new();
                for range in &block.rows {
                    let mut text = ctx.buf.slice(range.clone());
                    // pad rows to the block width so blockwise put keeps
                    // its rectangle
                    let mut w: usize = text.chars().map(crate::buffer::char_display_width).sum();
                    while w < width {
                        text.push(' ');
                        w += 1;
                    }
                    texts.push(text);
                }
                self.registers.store_yank(
                    self.register,
                    texts.join("\n"),
                    crate::registers::RegisterKind::Blockwise,
                );
                self.cursor.offset =
                    clamp_cursor(ctx.buf, block.rows.first().map(|r| r.start).unwrap_or(0));
                self.cursor.desired_col = None;
                self.finish_visual_op(ctx);
            }
            Operator::Change => {
                self.begin_edit();
                // block `c` continues into insert: stash the block bounds for
                // `gv` (see `pending_visual_marks`)
                self.pending_visual_marks = Some((
                    block.rows.first().map(|r| r.start).unwrap_or(0),
                    block.rows.last().map(|r| r.end).unwrap_or(0),
                    crate::mode::VisualKind::Block,
                ));
                let adjusted = self.delete_block_rows(ctx, &block.rows);
                self.bump(ctx);
                self.reset_pending();
                self.cursor.offset = clamp_cursor(ctx.buf, adjusted[0]);
                let typing_line = ctx.buf.offset_to_line(self.cursor.offset);
                self.block_insert = Some(BlockInsert {
                    rows: adjusted[1..]
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(i, offset)| (offset, block.first_line + 1 + i))
                        .collect(),
                    typing_line,
                    typing_line_len: ctx.buf.line_end(typing_line)
                        - ctx.buf.line_start(typing_line),
                    text: String::new(),
                    typed_end: None,
                });
                self.begin_insert(InsertKind::Insert);
                // blockwise `c` needs per-row text replication that a replay of
                // the recorded keys cannot reproduce (the typed text is applied
                // as one inline step) — keep it out of `.`
                self.recording_blocked = true;
            }
            // blockwise case flip: every row's covered span maps per char
            // (vim 9.1: `<C-v>jllU` uppercases the block, cursor parks at the
            // block's start). Bottom-up so earlier rows survive the byte-
            // length changes multi-char case mappings (ß→SS) make below.
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                let gen = self.edit_generation;
                self.begin_edit();
                for range in block.rows.iter().rev() {
                    if range.is_empty() {
                        continue;
                    }
                    let text = ctx.buf.slice(range.clone());
                    let mapped: String = text
                        .chars()
                        .map(|c| match op {
                            Operator::Lowercase => c.to_lowercase().collect::<String>(),
                            Operator::Uppercase => c.to_uppercase().collect::<String>(),
                            _ => crate::ops::toggle_case(c),
                        })
                        .collect();
                    self.edit_replace(ctx, range.clone(), &mapped);
                }
                self.end_edit();
                self.cursor.offset =
                    clamp_cursor(ctx.buf, block.rows.first().map(|r| r.start).unwrap_or(0));
                self.cursor.desired_col = None;
                self.bump_if_edited(ctx, gen);
                self.finish_visual_op(ctx);
            }
            _ => ctx.host.bell(),
        }
    }

    /// Delete every block row bottom-up; returns each row's insertion
    /// offset ADJUSTED for the deletions below it (valid after all rows
    /// are gone). Empty rows contribute no shift.
    fn delete_block_rows(&mut self, ctx: &mut Ctx, rows: &[std::ops::Range<usize>]) -> Vec<usize> {
        // a row's final offset shifts by the deletions ABOVE it: prefix-sum
        // top-down first, then delete bottom-up (keeps later offsets valid)
        let mut adjusted = vec![0usize; rows.len()];
        let mut shift = 0usize;
        for i in 0..rows.len() {
            adjusted[i] = rows[i].start.saturating_sub(shift);
            if !rows[i].is_empty() {
                shift += rows[i].len();
            }
        }
        for range in rows.iter().rev() {
            if !range.is_empty() {
                self.edit_delete(ctx, range.clone());
            }
        }
        adjusted
    }

    /// Visual-block `I` (insert at the left edge) and `A` (append at the
    /// right edge): typing lands on the cursor row, the rest replicate on
    /// exit. `I` on rows shorter than the block inserts at their line end;
    /// `A` pads short rows with spaces up to the block's right edge first,
    /// exactly like vim (vim 9.1: block cols 3-5, row `ab` → `ab   X`).
    fn begin_block_insert(&mut self, ctx: &mut Ctx, append: bool) {
        let Some(block) = ops::span_from_visual_block(self, ctx.buf) else {
            ctx.host.bell();
            return;
        };
        // pads for short rows under `A`, collected first and applied
        // BOTTOM-UP so the row offsets computed below stay valid. vim pads
        // any row that ends before the APPEND column (col_hi is the EXCLUSIVE
        // end = the append column itself; 9.1 probe: block cols 0-2 on rows
        // "123456"/"12" gives "123 X456"/"12  X" — the short row pads to the
        // append column, not just to col_lo). The old `range.is_empty()`
        // filter left rows ending between col_lo and col_hi unpadded, and
        // `A` appended at the row's own end instead.
        let pads: Vec<(usize, usize)> = if append {
            block
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, _)| {
                    let line = block.first_line + i;
                    let width = crate::buffer::display_column(ctx.buf, ctx.buf.line_end(line));
                    // `.then` (lazy): the subtraction must not run when the
                    // row is LONGER than the block — `then_some` would
                    // evaluate `col_hi - width` first and overflow
                    (width < block.col_hi).then(|| (line, block.col_hi - width))
                })
                .collect()
        } else {
            Vec::new()
        };
        if !pads.is_empty() {
            self.begin_edit();
            // pads move text after them; the cursor is caller-managed (edit
            // funnels never touch it), so a pad on a row ABOVE the cursor's
            // shifts the cursor's byte offset — without the adjustment it
            // landed mid-character in multi-byte text (fuzz round 13) and
            // the row recomputation below read a bogus cursor line
            let cursor_before = self.cursor.offset;
            let mut cursor_shift = 0usize;
            for (line, pad) in pads.iter().rev() {
                let at = ctx.buf.line_end(*line);
                if at <= cursor_before {
                    cursor_shift += pad;
                }
                self.edit_insert(ctx, at, &" ".repeat(*pad));
            }
            self.cursor.offset += cursor_shift;
            // the pads moved text after the match cache's offsets — every
            // other edit path republishes the hlsearch scan; skipping it
            // here left `last_matches` pointing mid-character until the
            // session exits (fuzz round 11)
            self.republish_search(ctx);
        }
        let cursor_line = ctx.buf.offset_to_line(self.cursor.offset);
        // the pads above shifted every row below them: recompute each row's
        // byte range against the CURRENT buffer instead of trusting the
        // pre-pad spans in block.rows
        let last_row_line = block.first_line + block.rows.len() - 1;
        let ranges: Vec<std::ops::Range<usize>> = (block.first_line..=last_row_line)
            .map(|line| ops::block_row_range(ctx.buf, line, block.col_lo, block.col_hi))
            .collect();
        let mut rows = Vec::new();
        let mut typing_offset = None;
        for (i, range) in ranges.iter().enumerate() {
            let line = block.first_line + i;
            let offset = if append {
                if range.is_empty() {
                    ctx.buf.line_end(line)
                } else {
                    range.end
                }
            } else if range.is_empty() {
                ctx.buf.line_end(line)
            } else {
                range.start
            };
            if line == cursor_line {
                typing_offset = Some(offset);
            } else {
                rows.push((offset, line));
            }
        }
        let Some(typing_offset) = typing_offset else {
            ctx.host.bell();
            return;
        };
        // block `I`/`A` continue into insert: stash the block bounds for `gv`
        // (see `pending_visual_marks`)
        self.pending_visual_marks = Some((
            block.rows.first().map(|r| r.start).unwrap_or(0),
            block.rows.last().map(|r| r.end).unwrap_or(0),
            crate::mode::VisualKind::Block,
        ));
        self.cursor.offset = typing_offset;
        self.cursor.desired_col = None;
        self.block_insert = Some(BlockInsert {
            rows,
            typing_line: cursor_line,
            typing_line_len: ctx.buf.line_end(cursor_line) - ctx.buf.line_start(cursor_line),
            text: String::new(),
            typed_end: None,
        });
        self.begin_insert(InsertKind::Insert);
        // visual-block I/A: like block `c`, the row replication is not
        // reproducible from a replayed text step — keep it out of `.`
        self.recording_blocked = true;
    }

    /// The `(lo, hi, kind)` bounds of the live selection, lo/hi CLAMPED onto
    /// the current text. An operator may have just deleted the selection's
    /// bytes: the raw anchor/cursor then point past the new end (or mid-char),
    /// and storing them un-clamped left `gv`/`'>` reading out-of-bounds
    /// offsets.
    fn clamped_visual_bounds(&self, buf: &dyn VimBuffer) -> Option<(usize, usize, VisualKind)> {
        let (anchor, cursor, kind) = self.visual_selection()?;
        let (lo, hi) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        let lo = crate::buffer::floor_to_char_boundary(buf, lo);
        let hi = crate::buffer::floor_to_char_boundary(buf, hi);
        Some((lo.min(hi), hi.max(lo), kind))
    }

    pub(crate) fn finish_visual_op(&mut self, ctx: &mut Ctx) {
        if let Some((lo, hi, kind)) = self.clamped_visual_bounds(ctx.buf) {
            let end = ctx.buf.next_char_offset(hi).unwrap_or(hi);
            self.marks.last_visual = Some((lo, end, kind));
        }
        self.visual_anchor = None;
        // the selection is resolved — the live range must go, or `parse_range`
        // keeps preferring it over the just-written `'<`/`'>` marks while the
        // offsets are already stale (pre-edit bytes deleted out from under
        // them, probe: `Vjd` then `:'<,'>d` emptied the whole buffer)
        self.marks.active_visual = None;
        if !matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.mode = Mode::Normal;
        }
        // the visual keys (`v`, the motions, the operator) were recorded by
        // the pipeline; committing them makes visual changes `.`-repeatable —
        // replay re-enters visual mode and rebuilds the selection at the
        // cursor. Committed AFTER the mode drop: `commit_change_record` keeps
        // accumulating while the mode is still Visual.
        self.commit_change_record();
        self.cursor.offset = clamp_cursor(ctx.buf, self.cursor.offset);
        ctx.host.changed();
    }
}

fn op_keys(op: Operator) -> &'static str {
    match op {
        Operator::Delete => "d",
        Operator::Change => "c",
        Operator::Yank => "y",
        Operator::IndentLeft => "<",
        Operator::IndentRight => ">",
        Operator::Lowercase => "gu",
        Operator::Uppercase => "gU",
        Operator::ToggleCase => "g~",
        Operator::Format => "gq",
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ProcessOutcome {
    Consumed,
    Unknown,
    Feed(Vec<Key>),
}

/// What the mapping layer decided for the key at the front of the pending
/// queue (see [`VimState::mapping_step`]).
#[derive(Debug, PartialEq, Eq)]
enum MappingStep {
    /// A mapping matched: the front keys were replaced by its expansion —
    /// run the pipeline again on the expansion.
    Expanded,
    /// The pipeline is finished for this keystroke: either the
    /// builtin-vs-mapping ambiguity resolved to the builtin (which was
    /// executed), or a runaway mapping expansion was aborted.
    Done,
    /// Input may still grow into a mapping (or is declined in insert
    /// mode): keep the queue and wait for more keys.
    Wait,
    /// No mapping applies — pop the front key and run it through the mode
    /// handlers.
    FallThrough,
}

impl VimState {
    // ---- normal mode -----------------------------------------------------

    // normal_key and visual_key share four pipeline stages (char-argument
    // completion, the `"{reg}` prefix, count digits and the final trie walk)
    // but DELIBERATELY differ elsewhere:
    // * Esc sits before the trie walk in visual (it must always abort the
    //   selection) but after it in normal (a partial prefix like `g` must
    //   first get its chance to miss);
    // * a missed multi-key sequence retries without its first key in normal,
    //   while visual just clears (the retry would re-run visual commands);
    // * operator doubling and the gq/gw spelling bookkeeping only exist in
    //   normal mode.

    /// Complete a pending `"{reg}` prefix: the next printable key names the
    /// register for the following command; anything else cancels it with a
    /// bell (vim keeps the pending count, so only the register is dropped).
    fn register_pending_key(&mut self, ctx: &mut Ctx, key: &Key) -> ProcessOutcome {
        self.register_pending = false;
        if let Some(c) = key.printable_char() {
            // `""` is vim's explicit unnamed register and behaves EXACTLY
            // like no prefix — in particular `""dd` still rotates the
            // numbered ring (probe 9.1: dd, ""dd, "1p pastes the SECOND
            // deleted line). Storing Some('"') would route the delete into
            // unnamed only and freeze the ring.
            if c == '"' {
                self.register = None;
            } else {
                self.register = Some(c);
            }
        } else {
            self.register = None;
            ctx.host.bell();
        }
        ProcessOutcome::Consumed
    }

    /// Absorb a count digit (`3` → count 3, `30` → count 30). A leading `0`
    /// is not a count — it falls through to the trie as the line-start
    /// motion. Returns `None` when the key is not a count digit. Saturating
    /// arithmetic: `99999999999999999dd` must not panic on usize overflow.
    fn count_digit_key(&mut self, key: &Key) -> Option<ProcessOutcome> {
        if let KeyKind::Char(c) = &key.kind {
            if key.modifiers.is_plain() && c.is_ascii_digit() {
                let d = c.to_digit(10).unwrap() as usize;
                if !(d == 0 && self.count.is_none()) {
                    self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
                    return Some(ProcessOutcome::Consumed);
                }
                // 0 falls through to the trie (line-start motion)
            }
        }
        None
    }

    fn normal_key(&mut self, ctx: &mut Ctx, key: Key) -> ProcessOutcome {
        // 0. escape clears pending state FIRST — before the `"{reg}` prefix
        //    or a partial trie walk can swallow it: `3"<Esc>` must cancel
        //    the count (not leave it armed for the next command), and
        //    `g<Esc>` must cancel QUIETLY (routing it through the trie-miss
        //    retry rang the bell; vim cancels silently)
        if key == Key::escape() || key == Key::ctrl_char('[') {
            self.reset_pending();
            self.discard_change_record();
            if !self.search.last_matches.is_empty() {
                crate::search::clear_highlights(self, ctx);
            }
            return ProcessOutcome::Consumed;
        }

        // 1. complete a pending char-argument
        if self.char_arg_cmd.is_some() {
            return self.complete_char_arg(ctx, key);
        }
        if self.register_pending {
            return self.register_pending_key(ctx, &key);
        }

        // 2. operator doubling: dd / yy / >> / guu / g~~ / gqq / gww ...
        // (gq/gw need the remembered spelling: their trigger letters are
        // not derivable from the operator alone)
        if self.op.is_some() && self.cmd_seq.is_empty() {
            let trigger = if self.op == Some(Operator::Format) {
                self.format_trigger
            } else {
                Self::operator_trigger(self.op.unwrap())
            };
            if let (Some(trigger), KeyKind::Char(c), true) =
                (trigger, &key.kind, key.modifiers.is_plain())
            {
                if trigger == *c {
                    let count = self.take_total_count();
                    let line = ctx.buf.offset_to_line(self.cursor.offset);
                    let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                    let span = ops::OpSpan {
                        start: ctx.buf.line_start(line),
                        end: ctx.buf.line_range(last).end,
                        linewise: true,
                    };
                    self.complete_operator_with_span(ctx, span);
                    return ProcessOutcome::Consumed;
                }
            }
        }

        // 3. a pending trie walk
        if !self.cmd_seq.is_empty() {
            let phase = if self.op.is_some() {
                Phase::Pending
            } else {
                Phase::Normal
            };
            let mut seq = self.cmd_seq.clone();
            seq.push(key.clone());
            match self.tables.trie(phase).get(&seq) {
                keymap::Walk::Hit(kind) => {
                    // remember the gq/gw spelling BEFORE cmd_seq is cleared
                    // (execute_command reads it for the linewise doubling)
                    if *kind == CmdKind::Operator(Operator::Format) {
                        // the spelling letter is the LAST key of [g, q|w]
                        self.format_trigger = match seq.last().map(|k| &k.kind) {
                            Some(KeyKind::Char('w')) => Some('w'),
                            _ => Some('q'),
                        };
                    }
                    self.cmd_seq.clear();
                    return self.execute_command(ctx, *kind);
                }
                keymap::Walk::Pending => {
                    self.cmd_seq = seq;
                    return ProcessOutcome::Consumed;
                }
                keymap::Walk::Miss => {
                    // execute the longest terminal prefix, re-feed the rest
                    if let Some((len, kind)) = self.tables.longest_terminal(phase, &seq) {
                        self.cmd_seq.clear();
                        let rest = seq[len..].to_vec();
                        let outcome = self.execute_command(ctx, kind);
                        if rest.is_empty() {
                            return outcome;
                        }
                        return ProcessOutcome::Feed(rest);
                    }
                    // nothing matched: drop the first key, retry the rest
                    self.cmd_seq.remove(0);
                    let mut rest = self.cmd_seq.clone();
                    self.cmd_seq.clear();
                    rest.push(key);
                    // `gugu` & co. reach this retry (the pending-phase trie
                    // has no operator rows): the re-fed trigger completes
                    // the operator doubling, which must stay quiet — vim
                    // finishes `gugu` without a bell. Any other remainder
                    // still rings (`dgx` = no such motion).
                    if !self.completes_operator_doubling(&rest) {
                        ctx.host.bell();
                    }
                    if rest.is_empty() {
                        return ProcessOutcome::Consumed;
                    }
                    return ProcessOutcome::Feed(rest);
                }
            }
        }

        // 4. count digits
        if let Some(outcome) = self.count_digit_key(&key) {
            return outcome;
        }

        // 5. register prefix
        if key.kind == KeyKind::Char('"') && key.modifiers.is_plain() {
            self.register_pending = true;
            return ProcessOutcome::Consumed;
        }

        // 6. search prompts & the Ex command line
        if key.modifiers.is_plain() {
            match &key.kind {
                KeyKind::Char('/') => {
                    self.begin_cmdline('/');
                    return ProcessOutcome::Consumed;
                }
                KeyKind::Char('?') => {
                    self.begin_cmdline('?');
                    return ProcessOutcome::Consumed;
                }
                KeyKind::Char(':') => {
                    self.begin_cmdline(':');
                    return ProcessOutcome::Consumed;
                }
                _ => {}
            }
        }

        // 7. arrow / navigation keys
        if let Some(outcome) = self.navigation_key(ctx, &key) {
            return outcome;
        }

        // 8. the command trie
        let phase = if self.op.is_some() {
            Phase::Pending
        } else {
            Phase::Normal
        };
        let single = [key.clone()];
        match self.tables.trie(phase).get(&single) {
            keymap::Walk::Hit(kind) => self.execute_command(ctx, *kind),
            keymap::Walk::Pending => {
                self.cmd_seq.push(key);
                ProcessOutcome::Consumed
            }
            keymap::Walk::Miss => {
                if key.modifiers.control || key.modifiers.alt {
                    return ProcessOutcome::Unknown;
                }
                self.reset_pending();
                ctx.host.bell();
                ProcessOutcome::Consumed
            }
        }
    }

    /// Arrow keys etc. behave like their vim equivalents in normal/visual.
    fn navigation_key(&mut self, ctx: &mut Ctx, key: &Key) -> Option<ProcessOutcome> {
        if !key.modifiers.is_plain() {
            return None;
        }
        let name = match &key.kind {
            KeyKind::Named(name) => name.as_str(),
            _ => return None,
        };
        let motion = match name {
            "left" => Motion::Left,
            "right" => Motion::Right,
            "up" => Motion::Up,
            "down" => Motion::Down,
            "home" => Motion::LineStart,
            "end" => Motion::LineEnd,
            "pageup" => Motion::PageUp,
            "pagedown" => Motion::PageDown,
            // visual <Del> is vim's `d` (delete the whole selection, probe:
            // `viw<Del>` removes the word) — it must fall through to the
            // visual command table's Operator(Delete) row, not delete one
            // char at the cursor while the selection stays alive
            "delete" if matches!(self.mode, Mode::Visual { .. }) => return None,
            "delete" => {
                let count = self.count.take().unwrap_or(1);
                let register = self.register.take();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::delete_chars(self, ctx, count, false, register);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
                return Some(ProcessOutcome::Consumed);
            }
            _ => return None,
        };
        let count = self.count.take().unwrap_or(1);
        if !self.goto_motion(ctx, motion, count) {
            ctx.host.bell();
        }
        Some(ProcessOutcome::Consumed)
    }

    // ---- visual mode ---------------------------------------------------------

    fn visual_key(&mut self, ctx: &mut Ctx, key: Key) -> ProcessOutcome {
        // Esc aborts the selection unconditionally — BEFORE the `"{reg}`
        // prefix or a partial trie walk can swallow it (`3"<Esc>` must
        // cancel count+register AND the selection; with register_pending
        // checked first the Esc was eaten as a register name, the count
        // survived and silently scaled the NEXT command — vim cancels all).
        // <C-c> is vim's cancel synonym.
        if key == Key::escape() || key == Key::ctrl_char('[') || key == Key::ctrl_char('c') {
            self.reset_pending();
            self.exit_visual(ctx);
            return ProcessOutcome::Consumed;
        }
        if self.char_arg_cmd.is_some() {
            return self.complete_char_arg(ctx, key);
        }
        if self.register_pending {
            return self.register_pending_key(ctx, &key);
        }

        if !self.cmd_seq.is_empty() {
            let mut seq = self.cmd_seq.clone();
            seq.push(key.clone());
            match self.tables.trie(Phase::Visual).get(&seq) {
                keymap::Walk::Hit(kind) => {
                    self.cmd_seq.clear();
                    return self.execute_command(ctx, *kind);
                }
                keymap::Walk::Pending => {
                    self.cmd_seq = seq;
                    return ProcessOutcome::Consumed;
                }
                keymap::Walk::Miss => {
                    if let Some((len, kind)) = self.tables.longest_terminal(Phase::Visual, &seq) {
                        self.cmd_seq.clear();
                        let rest = seq[len..].to_vec();
                        let outcome = self.execute_command(ctx, kind);
                        return if rest.is_empty() {
                            outcome
                        } else {
                            ProcessOutcome::Feed(rest)
                        };
                    }
                    self.cmd_seq.clear();
                    // a stale count must not survive an unmapped key: vim
                    // cancels it, and a surviving count would silently scale
                    // the NEXT motion (`3<C-unknown>` then `j` jumps 3 lines)
                    self.reset_pending();
                    ctx.host.bell();
                    return ProcessOutcome::Consumed;
                }
            }
        }

        if let Some(outcome) = self.count_digit_key(&key) {
            return outcome;
        }
        if key.kind == KeyKind::Char('"') && key.modifiers.is_plain() {
            self.register_pending = true;
            return ProcessOutcome::Consumed;
        }
        // `:` in visual mode seeds the cmdline with '<,'>; Esc at the
        // prompt returns to the selection, executing it drops to normal
        if key.modifiers.is_plain() && key.kind == KeyKind::Char(':') {
            let kind = match self.mode {
                Mode::Visual { kind } => kind,
                _ => crate::mode::VisualKind::Char,
            };
            self.cmdline_visual = Some((
                kind,
                self.visual_anchor.unwrap_or(self.cursor.offset),
                self.cursor.offset,
            ));
            self.begin_cmdline(':');
            self.cmdline.buffer.push_str("'<,'>");
            return ProcessOutcome::Consumed;
        }
        // visual-block I/A: insert at the block edge on every row
        if matches!(
            self.mode,
            Mode::Visual {
                kind: crate::mode::VisualKind::Block
            }
        ) {
            if let (KeyKind::Char(c), true) = (&key.kind, key.modifiers.is_plain()) {
                if *c == 'I' || *c == 'A' {
                    self.begin_block_insert(ctx, *c == 'A');
                    return ProcessOutcome::Consumed;
                }
            }
        }

        if let Some(outcome) = self.navigation_key(ctx, &key) {
            return outcome;
        }

        let single = [key.clone()];
        match self.tables.trie(Phase::Visual).get(&single) {
            keymap::Walk::Hit(kind) => self.execute_command(ctx, *kind),
            keymap::Walk::Pending => {
                self.cmd_seq.push(key);
                ProcessOutcome::Consumed
            }
            keymap::Walk::Miss => {
                if key.modifiers.control || key.modifiers.alt {
                    return ProcessOutcome::Unknown;
                }
                // a stale count must not survive an unmapped key: vim cancels
                // it, and a surviving count would silently scale the NEXT
                // motion (`3<C-unknown>` then `j` jumps 3 lines)
                self.reset_pending();
                ctx.host.bell();
                ProcessOutcome::Consumed
            }
        }
    }

    // ---- operator completion & command execution -----------------------------

    /// Execute a resolved command. `count`/`register` come from pending state.
    pub(crate) fn execute_command(&mut self, ctx: &mut Ctx, kind: CmdKind) -> ProcessOutcome {
        // `q` while recording stops immediately (vim semantics) — it must not
        // wait for a char argument, or the NEXT key would be eaten as the
        // "stop key" and the real trailing `q` would stay in the macro
        if kind == CmdKind::Normal(NormalCmd::RecordMacro) && self.macro_capture.is_some() {
            if let Some((reg, mut keys)) = self.macro_capture.take() {
                keys.pop(); // drop the stopping `q` (captured by the hook)
                self.macros.insert(reg, keys);
                // vim: the recording register counts as "used", so `@@`
                // replays it right after recording
                self.last_macro_played = Some(reg);
            }
            // `q` takes no count (vim drops it): a count typed before the
            // stop (`2q`) must not silently scale the NEXT command
            self.count = None;
            return ProcessOutcome::Consumed;
        }
        // char-argument commands wait for their argument first
        if kind.takes_char() {
            self.char_arg_cmd = Some(match kind {
                CmdKind::Motion(Motion::FindChar { forward, till }) => {
                    CharArgCmd::Find { forward, till }
                }
                CmdKind::Motion(Motion::MarkJump { linewise })
                | CmdKind::Normal(NormalCmd::JumpMark { linewise }) => {
                    CharArgCmd::JumpMark { linewise }
                }
                CmdKind::Normal(NormalCmd::ReplaceChar) => CharArgCmd::Replace,
                CmdKind::Normal(NormalCmd::MarkSet) => CharArgCmd::MarkSet,
                CmdKind::Normal(NormalCmd::RecordMacro) => CharArgCmd::MacroRecord,
                CmdKind::Normal(NormalCmd::PlayMacro) => CharArgCmd::MacroPlay,
                CmdKind::Visual(VisualCmd::ReplaceChar) => CharArgCmd::VisualReplace,
                _ => unreachable!("takes_char out of sync"),
            });
            return ProcessOutcome::Consumed;
        }
        match kind {
            CmdKind::Motion(mut motion) => {
                // `1G` lands on line 1 while a bare `G` lands on the last
                // line — but both reach `Motion::target` as count==1 (absent
                // counts are defaulted there). Rewrite the explicit-`1` case
                // to `gg` before the count collapses.
                if motion == (Motion::GoToLine { first: false })
                    && self.count == Some(1)
                    && self.op_count.is_none()
                {
                    motion = Motion::GoToLine { first: true };
                }
                let count = self.take_total_count();
                // Remember a `cw`-family command: its span gets special
                // post-processing below. (The old approach rewrote the
                // motion to `ce`, which swallowed the next line's word on
                // every crossing case — see the comment at the trim.)
                let is_cw = self.op == Some(Operator::Change)
                    && matches!(motion, Motion::WordStart { .. })
                    && ctx
                        .buf
                        .char_at(self.cursor.offset)
                        .is_some_and(|c| !c.is_whitespace());
                if self.op.is_some() {
                    let result = motion.target(self, ctx, count);
                    if !result.moved {
                        if matches!(motion, Motion::SelectMatch { .. }) {
                            self.report_search_miss(ctx);
                        } else {
                            ctx.host.bell();
                        }
                        self.reset_pending();
                        return ProcessOutcome::Consumed;
                    }
                    // gn as an operator target covers exactly the match
                    // (cursor..target would drag in the gap before it)
                    let mut span = match motion {
                        Motion::SelectMatch { .. } => match self.search.last_found_match.clone() {
                            Some(range) => ops::OpSpan {
                                start: range.start,
                                end: range.end,
                                linewise: false,
                            },
                            None => {
                                self.report_search_miss(ctx);
                                self.reset_pending();
                                return ProcessOutcome::Consumed;
                            }
                        },
                        _ => ops::span_from_motion(self, ctx.buf, motion, result),
                    };
                    // `cw`/`cW` = the `dw` span with the trailing whitespace
                    // excluded (all probed against vim 9.1): `cw` on "ab cd"
                    // changes "ab"; `c2w` on "a|b c|d" from 'a' changes
                    // "a\nb", not "a\nb c"; `cw` on a single-char word
                    // changes that word only; the blank-line promotion stays
                    // linewise. The old WordEnd rewrite ignored the crossing
                    // rules entirely — `cw` on "b" wiped "b\nc".
                    if is_cw && !span.linewise && span.end > span.start {
                        let covered = ctx.buf.slice(span.start..span.end);
                        let trimmed = covered.trim_end_matches([' ', '\t']);
                        span.end = span.start + trimmed.len();
                    }
                    self.complete_operator_with_span(ctx, span);
                } else {
                    if !self.goto_motion(ctx, motion, count) {
                        // `n`/`N`/`*` misses get vim's message channel, not
                        // just a bare bell (E35 before any search, E486 with
                        // the pattern otherwise)
                        if matches!(
                            motion,
                            Motion::SearchNext { .. } | Motion::StarSearch { .. }
                        ) {
                            self.report_search_miss(ctx);
                        } else {
                            ctx.host.bell();
                        }
                    }
                }
                self.end_command();
                ProcessOutcome::Consumed
            }
            CmdKind::Object(object) => {
                match self.mode {
                    Mode::Visual { .. } => {
                        // extend the selection to the object
                        let Some(span) = ops::object_span(self, ctx.buf, object) else {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        };
                        self.visual_anchor = Some(span.start);
                        let end = span.end.min(ctx.buf.len());
                        // an EMPTY object range (vi( on `()`) collapses to a
                        // zero-width selection at its start
                        self.cursor.offset = if end > span.start {
                            ctx.buf.prev_char_offset(end).unwrap_or(span.start)
                        } else {
                            span.start
                        };
                        self.cursor.desired_col = None;
                        // the live `'<`/`'>` range must follow the extension:
                        // a visual `:` right after `viw`/`vi(` resolves its
                        // range through marks.active_visual (same rule as
                        // motion-extended selections in apply_motion_result)
                        self.marks.active_visual = Some((span.start, span.end));
                    }
                    _ if self.op.is_some() => {
                        let Some(span) = ops::object_span(self, ctx.buf, object) else {
                            ctx.host.bell();
                            self.reset_pending();
                            return ProcessOutcome::Consumed;
                        };
                        self.complete_operator_with_span(ctx, span);
                    }
                    _ => ctx.host.bell(),
                }
                ProcessOutcome::Consumed
            }
            CmdKind::Operator(op) => {
                if matches!(self.mode, Mode::Visual { .. }) {
                    self.apply_visual_operator(ctx, op);
                } else {
                    // the prefix count becomes the OPERATOR count (`2d3w` =
                    // 6 words, vim 9.1): leaving it in `count` would let the
                    // motion's digit run concatenate onto it (2 then 3
                    // reading as 23). take_total_count multiplies the two.
                    self.op_count = self.count.take();
                    self.op = Some(op);
                }
                ProcessOutcome::Consumed
            }
            CmdKind::EnterInsert(insert) => {
                // vim's count-repeat insert: a count typed before i/a/I/A/
                // gI/gi/o/O repeats the typed text that many times on exit
                // (for c/s the count belongs to the motion; Replace and
                // visual-block sessions never repeat)
                let count = self.take_total_count();
                self.start_insert(ctx, insert);
                if count > 1
                    && !matches!(insert, InsertKind::Change | InsertKind::Replace)
                    && self.block_insert.is_none()
                {
                    self.insert_repeat = Some(InsertRepeat {
                        count,
                        linewise: matches!(insert, InsertKind::OpenLine { .. }),
                        text: String::new(),
                    });
                }
                self.end_command();
                ProcessOutcome::Consumed
            }
            CmdKind::EnterVisual(kind) => {
                self.enter_visual(kind);
                self.end_command();
                ProcessOutcome::Consumed
            }
            CmdKind::Normal(cmd) => {
                self.execute_normal_cmd(ctx, cmd);
                self.end_command();
                ProcessOutcome::Consumed
            }
            CmdKind::Visual(cmd) => {
                self.execute_visual_cmd(ctx, cmd);
                self.end_command();
                ProcessOutcome::Consumed
            }
        }
    }

    /// Apply an operator to the current visual selection and leave visual mode
    /// (unless the operator opened insert, e.g. `c`).
    ///
    /// A pending count applies only to the INDENT operators (vim 9.1: `Vj3>`
    /// shifts three shiftwidths; `Vj3d` deletes the selection once — a count
    /// before d/y is meaningless there).
    fn apply_visual_operator(&mut self, ctx: &mut Ctx, op: Operator) {
        if matches!(
            self.mode,
            Mode::Visual {
                kind: crate::mode::VisualKind::Block
            }
        ) {
            self.apply_block_operator(ctx, op);
            return;
        }
        let Some(span) = ops::span_from_visual(self, ctx.buf) else {
            ctx.host.bell();
            return;
        };
        // `c` continues into insert: remember the selection for `gv` (see
        // `pending_visual_marks`) before the operator's edits shift it
        if op == Operator::Change {
            let kind = match self.mode {
                Mode::Visual { kind } => kind,
                _ => crate::mode::VisualKind::Char,
            };
            self.pending_visual_marks = Some((span.start, span.end, kind));
        }
        let count = self.take_total_count().max(1);
        let gen_before = self.edit_generation;
        self.begin_edit();
        if matches!(op, Operator::IndentLeft | Operator::IndentRight) {
            // the count applies to indent operators (vim 9.1: `Vj3>` shifts
            // three shiftwidths). The line range is resolved ONCE — looping
            // `ops::apply` would reuse the original byte span, which drifts
            // off the tail lines as earlier indents insert bytes. The cursor
            // lands where the single-count apply would put it.
            let first = ctx.buf.offset_to_line(span.start);
            let last = ops::last_line_of_span(ctx.buf, &span);
            for _ in 0..count {
                for line in first..=last {
                    ops::shift_line(self, ctx, line, matches!(op, Operator::IndentRight));
                }
            }
            if self.edit_generation != gen_before {
                self.cursor.offset = ctx.buf.first_non_blank(first.min(ctx.buf.line_count() - 1));
                self.cursor.desired_col = None;
            }
        } else {
            ops::apply(self, ctx, op, &span, self.register);
        }
        // an operator that entered insert mode (visual `c`) keeps its group
        // open so deletion + typing undo as one step
        if self.insert_session.is_none() {
            self.end_edit();
        }
        self.bump_if_edited(ctx, gen_before);
        self.reset_pending();
        if matches!(self.mode, Mode::Visual { .. }) {
            self.finish_visual_op(ctx);
        }
    }

    /// An operator got its motion/object (from the trie or a doubled key).
    pub(crate) fn complete_operator_with_span(&mut self, ctx: &mut Ctx, span: ops::OpSpan) {
        let Some(op) = self.op.take() else { return };
        self.op_count = None;
        let gen_before = self.edit_generation;
        self.begin_edit();
        ops::apply(self, ctx, op, &span, self.register);
        // an operator that entered insert mode (cw/ciw/cc) keeps its group
        // open so deletion + typing undo as one step
        if self.insert_session.is_none() {
            self.end_edit();
        }
        // pure yanks and empty spans (`yw`, `d$` on an empty line) must not
        // feed the changelist / `.` mark / host.changed — vim 9.1 keeps yank
        // out of `:changes` (probe: yiw adds no entry)
        self.bump_if_edited(ctx, gen_before);
        self.reset_pending();
        if self.insert_session.is_none() {
            self.commit_change_record();
        }
    }

    /// vim's feedback for a failed `n`/`N`/`*` jump: E35 before any search
    /// was entered, E486 with the pattern otherwise — plus the bell.
    fn report_search_miss(&mut self, ctx: &mut Ctx) {
        match &self.search.pattern {
            Some(pattern) => {
                ctx.host
                    .status_message(&format!("E486: Pattern not found: {pattern}"));
            }
            None => ctx
                .host
                .status_message("E35: No previous regular expression"),
        }
        ctx.host.bell();
    }

    /// Does `rest` (re-fed after a trie miss) complete the pending operator's
    /// doubling — `gu` pending, rest == [`u`]? Only then is the retry silent;
    /// anything else keeps its bell.
    fn completes_operator_doubling(&self, rest: &[Key]) -> bool {
        if rest.len() != 1 {
            return false;
        }
        let Some(op) = self.op else {
            return false;
        };
        let trigger = if op == Operator::Format {
            self.format_trigger
        } else {
            Self::operator_trigger(op)
        };
        let Some(trigger) = trigger else {
            return false;
        };
        matches!(
            &rest[0].kind,
            KeyKind::Char(c) if *c == trigger && rest[0].modifiers.is_plain()
        )
    }

    /// Prefix count × operator count, defaulting each to 1. Capped at a
    /// billion: counts arrive unvalidated from the keyboard, and `3p` scales
    /// the register text by the count — an absurd count must not multiply
    /// into an unbounded allocation downstream.
    fn take_total_count(&mut self) -> usize {
        const COUNT_CAP: usize = 1_000_000_000;
        let pre = self.count.take().unwrap_or(1);
        let post = self.op_count.take().unwrap_or(1);
        pre.saturating_mul(post).min(COUNT_CAP)
    }

    /// End of a complete top-level command: commit the recording if the
    /// command mutated the buffer. An active insert session commits later,
    /// in `exit_insert` (the session is part of the same change).
    pub(crate) fn commit_change_record(&mut self) {
        if self.replaying {
            return;
        }
        if self.recording_blocked {
            self.discard_change_record();
        } else if matches!(self.mode, Mode::Visual { .. }) {
            // a visual change spans several commands (`v`, the motions, the
            // operator) — keep accumulating; the commit lands when the
            // selection resolves (`finish_visual_op`, `exit_insert`)
        } else if self.recording_mutated && !self.recording.is_empty() {
            self.last_change = std::mem::take(&mut self.recording);
            self.recording_mutated = false;
        } else {
            self.recording.clear();
            self.recording_mutated = false;
        }
    }

    /// Drop the in-progress recording (Esc / canceled command): the keys
    /// typed so far never became a change and must not leak into the next one.
    pub(crate) fn discard_change_record(&mut self) {
        self.recording.clear();
        self.recording_mutated = false;
        self.recording_blocked = false;
    }

    /// End of a complete top-level command.
    fn end_command(&mut self) {
        if self.insert_session.is_none() {
            self.commit_change_record();
        }
        self.count = None;
        self.register = None;
        self.register_pending = false;
        self.cmd_seq.clear();
        if self.insert_session.is_none() {
            self.end_edit();
        }
    }

    /// The operator's own key, for `dd`/`yy`/`>>` and `guu`/`g~~` doubling —
    /// derived from [`op_keys`] so the two spellings can't drift apart.
    /// `Format` has none: its trigger letter (`q`/`w`) is remembered
    /// separately because either spelling starts the same operator.
    pub(crate) fn operator_trigger(op: Operator) -> Option<char> {
        match op {
            Operator::Format => None,
            other => op_keys(other).chars().last(),
        }
    }

    /// The span END for `D`/`C` with a count: the covered lines disappear
    /// whole, but the LAST covered line keeps its trailing newline unless the
    /// count reached the buffer end (vim 9.1: `3D` from line 1 on a 4-line
    /// buffer → ['a','dddd']; `99D` on a 2-line buffer → ['a']).
    fn delete_to_end_span(ctx: &Ctx, line: usize, count: usize, last: usize) -> usize {
        if line + count > ctx.buf.line_count() - 1 {
            ctx.buf.line_range(last).end
        } else {
            ctx.buf.line_end(last)
        }
    }

    /// Execute a resolved Normal-mode command (the `CmdKind::Normal` arms of
    /// the command table). Conventions across the arms:
    /// * `take_total_count` collapses `[3]d[d]`-style prefix counts into the
    ///   command's own repeat count (absent count = 1);
    /// * every buffer mutation is bracketed by `begin_edit`/`end_edit` so a
    ///   whole command is ONE host undo group, and `bump` afterwards updates
    ///   marks/jumplist bookkeeping;
    /// * the char-argument commands (`r`, `m`, `q`, `@`, `` ` ``/`'`) only
    ///   arm the pending state here — the next keystroke completes them in
    ///   `complete_char_arg`.
    pub(crate) fn execute_normal_cmd(&mut self, ctx: &mut Ctx, cmd: NormalCmd) {
        match cmd {
            // x: delete count chars starting at the cursor
            NormalCmd::DeleteCharForward => {
                let count = self.take_total_count();
                let register = self.register.take();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::delete_chars(self, ctx, count, false, register);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // X: delete count chars before the cursor (never crosses the
            // line start)
            NormalCmd::DeleteCharBackward => {
                let count = self.take_total_count();
                let register = self.register.take();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::delete_chars(self, ctx, count, true, register);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // s: like x, but drop into insert (one undo group covers the
            // delete AND the typed replacement via `begin_insert`'s group
            // reuse)
            NormalCmd::SubstituteChar => {
                let count = self.take_total_count();
                let register = self.register.take();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::delete_chars(self, ctx, count, false, register);
                self.start_insert(ctx, InsertKind::Change);
                self.bump_if_edited(ctx, gen);
            }
            // S: clear [count] lines' content but keep the lines themselves
            // (linewise `cc` — ops::apply preserves the indent; vim 9.1
            // probe: `3S` on lines 2-4 clears exactly those three)
            NormalCmd::SubstituteLine => {
                let count = self.take_total_count();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                let span = ops::OpSpan {
                    start: ctx.buf.line_start(line),
                    end: ctx.buf.line_range(last).end,
                    linewise: true,
                };
                let gen = self.edit_generation;
                self.begin_edit();
                ops::apply(self, ctx, Operator::Change, &span, self.register);
                self.bump_if_edited(ctx, gen);
            }
            // C: change to end of line; a count changes through the END of
            // the count-th line down. vim's model is "delete [count] lines,
            // at least to end of line" (9.1 probes: `2C` from (1,2) on
            // ['aaaa','bbbb','cccc','dddd'] → ['anew','cccc','dddd'] — the
            // covered lines vanish whole, the LAST covered line keeps its
            // trailing newline so the following line survives; only when the
            // count reaches the buffer end does that newline go too, `99D`
            // on a 2-line buffer → ['a']). Count 1 never joins, like d$.
            NormalCmd::ChangeToEnd => {
                let count = self.take_total_count();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                let span = ops::OpSpan {
                    start: self.cursor.offset,
                    end: Self::delete_to_end_span(ctx, line, count, last),
                    linewise: false,
                };
                if span.end > span.start {
                    self.begin_edit();
                    ops::apply(self, ctx, Operator::Change, &span, self.register);
                    self.bump(ctx);
                } else {
                    self.start_insert(ctx, InsertKind::AppendLineEnd);
                }
            }
            // D: delete to end of line; a count deletes the covered lines
            // whole (same end rules as `C` above)
            NormalCmd::DeleteToEnd => {
                let count = self.take_total_count();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                let span = ops::OpSpan {
                    start: self.cursor.offset,
                    end: Self::delete_to_end_span(ctx, line, count, last),
                    linewise: false,
                };
                if span.end > span.start {
                    self.begin_edit();
                    ops::apply(self, ctx, Operator::Delete, &span, self.register);
                    self.bump(ctx);
                }
            }
            // Y: yank count whole lines (linewise, so `p` opens lines)
            NormalCmd::YankLine => {
                let count = self.take_total_count();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                let span = ops::OpSpan {
                    start: ctx.buf.line_start(line),
                    end: ctx.buf.line_range(last).end,
                    linewise: true,
                };
                ops::yank_span(self, ctx, &span, self.register);
            }
            // r{char}: the replacement char arrives as a char argument
            NormalCmd::ReplaceChar => {
                self.char_arg_cmd = Some(CharArgCmd::Replace);
            }
            // ~: swap case under the cursor, advancing per char
            NormalCmd::ToggleChar => {
                let count = self.take_total_count();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::toggle_chars(self, ctx, count);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // p: put after the cursor / below the current line
            NormalCmd::PutAfter => {
                let count = self.take_total_count();
                let register = self.register.unwrap_or(crate::registers::UNNAMED);
                let gen = self.edit_generation;
                self.begin_edit();
                ops::put(self, ctx, register, count, true);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // P: put before the cursor / above the current line
            NormalCmd::PutBefore => {
                let count = self.take_total_count();
                let register = self.register.unwrap_or(crate::registers::UNNAMED);
                let gen = self.edit_generation;
                self.begin_edit();
                ops::put(self, ctx, register, count, false);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // J: join with separator logic (space unless line ends in
            // whitespace or next starts with `)`)
            NormalCmd::Join => {
                let count = self.take_total_count();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::join_lines(self, ctx, count, false);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // gJ: join without any separator, keep the next line's indent
            NormalCmd::JoinLiteral => {
                let count = self.take_total_count();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::join_lines(self, ctx, count, true);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // u: step the HOST undo stack back count times. The host swaps
            // the buffer text underneath the engine, so the cached search
            // match offsets go stale and must be re-scanned (generation
            // bump) before the next `n`/highlight refresh.
            NormalCmd::Undo => {
                let count = self.take_total_count();
                for _ in 0..count {
                    if let Some(offset) = ctx.host.undo() {
                        // the host swapped the text underneath the engine:
                        // cached match offsets are stale until a re-scan,
                        // and the restored cursor may now sit inside a
                        // multi-byte char — floor before cursor math
                        self.edit_generation += 1;
                        self.sanitize_stored_offsets(ctx.buf);
                        self.cursor.offset = clamp_cursor(
                            ctx.buf,
                            crate::buffer::floor_to_char_boundary(ctx.buf, offset),
                        );
                    } else {
                        ctx.host.bell();
                        break;
                    }
                }
                self.republish_search(ctx);
                ctx.host.changed();
            }
            // <C-r>: undo's mirror image
            NormalCmd::Redo => {
                let count = self.take_total_count();
                for _ in 0..count {
                    if let Some(offset) = ctx.host.redo() {
                        self.edit_generation += 1;
                        self.sanitize_stored_offsets(ctx.buf);
                        self.cursor.offset = clamp_cursor(
                            ctx.buf,
                            crate::buffer::floor_to_char_boundary(ctx.buf, offset),
                        );
                    } else {
                        ctx.host.bell();
                        break;
                    }
                }
                self.republish_search(ctx);
                ctx.host.changed();
            }
            // m{char}: mark set is completed by the char argument
            NormalCmd::MarkSet => {
                self.char_arg_cmd = Some(CharArgCmd::MarkSet);
            }
            // q{reg}: record; the SAME `q` command stops it (see
            // execute_command's early handling)
            NormalCmd::RecordMacro => {
                self.char_arg_cmd = Some(CharArgCmd::MacroRecord);
            }
            // @{reg} / @@: play; replay goes through the key pipeline
            NormalCmd::PlayMacro => {
                self.char_arg_cmd = Some(CharArgCmd::MacroPlay);
            }
            // `{char} (linewise=false) / '{char} (linewise): jump to mark
            NormalCmd::JumpMark { linewise } => {
                self.char_arg_cmd = Some(CharArgCmd::JumpMark { linewise });
            }
            // doubled case/indent operators (guu, g~~, >> with count, ...):
            // linewise application over count lines
            NormalCmd::LinewiseOp(op) => {
                let count = self.take_total_count();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let last = (line + count - 1).min(ctx.buf.line_count() - 1);
                let span = ops::OpSpan {
                    start: ctx.buf.line_start(line),
                    end: ctx.buf.line_range(last).end,
                    linewise: true,
                };
                let gen = self.edit_generation;
                self.begin_edit();
                ops::apply(self, ctx, op, &span, self.register);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            // zz / zt / zb: hosts own actual scrolling; the engine only
            // reports which line should land where in the viewport
            NormalCmd::ScrollCenter | NormalCmd::ScrollTop | NormalCmd::ScrollBottom => {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                // hosts implement the actual scroll; notify with the line
                // and the anchor (zz/zt/zb semantics)
                let anchor = match cmd {
                    NormalCmd::ScrollCenter => ScrollAnchor::Center,
                    NormalCmd::ScrollTop => ScrollAnchor::Top,
                    _ => ScrollAnchor::Bottom,
                };
                ctx.host.scroll_to_line_anchored(line, anchor);
            }
            // `.`: replay the last recorded change, count times. Steps are
            // queued, not executed inline, so each step flows back through
            // the normal key pipeline (mappings off while replaying).
            NormalCmd::RepeatChange => {
                let count = self.take_total_count().max(1);
                if self.last_change.is_empty() {
                    ctx.host.bell();
                    return;
                }
                let steps = self.last_change.clone();
                self.begin_replay();
                self.enqueue_replay(&steps, count);
            }
            // g; / g,: walk the changelist. Hits the edge → vim's E662/E664
            // style message + bell, cursor stays at the last valid entry.
            NormalCmd::OlderChange | NormalCmd::NewerChange => {
                let older = cmd == NormalCmd::OlderChange;
                let count = self.take_total_count().max(1);
                if self.changes.is_empty() {
                    ctx.host.status_message("E664: changelist is empty");
                    ctx.host.bell();
                    return;
                }
                for _ in 0..count {
                    let moved = if older {
                        if self.change_pos == 0 {
                            false
                        } else {
                            self.change_pos -= 1;
                            true
                        }
                    } else if self.change_pos + 1 < self.changes.len() {
                        self.change_pos += 1;
                        true
                    } else {
                        false
                    };
                    if !moved {
                        // vim separates the two dead ends: E662 walking
                        // backward past the oldest entry, E663 forward past
                        // the newest (the engine used to report E662 both ways)
                        let message = if older {
                            "E662: At start of changelist"
                        } else {
                            "E663: At end of changelist"
                        };
                        ctx.host.status_message(message);
                        ctx.host.bell();
                        break;
                    }
                }
                let offset =
                    crate::buffer::floor_to_char_boundary(ctx.buf, self.changes[self.change_pos]);
                self.cursor.offset = clamp_cursor(ctx.buf, offset);
                self.cursor.desired_col = None;
                ctx.host
                    .scroll_to_line(ctx.buf.offset_to_line(self.cursor.offset));
                ctx.host.changed();
            }
            // gn / gN: visual-select the match containing the cursor, else
            // the next one in the search direction (`gN`: before it). In
            // visual mode the selection is reshaped to the match. Feeds the
            // cgn + `.` workflow: change one match, then repeat on the rest.
            NormalCmd::SelectMatch { backward } => {
                let count = self.take_total_count().max(1);
                let mut from = self.cursor.offset;
                let mut found = None;
                for step in 0..count {
                    // see motions.rs SelectMatch: step > 0 is strict so the
                    // backward iteration advances instead of re-selecting
                    match crate::search::find_match_from(self, ctx.buf, from, backward, step > 0) {
                        Some(range) => {
                            from = if backward { range.start } else { range.end };
                            found = Some(range);
                        }
                        None => {
                            found = None;
                            break;
                        }
                    }
                }
                let Some(range) = found else {
                    self.report_search_miss(ctx);
                    return;
                };
                self.visual_anchor = Some(range.start);
                // cursor ON the last char of the match: `end - 1` bytes
                // would sit INSIDE a multi-byte final char (fuzz: a `中`
                // match parked the cursor mid-char and the next host read
                // panicked)
                self.cursor.offset = ctx
                    .buf
                    .prev_char_offset(range.end)
                    .unwrap_or(range.start)
                    .max(range.start);
                self.cursor.desired_col = None;
                if !matches!(self.mode, Mode::Visual { .. }) {
                    self.mode = Mode::Visual {
                        kind: crate::mode::VisualKind::Char,
                    };
                }
                self.marks.active_visual = Some((range.start, range.end));
                ctx.host
                    .scroll_to_line(ctx.buf.offset_to_line(self.cursor.offset));
                ctx.host.changed();
            }
            // <C-a> / <C-x>: increment/decrement the number at or after the
            // cursor (a count adds that many)
            NormalCmd::IncrementNumber | NormalCmd::DecrementNumber => {
                let delta: i64 = if cmd == NormalCmd::IncrementNumber {
                    self.take_total_count().max(1) as i64
                } else {
                    -(self.take_total_count().max(1) as i64)
                };
                if !self.increment_number_at_cursor(ctx, delta) {
                    ctx.host.status_message("E18: Unexpected end of file");
                    ctx.host.bell();
                }
            }
            // <C-o> / <C-i>: walk the jumplist backwards/forwards. The list
            // behaves like browser history: a fresh jump discards the
            // forward entries (see record_jump).
            NormalCmd::JumpBackward | NormalCmd::JumpForward => {
                let backward = cmd == NormalCmd::JumpBackward;
                let count = self.take_total_count().max(1);
                for _ in 0..count {
                    let moved = if backward {
                        if self.jump_pos == 0 {
                            false
                        } else {
                            self.jump_pos -= 1;
                            true
                        }
                    } else if self.jump_pos + 1 < self.jumps.len() {
                        self.jump_pos += 1;
                        true
                    } else {
                        false
                    };
                    if !moved {
                        ctx.host.bell();
                        break;
                    }
                    self.cursor.offset = clamp_cursor(
                        ctx.buf,
                        crate::buffer::floor_to_char_boundary(ctx.buf, self.jumps[self.jump_pos]),
                    );
                    self.cursor.desired_col = None;
                    ctx.host
                        .scroll_to_line(ctx.buf.offset_to_line(self.cursor.offset));
                }
                ctx.host.changed();
            }
            // gv: re-select the last visual range (its kind, too)
            NormalCmd::RestoreVisual => {
                if let Some((lo, hi, kind)) = self.marks.last_visual {
                    // floor the anchor too (defensive: the stored span is
                    // refloored on every edit, but a host text swap between
                    // engines must not resurrect a mid-char offset)
                    self.visual_anchor = Some(crate::buffer::floor_to_char_boundary(
                        ctx.buf,
                        lo.min(ctx.buf.len()),
                    ));
                    // hi is the exclusive end; floor(hi-1) is the START of
                    // the last covered char, boundary-safe for multi-byte
                    self.cursor.offset =
                        crate::buffer::floor_to_char_boundary(ctx.buf, hi.saturating_sub(1))
                            .min(ctx.buf.len());
                    self.mode = Mode::Visual { kind };
                }
            }
            // ZZ / ZQ: the host owns persistence and window lifetime. Both
            // may close with unsaved changes (ZZ writes first, ZQ is :q!)
            NormalCmd::WriteQuit => {
                ctx.host.save();
                ctx.host.request_close_forced(true);
            }
            NormalCmd::QuitNoSave => {
                ctx.host.request_close_forced(true);
            }
            // C-e / C-y: scroll the VIEW `count` lines; the cursor only
            // follows when the scroll would push it out of the viewport
            NormalCmd::ScrollLines { down } => {
                let count = self.take_total_count().max(1);
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let delta = if down { count as i32 } else { -(count as i32) };
                ctx.host.scroll_lines(delta);
                ctx.host.scroll_to_line(line);
            }
            // &: repeat the last :s on the current line (the stored command
            // line re-runs through the full Ex parser, so ranges inside it
            // behave as typed — vim repeats them relative to the cursor).
            // Flags are dropped on this replay (`&` has none of its own,
            // vim 9.1), like the bare `:s` repeat.
            NormalCmd::RepeatSubstitute => match self.cmdline.last_substitute.clone() {
                Some(last) => {
                    let last = Self::strip_substitute_flags_for_repeat(&last);
                    self.execute_ex(ctx, &last)
                }
                None => {
                    // vim: E33 "No previous substitute regular expression"
                    ctx.host
                        .status_message("E33: No previous substitute regular expression");
                    ctx.host.bell();
                }
            },
        }
    }

    /// Visual-block `p`/`P`: a blockwise register replaces the block row by
    /// row; any other register's text is replicated onto every row.
    fn block_put_replace(&mut self, ctx: &mut Ctx) {
        let register = self.register.unwrap_or(crate::registers::UNNAMED);
        let Some(block) = ops::span_from_visual_block(self, ctx.buf) else {
            ctx.host.bell();
            return;
        };
        let Some(data) = self.registers.get_for_paste(register, ctx.host) else {
            ctx.host.bell();
            return;
        };
        self.recording_blocked = true;
        self.begin_edit();
        let cursor_to = block.rows.first().map(|r| r.start).unwrap_or(0);
        match data.kind {
            crate::registers::RegisterKind::Blockwise => {
                let rows: Vec<&str> = data.text.trim_end_matches('\n').split('\n').collect();
                for (i, range) in block.rows.iter().enumerate().rev() {
                    if range.is_empty() {
                        continue;
                    }
                    // a register with FEWER rows than the selection leaves the
                    // exhausted rows EMPTY — vim does not cycle or repeat the
                    // last row (9.1 probe: 2-row register on a 3-row block →
                    // the third row's covered span is just deleted)
                    let text = rows.get(i).copied().unwrap_or("");
                    self.edit_delete(ctx, range.clone());
                    if !text.is_empty() {
                        self.edit_insert(ctx, range.start, text);
                    }
                }
            }
            _ => {
                let adjusted = self.delete_block_rows(ctx, &block.rows);
                let text = data.text.trim_end_matches('\n');
                if !text.is_empty() {
                    for &offset in adjusted.iter().rev() {
                        self.edit_insert(ctx, offset, text);
                    }
                }
            }
        }
        self.cursor.offset = clamp_cursor(ctx.buf, cursor_to);
        self.cursor.desired_col = None;
        self.end_edit();
        self.bump(ctx);
        self.finish_visual_op(ctx);
    }

    pub(crate) fn execute_visual_cmd(&mut self, ctx: &mut Ctx, cmd: VisualCmd) {
        match cmd {
            VisualCmd::Exit => {
                self.exit_visual(ctx);
            }
            VisualCmd::ToggleKind { to } => {
                let current = match self.mode {
                    Mode::Visual { kind } => kind,
                    _ => return,
                };
                let target = match to {
                    'v' => VisualKind::Char,
                    'V' => VisualKind::Line,
                    _ => VisualKind::Block,
                };
                if current == target {
                    self.exit_visual(ctx);
                } else {
                    self.mode = Mode::Visual { kind: target };
                }
            }
            VisualCmd::SwapEnds => {
                if let Some((anchor, cursor, _)) = self.visual_selection() {
                    self.visual_anchor = Some(cursor);
                    self.cursor.offset = anchor;
                }
            }
            // O: in block mode the cursor moves to the other corner of the
            // block IN THE SAME LINE (`:h v_O`; three vim 9.1 probes agree:
            // anchor (0,0) cursor (1,2) → cursor (1,0) + anchor (0,2), i.e.
            // cursor keeps its ROW and takes the block's other COLUMN edge,
            // the anchor mirrors it so the rectangle is unchanged — the
            // round-8 tests had this backwards from a misread probe). On a
            // single-row block this is a real horizontal move (cursor
            // (0,2) → (0,0)), not a no-op. In char/line visual O is vim's
            // synonym of `o`.
            VisualCmd::SwapEndsKeepCol => {
                let is_block = matches!(
                    self.mode,
                    Mode::Visual {
                        kind: crate::mode::VisualKind::Block
                    }
                );
                let Some((anchor, cursor, _)) = self.visual_selection() else {
                    return;
                };
                if !is_block {
                    // char/line visual: O is vim's synonym of `o` (probe 9.1:
                    // `vllO` and `VjO` both mirror the ends, no bell)
                    self.visual_anchor = Some(cursor);
                    self.cursor.offset = anchor;
                    return;
                }
                let (a_line, c_line) = (
                    ctx.buf.offset_to_line(anchor),
                    ctx.buf.offset_to_line(cursor),
                );
                let (a_col, c_col) = (
                    crate::buffer::display_column(ctx.buf, anchor),
                    crate::buffer::display_column(ctx.buf, cursor),
                );
                self.visual_anchor = Some(crate::buffer::offset_for_display_column(
                    ctx.buf, a_line, c_col,
                ));
                self.cursor.offset =
                    crate::buffer::offset_for_display_column(ctx.buf, c_line, a_col);
                self.cursor.desired_col = None;
            }
            VisualCmd::PutReplace => {
                if matches!(
                    self.mode,
                    Mode::Visual {
                        kind: crate::mode::VisualKind::Block
                    }
                ) {
                    self.block_put_replace(ctx);
                    return;
                }
                let register = self.register.unwrap_or(crate::registers::UNNAMED);
                let Some(span) = ops::span_from_visual(self, ctx.buf) else {
                    return;
                };
                // an unset register: vim reports E353 and leaves the
                // selection INTACT — delete-then-nothing would lose it
                let Some(stashed) = self.registers.get_for_paste(register, ctx.host) else {
                    ctx.host.bell();
                    return;
                };
                // the stash doubles as protection: delete_span rewrites the
                // registers with the deleted selection before the paste
                self.begin_edit();
                ops::delete_span(self, ctx, &span, self.register);
                {
                    // visual `p` with a count repeats the register; the byte
                    // ceiling keeps `99999999p` from allocating register × count
                    let data = stashed;
                    let repeated = crate::registers::clamped_repeat(
                        &data.text,
                        self.take_total_count().max(1),
                    );
                    if data.kind == crate::registers::RegisterKind::Linewise {
                        let text = if repeated.ends_with('\n') {
                            repeated
                        } else {
                            format!("{repeated}\n")
                        };
                        // A LINEWISE selection pastes at the start of the line
                        // where the selection began — NOT at the post-delete
                        // cursor: delete_span parks it on the surviving line's
                        // first non-blank, and inserting there shreds that line
                        // (vim 9.1 probe: `Vp` with "XY" over ['abc'] when
                        // ' ghi' survives → ['XY', ' ghi', …], indent intact).
                        // span_from_visual clamps a Line-kind span.start to its
                        // line start; the murky charwise-selection case keeps
                        // the cursor anchor (documented divergence).
                        let at = if span.linewise {
                            span.start.min(ctx.buf.len())
                        } else {
                            self.cursor.offset
                        };
                        self.edit_insert(ctx, at, &text);
                        self.cursor.offset = ctx.buf.first_non_blank(ctx.buf.offset_to_line(at));
                    } else {
                        // 插入位取删除区的真实起点，不走 cursor.offset：
                        // delete_span 经 clamp_cursor 停放光标，而 clamp 是
                        // 「停放」语义（offset 落在行尾时回拉到末字符）——
                        // span.start 恰在行尾时（选区贴行尾，如 v$）粘贴会
                        // 插到行尾字符之前，行尾字符反落在粘贴文本之后
                        let at = span.start.min(ctx.buf.len());
                        self.edit_insert(ctx, at, &repeated);
                        // cursor on the last pasted char's START — byte - 1
                        // would sit inside a multi-byte character
                        let end = at + repeated.len();
                        self.cursor.offset = ctx.buf.prev_char_offset(end).unwrap_or(at);
                    }
                }
                self.end_edit();
                self.bump(ctx);
                self.finish_visual_op(ctx);
            }
            VisualCmd::Join { literal } => {
                let Some(span) = ops::span_from_visual(self, ctx.buf) else {
                    return;
                };
                let count = self.take_total_count();
                let first = ctx.buf.offset_to_line(span.start);
                let last = ops::last_line_of_span(ctx.buf, &span);
                self.begin_edit();
                self.cursor.offset = span.start;
                ops::join_lines(self, ctx, (last - first + 1).max(count), literal);
                self.end_edit();
                self.bump(ctx);
                self.finish_visual_op(ctx);
            }
            // `r{char}` waits for its argument: the replacement is applied
            // in `complete_char_arg`'s VisualReplace arm
            VisualCmd::ReplaceChar => {}
            // Y / D / X / C / S: vim's LINEWISE visual spellings (9.1
            // probes: `vlD` deletes the covered line whole, `vlY` yanks it
            // linewise, `vlC`/`vlS` linewise-change it). The covered-lines
            // span replaces the charwise selection; everything else runs
            // the standard operator path.
            VisualCmd::LinewiseOp(op) => {
                let Some(span) = ops::span_from_visual(self, ctx.buf) else {
                    return;
                };
                let first = ctx.buf.offset_to_line(span.start);
                let last = ops::last_line_of_span(ctx.buf, &span);
                let line_span = ops::OpSpan {
                    start: ctx.buf.line_start(first),
                    end: ctx.buf.line_range(last).end,
                    linewise: true,
                };
                // `c` continues into insert: remember the covered lines for
                // `gv` (see `pending_visual_marks`) before the edit shifts
                if op == Operator::Change {
                    let kind = match self.mode {
                        Mode::Visual { kind } => kind,
                        _ => crate::mode::VisualKind::Char,
                    };
                    self.pending_visual_marks = Some((line_span.start, line_span.end, kind));
                }
                let count = self.take_total_count();
                let gen_before = self.edit_generation;
                self.begin_edit();
                if count > 1 && matches!(op, Operator::IndentLeft | Operator::IndentRight) {
                    for _ in 0..count {
                        for line in first..=last {
                            ops::shift_line(self, ctx, line, matches!(op, Operator::IndentRight));
                        }
                    }
                } else {
                    ops::apply(self, ctx, op, &line_span, self.register);
                }
                if self.insert_session.is_none() {
                    self.end_edit();
                }
                self.bump_if_edited(ctx, gen_before);
                self.reset_pending();
                if matches!(self.mode, Mode::Visual { .. }) {
                    self.finish_visual_op(ctx);
                }
            }
            // zz / zt / zb: report the anchored scroll and KEEP the
            // selection — vim scrolls without leaving visual mode
            VisualCmd::Scroll(anchor) => {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                ctx.host.scroll_to_line_anchored(line, anchor);
            }
        }
    }

    /// `start_insert` places the cursor per `InsertKind` then begins a session.
    pub(crate) fn start_insert(&mut self, ctx: &mut Ctx, kind: InsertKind) {
        match kind {
            InsertKind::Insert | InsertKind::Change | InsertKind::Replace => {}
            InsertKind::Append => {
                if !ctx.buf.at_line_end(self.cursor.offset) {
                    self.cursor.offset =
                        crate::buffer::next_grapheme_offset(ctx.buf, self.cursor.offset)
                            .unwrap_or(self.cursor.offset);
                }
            }
            InsertKind::InsertFirstNonBlank => {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                if ctx.buf.line_is_blank(line) {
                    // all-blank line: vim's `I` types AFTER the blanks (`"   "
                    // + IZ → "   Z"`, vim 9.1) — at the line end, not before
                    // the last space where `^` parks the block cursor
                    self.cursor.offset = ctx.buf.line_end(line);
                } else {
                    self.cursor.offset = ctx.buf.first_non_blank(line);
                }
            }
            InsertKind::AppendLineEnd => {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                self.cursor.offset = ctx.buf.line_end(line);
            }
            InsertKind::InsertAtColumnZero => {
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                self.cursor.offset = ctx.buf.line_start(line);
            }
            InsertKind::LastInsertExit => {
                if let Some(off) = self.marks.last_insert_exit {
                    self.cursor.offset = crate::buffer::floor_to_char_boundary(ctx.buf, off);
                }
            }
            InsertKind::OpenLine { below } => {
                // open the undo group BEFORE mutating, so the snapshot the
                // host takes can actually undo the inserted line
                self.begin_edit();
                let line = ctx.buf.offset_to_line(self.cursor.offset);
                let line_start = ctx.buf.line_start(line);
                // copy the line's indent VERBATIM (tabs stay tabs; the old
                // " ".repeat(indent) silently retabbed tab-indented files) —
                // gated on 'autoindent' like vim (`:set noai` + `o` starts
                // the line at column 0)
                let indent_str = if self.options.autoindent {
                    let (indent, _) = ctx.buf.line_indent(line);
                    ctx.buf.slice(line_start..line_start + indent)
                } else {
                    String::new()
                };
                if below {
                    let at = ctx.buf.line_end(line);
                    self.edit_insert(ctx, at, &format!("\n{indent_str}"));
                    self.cursor.offset = at + 1 + indent_str.len();
                } else {
                    let at = ctx.buf.line_start(line);
                    self.edit_insert(ctx, at, &format!("{indent_str}\n"));
                    self.cursor.offset = at + indent_str.len();
                }
                // the opened line is a change even before typing (`o` + Esc
                // still created a line): record it for `.`/`g;`
                self.bump(ctx);
            }
        }
        self.begin_insert(kind);
        if kind == InsertKind::Replace {
            self.replace_overwritten.clear();
        }
    }

    // ---- char-argument commands -------------------------------------------------

    /// The `char_arg_cmd` set by the command table is completed here: the
    /// next key supplies the argument (a register name for `@`/`q`, a mark
    /// for `` ` ``/`'`/`m`, a replacement char for `r`). Esc cancels the
    /// whole pending command; a non-printable key is swallowed as "still
    /// waiting" (vim ignores it too, e.g. `r` followed by a stray arrow).
    fn complete_char_arg(&mut self, ctx: &mut Ctx, key: Key) -> ProcessOutcome {
        let Some(cmd) = self.char_arg_cmd.take() else {
            return ProcessOutcome::Consumed;
        };
        if key == Key::escape() || key == Key::ctrl_char('[') {
            self.reset_pending();
            return ProcessOutcome::Consumed;
        }
        // <CR> carries a printable meaning for the commands below even
        // though it is not "printable": `r<CR>` replaces the char with a
        // line break (vim splits the line). For the other char-argument
        // commands an enter argument harmlessly fails lookup.
        let c = if key == Key::enter() {
            '\n'
        } else if let Some(c) = key.printable_char() {
            c
        } else {
            self.reset_pending();
            ctx.host.bell();
            return ProcessOutcome::Consumed;
        };
        self.char_arg = Some(c);
        match cmd {
            CharArgCmd::Find { forward, till } => {
                self.last_find = Some((c, forward, till));
                let motion = Motion::FindChar { forward, till };
                let count = self.take_total_count();
                if self.op.is_some() {
                    let result = motion.target(self, ctx, count);
                    if !result.moved {
                        ctx.host.bell();
                        self.reset_pending();
                        return ProcessOutcome::Consumed;
                    }
                    let span = ops::span_from_motion(self, ctx.buf, motion, result);
                    self.complete_operator_with_span(ctx, span);
                } else if !self.goto_motion(ctx, motion, count) {
                    ctx.host.bell();
                }
            }
            CharArgCmd::Replace => {
                let count = self.take_total_count();
                let gen = self.edit_generation;
                self.begin_edit();
                ops::replace_chars(self, ctx, c, count);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
            }
            CharArgCmd::VisualReplace => {
                // Visual `r{char}`: replace every selected char (the visual
                // op path exits visual mode + commits the change record)
                let gen = self.edit_generation;
                self.begin_edit();
                ops::visual_replace(self, ctx, c);
                self.end_edit();
                self.bump_if_edited(ctx, gen);
                self.reset_pending();
                if matches!(self.mode, Mode::Visual { .. }) {
                    self.finish_visual_op(ctx);
                }
            }
            CharArgCmd::MarkSet => {
                self.marks.set(c, self.cursor.offset);
            }
            CharArgCmd::MacroRecord => {
                // starting `q{reg}`; the stop is handled in execute_command.
                // vim only accepts a-zA-Z0-9 — `q/` beeps and stays idle, so
                // a stray key can't hijack a slot the `@` lookup expects to
                // be a real register. The count before `q` is dropped too.
                if !c.is_ascii_alphanumeric() {
                    self.char_arg = None;
                    self.reset_pending();
                    ctx.host.bell();
                    return ProcessOutcome::Consumed;
                }
                // `qA` APPENDS to register a's recording (vim): the capture
                // starts from the register's existing steps and lands back
                // in the lowercase slot on stop
                let slot = c.to_ascii_lowercase();
                let seed = if c.is_ascii_uppercase() {
                    self.macros.get(&slot).cloned().unwrap_or_default()
                } else {
                    Vec::new()
                };
                self.macro_capture = Some((slot, seed));
            }
            CharArgCmd::MacroPlay => {
                // `@:` repeats the last executed Ex command line
                if c == ':' {
                    match self.cmdline.last_command.clone() {
                        Some(line) => self.execute_ex(ctx, &line),
                        None => ctx.host.bell(),
                    }
                    self.char_arg = None;
                    self.end_command();
                    return ProcessOutcome::Consumed;
                }
                let reg = if c == '@' {
                    self.last_macro_played
                } else {
                    Some(c)
                };
                match reg.and_then(|r| self.macros.get(&r).map(|keys| (r, keys.clone()))) {
                    Some((r, keys)) => {
                        // remember the RESOLVED register: for `@@` the pressed
                        // key is '@' itself, which is not a stored macro
                        self.last_macro_played = Some(r);
                        let count = self.take_total_count().max(1);
                        self.begin_replay();
                        self.enqueue_replay(&keys, count);
                    }
                    None => ctx.host.bell(),
                }
            }
            CharArgCmd::JumpMark { linewise } => {
                // Mirror of the Find arm: under an operator the mark jump is
                // the SPAN target (`d'a` linewise-deletes through the mark's
                // line, `d`a` charwise cursor..mark — direction-independent,
                // vim 9.1 probes); without one it is a plain jump. Both go
                // through Motion::MarkJump::target, which resolves
                // `self.char_arg` (set above).
                let motion = Motion::MarkJump { linewise };
                let count = self.take_total_count();
                if self.op.is_some() {
                    let result = motion.target(self, ctx, count);
                    if !result.moved {
                        ctx.host.bell();
                        self.reset_pending();
                        return ProcessOutcome::Consumed;
                    }
                    let span = ops::span_from_motion(self, ctx.buf, motion, result);
                    self.complete_operator_with_span(ctx, span);
                } else if !self.goto_motion(ctx, motion, count) {
                    ctx.host.bell();
                }
            }
        }
        self.char_arg = None;
        self.end_command();
        ProcessOutcome::Consumed
    }
}
