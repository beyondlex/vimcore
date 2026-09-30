//! Operators: applying delete/change/yank/indent/case to a span.
//!
//! This module owns vim's operator-range special cases:
//! - `w` as an operator target stops at the end of the last word moved over,
//!   so `dw` never joins lines.
//! - An exclusive motion ending in column 1 becomes inclusive at the end of
//!   the previous line (`d}` keeps the following blank line).
//! - Column 1 + start at/before first non-blank becomes linewise.

use crate::buffer::{clamp_cursor, VimBuffer};
use crate::mode::Mode;
use crate::motions::{Motion, MotionKind, MotionResult};
use crate::objects::{self, ObjectRange};
use crate::registers::RegisterKind;
use crate::state::{Ctx, InsertKind, VimState};
use crate::word;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    IndentLeft,
    IndentRight,
    Lowercase,
    Uppercase,
    ToggleCase,
    Format,
}

/// A concrete span to operate on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpSpan {
    pub start: usize,
    pub end: usize,
    pub linewise: bool,
}

/// Turn a motion result into an operator span, applying the special cases.
pub fn span_from_motion(
    vim: &VimState,
    buf: &dyn VimBuffer,
    motion: Motion,
    result: MotionResult,
) -> OpSpan {
    let start = vim.cursor.offset;
    let start_line = buf.offset_to_line(start);
    // a motion may land past a line's trailing newline (e.g. `w` at the
    // last word of a buffer lands in the phantom final line); anchor the
    // span to the line end so `dw` etc. never swallow the newline
    let target = crate::buffer::clamp_to_line_end(buf, result.offset);
    let target_line = buf.offset_to_line(target);

    if result.kind == MotionKind::Linewise {
        return OpSpan {
            start: buf.line_start(start_line.min(target_line)),
            end: buf.line_range(start_line.max(target_line)).end,
            linewise: true,
        };
    }

    let crossed_lines = target_line != start_line;

    // `w`/`W` operator special cases (each row verified against vim 9.1):
    // * landing in column 1 of the target line — from an EMPTY start line vim
    //   promotes the motion to LINEWISE over the covered line (`dw` on an
    //   empty line deletes the line). The promotion needs an empty line, not
    //   merely a blank one: on "   " vim's `dw` from col 0 deletes the three
    //   spaces and KEEPS the newline (["", …]) — the blank case falls to the
    //   single-`w` clamp below (vim 9.1 probes).
    // * a SINGLE-`w` crossing (count == 1, i.e. the landing is the first
    //   word start after the cursor — trailing blanks or the last word) is
    //   clamped to the end of the start line, so `dw`/`cw` never join lines.
    //   This must NOT rely on the column-1 rules: with an INDENTED next
    //   line the landing is not column 1, and without the clamp the span
    //   would swallow the newline and the indent.
    // * deeper crossings (count > 1 landing past the target line's first
    //   word) DO span the newline, like vim (`d3w` joins; from a blank-only
    //   line `d2w` removes the line's spaces together with the next line).
    if matches!(motion, Motion::WordStart { .. }) && crossed_lines {
        let big = matches!(motion, Motion::WordStart { big: true });
        if target == buf.line_start(target_line) {
            if buf.line_start(start_line) == buf.line_end(start_line) {
                return OpSpan {
                    start: buf.line_start(start_line),
                    end: buf.line_range(target_line - 1).end,
                    linewise: true,
                };
            }
            return OpSpan {
                start,
                end: buf.line_end(start_line),
                linewise: false,
            };
        }
        if word::next_word_start(buf, start, big) == result.offset {
            return OpSpan {
                start,
                end: buf.line_end(start_line),
                linewise: false,
            };
        }
        // multi-`w` crossing past the next line's first word: fall through
        // — the span covers the newline, like vim
    }

    if result.kind == MotionKind::Exclusive
        && crossed_lines
        && (target == buf.line_start(target_line) || result.offset >= buf.len())
    {
        // a landing PAST the buffer end (the count ran off the last word) is
        // column 1 of vim's phantom line after the trailing newline: the same
        // linewise promotion applies (`d2w` from a blank-only line removes
        // the last line INCLUDING its newline — vim 9.1 probe `d2w@0` on
        // "AAAA / \"   \" / BBBB\\n" leaves ["AAAA"], not ["AAAA", ""]).
        //
        // The column-1 rules cover BOTH crossing directions: `w` lands at the
        // target line's start going DOWN, `b`/`B` from a line start go UP.
        // vim 9.1 probes: `db` at line start deletes the PREVIOUS line
        // linewise (['abc','def'] → ['def']), `dB` on an indented line ditto.
        let phantom = result.offset >= buf.len();
        let first_line = start_line.min(target_line);
        let last_covered = start_line.max(target_line) - 1;
        let range_end = if phantom {
            buf.len()
        } else {
            // the line ADJACENT to the column-1 landing keeps the span from
            // covering the landing line itself
            buf.line_range(last_covered).end
        };
        if start <= buf.first_non_blank(start_line) {
            // exclusive + column 1 + started at/before first non-blank:
            // becomes linewise over the lines fully covered
            return OpSpan {
                start: buf.line_start(first_line),
                end: range_end,
                linewise: true,
            };
        }
        // exclusive + column 1, NOT promoted: the end moves to the last
        // CHARACTER of the adjacent line, and the motion becomes inclusive —
        // i.e. the span ends AT the line end, so the newline itself survives.
        // Including it (`line_end + 1`) made `dw` over trailing blanks join
        // two lines, which vim never does (verified against vim 9.1). Only
        // the downward direction reaches this for `w`; the upward `b` family
        // always promotes (crossing up implies the cursor is at the line
        // start, i.e. at/before its first non-blank) — guard anyway.
        if target_line > start_line {
            return OpSpan {
                start,
                end: buf.line_end(target_line - 1),
                linewise: false,
            };
        }
    }

    let (lo, hi) = if target >= start {
        (start, target)
    } else {
        (target, start)
    };
    let end = match result.kind {
        MotionKind::Inclusive => hi + buf.char_at(hi).map(|c| c.len_utf8()).unwrap_or(0),
        _ => hi,
    };
    OpSpan {
        start: lo,
        end,
        linewise: false,
    }
}

/// Span for a text object.
pub fn span_from_object(_buf: &dyn VimBuffer, object: ObjectRange) -> OpSpan {
    OpSpan {
        start: object.start,
        end: object.end,
        linewise: object.linewise,
    }
}

/// The last LINE a span touches. `span.end - 1` is only a char boundary when
/// the span does not end inside a multi-byte char — a linewise span over a
/// final line with no trailing newline (`"中"`: end-1 sits inside 中) needs a
/// floored probe, or hosts whose `offset_to_line` checks boundaries panic.
pub(crate) fn last_line_of_span(buf: &dyn VimBuffer, span: &OpSpan) -> usize {
    let mut probe = span.end.max(span.start.saturating_add(1)) - 1;
    while probe > span.start && buf.char_at(probe).is_none() {
        probe -= 1;
    }
    buf.offset_to_line(probe)
}

/// Resolve a text object against the cursor.
pub fn object_span(
    vim: &VimState,
    buf: &dyn VimBuffer,
    object: objects::TextObject,
) -> Option<OpSpan> {
    let range = objects::range(buf, vim.cursor.offset, object)?;
    Some(span_from_object(buf, range))
}

/// Span for the current visual selection.
pub fn span_from_visual(vim: &VimState, buf: &dyn VimBuffer) -> Option<OpSpan> {
    let (anchor, cursor, kind) = vim.visual_selection()?;
    let (lo, hi) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
    Some(match kind {
        crate::mode::VisualKind::Line => OpSpan {
            start: buf.line_start(buf.offset_to_line(lo)),
            end: buf.line_range(buf.offset_to_line(hi)).end,
            linewise: true,
        },
        crate::mode::VisualKind::Block | crate::mode::VisualKind::Char => OpSpan {
            start: lo,
            end: hi + buf.char_at(hi).map(|c| c.len_utf8()).unwrap_or(0),
            linewise: false,
        },
    })
}

fn register_kind(span: &OpSpan) -> RegisterKind {
    if span.linewise {
        RegisterKind::Linewise
    } else {
        RegisterKind::Charwise
    }
}

/// Delete the span into the register; updates the cursor.
///
/// An EMPTY charwise span is a failed motion (e.g. a `dw` whose landing
/// equals the cursor): it deletes nothing AND stores nothing — vim keeps the
/// previous register content there (vim 9.1 probe: `ci(` on `()` leaves the
/// unnamed register untouched). Zero-width inner text objects rely on this
/// (`ci(` on `()` still enters insert).
pub fn delete_span(vim: &mut VimState, ctx: &mut Ctx, span: &OpSpan, register: Option<char>) {
    if !span.linewise && span.start >= span.end {
        return;
    }
    let text = ctx.buf.slice(span.start..span.end);
    vim.registers
        .store_delete(register, text, register_kind(span));
    vim.edit_delete(ctx, span.start..span.end);

    vim.cursor.offset = if span.linewise {
        let line = ctx
            .buf
            .offset_to_line(span.start)
            .min(ctx.buf.line_count() - 1);
        ctx.buf.first_non_blank(line)
    } else {
        clamp_cursor(ctx.buf, span.start)
    };
    vim.cursor.desired_col = None;
}

/// A rectangular selection in DISPLAY-column space with per-line byte
/// ranges (Visual Block). Rows run top to bottom over the selected lines;
/// lines shorter than the block yield empty ranges.
#[derive(Debug)]
pub struct BlockSpan {
    /// First display column (inclusive).
    pub col_lo: usize,
    /// End display column (exclusive).
    pub col_hi: usize,
    pub first_line: usize,
    /// One byte range per selected line, in line order.
    pub rows: Vec<Range<usize>>,
}

/// The block's byte range on ONE line: every char whose display-column
/// span intersects `[col_lo, col_hi)`. Empty range when the line ends
/// before `col_lo`.
pub fn block_row_range(
    buf: &dyn crate::buffer::VimBuffer,
    line: usize,
    col_lo: usize,
    col_hi: usize,
) -> Range<usize> {
    use crate::buffer::char_display_width;
    let start = buf.line_start(line);
    let end = buf.line_end(line);
    let mut o = start;
    let mut col = 0usize;
    let mut lo: Option<usize> = None;
    let mut hi = start;
    while o < end {
        let Some(c) = buf.char_at(o) else { break };
        let w = char_display_width(c);
        if col >= col_hi {
            break;
        }
        if col + w > col_lo {
            if lo.is_none() {
                lo = Some(o);
            }
            hi = o + c.len_utf8();
        }
        col += w;
        o += c.len_utf8();
    }
    match lo {
        Some(lo) => lo..hi,
        None => start..start,
    }
}

/// The block span of the current visual-block selection.
pub fn span_from_visual_block(
    vim: &VimState,
    buf: &dyn crate::buffer::VimBuffer,
) -> Option<BlockSpan> {
    let (anchor, cursor, kind) = vim.visual_selection()?;
    if kind != crate::mode::VisualKind::Block {
        return None;
    }
    let a_col = crate::buffer::display_column(buf, anchor);
    let c_col = crate::buffer::display_column(buf, cursor);
    let (col_lo, col_hi) = if a_col <= c_col {
        (a_col, c_col)
    } else {
        (c_col, a_col)
    };
    let first_line = buf.offset_to_line(anchor.min(cursor));
    let last_line = buf.offset_to_line(anchor.max(cursor));
    // the cursor char is part of the block: exclusive end = corner col + 1
    let rows = (first_line..=last_line)
        .map(|line| block_row_range(buf, line, col_lo, col_hi + 1))
        .collect();
    Some(BlockSpan {
        col_lo,
        col_hi: col_hi + 1,
        first_line,
        rows,
    })
}

/// Yank the span into the register. An empty charwise span (failed motion)
/// stores nothing — the register keeps its previous content, like vim.
pub fn yank_span(vim: &mut VimState, ctx: &mut Ctx, span: &OpSpan, register: Option<char>) {
    if !span.linewise && span.start >= span.end {
        return;
    }
    let text = ctx.buf.slice(span.start..span.end);
    vim.registers
        .store_yank(register, text, register_kind(span));
}

/// Apply an operator to a span (operator-pending and visual paths).
pub fn apply(
    vim: &mut VimState,
    ctx: &mut Ctx,
    op: Operator,
    span: &OpSpan,
    register: Option<char>,
) {
    match op {
        Operator::Delete => delete_span(vim, ctx, span, register),
        Operator::Yank => yank_span(vim, ctx, span, register),
        Operator::Change => {
            // keep the indent of the first line when changing linewise
            let indent_text = if span.linewise {
                let first = ctx.buf.offset_to_line(span.start);
                let (indent, _) = ctx.buf.line_indent(first);
                ctx.buf
                    .slice(ctx.buf.line_start(first)..ctx.buf.line_start(first) + indent)
            } else {
                String::new()
            };
            let effective = if span.linewise {
                // `cc` clears the lines' contents but the lines themselves
                // survive: never consume the last newline of the span
                let last = last_line_of_span(ctx.buf, span);
                OpSpan {
                    start: span.start,
                    end: ctx.buf.line_end(last),
                    linewise: false,
                }
            } else {
                *span
            };
            delete_span(vim, ctx, &effective, register);
            // the TYPING position is the span start (plus restored indent) —
            // it may legitimately sit on the line end (`cw` after the last
            // word), where insert-mode offsets are allowed to be; only the
            // block cursor gets pulled off the `\n`
            let mut typing_at = effective.start;
            if !indent_text.is_empty() {
                let at = vim.cursor.offset.min(typing_at);
                vim.edit_insert(ctx, at, &indent_text);
                typing_at = at + indent_text.len();
            }
            vim.cursor.offset = typing_at;
            vim.begin_insert(InsertKind::Change);
        }
        Operator::IndentLeft | Operator::IndentRight => {
            let first = ctx.buf.offset_to_line(span.start);
            let last = last_line_of_span(ctx.buf, span);
            for line in first..=last {
                shift_line(vim, ctx, line, matches!(op, Operator::IndentRight));
            }
            vim.cursor.offset = ctx.buf.first_non_blank(first.min(ctx.buf.line_count() - 1));
            vim.cursor.desired_col = None;
        }
        Operator::Format => {
            let last_line = last_line_of_span(ctx.buf, span);
            format_lines(vim, ctx, span.start, last_line);
        }
        Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
            let text = ctx.buf.slice(span.start..span.end);
            // multi-char case mappings exist (ß ↔ SS, İ → i̇), so each char
            // maps to a String; edit_replace absorbs the changed byte length
            // (marks and the visual span shift with it through the wrapper)
            let mapped: String = text
                .chars()
                .map(|c| match op {
                    Operator::Lowercase => c.to_lowercase().collect::<String>(),
                    Operator::Uppercase => c.to_uppercase().collect::<String>(),
                    _ => toggle_case(c),
                })
                .collect();
            vim.edit_replace(ctx, span.start..span.end, &mapped);
            vim.cursor.offset = clamp_cursor(ctx.buf, span.start);
            vim.cursor.desired_col = None;
        }
    }
    // The final off-the-newline clamp is for BLOCK cursors only: an operator
    // that entered insert mode (`cw`/`cc`/…) leaves the cursor at the typing
    // position, where a line-end offset is legal (typing continues there).
    if !matches!(vim.mode, Mode::Insert | Mode::Replace) {
        vim.cursor.offset = clamp_cursor(ctx.buf, vim.cursor.offset);
    }
}

/// The `~` per-char case swap. Multi-char case mappings exist (ß ↔ SS), so
/// the result is a String, not a char.
pub fn toggle_case(c: char) -> String {
    if c.is_lowercase() {
        c.to_uppercase().collect()
    } else {
        c.to_lowercase().collect()
    }
}

/// Shift one line by `shiftwidth` left or right.
pub fn shift_line(vim: &mut VimState, ctx: &mut Ctx, line: usize, right: bool) {
    if line >= ctx.buf.line_count() {
        return;
    }
    let start = ctx.buf.line_start(line);
    let (indent, blank) = ctx.buf.line_indent(line);
    if blank {
        return;
    }
    let sw = vim.options.shiftwidth.max(1);
    if right {
        let unit = if vim.options.expandtab {
            " ".repeat(sw)
        } else {
            "\t".to_owned()
        };
        vim.edit_insert(ctx, start, &unit);
    } else if indent > 0 {
        // remove up to `sw` columns of indent; a tab counts as a full unit
        let indent_end = start + indent;
        let mut removed = 0usize;
        let mut o = start;
        let mut cut = start;
        while o < indent_end && removed < sw {
            let Some(c) = ctx.buf.char_at(o) else { break };
            let width = if c == '\t' { sw } else { 1 };
            if removed + width > sw {
                break;
            }
            removed += width;
            o += c.len_utf8();
            cut = o;
        }
        vim.edit_delete(ctx, start..cut);
    }
}

/// `gq`/`gw`: reflow the span's lines to `textwidth` as one paragraph per
/// blank-line-separated group. The span's first-line indent becomes the
/// paragraph indent for all wrapped lines. `start` is a byte OFFSET (the
/// span start); `last_line` is the last affected LINE index.
pub fn format_lines(vim: &mut VimState, ctx: &mut Ctx, start: usize, last_line: usize) {
    let width = vim.options.textwidth.max(1);
    let span_end = ctx.buf.line_end(last_line);
    if start >= span_end {
        vim.cursor.offset = clamp_cursor(ctx.buf, start);
        return;
    }
    // `start` is a byte offset, but the loop below walks LINE indices: the
    // first line must be derived. (Feeding the raw offset into the loop made
    // `gqq` on any line past the first iterate an empty range and REPLACE the
    // whole paragraph with nothing.)
    let first_line = ctx.buf.offset_to_line(start);

    let mut out = String::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut indent = String::new();

    for line in first_line..=last_line {
        let ls = ctx.buf.line_start(line);
        let le = ctx.buf.line_end(line);
        let text = ctx.buf.slice(ls..le);
        let (ind, blank) = ctx.buf.line_indent(line);
        if blank || text.trim().is_empty() {
            flush_paragraph(&mut paragraph, &indent, width, &mut out);
            out.push('\n'); // keep the blank separator line
        } else {
            if paragraph.is_empty() {
                indent = text[..ind].to_owned();
            }
            for word in text[ind..].split_whitespace() {
                paragraph.push(word.to_owned());
            }
        }
    }
    flush_paragraph(&mut paragraph, &indent, width, &mut out);

    // each flushed paragraph ends with one newline; drop only the final
    // terminator (it belongs to the buffer structure, not the text)
    if out.ends_with('\n') {
        out.pop();
    }
    vim.edit_replace(ctx, start..span_end, &out);
    // vim: the cursor lands on the first non-blank of the LAST formatted
    // line (the reflow may change the line count, so recompute from the
    // replacement's end)
    let last_out = ctx
        .buf
        .offset_to_line((start + out.len()).min(ctx.buf.len()));
    vim.cursor.offset = ctx.buf.first_non_blank(last_out);
    vim.cursor.desired_col = None;
}

/// Wrap the queued words to `width` under `indent` and append them to `out`.
///
/// Contract: exactly one trailing `'\n'` is appended per call — callers
/// joining several paragraphs are responsible for dropping the final
/// terminator before writing back to the buffer.
///
/// Widths are DISPLAY columns (CJK chars cover two cells), matching how the
/// terminal renders the reflowed paragraph.
fn flush_paragraph(paragraph: &mut Vec<String>, indent: &str, width: usize, out: &mut String) {
    let width_of = |s: &str| -> usize { s.chars().map(crate::buffer::char_display_width).sum() };
    let indent_w = width_of(indent);
    let mut col = indent_w;
    let mut line = String::from(indent);
    for word in paragraph.drain(..) {
        let w = width_of(&word);
        if col > indent_w && col + w > width {
            out.push_str(line.trim_end());
            out.push('\n');
            line = String::from(indent);
            col = indent_w;
        }
        line.push_str(&word);
        line.push(' ');
        col += w + 1;
    }
    out.push_str(line.trim_end());
    out.push('\n');
}

/// `p` / `P`: paste a register. An unset register reports vim's feedback
/// (bell — vim shows E353: Nothing in register) instead of a silent no-op.
pub fn put(vim: &mut VimState, ctx: &mut Ctx, register: char, count: usize, after: bool) {
    let Some(data) = vim.registers.get_for_paste(register, ctx.host) else {
        ctx.host.bell();
        return;
    };
    // A linewise register always represents WHOLE lines, so an empty one is
    // one empty line (e.g. `yy` on the only line of an empty buffer): `p`
    // inserts an empty line rather than nothing (vim 9.1 parity). A charwise
    // register that is empty has nothing to place.
    let text = if data.text.is_empty() {
        if data.kind != RegisterKind::Linewise {
            return;
        }
        "\n".to_owned()
    } else {
        data.text.clone()
    };
    let count = clamped_repeat_count(text.len(), count.max(1));

    if data.kind == RegisterKind::Blockwise {
        put_blockwise(vim, ctx, &text, count, after);
        return;
    }
    let repeated = text.repeat(count);

    if data.kind == RegisterKind::Linewise {
        let line = ctx.buf.offset_to_line(vim.cursor.offset);
        let line_end = ctx.buf.line_end(line);
        let has_newline = ctx.buf.line_range(line).end > line_end;
        // `after` = `p` (below the current line) vs `P` (above). The old
        // code ignored the flag in this branch entirely: P pasted BELOW the
        // cursor line, invisible to tests that yanked the current line.
        let (insert_at, text) = if after && has_newline {
            // between this line and the next
            let at = ctx.buf.line_range(line).end;
            let text = if repeated.ends_with('\n') {
                repeated
            } else {
                format!("{repeated}\n")
            };
            (at, text)
        } else if after {
            // last line without trailing newline: open a new line for it
            (
                ctx.buf.len(),
                format!("\n{}", repeated.trim_end_matches('\n')),
            )
        } else {
            // above the current line: insert before its first byte
            (
                ctx.buf.line_start(line),
                format!("{}\n", repeated.trim_end_matches('\n')),
            )
        };
        vim.edit_insert(ctx, insert_at, &text);
        // vim leaves the cursor on the FIRST line of the put text, at its
        // first non-blank (verified against vim 9.1 for `p`/`P` with 1..4
        // pasted lines). After the insert, `insert_at` normally IS the start
        // of the first pasted line — except `p` opening a line past the
        // buffer's trailing newline, where insert_at lands ON the separator
        // `\n` (still the old last line) and the paste starts one byte later
        // (vim probe: `yy p` on the single line "abc" parks on line 2).
        let cursor_at = if after && !has_newline {
            insert_at + 1
        } else {
            insert_at
        };
        let cursor_line = ctx
            .buf
            .offset_to_line(cursor_at)
            .min(ctx.buf.line_count() - 1);
        vim.cursor.offset = ctx.buf.first_non_blank(cursor_line);
    } else {
        let mut at = vim.cursor.offset;
        if after && !ctx.buf.at_line_end(at) {
            at = ctx.buf.next_char_offset(at).unwrap_or(at);
        }
        vim.edit_insert(ctx, at, &repeated);
        // block cursor sits on the last pasted character: step back to the
        // START of the last char — `end - 1` is byte arithmetic and would
        // park the cursor inside a multi-byte character
        let end = at + repeated.len();
        vim.cursor.offset = clamp_cursor(ctx.buf, ctx.buf.prev_char_offset(end).unwrap_or(at));
    }
    vim.cursor.desired_col = None;
}

/// The paste-side guard of [`crate::registers::clamped_repeat`]: how many
/// repeats of `per`-byte text stay under the paste byte ceiling.
pub(crate) fn clamped_repeat_count(per: usize, count: usize) -> usize {
    const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;
    count.min((MAX_PASTE_BYTES / per.max(1)).max(1))
}

/// Blockwise `p`/`P` from a block register (normal mode). Semantics probed
/// against vim 9.1: the FIRST row lands at the target column on the cursor
/// line (`p`: one column right of the cursor char; `P`: at it, short lines
/// padded with spaces), and every following row becomes a NEW line inserted
/// below the cursor line, padded out to the same column — existing lines are
/// pushed down, never merged into. The cursor sits on the first pasted
/// character. `count` repeats each row horizontally (vim's blockwise count).
fn put_blockwise(vim: &mut VimState, ctx: &mut Ctx, text: &str, count: usize, after: bool) {
    let rows: Vec<String> = text
        .trim_end_matches('\n')
        .split('\n')
        .map(|row| row.repeat(count))
        .collect();
    let cur_col = crate::buffer::display_column(ctx.buf, vim.cursor.offset);
    let col = if after { cur_col + 1 } else { cur_col };

    let line = ctx.buf.offset_to_line(vim.cursor.offset);
    let line_start = ctx.buf.line_start(line);
    let line_end = ctx.buf.line_end(line);
    // rows 2.. become new lines after the cursor line's content (before its
    // newline), each padded out to the column
    let mut suffix = String::new();
    for row in &rows[1..] {
        suffix.push('\n');
        for _ in 0..col {
            suffix.push(' ');
        }
        suffix.push_str(row);
    }
    if !suffix.is_empty() {
        vim.edit_insert(ctx, line_end, &suffix);
    }
    // first row: byte boundary at display column `col` on the cursor line;
    // short lines get padded with spaces up to the column first
    let mut at = line_start;
    let mut covered = 0usize;
    while covered < col && at < line_end {
        let Some(c) = ctx.buf.char_at(at) else { break };
        covered += crate::buffer::char_display_width(c);
        at += c.len_utf8();
    }
    let pad = col.saturating_sub(covered);
    if pad > 0 {
        vim.edit_insert(ctx, at, &" ".repeat(pad));
        at += pad;
    }
    vim.edit_insert(ctx, at, &rows[0]);
    // block cursor on the first pasted char (vim 9.1: `p` of a 2-row block
    // leaves the cursor at the insert column of the cursor line)
    vim.cursor.offset = clamp_cursor(ctx.buf, at);
    vim.cursor.desired_col = None;
}

/// `J` / `gJ`: join `count` lines (at least one join). `literal` = `gJ`
/// (no separator, keep the next line's indent).
pub fn join_lines(vim: &mut VimState, ctx: &mut Ctx, count: usize, literal: bool) {
    let joins = count.max(2) - 1;
    for _ in 0..joins {
        let line = ctx.buf.offset_to_line(vim.cursor.offset);
        if line + 1 >= ctx.buf.line_count() {
            break;
        }
        let join_at = ctx.buf.line_end(line);
        let next_start = ctx.buf.line_start(line + 1);

        if literal {
            vim.edit_delete(ctx, join_at..next_start);
        } else {
            let next_end = ctx.buf.line_end(line + 1);
            let next_content = ctx.buf.slice(next_start..next_end);
            let next_trimmed = next_content.trim_start();
            let next_indent_len = next_content.len() - next_trimmed.len();

            // no extra space when the line already ends with whitespace or
            // the next line starts with `)`
            let cur_tail = ctx.buf.slice(ctx.buf.line_start(line)..join_at);
            let separator = if cur_tail.ends_with(' ')
                || cur_tail.ends_with('\t')
                || next_trimmed.starts_with(')')
            {
                ""
            } else {
                " "
            };
            // MUST go through edit_replace (not the raw buffer): the wrapper
            // keeps marks, `last_visual` and the search-match generation in
            // sync with the shifted text, same as the `gJ` arm above.
            vim.edit_replace(ctx, join_at..next_start + next_indent_len, separator);
        }
        vim.cursor.offset = join_at;
    }
    vim.cursor.desired_col = None;
}

/// Advance over up to `count` grapheme clusters (emoji and combining-mark
/// sequences move atomically), never crossing `limit` — normally the line
/// end. Returns the new offset, which may be short of `count` steps.
fn advance_graphemes(buf: &dyn VimBuffer, offset: usize, count: usize, limit: usize) -> usize {
    let mut o = offset;
    for _ in 0..count {
        match crate::buffer::next_grapheme_offset(buf, o) {
            Some(next) if next <= limit => o = next,
            _ => break,
        }
    }
    o
}

/// [`advance_graphemes`] backwards, never crossing back over `limit`.
fn retreat_graphemes(buf: &dyn VimBuffer, offset: usize, count: usize, limit: usize) -> usize {
    let mut o = offset;
    for _ in 0..count {
        match crate::buffer::prev_grapheme_offset(buf, o) {
            Some(prev) if prev >= limit => o = prev,
            _ => break,
        }
    }
    o
}

/// `x` / `X`: delete `count` chars under/before the cursor.
pub fn delete_chars(vim: &mut VimState, ctx: &mut Ctx, count: usize, backward: bool) {
    let start = vim.cursor.offset;
    let line = ctx.buf.offset_to_line(start);
    let line_start = ctx.buf.line_start(line);
    let line_end = ctx.buf.line_end(line);
    let (lo, hi) = if backward {
        let lo = retreat_graphemes(ctx.buf, start, count, line_start);
        if lo == start {
            return;
        }
        (lo, start)
    } else {
        if start >= line_end {
            return;
        }
        (start, advance_graphemes(ctx.buf, start, count, line_end))
    };
    delete_span(
        vim,
        ctx,
        &OpSpan {
            start: lo,
            end: hi,
            linewise: false,
        },
        None,
    );
}

/// `r{char}`: replace `count` chars with `char` (never crosses the line end).
pub fn replace_chars(vim: &mut VimState, ctx: &mut Ctx, ch: char, count: usize) {
    let count = count.max(1);
    let start = vim.cursor.offset;
    let line_end = ctx.buf.line_end(ctx.buf.offset_to_line(start));
    let mut replacements = String::new();
    let mut o = start;
    for _ in 0..count {
        if o >= line_end {
            // `3rx` with only two chars left on the line: vim cancels the
            // whole replace instead of partially filling it
            return;
        }
        replacements.push(ch);
        let next = advance_graphemes(ctx.buf, o, 1, line_end);
        if next == o {
            break;
        }
        o = next;
    }
    vim.edit_replace(ctx, start..o, &replacements);
    vim.cursor.offset = clamp_cursor(ctx.buf, start + replacements.len() - ch.len_utf8());
    vim.cursor.desired_col = None;
}

/// Visual `r{char}`: replace every selected character with `char` (vim 9.1
/// probes: charwise replaces the covered chars; linewise fills each selected
/// line to its own CHAR count — `中文ab` becomes `----`; blockwise fills each
/// row's covered span, short rows keep their tail). The cursor lands on the
/// selection start.
pub fn visual_replace(vim: &mut VimState, ctx: &mut Ctx, ch: char) {
    use crate::mode::VisualKind;
    let kind = match vim.mode {
        crate::mode::Mode::Visual { kind } => kind,
        _ => return,
    };
    let replacement = ch.to_string();
    match kind {
        VisualKind::Block => {
            let Some(block) = span_from_visual_block(vim, ctx.buf) else {
                return;
            };
            // bottom-up so earlier row offsets stay valid
            for range in block.rows.iter().rev() {
                if range.is_empty() {
                    continue;
                }
                let n = ctx.buf.slice(range.clone()).chars().count();
                vim.edit_replace(ctx, range.clone(), &replacement.repeat(n));
            }
            vim.cursor.offset =
                clamp_cursor(ctx.buf, block.rows.first().map(|r| r.start).unwrap_or(0));
        }
        VisualKind::Line => {
            let Some(span) = span_from_visual(vim, ctx.buf) else {
                return;
            };
            let first = ctx.buf.offset_to_line(span.start);
            let last = last_line_of_span(ctx.buf, &span);
            for line in (first..=last).rev() {
                let ls = ctx.buf.line_start(line);
                let le = ctx.buf.line_end(line);
                let n = ctx.buf.slice(ls..le).chars().count();
                vim.edit_replace(ctx, ls..le, &replacement.repeat(n));
            }
            vim.cursor.offset = ctx.buf.first_non_blank(first.min(ctx.buf.line_count() - 1));
        }
        VisualKind::Char => {
            let Some(span) = span_from_visual(vim, ctx.buf) else {
                return;
            };
            let n = ctx.buf.slice(span.start..span.end).chars().count();
            vim.edit_replace(ctx, span.start..span.end, &replacement.repeat(n));
            vim.cursor.offset = clamp_cursor(ctx.buf, span.start);
        }
    }
    vim.cursor.desired_col = None;
}

/// `~`: toggle case of `count` chars, cursor ends on the last one.
pub fn toggle_chars(vim: &mut VimState, ctx: &mut Ctx, count: usize) {
    let count = count.max(1);
    let start = vim.cursor.offset;
    let line_end = ctx.buf.line_end(ctx.buf.offset_to_line(start));
    let mut mapped = String::new();
    let mut o = start;
    for _ in 0..count {
        if o >= line_end {
            break;
        }
        let Some(c) = ctx.buf.char_at(o) else { break };
        mapped.push_str(&crate::ops::toggle_case(c));
        let next = advance_graphemes(ctx.buf, o, 1, line_end);
        if next == o {
            break;
        }
        o = next;
    }
    if mapped.is_empty() {
        return;
    }
    // the replaced range must be the CONSUMED byte span (start..o), not
    // `start + mapped.len()`: a multi-char case mapping (ß → SS) changes the
    // byte length, and deriving the range from the replacement would cut the
    // span short and leave trailing original bytes behind
    vim.edit_replace(ctx, start..o, &mapped);
    // vim's `~` moves right past the last toggled char (staying on it only
    // at line end); byte arithmetic uses the consumed span so multi-byte
    // chars don't park the cursor mid-character
    vim.cursor.offset = clamp_cursor(ctx.buf, start + mapped.len());
    vim.cursor.desired_col = None;
}
