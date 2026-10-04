//! The host-facing text model.
//!
//! The engine never owns the buffer: the host implements [`VimBuffer`] (and
//! [`VimBufferMut`] if it wants the engine to edit). All offsets are **UTF-8
//! byte offsets**; conversion to/from UTF-16 (the gpui `InputHandler`
//! coordinate space) happens once, at the integration boundary.

use std::ops::Range;

/// Read-only text access with line semantics.
///
/// Lines are 0-based. A line's range extends over its terminating `\n` when
/// present. The final line of a buffer typically has no trailing newline.
/// An empty buffer still has exactly one line.
pub trait VimBuffer {
    /// Total length in bytes.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Number of lines (>= 1).
    fn line_count(&self) -> usize;
    /// The character that starts at byte `offset`, if `offset` is a char
    /// boundary inside the buffer.
    fn char_at(&self, offset: usize) -> Option<char>;
    /// The byte offset of the character that *ends* at `offset`
    /// (i.e. the previous char boundary), if any.
    fn prev_char_offset(&self, offset: usize) -> Option<usize>;
    /// `[start, end)` covering `line` including its newline, if any.
    fn line_range(&self, line: usize) -> Range<usize>;
    /// The 0-based line containing byte `offset`.
    fn offset_to_line(&self, offset: usize) -> usize;
    /// Copy a byte range out of the buffer.
    fn slice(&self, range: Range<usize>) -> String;

    // ---- provided helpers -------------------------------------------------

    fn line_start(&self, line: usize) -> usize {
        self.line_range(line).start
    }

    /// Offset of the line's end, *before* the newline.
    fn line_end(&self, line: usize) -> usize {
        let range = self.line_range(line);
        if range.end > range.start && self.char_at(range.end - 1) == Some('\n') {
            range.end - '\n'.len_utf8()
        } else {
            range.end
        }
    }

    fn line_content(&self, line: usize) -> String {
        let range = self.line_range(line);
        let end = self.line_end(line);
        self.slice(range.start..end)
    }

    fn next_char_offset(&self, offset: usize) -> Option<usize> {
        let c = self.char_at(offset)?;
        Some(offset + c.len_utf8())
    }

    /// Byte length of the leading indentation of `line`, and whether the line
    /// is blank.
    fn line_indent(&self, line: usize) -> (usize, bool) {
        let start = self.line_start(line);
        let end = self.line_end(line);
        let mut offset = start;
        while offset < end {
            match self.char_at(offset) {
                Some(c @ (' ' | '\t')) => offset += c.len_utf8(),
                _ => return (offset - start, false),
            }
        }
        (offset - start, true)
    }

    /// Offset of the first non-blank character of `line`. On a whitespace
    /// ONLY line there is none — vim parks the cursor on the LAST character
    /// instead (vim 9.1: `^` on `"   "` sits on col 3), never on the newline:
    /// a cursor on the `\n` would make later `dw`/`diw` swallow the line
    /// break. An empty line keeps its start.
    fn first_non_blank(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let (indent, _) = self.line_indent(line);
        let end = self.line_end(line);
        let at = (start + indent).min(end);
        if at == end && end > start && self.char_at(end) == Some('\n') {
            return self.prev_char_offset(end).unwrap_or(start);
        }
        at
    }

    /// Is `offset` at (or past) the end of its line, before the newline?
    fn at_line_end(&self, offset: usize) -> bool {
        let line = self.offset_to_line(offset);
        offset >= self.line_end(line)
    }

    fn line_is_blank(&self, line: usize) -> bool {
        self.line_indent(line).1
    }
}

/// Mutating text access. Every engine edit funnels through the two required
/// methods, which keeps the undo transaction boundary visible to the host.
pub trait VimBufferMut: VimBuffer {
    fn insert_text(&mut self, offset: usize, text: &str);
    fn delete_range(&mut self, range: Range<usize>);

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.delete_range(range.clone());
        self.insert_text(range.start, text);
    }
}

// Clamp helpers: engine offsets are always char boundaries that stay inside
// their line — every offset arriving from OUTSIDE the engine (a host click,
// a stored mark after a length-preserving replace, a stale jumplist entry)
// passes through these before the cursor or `offset_to_line` sees it.

/// Never park the cursor inside the `\n`.
pub fn clamp_to_line_end(buf: &dyn VimBuffer, offset: usize) -> usize {
    let line = buf.offset_to_line(offset);
    offset.min(buf.line_end(line))
}

/// Floor `offset` to the nearest char boundary at or below it. Equal-length
/// replacements can move inner byte boundaries, leaving stored offsets
/// (marks, changelist, jumplist) pointing mid-character.
///
/// `offset == buf.len()` is always a boundary (the buffer end) and returns
/// unchanged — `char_at(len)` is `None` only because no char STARTS there,
/// not because the offset is mid-char. Flooring it to the last char's start
/// silently SHRANK host-supplied replace ranges that end at the buffer end
/// (`replace_range(5..7, …)` on a 7-byte buffer edited only 5..6, leaving
/// the last char behind — the IME commit-at-EOF corruption this guard
/// fixed).
pub fn floor_to_char_boundary(buf: &dyn VimBuffer, offset: usize) -> usize {
    let mut offset = offset.min(buf.len());
    if offset == buf.len() {
        return offset;
    }
    while offset > 0 && buf.char_at(offset).is_none() {
        offset -= 1;
    }
    offset
}

/// Clamp for CURSOR placement (normal/visual): like [`clamp_to_line_end`],
/// but when the result sits past the line's last character it steps back ONTO
/// that character. Vim's normal-mode cursor never rests past the last char —
/// neither on the `\n` of a non-empty line (where later `dw`/`diw` would
/// swallow the line break) nor on a final line's phantom end (`w` at the
/// buffer end must leave the cursor ON the last character, vim probe: `wx`
/// after a single-word line deletes it). Empty lines keep their start — there
/// is no character to sit on, the start IS the newline position.
pub fn clamp_cursor(buf: &dyn VimBuffer, offset: usize) -> usize {
    let offset = clamp_to_line_end(buf, offset);
    let line = buf.offset_to_line(offset);
    let end = buf.line_end(line);
    if offset >= end && end > buf.line_start(line) {
        // step back onto the last GRAPHEME start, not the last char: on a
        // line ending in a cluster (`"e\u{0301}"`, ZWJ families) the last
        // CHAR is a continuation — a cursor parked there lets `x` split the
        // cluster (deleting only the mark, leaving a bare base)
        return prev_grapheme_offset(buf, end).unwrap_or(end);
    }
    offset
}

// ---- display columns & graphemes (wide-char aware) ---------------------------
//
// The engine's cursor and columns are BYTE offsets, but "column" semantics
// (j/k preservation, `|`) are DISPLAY columns: CJK chars are 2 cells,
// combining marks are 0. Grapheme boundaries keep multi-char clusters
// (emoji families, accents) atomic under h/l/x/r/~.

/// Display width of a char: 0 for combining marks, 2 for East Asian
/// wide/fullwidth, 1 otherwise. Control chars count as 1 for bookkeeping.
pub fn char_display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)
}

/// The display column of `offset` within its line.
pub fn display_column(buf: &dyn VimBuffer, offset: usize) -> usize {
    let start = buf.line_start(buf.offset_to_line(offset));
    let mut column = 0;
    let mut o = start;
    while o < offset {
        match buf.char_at(o) {
            Some(c) => {
                column += char_display_width(c);
                o += c.len_utf8();
            }
            None => break,
        }
    }
    column
}

/// The byte offset of the char covering display column `col` in `line`
/// (the line end when `col` is at or past the last char).
pub fn offset_for_display_column(buf: &dyn VimBuffer, line: usize, col: usize) -> usize {
    let start = buf.line_start(line);
    let end = buf.line_end(line);
    let mut covered = 0usize;
    let mut o = start;
    while o < end {
        match buf.char_at(o) {
            Some(c) => {
                let w = char_display_width(c);
                if covered + w > col {
                    break;
                }
                covered += w;
                o += c.len_utf8();
            }
            None => break,
        }
    }
    if o >= end && end > start {
        // past the last char: vim clamps `j`/`|` onto the last character —
        // the last GRAPHEME start (see clamp_cursor: a continuation char is
        // not a valid resting spot)
        return prev_grapheme_offset(buf, end).unwrap_or(start);
    }
    o
}

/// Next grapheme boundary: trailing width-0 chars (combining marks,
/// variation selectors) attach to the base char, and a ZWJ glues the next
/// char into the same cluster (emoji families).
pub fn next_grapheme_offset(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    let mut o = buf.next_char_offset(offset)?;
    while let Some(c) = buf.char_at(o) {
        if c == '\u{200D}' {
            // skip the ZWJ *and* the character it joins, then keep scanning:
            // a family emoji is base-ZWJ-base-ZWJ-base
            let Some(after) = buf.next_char_offset(o) else {
                break;
            };
            let Some(joined_end) = buf.next_char_offset(after) else {
                break;
            };
            o = joined_end;
        } else if char_display_width(c) == 0 {
            o += c.len_utf8();
        } else {
            break;
        }
    }
    Some(o)
}

/// Start of the last grapheme cluster in `s` (`None` when empty) — the
/// `&str` mirror of [`prev_grapheme_offset`], for byte records kept outside
/// a buffer (the block-insert session's typed text) that must shrink by the
/// same span a cluster-aware backspace removed from the buffer.
pub fn last_grapheme_start(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut start = 0usize;
    let mut o = 0usize;
    let mut prev_zwj = false;
    for c in s.chars() {
        let at = o;
        o += c.len_utf8();
        if c == '\u{200D}' || char_display_width(c) == 0 {
            // continuation char: the cluster keeps its current start
            prev_zwj = c == '\u{200D}';
            continue;
        }
        // a starter glued by a preceding ZWJ belongs to the cluster behind
        if !prev_zwj {
            start = at;
        }
        prev_zwj = false;
    }
    Some(start)
}

/// Previous grapheme boundary (the mirror of [`next_grapheme_offset`]).
///
/// The scan must decide whether the candidate `s` (a char start) is a cluster
/// START or a cluster CONTINUATION. The char AT `s` answers that: a width-0
/// char (combining mark / variation selector) or a ZWJ attaches backward, so
/// the cluster starts further left. A starter whose immediately preceding
/// char is a ZWJ is glued to the cluster behind it (emoji families:
/// base-ZWJ-base-ZWJ-base resolves back to the first base).
///
/// Checking the char BEFORE `s` instead (the original implementation) missed
/// trailing zero-width chars entirely: on `"e\u{0301}"` it returned the
/// mark's own offset — parking the cursor mid-cluster after `<Esc>` at line
/// end, where `x` then split the cluster (deleting only the mark).
pub fn prev_grapheme_offset(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    let mut s = buf.prev_char_offset(offset)?;
    loop {
        let Some(c) = buf.char_at(s) else { break };
        if c == '\u{200D}' || char_display_width(c) == 0 {
            // s sits on a continuation char: the cluster starts further left
            match buf.prev_char_offset(s) {
                Some(p) => s = p,
                None => break,
            }
            continue;
        }
        // s is a starter; a ZWJ immediately before it glues it to the
        // cluster behind (mirror of the forward ZWJ handling)
        match buf.prev_char_offset(s) {
            Some(p) if buf.char_at(p) == Some('\u{200D}') => s = p,
            _ => break,
        }
    }
    Some(s)
}
