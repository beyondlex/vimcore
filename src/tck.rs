//! Host contract tests (TCK) — the executable form of the pitfall checklist
//! in INTEGRATION.md.
//!
//! Each function is a contract the host must satisfy. The host calls them on
//! its own buffer / host implementations (usually `.unwrap()` inside
//! `#[cfg(test)]`), turning "integration quality relies on reading the docs"
//! into "integration quality has a tested floor". Everything is pure headless:
//! no gpui dependency, no window opened.
//!
//! Typical usage (in the host repo):
//!
//! ```ignore
//! #[test]
//! fn my_buffer_meets_engine_contract() {
//!     let mut buf = MyBuffer::default();
//!     vimcore::tck::buffer_edit_contract(&mut buf).unwrap();
//!     let mut host = MyHost::default();
//!     vimcore::tck::engine_smoke_contract(&mut buf, &mut host).unwrap();
//! }
//! ```
//!
//! Deliberately uncovered parts: undo/redo (host semantics vary — PandaGit
//! uses an app-level unified undo stack, where `undo()` legitimately returns
//! None), clipboard, viewport scrolling. These are bounded by the host's own
//! tests together with the existence boundary of [`TckHost`].

use std::ops::Range;

use crate::buffer::{VimBuffer, VimBufferMut};
use crate::host::{VimHost, VimHostRo};
use crate::key::Key;
use crate::state::{Ctx, CtxRo, VimState};

/// Read-only contract: `VimBuffer` line semantics and character boundaries.
///
/// Checks each item against the buffer's **current content**; recommended to
/// run on multi-line content containing CJK, multibyte characters and empty
/// lines. Items checked:
///
/// - `line_count() >= 1` (an empty buffer must still have exactly 1 line);
/// - `line_range(i)` includes the terminating `\n` (for non-last lines), the
///   last line extends to the end of the buffer — the engine's linewise
///   operations (dd/yy/V-line) rely on this; a missing `\n` leaves an empty
///   line in place;
/// - `slice(range).len() == range.len()`;
/// - `offset_to_line(line_start(i)) == i`;
/// - `char_at` returns `Some` at every character boundary and `None` at
///   non-boundary/out-of-range offsets;
/// - `prev_char_offset` chains back from `len` to 0 with the step count
///   exactly equal to the character count;
/// - **phantom-line tolerance**: the engine's plain cursor commands (`w`/`}`
///   at the end of the buffer) call `line_range`/`line_end` with
///   `line == line_count` and call `offset_to_line` with arbitrary bytes
///   (including mid-multibyte-character) — implementations must, like this
///   reference one, return `len..len` / end-of-line and floor offsets to the
///   nearest character boundary; hosts that panic out-of-range crash on the
///   most ordinary keystrokes.
pub fn buffer_read_contract(buf: &(impl VimBuffer + ?Sized)) -> Result<(), String> {
    let len = buf.len();
    let lines = buf.line_count();
    if lines == 0 {
        return Err("line_count() == 0: an empty buffer must still have exactly 1 line".into());
    }
    for line in 0..lines {
        let range = buf.line_range(line);
        if range.start > range.end || range.end > len {
            return Err(format!(
                "line {line}: line_range {range:?} out of range (len={len})"
            ));
        }
        let text = buf.slice(range.clone());
        if text.len() != range.len() {
            return Err(format!(
                "line {line}: slice(range).len() = {} ≠ range.len() = {}",
                text.len(),
                range.len()
            ));
        }
        if line + 1 < lines {
            if !text.ends_with('\n') {
                return Err(format!(
                    "line {line}: non-last line_range must include the terminating \\n (the engine's dd/yy rely on it)"
                ));
            }
        } else if range.end != len {
            return Err(format!(
                "line {line}: last-line range must extend to the end of the buffer (end={} ≠ len={len})",
                range.end
            ));
        }
        if buf.offset_to_line(range.start) != line {
            return Err(format!(
                "line {line}: offset_to_line(line start={}) = {}",
                range.start,
                buf.offset_to_line(range.start)
            ));
        }
    }
    // Phantom lines and out-of-range / mid-character offsets must be tolerated
    // (see the contract notes above): the plain `w` command at the end of the
    // buffer already reads line_range with line == line_count
    if buf.line_range(lines) != (len..len) && len > 0 {
        return Err(format!(
            "line_range(line_count) must tolerate the phantom line (return len..len), got {:?}",
            buf.line_range(lines)
        ));
    }
    let full = buf.slice(0..len);
    if full.len() != len {
        return Err(format!(
            "slice(0..len).len() = {} ≠ len = {len}",
            full.len()
        ));
    }
    let mut o = 0;
    while o < len {
        let Some(c) = buf.char_at(o) else {
            return Err(format!(
                "char_at({o}) = None: must return Some at a character boundary"
            ));
        };
        // Multibyte characters: mid offsets must answer the same as the
        // boundary — the engine's stored-offset fallback
        // (refloor/floor_to_char_boundary) drops stale offsets onto the
        // "nearest boundary", but queries before the landing point happen at
        // arbitrary bytes, and a host that panics mid-character crashes right
        // after ordinary edits (lesson of round13 #26; the reverse assertion
        // is nailed down here)
        if c.len_utf8() > 1 {
            for mid in (o + 1)..(o + c.len_utf8()) {
                if buf.char_at(mid).is_some() {
                    return Err(format!(
                        "char_at({mid}) = Some: inside a multibyte character must return None"
                    ));
                }
                let (a, b) = (buf.offset_to_line(mid), buf.offset_to_line(o));
                if a != b {
                    return Err(format!(
                        "offset_to_line({mid}) = {a} ≠ offset_to_line(boundary {o}) = {b}: mid-char offsets must resolve to the owning line"
                    ));
                }
                if let Some(prev) = buf.prev_char_offset(mid) {
                    // The engine itself floors with char_at + byte decrement
                    // and never requires the host's fallback to succeed for
                    // mid-char offsets — it only requires that a Some return
                    // points at a character boundary
                    if buf.char_at(prev).is_none() {
                        return Err(format!(
                            "prev_char_offset({mid}) = Some({prev}): must point at a character boundary"
                        ));
                    }
                }
            }
        }
        o += c.len_utf8();
    }
    if len > 0 {
        if buf.char_at(len).is_some() {
            return Err("char_at(len) must be None".into());
        }
        // offset_to_line(len): clamping to the last line (tests host) or
        // returning the phantom line number (this reference host) are both
        // legal — the engine tolerates either (line_range tolerates
        // line == line_count) — but it must never panic
        let at_len = buf.offset_to_line(len);
        if at_len + 1 < lines {
            return Err(format!(
                "offset_to_line(len) = {at_len} is below the last line (last line = {})",
                lines - 1
            ));
        }
        if buf.prev_char_offset(len + 42).is_some() {
            return Err(
                "prev_char_offset(len+42) must be None (out-of-range offset refused)".into(),
            );
        }
        if buf.prev_char_offset(0).is_some() {
            return Err("prev_char_offset(0) must be None".into());
        }
        let chars = full.chars().count();
        let mut o = len;
        let mut steps = 0;
        while let Some(prev) = buf.prev_char_offset(o) {
            steps += 1;
            o = prev;
            if steps > chars {
                return Err(
                    "prev_char_offset chain does not converge (steps exceed the character count)"
                        .into(),
                );
            }
        }
        if o != 0 || steps != chars {
            return Err(format!(
                "prev_char_offset chain ends at {o} (should be 0) or steps {steps} ≠ char count {chars}"
            ));
        }
    }
    Ok(())
}

/// Mutable contract: UTF-8 safety of the two `VimBufferMut` edit primitives.
///
/// Requires an **empty buffer**; after every edit step the read-only contract
/// must still hold. The sequence covers: trailing multibyte insertion,
/// line-splitting insertion at line start, deleting a newline, clearing.
pub fn buffer_edit_contract(buf: &mut impl VimBufferMut) -> Result<(), String> {
    if !buf.is_empty() {
        return Err("buffer_edit_contract requires an empty buffer".into());
    }
    buf.insert_text(0, "ab");
    assert_content(buf, "ab")?;
    buf.insert_text(buf.len(), "中"); // trailing multibyte insertion
    assert_content(buf, "ab中")?;
    buf.insert_text(0, "\n"); // line-splitting insertion at line start
    assert_content(buf, "\nab中")?;
    if buf.line_count() != 2 {
        return Err(format!("line_count() = {} ≠ 2", buf.line_count()));
    }
    if buf.offset_to_line(1) != 1 {
        return Err(format!(
            "offset_to_line(1) = {} ≠ 1 (bytes after the \\n belong to line 1)",
            buf.offset_to_line(1)
        ));
    }
    buf.delete_range(0..1); // delete only the newline
    assert_content(buf, "ab中")?;
    buf.delete_range(0..buf.len()); // clear
    if !buf.is_empty() || buf.line_count() != 1 {
        return Err(format!(
            "after clearing len={} line_count={} (expected 0 and 1)",
            buf.len(),
            buf.line_count()
        ));
    }
    Ok(())
}

/// Engine smoke contract: run a deterministic edit sequence through the
/// engine's key pipeline on the host buffer, verifying full content and the
/// read-only contract at each step.
///
/// Requires an **empty buffer** (use [`TckHost`] when the host's own semantics
/// vary). Sequence: `ggx` (delete), `$` (end-of-line cursor), `Vjd` (visual
/// line delete), `p` (linewise put), `gg$A` + text + Esc (append at
/// end-of-line). Deliberately no `u`/`C-r` — see the module header.
pub fn engine_smoke_contract(
    buf: &mut impl VimBufferMut,
    host: &mut dyn VimHost,
) -> Result<(), String> {
    if !buf.is_empty() {
        return Err("engine_smoke_contract requires an empty buffer".into());
    }
    buf.insert_text(0, "alpha\nbeta\ngamma\n");
    let mut vim = VimState::new();

    feed(&mut vim, buf, host, "gg");
    assert_content(buf, "alpha\nbeta\ngamma\n")?;
    feed(&mut vim, buf, host, "x");
    assert_content(buf, "lpha\nbeta\ngamma\n")?;
    feed(&mut vim, buf, host, "$");
    let want = buf.line_end(0).saturating_sub(1);
    if vim.cursor_offset() != want {
        return Err(format!(
            "cursor_offset() after $ = {} ≠ last character of the line {want}",
            vim.cursor_offset()
        ));
    }
    feed(&mut vim, buf, host, "Vjd");
    assert_content(buf, "gamma\n")?;
    feed(&mut vim, buf, host, "p");
    assert_content(buf, "gamma\nlpha\nbeta\n")?;
    feed(&mut vim, buf, host, "gg$A");
    type_text(&mut vim, buf, host, "x");
    {
        let mut ctx = Ctx { buf, host };
        vim.handle_key(&mut ctx, Key::escape());
    }
    assert_content(buf, "gammax\nlpha\nbeta\n")?;
    Ok(())
}

/// Undo-group closure contract (the `end_undo_group` signal, since 0.2.3):
/// every group the engine ANNOUNCES through `begin_undo_group` is closed
/// deterministically by a matching `end_undo_group(id)` — a host must never
/// have to infer closure from the mode ("push when not in Insert/Replace"),
/// the inference that let an edit-less `<Esc>` strand a snapshot (vim's
/// `iZ<esc>u` reporting "Already at oldest change").
///
/// Runs the engine against the contract's own recording host, on a fresh
/// `VimState` per sequence. Items checked:
///
/// - `i` + text + `<Esc>`: exactly one begin, and after the `<Esc>` exactly
///   one end with the SAME id — the closed insert session;
/// - mid-session (`i` + text, no `<Esc>` yet): the begin has arrived, the
///   end must NOT have (the group is still open);
/// - an edit-less `i<Esc>`: no begin and no end at all — an unannounced
///   group must not produce a spurious close signal;
/// - a normal-mode command (`x`): one begin and one end within the same key;
/// - two consecutive commands (`x` twice): two begins with DIFFERENT ids,
///   each closed.
pub fn undo_group_closure_contract(buf: &mut impl VimBufferMut) -> Result<(), String> {
    if !buf.is_empty() {
        return Err("undo_group_closure_contract requires an empty buffer".into());
    }
    buf.insert_text(0, "alpha\nbeta\n");
    let paired = |host: &TckUndoHost| -> Result<(), String> {
        if host.begins.len() != host.ends.len() {
            return Err(format!(
                "announced groups (begins {:?}) and close signals (ends {:?}) do not pair up",
                host.begins, host.ends
            ));
        }
        for (b, e) in host.begins.iter().zip(&host.ends) {
            if b != e {
                return Err(format!(
                    "close signal id {e} does not match the announced group id {b} (begins {:?}, ends {:?})",
                    host.begins, host.ends
                ));
            }
        }
        Ok(())
    };
    let reset = |buf: &mut dyn VimBufferMut| {
        buf.delete_range(0..buf.len());
        buf.insert_text(0, "alpha\nbeta\n");
    };

    // 1. i + Z + <Esc>: one announced group, closed by the esc.
    {
        let mut host = TckUndoHost::default();
        let mut vim = VimState::new();
        feed(&mut vim, buf, &mut host, "i");
        type_text(&mut vim, buf, &mut host, "Z");
        {
            let mut ctx = Ctx {
                buf,
                host: &mut host,
            };
            vim.handle_key(&mut ctx, Key::escape());
        }
        if host.begins.len() != 1 || host.ends.len() != 1 {
            return Err(format!(
                "i Z <Esc>: expected exactly one begin/end pair, got begins {:?} ends {:?}",
                host.begins, host.ends
            ));
        }
        paired(&host)?;
        reset(buf);
    }

    // 2. Mid-session: begin arrived, end must not have.
    {
        let mut host = TckUndoHost::default();
        let mut vim = VimState::new();
        feed(&mut vim, buf, &mut host, "i");
        type_text(&mut vim, buf, &mut host, "Z");
        if host.begins.len() != 1 {
            return Err(format!(
                "i Z (no esc yet): the insert session must have announced its group, got begins {:?}",
                host.begins
            ));
        }
        if !host.ends.is_empty() {
            return Err(format!(
                "i Z (no esc yet): a close signal before the session ended (ends {:?})",
                host.ends
            ));
        }
        reset(buf);
    }

    // 3. Edit-less i<Esc>: no begin, no end — an unannounced group must not
    //    produce a spurious close signal.
    {
        let mut host = TckUndoHost::default();
        let mut vim = VimState::new();
        feed(&mut vim, buf, &mut host, "i");
        {
            let mut ctx = Ctx {
                buf,
                host: &mut host,
            };
            vim.handle_key(&mut ctx, Key::escape());
        }
        if !host.begins.is_empty() || !host.ends.is_empty() {
            return Err(format!(
                "edit-less i <Esc>: no group was announced, yet the host saw begins {:?} ends {:?}",
                host.begins, host.ends
            ));
        }
    }

    // 4. One normal-mode command: begin + end within the same key.
    {
        let mut host = TckUndoHost::default();
        let mut vim = VimState::new();
        feed(&mut vim, buf, &mut host, "x");
        if host.begins.len() != 1 || host.ends.len() != 1 {
            return Err(format!(
                "x: expected the command's group to open and close within the key, got begins {:?} ends {:?}",
                host.begins, host.ends
            ));
        }
        paired(&host)?;
        reset(buf);
    }

    // 5. Two consecutive commands: distinct ids, each closed.
    {
        let mut host = TckUndoHost::default();
        let mut vim = VimState::new();
        feed(&mut vim, buf, &mut host, "xx");
        if host.begins.len() != 2 || host.ends.len() != 2 {
            return Err(format!(
                "xx: expected two begin/end pairs, got begins {:?} ends {:?}",
                host.begins, host.ends
            ));
        }
        paired(&host)?;
        if host.begins[0] == host.begins[1] {
            return Err(format!(
                "xx: consecutive commands must open DISTINCT groups, got {:?}",
                host.begins
            ));
        }
        reset(buf);
    }

    assert_content(buf, "alpha\nbeta\n")?;
    Ok(())
}

/// String-backed reference buffer implementation: `line_range` includes the
/// terminating `\n`, `char_at` returns `None` at non-character boundaries
/// (never panics). Hosts can reuse this directly or copy the implementation —
/// the engine's `floor_to_char_boundary` deliberately probes non-boundary
/// offsets, and a `char_at` using raw slicing would panic from legitimate
/// engine call paths (a host-copy trap caught in the fifth review round).
#[derive(Default, Clone)]
pub struct TckStrBuf(pub String);

impl TckStrBuf {
    pub fn from_text(text: &str) -> Self {
        TckStrBuf(text.to_owned())
    }
}

impl VimBuffer for TckStrBuf {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn line_count(&self) -> usize {
        if self.0.is_empty() {
            1
        } else if self.0.ends_with('\n') {
            self.0.split('\n').count() - 1
        } else {
            self.0.split('\n').count()
        }
    }
    fn char_at(&self, offset: usize) -> Option<char> {
        if !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[offset..].chars().next()
    }
    fn prev_char_offset(&self, offset: usize) -> Option<usize> {
        if offset == 0 || offset > self.0.len() || !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[..offset]
            .chars()
            .next_back()
            .map(|c| offset - c.len_utf8())
    }
    fn line_range(&self, line: usize) -> Range<usize> {
        // single pass: walk the pieces once, counting as we go — the old
        // shape called line_count() up front (a full split/count) and again
        // for `total`, so every line_start/line_end/line_content via the
        // trait defaults cost three O(text) scans per call
        let mut start = 0;
        for (i, part) in self.0.split('\n').enumerate() {
            if i == line {
                // Include the terminating \n in the range when present; the last line extends to the buffer end
                let end = if start + part.len() < self.0.len() {
                    start + part.len() + 1
                } else {
                    self.0.len()
                };
                return start..end;
            }
            start += part.len() + 1;
        }
        // line >= line_count: the phantom line past the end
        self.0.len()..self.0.len()
    }
    fn offset_to_line(&self, offset: usize) -> usize {
        // offsets arrive from OUTSIDE (stored marks, host clicks) and can be
        // past the end or mid-character. Round13 #26 "fixed" this with
        // `offset.min(ceil_char_boundary(offset))` — which is the identity
        // for a mid-char offset (ceil ≥ offset) and still sliced mid-char.
        // Floor to the character START like the engine's own
        // floor_to_char_boundary does.
        let offset = offset.min(self.0.len());
        let offset = if self.0.is_char_boundary(offset) {
            offset
        } else {
            self.0.floor_char_boundary(offset)
        };
        self.0[..offset].split('\n').count() - 1
    }
    fn slice(&self, range: Range<usize>) -> String {
        self.0[range].to_owned()
    }
}

impl VimBufferMut for TckStrBuf {
    fn insert_text(&mut self, offset: usize, text: &str) -> bool {
        self.0.insert_str(offset, text);
        true
    }
    fn delete_range(&mut self, range: Range<usize>) -> bool {
        self.0.replace_range(range, "");
        true
    }
}

/// Read-only contract: with `'modifiable'` off, editing commands must be
/// refused with vim's E21.
///
/// Runs on the real key pipeline with the built-in recording host (`&mut impl
/// VimBufferMut` only supplies the buffer): after each editing keystroke is
/// fed in, assert that
///
/// - **the buffer is byte-for-byte unchanged** (a refusal must not leave a
///   half-applied change);
/// - **E21 arrives on the status channel** (not a silent no-op, not a panic);
/// - **subsequent motion / search / mark / yank still work** (the refusal does
///   not pollute the state machine: no operator residue, no count residue,
///   cursor unmoved).
///
/// Semantics taken from vim 9.1 evidence: under `:set nomodifiable`, `x` `dw`
/// `dd` `i` `o` `O` `C` `S` `~` `p` `J` `r` `u` `>` `:s` `:d` `:j` `:m` `:pu`
/// all report E21; `y` `yy` `m{a}` `q` `:q` do not.
/// Probe text: the contract's cursor/register assertions (`gg`→0, `w`→6,
/// `yiw`→"beta") are all hard-coded against it.
pub const READONLY_PROBE_TEXT: &str = "alpha\nbeta\ngamma\n";

///
/// Requires the buffer content to be exactly [`READONLY_PROBE_TEXT`] (the
/// asserted cursor/register values are hard-coded in the probe; a read-only
/// host cannot do the "empty buffer + write" two steps).
pub fn readonly_contract(buf: &mut impl VimBufferMut) -> Result<(), String> {
    let mut host = TckHostRo::default();
    let mut vim = VimState::new();
    vim.set_modifiable(false);
    let before = buf.slice(0..buf.len());
    if before != READONLY_PROBE_TEXT {
        return Err(format!(
            "readonly_contract requires the buffer content to be exactly {READONLY_PROBE_TEXT:?} (got {before:?})"
        ));
    }
    buffer_read_contract(buf)?;
    // The A-step shape: a plain `Ctx` over the caller's `VimBufferMut`, with
    // `'modifiable'` already off.
    readonly_probe(&mut vim, &before, &mut host, |vim, host, keys| {
        let mut ctx = Ctx { buf, host };
        for k in keys {
            vim.handle_key(&mut ctx, Key::parse(k));
        }
        buf.slice(0..buf.len())
    })
}

/// Read-only-pipeline variant of [`readonly_contract`]: the buffer is passed
/// as `&dyn VimBuffer`, **write primitives are unreachable at the type level**,
/// and the host only implements [`VimHostRo`].
///
/// This is the integration shape for diff-view / log-viewer style hosts
/// (`VimState::handle_key_ro` + `CtxRo`), and the strongest form of this
/// contract: the buffer being unmodified is not asserted — it is impossible by
/// construction.
///
/// Requires the buffer content to be exactly [`READONLY_PROBE_TEXT`] (the
/// read-only pipeline cannot obtain `&mut`, so nothing can be written).
pub fn readonly_pipeline_contract(buf: &dyn VimBuffer) -> Result<(), String> {
    let mut host = TckHostRo::default();
    let mut vim = VimState::new();
    let before = buf.slice(0..buf.len());
    if before != READONLY_PROBE_TEXT {
        return Err(format!(
            "readonly_pipeline_contract requires the buffer content to be exactly {READONLY_PROBE_TEXT:?} (got {before:?})"
        ));
    }
    buffer_read_contract(buf)?;
    // No `set_modifiable(false)` and no `&mut`: `handle_key_ro` forces
    // `'modifiable'` off for the call and the buffer reference is shared, so
    // a mutation is impossible BY CONSTRUCTION rather than by assertion.
    readonly_probe(&mut vim, &before, &mut host, |vim, host, keys| {
        let mut ctx = CtxRo { buf, host };
        for k in keys {
            vim.handle_key_ro(&mut ctx, Key::parse(k));
        }
        buf.slice(0..buf.len())
    })
}

/// Editing keystroke table: in read-only mode every entry must be refused
/// with E21 and leave the buffer unchanged.
/// All backed by vim 9.1 evidence (each sampled under `:set nomodifiable`).
///
/// **Refuse-on-entry vs refuse-on-first-char**: in vim `i`/`I`/`a`/`A`/`o`/`O`/
/// `gi`/`R` report E21 right on entry (mode stays Normal) — the engine matches.
/// But `gI`, visual `I`/`A` (`v`/`V`) and block `I`/`A` (`<C-v>`) in vim
/// **silently enter insert**, and only the first typed character reports E21 —
/// for these three families the engine chooses refusal at entry (stricter, the
/// safe direction). Deviation recorded here: refusal happens earlier + the
/// visual selection is preserved (better read-only pane UX), and under `CtxRo`
/// write primitives are unreachable at the type level, so there is no path
/// that "lets insert in and then corrupts the write".
const EDIT_KEYS: &[&[&str]] = &[
    &["x"],
    &["X"],
    &["d", "w"],
    &["d", "d"],
    &["d", "i", "w"],
    &["c", "w"],
    &["s"],
    &["S"],
    &["C"],
    &["D"],
    &["o"],
    &["O"],
    &["i"],
    &["a"],
    &["I"],
    &["A"],
    &["R"],
    &["~"],
    &["r", "Z"],
    &["J"],
    &["g", "J"],
    &["p"],
    &["P"],
    &["u"],
    &["<C-r>"],
    &["."],
    &["<C-a>"],
    &["<C-x>"],
    &[">", ">"],
    &["<", "<"],
    &["g", "u", "u"],
    &["g", "U", "U"],
    // Insert-family corners: vim silently enters insert and E21s on the first
    // char; the engine refuses at entry (see the note above)
    &["g", "I"],
    &["g", "I", "x"],
    &["v", "I"],
    &["v", "A"],
    &["V", "I"],
    &["V", "A"],
    &["<C-v>", "j", "l", "I"],
    &["<C-v>", "j", "l", "A"],
    // Ex commands (fed char by char, finished with Enter)
    &[":", "s", "/", "a", "/", "b", "/", "<CR>"],
    &[":", "%", "s", "/", "a", "/", "b", "/", "g", "<CR>"],
    &[":", "d", "<CR>"],
    &[":", "%", "d", "<CR>"],
    &[":", "j", "<CR>"],
    &[":", "1", "m", "2", "<CR>"],
    &[":", "p", "u", "<CR>"],
    // Editing under visual mode
    &["v", "l", "d"],
    &["V", "d"],
    &["v", "l", "~"],
];

/// Viewing keystroke table: in read-only mode every entry must keep working
/// normally and **must not** produce E21.
/// `y`/`yy`/`m{a}`/`q`/`:q` all report no E21 in vim (same evidence batch).
const VIEW_KEYS: &[&[&str]] = &[
    &["g", "g"],
    &["w"],
    &["y", "i", "w"],
    &["y", "y"],
    &["m", "b"],
    &["V", "j", "y"],
    &["/", "g", "a", "m", "m", "a", "<CR>"],
    &["G"],
    &["<C-o>"],
    &["z", "z"],
];

fn readonly_probe(
    vim: &mut VimState,
    before: &str,
    host: &mut TckHostRo,
    // `run` feeds the keys and hands back the buffer text afterwards — one
    // closure so the two contracts can share the whole body while differing
    // in how they reach the buffer (`Ctx` vs `CtxRo`).
    mut run: impl FnMut(&mut VimState, &mut TckHostRo, &[&str]) -> String,
) -> Result<(), String> {
    for keys in EDIT_KEYS {
        run(vim, host, &["<Esc>"]);
        host.statuses.clear();
        let after = run(vim, host, keys);
        if after != before {
            return Err(format!(
                "{keys:?}: the buffer was modified ({before:?} → {after:?}) — edits in read-only mode must be refused whole"
            ));
        }
        if !host.statuses.iter().any(|s| s.contains("E21")) {
            return Err(format!(
                "{keys:?}: no E21 received (got {:?}) — refusal must be a bell + status message, not a silent no-op",
                host.statuses
            ));
        }
    }
    // The refusal does not pollute the state machine: viewing keystrokes keep working (motion / search / mark / yank)
    host.statuses.clear();
    run(vim, host, &["<Esc>"]);
    run(vim, host, &["g", "g"]);
    if vim.cursor.offset != 0 {
        return Err(format!(
            "cursor after the refusal + gg = {} (expected 0) — the state machine was polluted by E21",
            vim.cursor.offset
        ));
    }
    run(vim, host, &["w"]);
    if vim.cursor.offset != 6 {
        return Err(format!(
            "cursor after the refusal + w = {} (expected 6, start of \"beta\")",
            vim.cursor.offset
        ));
    }
    // yank still works: writes the register, and (with the `"+` prefix) the clipboard
    run(vim, host, &["y", "i", "w"]);
    let yanked = vim
        .regs()
        .get('"')
        .map(|r| r.text.clone())
        .unwrap_or_default();
    if yanked != "beta" {
        return Err(format!(
            "yiw in read-only mode yanked {yanked:?} (expected \"beta\")"
        ));
    }
    host.clipboard = None;
    run(vim, host, &["\"", "+", "y", "i", "w"]);
    let clipped = vim
        .regs()
        .get('+')
        .map(|r| r.text.clone())
        .unwrap_or_default();
    if clipped != "beta" {
        return Err(format!(
            "\"+yiw in read-only mode yanked {clipped:?} (expected \"beta\")"
        ));
    }
    if host.clipboard.as_deref() != Some("beta") {
        return Err(format!(
            "\"+y in read-only mode did not write the clipboard ({:?}) — viewing hosts must be able to copy too",
            host.clipboard
        ));
    }
    for keys in VIEW_KEYS {
        host.statuses.clear();
        run(vim, host, keys);
        if host.statuses.iter().any(|s| s.contains("E21")) {
            return Err(format!(
                "{keys:?}: viewing keystroke wrongly reported E21 ({:?})",
                host.statuses
            ));
        }
    }
    let after = run(vim, host, &[]);
    if after != before {
        return Err(format!(
            "viewing keystrokes modified the buffer ({before:?} → {after:?})"
        ));
    }
    Ok(())
}

/// Side-effect-free host: use when you only want the engine to drive the
/// buffer. The host's own undo, clipboard, highlight and similar semantics are
/// out of TCK scope and are covered by the host's own tests.
#[derive(Default)]
pub struct TckHost;

impl VimHost for TckHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, _: &[Range<usize>], _: Option<Range<usize>>) {}
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
}

/// Recording host for [`undo_group_closure_contract`]: the undo-group
/// protocol calls are logged for pairing, everything else is inert. The
/// contract is about the SIGNAL pairing (begin/end symmetry, closure
/// determinism), not undo data — `undo`/`redo` report an empty stack.
#[derive(Default)]
pub struct TckUndoHost {
    pub begins: Vec<u64>,
    pub ends: Vec<u64>,
}

impl VimHost for TckUndoHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, _: &[Range<usize>], _: Option<Range<usize>>) {}
    fn begin_undo_group(&mut self, id: u64, _cursor_offset: usize) {
        self.begins.push(id);
    }
    fn end_undo_group(&mut self, id: u64) {
        self.ends.push(id);
    }
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
}

/// Recording read-only host: minimal `VimHostRo` implementation + status/bell/
/// clipboard recording, for [`readonly_contract`] and
/// [`readonly_pipeline_contract`] to assert that "refusals are reported, not
/// silently swallowed". It also implements [`VimHost`] (all edit-side hooks
/// empty), so both pipelines (the A-step compatibility layer and the B-step
/// `handle_key_ro`) share one set of probes.
#[derive(Default)]
pub struct TckHostRo {
    pub statuses: Vec<String>,
    pub bells: usize,
    pub clipboard: Option<String>,
    pub highlights: Vec<Range<usize>>,
    pub current_highlight: Option<Range<usize>>,
    pub scrolled_to: Vec<usize>,
    pub close_requested: bool,
    pub close_forced: bool,
}

impl VimHostRo for TckHostRo {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, line: usize) {
        self.scrolled_to.push(line);
    }
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current_highlight = current;
    }
    fn changed(&mut self) {}
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
        self.close_forced = false;
    }
    fn request_close_forced(&mut self, forced: bool) {
        self.close_requested = true;
        self.close_forced = forced;
    }
}

impl VimHost for TckHostRo {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, line: usize) {
        self.scrolled_to.push(line);
    }
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current_highlight = current;
    }
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
        self.close_forced = false;
    }
    fn request_close_forced(&mut self, forced: bool) {
        self.close_requested = true;
        self.close_forced = forced;
    }
}

fn feed(vim: &mut VimState, buf: &mut dyn VimBufferMut, host: &mut dyn VimHost, keys: &str) {
    let mut ctx = Ctx { buf, host };
    for c in keys.chars() {
        vim.handle_key(&mut ctx, Key::char(c));
    }
}

/// The typing path of insert mode: printable characters deliberately return
/// `Unknown` in insert mode (the IME contract); text lands directly on the
/// buffer via `record_typed_text` + `insert_text_at_cursor` — same shape as
/// the integration layer's `place_text`.
fn type_text(vim: &mut VimState, buf: &mut dyn VimBufferMut, host: &mut dyn VimHost, text: &str) {
    let mut ctx = Ctx { buf, host };
    vim.record_typed_text(text);
    vim.insert_text_at_cursor(&mut ctx, text);
}

fn assert_content(buf: &impl VimBuffer, want: &str) -> Result<(), String> {
    buffer_read_contract(buf)?;
    let got = buf.slice(0..buf.len());
    if got != want {
        return Err(format!("content {got:?} ≠ expected {want:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counter-example: a non-last-line range without the \n — a common
    /// implementation mistake that the TCK must catch.
    struct NoNewlineBuf(pub String);

    impl VimBuffer for NoNewlineBuf {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn line_count(&self) -> usize {
            TckStrBuf(self.0.clone()).line_count()
        }
        fn char_at(&self, offset: usize) -> Option<char> {
            TckStrBuf(self.0.clone()).char_at(offset)
        }
        fn prev_char_offset(&self, offset: usize) -> Option<usize> {
            TckStrBuf(self.0.clone()).prev_char_offset(offset)
        }
        fn line_range(&self, line: usize) -> Range<usize> {
            let mut start = 0;
            for (i, part) in self.0.split('\n').enumerate() {
                if i == line {
                    return start..start + part.len(); // WRONG: drops the \n
                }
                start += part.len() + 1;
            }
            self.0.len()..self.0.len()
        }
        fn offset_to_line(&self, offset: usize) -> usize {
            TckStrBuf(self.0.clone()).offset_to_line(offset)
        }
        fn slice(&self, range: Range<usize>) -> String {
            self.0[range].to_owned()
        }
    }

    #[test]
    fn read_contract_accepts_reference_impl() {
        let buf = TckStrBuf("alpha\nbeta 中文\n\n尾行\n".into());
        buffer_read_contract(&buf).unwrap();
        let buf = TckStrBuf("中文 mix emoji 👨‍👩‍👧".into());
        buffer_read_contract(&buf).unwrap();
        let buf = TckStrBuf(String::new());
        buffer_read_contract(&buf).unwrap();
    }

    #[test]
    fn read_contract_catches_missing_newline_in_range() {
        let buf = NoNewlineBuf("alpha\nbeta".into());
        assert!(buffer_read_contract(&buf).is_err());
    }

    #[test]
    fn edit_contract_passes_reference_impl() {
        let mut buf = TckStrBuf::default();
        buffer_edit_contract(&mut buf).unwrap();
    }

    #[test]
    fn engine_smoke_passes_reference_impl() {
        let mut buf = TckStrBuf::default();
        let mut host = TckHost;
        engine_smoke_contract(&mut buf, &mut host).unwrap();
        assert_eq!(buf.0, "gammax\nlpha\nbeta\n");
    }

    #[test]
    fn undo_group_closure_passes_reference_impl() {
        let mut buf = TckStrBuf::default();
        undo_group_closure_contract(&mut buf).unwrap();
        assert_eq!(buf.0, "alpha\nbeta\n");
    }

    /// Minimal form of a read-only host: `impl VimBuffer` + a **zero-method**
    /// `VimBufferMut`. The contract must pass even on such a "cannot write
    /// anything" implementation.
    struct RoBuf(&'static str);

    impl VimBuffer for RoBuf {
        fn len(&self) -> usize {
            TckStrBuf(self.0.to_owned()).len()
        }
        fn line_count(&self) -> usize {
            TckStrBuf(self.0.to_owned()).line_count()
        }
        fn char_at(&self, offset: usize) -> Option<char> {
            TckStrBuf(self.0.to_owned()).char_at(offset)
        }
        fn prev_char_offset(&self, offset: usize) -> Option<usize> {
            TckStrBuf(self.0.to_owned()).prev_char_offset(offset)
        }
        fn line_range(&self, line: usize) -> Range<usize> {
            TckStrBuf(self.0.to_owned()).line_range(line)
        }
        fn offset_to_line(&self, offset: usize) -> usize {
            TckStrBuf(self.0.to_owned()).offset_to_line(offset)
        }
        fn slice(&self, range: Range<usize>) -> String {
            TckStrBuf(self.0.to_owned()).slice(range)
        }
    }

    impl VimBufferMut for RoBuf {}

    #[test]
    fn readonly_contract_passes_a_write_refusing_buffer() {
        // an EMPTY `impl VimBufferMut` — the whole point of P1-A
        let mut buf = RoBuf(READONLY_PROBE_TEXT);
        readonly_contract(&mut buf).unwrap();
        assert_eq!(buf.slice(0..buf.len()), READONLY_PROBE_TEXT);
    }

    #[test]
    fn readonly_pipeline_contract_passes_the_reference_impl() {
        let buf = TckStrBuf::from_text(READONLY_PROBE_TEXT);
        readonly_pipeline_contract(&buf).unwrap();
    }

    /// Safety net for the A step: the host forgot `set_modifiable(false)` and
    /// only the write primitives are refusing. In that case nothing still
    /// changes, and E21 must be reported (not a silent no-op).
    #[test]
    fn write_primitive_refusal_reports_e21_even_with_modifiable_on() {
        let mut buf = RoBuf(READONLY_PROBE_TEXT);
        let mut host = TckHostRo::default();
        let mut vim = VimState::new(); // 'modifiable' stays ON
        {
            let mut ctx = Ctx {
                buf: &mut buf,
                host: &mut host,
            };
            vim.handle_key(&mut ctx, Key::parse("x"));
        }
        assert_eq!(buf.slice(0..buf.len()), READONLY_PROBE_TEXT);
        assert!(
            host.statuses.iter().any(|s| s.contains("E21")),
            "when a write primitive refuses, E21 must be reported; got {:?}",
            host.statuses
        );
    }
}
