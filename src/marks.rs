//! Marks: user marks `a-z`, special marks (` ^ . < >), and jump helpers.

use crate::buffer::VimBuffer;
use crate::mode::VisualKind;
use std::collections::HashMap;
use std::ops::Range;

/// Local (buffer) marks.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    offsets: HashMap<char, usize>,
    /// Range + kind of the last visual selection (`< .. >`, `gv`/`'<`/`'>`).
    /// THE single source of truth: `gv` reads the kind, the edit funnels
    /// shift it like every other stored offset, and `:marks` lists it. (It
    /// used to exist twice — a kind-less pair here and a kind-bearing copy
    /// on the engine — and the copies drifted apart under byte-grid-redrawing
    /// edits, letting `gv` restore a mid-char anchor; fuzz round 8 caught it.)
    ///
    /// The WRITE side carries the vim parity subtlety: an operator must
    /// store the selection's PRE-edit bounds (stashed in
    /// `VimState::pending_visual_marks`, the same treatment the `c` path
    /// always had), because the op's own `adjust_delete` collapses a pair
    /// inside the deleted span onto the deletion point — `vlld` used to
    /// leave `gv` zero-width, and no undo could bring the selection back
    /// (vim's line/col marks survive the op and, after undo, the original
    /// text re-projects onto them; 9.1 probes).
    pub last_visual: Option<(usize, usize, VisualKind)>,
    /// Live `(anchor, cursor_end)` of the current visual selection, kept in
    /// sync by the engine (used for `'<`/`'>` inside visual mode).
    pub(crate) active_visual: Option<(usize, usize)>,
    /// Position of the last change (`.`).
    pub last_change: Option<usize>,
    /// Position where the last insert session ended (`^`).
    pub last_insert_exit: Option<usize>,
    /// Where the last jump STARTED (`''` / `` `` `` return here; every
    /// `record_jump` re-points it at the jump's origin).
    pub(crate) last_jump: Option<usize>,
    /// Marks destroyed by edits, per undo group (audit E4 — vim's undo
    /// restores the marks of deleted text together with the text). Each
    /// batch carries `(name, pre-edit offset)`; the newest batch is the one
    /// the next `u` restores. Entries ride the ordinary funnel shifts, so a
    /// second edit inside the same group keeps them addressable.
    pub(crate) deleted_marks_log: Vec<Vec<(char, usize)>>,
}

impl Marks {
    pub fn get(&self, name: char) -> Option<usize> {
        self.offsets.get(&name).copied()
    }

    /// All set marks as `(name, offset)`, sorted by name — for `:marks`
    /// style listings (the map itself stays private).
    pub fn items(&self) -> Vec<(char, usize)> {
        let mut v: Vec<(char, usize)> = self.offsets.iter().map(|(c, o)| (*c, *o)).collect();
        v.sort();
        v
    }

    /// Delete a mark (`:delmarks`); the specials (`'<'>gv` pair) are NOT
    /// deletable through this path — only named marks and `^`/`.`.
    pub fn remove(&mut self, name: char) {
        self.offsets.remove(&name);
    }

    pub fn set(&mut self, name: char, offset: usize) {
        if name.is_ascii_alphabetic() || matches!(name, '^' | '.' | '[' | ']') {
            self.offsets.insert(name, offset);
        }
        // a fresh set outranks any undo-pending resurrection of the same
        // mark: `dd ma u` must keep the NEW position, not the pre-delete one
        for batch in self.deleted_marks_log.iter_mut() {
            batch.retain(|(n, _)| *n != name);
        }
        self.deleted_marks_log.retain(|batch| !batch.is_empty());
    }

    /// Live bounds of the ACTIVE visual selection (anchor..cursor+1), if any.
    pub fn active_visual(&self) -> Option<(usize, usize)> {
        // set by the engine each time the selection changes; keeps resolve()
        // valid inside visual mode, before '< '/'> are written on exit
        self.active_visual
    }

    /// Resolve special names used by `` ` ``/`'` jumps: the visual marks,
    /// the jump-context marks (`''` / `` `` ``), the last change and the
    /// last insert exit — beyond the plain named marks. The visual marks
    /// come back floored onto the CURRENT text: the buffer may have changed
    /// since they were written (a host text swap between engine calls), and
    /// an unfloored read would hand jump/range math a mid-char offset.
    pub fn resolve(&self, name: char, buf: &dyn VimBuffer) -> Option<usize> {
        match name {
            '<' => self
                .last_visual
                .map(|(a, _, _)| crate::buffer::floor_to_char_boundary(buf, a)),
            // `'>` resolves to the LAST SELECTED CHARACTER, not the exclusive
            // end byte (audit E3 — vim 9.1: `vllly` then `` `> `` parks on the
            // final 'l'; a linewise selection parks on its last line's last
            // char, never on the newline). The stored `b` stays exclusive —
            // `gv` reads it raw — only this read side projects it.
            '>' => self.last_visual.map(|(a, b, kind)| {
                let b = crate::buffer::floor_to_char_boundary(buf, b);
                let b = if matches!(kind, VisualKind::Line)
                    && b > a
                    && buf.char_at(b - 1) == Some('\n')
                {
                    b - 1
                } else {
                    b
                };
                buf.prev_char_offset(b).unwrap_or(b)
            }),
            // `''` (linewise) and `` `` `` (exact) share the jump origin
            '\'' | '`' => self.last_jump,
            '.' => self.last_change.or_else(|| self.get('.')),
            '^' => self.last_insert_exit.or_else(|| self.get('^')),
            _ => self.get(name),
        }
    }

    /// Shift every stored mark after an insertion of `len` bytes at `at`.
    /// A mark exactly AT `at` stays: vim keeps it before the inserted text.
    pub fn adjust_insert(&mut self, at: usize, len: usize) {
        if len == 0 {
            return;
        }
        self.for_each_pos(|pos| {
            if *pos > at {
                *pos += len;
            }
        });
    }

    /// Shift marks around a deletion of `range`: marks past the end move
    /// back by its length, marks strictly inside land on the range start
    /// (a mark exactly at the start stays there).
    pub fn adjust_delete(&mut self, range: Range<usize>) {
        if range.start >= range.end {
            return;
        }
        let len = range.end - range.start;
        self.for_each_pos(|pos| {
            if *pos >= range.end {
                *pos -= len;
            } else if *pos > range.start {
                *pos = range.start;
            }
        });
    }

    /// Marks around a replacement (`gu`/`~`/`:s`): past-the-end marks shift
    /// by the length delta, marks inside the replaced range land on its start.
    pub fn adjust_replace(&mut self, range: Range<usize>, new_len: usize) {
        if range.start >= range.end {
            return;
        }
        let delta = new_len as isize - (range.end - range.start) as isize;
        // equal-length replacement (case operators, `~`): inner marks keep
        // their BYTE position untouched — the text at that byte changed but
        // the length didn't. (`refloor_stored_offsets` still floors any mark
        // that now sits mid-character after a same-length swap with wider
        // chars, so this pass intentionally does no boundary checking.)
        let preserve_inner = new_len == range.end - range.start;
        self.for_each_pos(move |pos| {
            if *pos >= range.end {
                *pos = (*pos as isize + delta).max(0) as usize;
            } else if *pos > range.start && !preserve_inner {
                *pos = range.start;
            }
        });
    }

    /// Visit every stored byte offset (named marks, the visual pair, the
    /// live selection, `.`/`^`/jump-origin). The engine's edit funnels and
    /// the post-edit clamp pass both run their shift/floor closures through
    /// this single walk.
    pub(crate) fn for_each_pos(&mut self, mut f: impl FnMut(&mut usize)) {
        for pos in self.offsets.values_mut() {
            f(pos);
        }
        if let Some((a, b, _)) = self.last_visual.as_mut() {
            f(a);
            f(b);
        }
        // the live selection rides along: edits DO happen while it is
        // active (an Ex command run from the visual `:` prompt), and an
        // unadjusted pair handed `parse_range` mid-character offsets
        // (fuzz round 13) — same treatment as `last_visual`
        if let Some((a, b)) = self.active_visual.as_mut() {
            f(a);
            f(b);
        }
        if let Some(p) = self.last_change.as_mut() {
            f(p);
        }
        if let Some(p) = self.last_insert_exit.as_mut() {
            f(p);
        }
        if let Some(p) = self.last_jump.as_mut() {
            f(p);
        }
        // NOTE: `deleted_marks_log` is deliberately NOT walked here — its
        // entries keep the PRE-EDIT offsets of the deletion that logged them
        // (the very same walk would collapse them onto the deletion point,
        // defeating the restore). The undo-side restore floors each position
        // onto the current text instead.
    }

    /// Record marks destroyed by the edit now running (called BEFORE the
    /// buffer change, with the pre-edit offsets — audit E4). The batch joins
    /// the current undo group; the next `u` restores them.
    pub(crate) fn log_deleted_marks(&mut self, destroyed: Vec<(char, usize)>) {
        if !destroyed.is_empty() {
            self.deleted_marks_log.push(destroyed);
        }
    }

    pub(crate) fn debug_log_len(&self) -> usize {
        self.deleted_marks_log.len()
    }

    /// The mark batch the next undo restores (newest non-empty batch —
    /// audit E4).
    pub(crate) fn take_restorable_marks(&mut self) -> Option<Vec<(char, usize)>> {
        while let Some(batch) = self.deleted_marks_log.pop() {
            if !batch.is_empty() {
                return Some(batch);
            }
        }
        None
    }

    /// Floor every stored offset onto the text AFTER a host-driven swap
    /// (undo/redo), where relative adjustment is impossible — the engine
    /// only sees the new text, so the best it can do is the nearest
    /// surviving char boundary.
    pub(crate) fn floor_all(&mut self, floor: &impl Fn(usize) -> usize) {
        self.for_each_pos(|pos| {
            *pos = floor(*pos);
        });
    }
}
