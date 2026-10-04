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
    ///
    /// A width-0 char at the landing spot (a leading combining mark —
    /// `"\u{0301}abc"`, or a mark trailing the indent) is skipped: it is a
    /// cluster continuation, invisible, and parking there would let `x`
    /// split the cluster. The first VISIBLE non-blank char is the target.
    fn first_non_blank(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let (indent, _) = self.line_indent(line);
        let end = self.line_end(line);
        let mut at = (start + indent).min(end);
        while at < end && self.char_at(at).is_some_and(|c| c == '\u{200D}' || char_display_width(c) == 0) {
            at += self.char_at(at).map_or(1, |c| c.len_utf8());
        }
        if at == end && end > start && self.char_at(end) == Some('\n') {
            // whitespace-only line: park on the last VISIBLE char — walk
            // back off any trailing continuation run first (a line of
            // spaces + a combining mark ends in a mark; parking there
            // would put the cursor mid-cluster)
            let mut p = end;
            while let Some(prev) = self.prev_char_offset(p) {
                if prev < start {
                    break;
                }
                match self.char_at(prev) {
                    Some(c) if c == '\u{200D}' || char_display_width(c) == 0 => p = prev,
                    _ => break,
                }
            }
            return self.prev_char_offset(p).unwrap_or(start).max(start);
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

/// The start of the grapheme cluster CONTAINING byte `offset` (`offset`
/// itself when it sits on a visible starter). A width-0 char at `offset`
/// walks back to its base — except across a `\n` (GB4): a mark at a line
/// start is its own cluster and stays put. This is the right landing rule
/// for a cursor parked on a byte that an edit just turned into a
/// continuation char (a pasted leading mark glues onto the char before it).
pub fn grapheme_cluster_start(buf: &dyn VimBuffer, offset: usize) -> usize {
    let mut s = offset;
    while let Some(c) = buf.char_at(s) {
        if c == '\u{200D}' || char_display_width(c) == 0 {
            match buf.prev_char_offset(s) {
                Some(p) if buf.char_at(p) != Some('\n') => s = p,
                _ => break,
            }
        } else {
            break;
        }
    }
    s
}

/// Clamp for CURSOR placement (normal/visual): like [`clamp_to_line_end`],
/// but when the result sits past the line's last character it steps back ONTO
/// that character. Vim's normal-mode cursor never rests past the last char —
/// neither on the `\n` of a non-empty line (where later `dw`/`diw` would
/// swallow the line break) nor on a final line's phantom end (`w` at the
/// buffer end must leave the cursor ON the last character, vim probe: `wx`
/// after a single-word line deletes it). Empty lines keep their start — there
/// is no character to sit on, the start IS the newline position.
///
/// The result is additionally snapped onto a GRAPHEME START: an edit can land
/// a width-0 continuation char exactly on the clamped offset (a visual-block
/// `p` pasting a leading combining mark glues it onto the char before it, a
/// `gJ` seam can expose a mark mid-line) — resting there would let `x` split
/// the cluster, so the cursor rests on the cluster's base instead (fuzz
/// round 25).
pub fn clamp_cursor(buf: &dyn VimBuffer, offset: usize) -> usize {
    let offset = clamp_to_line_end(buf, offset);
    let line = buf.offset_to_line(offset);
    let end = buf.line_end(line);
    let offset = if offset >= end && end > buf.line_start(line) {
        // step back onto the last GRAPHEME start, not the last char: on a
        // line ending in a cluster (`"e\u{0301}"`, ZWJ families) the last
        // CHAR is a continuation — a cursor parked there lets `x` split the
        // cluster (deleting only the mark, leaving a bare base)
        prev_grapheme_offset(buf, end).unwrap_or(end)
    } else {
        offset
    };
    // mid-line snap: the char AT the offset may itself be a continuation
    if matches!(
        buf.char_at(offset),
        Some(c) if c == '\u{200D}' || char_display_width(c) == 0
    ) {
        grapheme_cluster_start(buf, offset)
    } else {
        offset
    }
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
/// char into the same cluster (emoji families). A width-0 char whose LEFT
/// neighbor is `\n` is NOT glued — the newline is a hard boundary (UAX #29
/// GB4) and the mark forms its own cluster (mirror of
/// [`prev_grapheme_offset`]).
pub fn next_grapheme_offset(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    let mut o = buf.next_char_offset(offset)?;
    while let Some(c) = buf.char_at(o) {
        if c == '\u{200D}' {
            // skip the ZWJ *and* the character it joins, then keep scanning:
            // a family emoji is base-ZWJ-base-ZWJ-base. A newline is a hard
            // boundary on BOTH sides (UAX #29 GB4/GB5 outrank the ZWJ rules):
            // a ZWJ at a line start stands alone, and "…base ZWJ \n" ends
            // the cluster at the ZWJ (the joined char must not be the `\n`)
            if buf.prev_char_offset(o).and_then(|p| buf.char_at(p)) == Some('\n') {
                break;
            }
            let Some(after) = buf.next_char_offset(o) else {
                break;
            };
            if buf.char_at(after) == Some('\n') {
                o = after;
                break;
            }
            let Some(joined_end) = buf.next_char_offset(after) else {
                break;
            };
            o = joined_end;
        } else if char_display_width(c) == 0 {
            // a mark right after a newline stands alone (GB4)
            match buf.prev_char_offset(o) {
                Some(p) if buf.char_at(p) == Some('\n') => break,
                _ => o += c.len_utf8(),
            }
        } else {
            break;
        }
    }
    Some(o)
}

/// Number of grapheme clusters in `range` — clusters, not chars: a base
/// plus its combining marks is ONE unit (vim's visual `r` fills composed
/// characters, so `#`+VS16 takes one replacement char, not two).
pub fn grapheme_count(buf: &dyn VimBuffer, range: Range<usize>) -> usize {
    let mut n = 0usize;
    let mut o = range.start;
    while o < range.end {
        match next_grapheme_offset(buf, o) {
            Some(next) if next <= range.end => {
                n += 1;
                o = next;
            }
            _ => break,
        }
    }
    n
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
    let mut prev_char: Option<char> = None;
    for c in s.chars() {
        let at = o;
        o += c.len_utf8();
        let prev = prev_char;
        prev_char = Some(c);
        if c == '\u{200D}' || char_display_width(c) == 0 {
            // continuation char: the cluster keeps its current start — but a
            // newline behind it is a hard boundary (GB4): the mark stands
            // alone and starts its own cluster (mirror of
            // [`prev_grapheme_offset`])
            if prev == Some('\n') {
                start = at;
            }
            prev_zwj = c == '\u{200D}';
            continue;
        }
        // a starter glued by a preceding ZWJ belongs to the cluster behind —
        // a newline is never glued (GB4/GB5, mirror of
        // [`prev_grapheme_offset`])
        if !prev_zwj || c == '\n' {
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
/// The backward walk STOPS at a `\n`: a newline is a hard grapheme boundary
/// (UAX #29 GB4/GB5), so a width-0 char at a line start is its OWN cluster —
/// gluing it onto the previous line's `\n` made [`clamp_cursor`] and
/// `Motion::LineEnd` walk the cursor onto the PREVIOUS line's newline
/// (`"x\n\u{0301}"` + `w` to the buffer end parked at the `\n`, where `x`
/// joined the lines). Callers may still clamp to their own line with
/// `.max(line_start)`, but the function itself now never crosses one.
///
/// Checking the char BEFORE `s` instead (the original implementation) missed
/// trailing zero-width chars entirely: on `"e\u{0301}"` it returned the
/// mark's own offset — parking the cursor mid-cluster after `<Esc>` at line
/// end, where `x` then split the cluster (deleting only the mark).
pub fn prev_grapheme_offset(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    let mut s = buf.prev_char_offset(offset)?;
    while let Some(c) = buf.char_at(s) {
        if c == '\u{200D}' || char_display_width(c) == 0 {
            // s sits on a continuation char: the cluster starts further left
            // — unless that left neighbor is the newline (hard boundary):
            // the mark stands alone and s IS the cluster start
            match buf.prev_char_offset(s) {
                Some(p) if buf.char_at(p) != Some('\n') => s = p,
                _ => break,
            }
            continue;
        }
        // s is a starter; a ZWJ immediately before it glues it to the
        // cluster behind (mirror of the forward ZWJ handling) — but a
        // newline is never glued (GB4/GB5: the hard boundary outranks the
        // ZWJ join)
        match buf.prev_char_offset(s) {
            Some(p) if c != '\n' && buf.char_at(p) == Some('\u{200D}') => s = p,
            _ => break,
        }
    }
    Some(s)
}
