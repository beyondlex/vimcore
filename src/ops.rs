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
    /// visual `D`: blockwise — from the block column to each row's end;
    /// charwise/linewise — from the span start to that line's end
    /// (`:h v_D`; audit G2)
    DeleteToEnd,
    Change,
    Yank,
    IndentLeft,
    IndentRight,
    Lowercase,
    Uppercase,
    ToggleCase,
    Format,
    /// `!{motion}{cmd}` — pipe the lines through a shell command (audit B3;
    /// the engine spawns `sh -c` itself, exactly like vim does)
    Filter,
    /// `={motion}` — reindent by the built-in rules (audit B4; without a
    /// filetype only the `'lisp'` rule is deterministic, so that is the
    /// one modeled)
    Reindent,
    /// block `C`/`S`: change each highlighted row from the block's left
    /// column to that row's END — one line per highlighted line survives
    /// (`:h v_b_C`, audit C10; the LinewiseOp spelling collapsed the rows)
    BlockChangeToEnd,
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
    count: usize,
    is_change: bool,
) -> OpSpan {
    let start = vim.cursor.offset;
    let start_line = buf.offset_to_line(start);
    // a till motion parks the cursor one char short of the match; the
    // operator span reaches the MATCH itself (vim: `dt3` on `a1b2c3d3e`
    // deletes `a1b2c` — through the char in front of the target — while a
    // bare `t` parks the cursor on the stop; probe 9.1, round 14)
    let span_target = result.till_target.unwrap_or(result.offset);
    // a motion may land past a line's trailing newline (e.g. `w` at the
    // last word of a buffer lands in the phantom final line); anchor the
    // span to the line end so `dw` etc. never swallow the newline
    let target = crate::buffer::clamp_to_line_end(buf, span_target);
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
            // The column-1 clamp is vim's SINGLE-`w` rule (`dw` never joins
            // lines). For CHANGE + a count of 2+ the span keeps going: `cw`
            // behaves like `ce` (`:h cw`), so `2cw` from 'b' on "ab \n\ncd"
            // changes "b \n\ncd" — through the END of the landing word
            // (audit B1). A DELETE keeps the clamp (vim 9.1: `d2w` on
            // "foo x\nbar" leaves the emptied line behind)
            if count > 1 && is_change {
                // next_word_end reports the INCLUSIVE last char
                let word_end = word::next_word_end(buf, target, big) + 1;
                return OpSpan {
                    start,
                    end: word_end.max(buf.line_end(start_line)),
                    linewise: false,
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
            // the phantom landing (past the buffer end) is column 1 of the
            // line AFTER the last one: rule (a)'s "end of the previous line"
            // is the LAST line's end — the clamp pulled `target_line` onto
            // that last line, so `target_line - 1` was the CURSOR's own line
            // and the span crossed the join point (`d}` from mid-line merged
            // two lines, audit B13)
            let end_line = if phantom {
                start_line.max(target_line)
            } else {
                target_line - 1
            };
            return OpSpan {
                start,
                end: buf.line_end(end_line),
                linewise: false,
            };
        }
    }
    if result.kind == MotionKind::Exclusive
        // nv_cursormark/nv_mark set vim's end_adjusted: rule (a) does NOT
        // apply to `` ` ``/`'` motions — `d`a` keeps its raw exclusive span
        // and JOINS the lines (oracle one two/three four; audit5 B-20's
        // upward extension below must respect the same exclusion)
        && !matches!(motion, Motion::MarkJump { .. })
        && target_line < start_line
        && start == buf.line_start(start_line)
    {
        // UPWARD exclusive whose span END (the cursor side) sits in column
        // 1: vim's rule (a) moves the end to the END of the previous line —
        // the newline survives and the lines never join (audit5 B-20:
        // `2Gdb` on "one two\nfoo" deletes "two" leaving "one \nfoo"; the
        // old comment claimed upward always promotes, which was only true
        // for the COLUMN-1 LANDING shape above). An EMPTY adjacent line
        // contributes its own start only: line_end of "" is its newline
        // position and stays outside the span (audit5 B-3: `3Gy2b` yanks
        // "two\n", not "two\n\n")
        return OpSpan {
            start: target,
            end: buf.line_end(start_line - 1),
            linewise: false,
        };
    }

    let (lo, hi) = if target >= start {
        (start, target)
    } else {
        (target, start)
    };
    // vim's exclusive rule 3 (`:h exclusive`): an INCLUSIVE motion whose
    // start is at or before the first non-blank moves its start to the
    // first non-blank (audit5 B-12: `dg_` on "  ab  cd  " deletes only
    // "ab  cd" — the leading blanks survive with the trailing ones). Only
    // the downward/cursor-side shape adjusts here; the upward cursor side
    // is the span's hi and has no pinned oracle shape.
    let mut lo = lo;
    if result.kind == MotionKind::Inclusive
        && target >= start
        && start <= buf.first_non_blank(start_line)
    {
        // guards: an all-whitespace line has no content start (the fnb
        // probe clamps onto the LAST char — parity T6: dg_ on "   " deletes
        // 1..3 chars from the cursor), and a zero-width span has nothing to
        // move
        let fnb = buf.first_non_blank(start_line);
        if fnb < buf.line_end(start_line) && lo < hi {
            lo = lo.max(fnb);
        }
    }
    let end = match result.kind {
        MotionKind::Inclusive => {
            // an inclusive span must never swallow the newline itself —
            // `hi` pointing at `\n` would otherwise delete the line break;
            // vim's charwise operators stop at the last character
            if buf.char_at(hi) == Some('\n') {
                hi
            } else {
                hi + buf.char_at(hi).map(|c| c.len_utf8()).unwrap_or(0)
            }
        }
        _ => hi,
    };
    OpSpan {
        start: lo,
        end,
        linewise: false,
    }
}

/// Span for a text object.
pub fn span_from_object(object: ObjectRange) -> OpSpan {
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
    object_span_count(vim, buf, object, 1)
}

/// [`object_span`] with an explicit repetition count (`d2aw`, `v3ap`).
pub fn object_span_count(
    vim: &VimState,
    buf: &dyn VimBuffer,
    object: objects::TextObject,
    count: usize,
) -> Option<OpSpan> {
    let mut range = objects::range(buf, vim.cursor.offset, object, count)?;
    // a count repeats the object by RE-SCANNING from the span's end (vim
    // 9.1: `v2aw` covers "foo bar ", `v2i"` both quoted strings, `v2i(`
    // climbs to the enclosing block, `v2ap` the next paragraph — all fall
    // out of scan-from-end). It even reproduces vim's odd inner-word
    // counting: `2iw` = "foo " (word + its trailing blank run), `3iw` =
    // "foo bar" on "foo bar baz". The old engine dropped the count
    // entirely, so `d2aw` behaved as `daw`.
    // Block: a forward-found block consumed the count (the count-th
    // unmatched opener). Sentence: the count steps sentences inside
    // sentence_range — either way the climb below must not re-apply.
    let count_consumed = match &object {
        // a block found by the FORWARD search consumed the count (the
        // count-th unmatched opener); a block the cursor is INSIDE —
        // including ON its open bracket — must still climb
        objects::TextObject::Block { open, .. } => {
            range.start > vim.cursor.offset
                && buf.char_at(vim.cursor.offset) != Some(*open)
        }
        objects::TextObject::Sentence { .. } => true,
        _ => false,
    };
    // count-beyond detection (audit5 B-13/B-16: `3yap` without a third
    // paragraph and `y3aw` without a third word both BEEP and select
    // nothing — testdir assert_beeps). `done` counts SELECTED OBJECTS; the
    // word-family's line-break bridge (below) extends one object across
    // the newline and must not count as an extra one. A forward-found
    // block consumed the whole count inside block_range (the count-th
    // unmatched opener) — count it as satisfied.
    let mut done = if count_consumed { count.max(1) } else { 1usize };
    for _ in 1..if count_consumed { 1 } else { count.max(1) } {
        let mut probe = range.end.min(buf.len());
        // an INNER block's end sits just past its closer; re-probing there
        // re-selects the same block (zero progress). Step over the closer so
        // the rescan sees the ENCLOSING block — vim's `v2i(` climbs one
        // level (9.1 probe: `v2i(` inside the inner pair yanks "1 + (2)")
        if let objects::TextObject::Block { inner: true, close, .. } = object {
            if buf.char_at(probe) == Some(close) {
                probe += close.len_utf8();
            }
        }
        match objects::range(buf, probe, object, 1) {
            Some(next) if next.end > range.end || next.start < range.start => {
                range.start = range.start.min(next.start);
                range.end = range.end.max(next.end);
                done += 1;
            }
            // an outer-word probe sitting ON a line terminator re-selects
            // nothing (the raw range floors back INTO the line just
            // covered): vim continues across the break — newline, any
            // swallowed empty lines, then the next word with its trailing
            // blanks (`d2aw` on "ab cd\nef gh" covers " cd\nef "; on
            // "foo\n\nbar" it empties the buffer — audit B1). The bridge
            // belongs to the object it extends: `done` does not advance.
            _ if matches!(object, objects::TextObject::Word { inner: false, .. })
                && buf.char_at(probe) == Some('\n') =>
            {
                match objects::newline_word_span(buf, big_of(object), probe) {
                    // a bridge that REACHES the next line's word completes an
                    // object (d2aw's second aw); a whitespace-only tail does
                    // not (y3aw on 2 words still beeps — audit5 B-16)
                    Some(next) if next.end > range.end => {
                        let bridged = buf.slice(range.end..next.end.min(buf.len()));
                        if bridged.chars().any(|c| !c.is_whitespace()) {
                            done += 1;
                        }
                        range.end = next.end;
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    if done < count.max(1) {
        return None;
    }
    Some(span_from_object(range))
}

/// The `big` flag of a word-family text object.
fn big_of(object: objects::TextObject) -> bool {
    matches!(object, objects::TextObject::Word { big: true, .. })
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
        crate::mode::VisualKind::Block | crate::mode::VisualKind::Char => {
            // the exclusive end covers the cursor's WHOLE cluster: the old
            // `hi + char_len` left the base's combining marks outside the
            // selection (a visual `p`/`r` on `#`+VS16 ate the base and left
            // the orphan mark behind)
            let end = match buf.char_at(hi) {
                Some(c) if c != '\n' && crate::buffer::char_display_width(c) > 0 => {
                    crate::buffer::next_grapheme_offset(buf, hi)
                        .unwrap_or(hi + c.len_utf8())
                }
                _ => hi + buf.char_at(hi).map(|c| c.len_utf8()).unwrap_or(0),
            };
            OpSpan {
                start: lo,
                end,
                linewise: false,
            }
        }
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
/// An EMPTY span is a failed operator, charwise OR LINEWISE: it deletes
/// nothing AND stores nothing. The charwise case is a `dw` whose landing
/// equals the cursor (`ci(` on `()` still enters insert, register untouched —
/// vim 9.1 probe); the linewise case is `dd`/`dip`/`dap`/`Vd`/`dG` on an
/// empty buffer, whose span is 0..0 — the old charwise-only guard let the
/// empty linewise delete into the numbered ring + unnamed register, so a
/// later `p` opened a stray empty line where vim beeps (round 28 probe).
/// YANK has the OPPOSITE rule: an empty linewise yank stores an empty line
/// (`yy` on an empty buffer puts an empty line in `"0`, 9.1 probe).
pub fn delete_span(vim: &mut VimState, ctx: &mut Ctx, span: &OpSpan, register: Option<char>) {
    if span.start >= span.end {
        return;
    }
    let text = ctx.buf.slice(span.start..span.end);
    crate::registers::sync_clipboard_host(ctx.host, register, &text);
    let use_reg_one = std::mem::take(&mut vim.use_reg_one);
    vim.registers
        .store_delete(register, text, register_kind(span), use_reg_one);
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
    /// The `$` spelling was used inside the block selection: the block
    /// covers each selected row from column 0 to its own content end —
    /// the anchor column is IGNORED, rows carry whole lines, and vim's
    /// 9.1 probes show delete/yank both treat them that way (`jll$d`
    /// empties the covered lines from col 0; the yank register holds the
    /// UNPADDED row texts). The flag rides along so the yank skips its
    /// rectangle padding and block `A` skips its append-column pads.
    pub dollar: bool,
}

/// The block's byte range on ONE line: every char whose display-column
/// span intersects `[col_lo, col_hi)`. Empty range when the line ends
/// before `col_lo`.
pub fn block_row_range(
    buf: &dyn crate::buffer::VimBuffer,
    line: usize,
    col_lo: usize,
    col_hi: usize,
    ts: usize,
) -> Range<usize> {
    use crate::buffer::char_display_width_at;
    let start = buf.line_start(line);
    let end = buf.line_end(line);
    let mut o = start;
    let mut col = 0usize;
    let mut lo: Option<usize> = None;
    let mut hi = start;
    while o < end {
        let Some(c) = buf.char_at(o) else { break };
        let w = char_display_width_at(c, col, ts);
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
        Some(lo) => {
            // the cell covers the WHOLE last cluster: extend past the last
            // included char's trailing marks (deleting only the base would
            // orphan the mark, which then composes onto the char before it)
            let mut hi = hi;
            while hi < end {
                match buf.char_at(hi) {
                    Some(c) if c == '\u{200D}' || crate::buffer::char_display_width(c) == 0 => hi += c.len_utf8(),
                    _ => break,
                }
            }
            lo..hi
        }
        // the row ends BEFORE the block column: an EMPTY range anchored at
        // the LINE END, not the line start — `c`/`I` replica rows must type
        // at the row's end (vim: block col 2 over rows "abcd"/"ab" changes
        // "c"→X on row 1 and APPENDS on row 2 → "abX"; the old line-start
        // anchor typed the replica text at column 0 — audit G3)
        None => end..end,
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
    let first_line = buf.offset_to_line(anchor.min(cursor));
    let last_line = buf.offset_to_line(anchor.max(cursor));
    // `$` in blockwise visual (`:h v_$`): the block extends to the end of
    // EVERY selected line — and the probes showed the anchor column drops
    // out entirely (each row covers col 0..content end). Tracked on the
    // engine by `VimState::block_dollar`, armed by the LineEnd motion.
    if vim.block_dollar {
        let rows = (first_line..=last_line)
            .map(|line| buf.line_start(line)..buf.line_end(line))
            .collect();
        return Some(BlockSpan {
            col_lo: 0,
            col_hi: 0, // unused: dollar skips both the yank pads and the A pads
            first_line,
            rows,
            dollar: true,
        });
    }
    // both block edges are VIRTUAL columns: the cursor edge via
    // `block_cursor_col` (audit G1/G3), the anchor edge via
    // `block_anchor_vcol` — after `o`/`O` the anchor byte sits clamped on a
    // short row and its display column would shrink the rectangle (audit E1).
    // Each edge is clamped to ITS OWN ROW's exclusive width at read time:
    // the curswant survives short rows (audit C2 — vim's j over an empty
    // line shows a zero-width rectangle there and restores the full width
    // on the way back)
    let a_row = buf.offset_to_line(anchor);
    let c_row = buf.offset_to_line(cursor);
    let a_col = vim
        .block_anchor_vcol
        .unwrap_or_else(|| crate::buffer::display_column(buf, anchor))
        .min(crate::buffer::display_column(buf, buf.line_end(a_row)));
    let c_col = vim
        .block_cursor_col
        .unwrap_or_else(|| crate::buffer::display_column(buf, cursor))
        .min(crate::buffer::display_column(buf, buf.line_end(c_row)));
    // An anchor sitting ON a TAB snaps the block's left edge to the tab's
    // END and measures the width as the cursor/anchor column distance + 1
    // (audit C15 — 9.1 oracle matrix at ts=4: `\tab` + `<C-v>l` covers
    // cols 4..6 = "ab" on the tab row, "ef" on the next; `<C-v>` alone
    // covers just col 4 = 'a'/'e'. The old ts=1 model read cols 0..2 and
    // shredded the tab.)
    if buf.char_at(anchor) == Some('\t') {
        let ts = vim.options.tabstop.max(1);
        let a_col_raw = vim
            .block_anchor_vcol
            .unwrap_or_else(|| crate::buffer::display_column(buf, anchor));
        let tab_end = (a_col_raw / ts + 1) * ts;
        let width = c_col.saturating_sub(a_col_raw) + 1;
        let (col_lo, col_hi) = (tab_end, tab_end + width);
        let rows = (first_line..=last_line)
            .map(|line| block_row_range(buf, line, col_lo, col_hi, vim.options.tabstop))
            .collect();
        return Some(BlockSpan {
            col_lo,
            col_hi,
            first_line,
            rows,
            dollar: false,
        });
    }
    let (col_lo, col_hi) = if a_col <= c_col {
        (a_col, c_col)
    } else {
        (c_col, a_col)
    };
    // the cursor char is part of the block: exclusive end = corner col + 1
    let rows = (first_line..=last_line)
        .map(|line| block_row_range(buf, line, col_lo, col_hi + 1, vim.options.tabstop))
        .collect();
    Some(BlockSpan {
        col_lo,
        col_hi: col_hi + 1,
        first_line,
        rows,
        dollar: false,
    })
}

/// Yank the span into the register. An empty charwise span (failed motion)
/// stores nothing — the register keeps its previous content, like vim.
pub fn yank_span(vim: &mut VimState, ctx: &mut Ctx, span: &OpSpan, register: Option<char>) {
    if !span.linewise && span.start >= span.end {
        return;
    }
    let text = ctx.buf.slice(span.start..span.end);
    crate::registers::sync_clipboard_host(ctx.host, register, &text);
    vim.registers
        .store_yank(register, text, register_kind(span));
}

/// Apply an operator to a span (operator-pending and visual paths).
/// `daw`/`caw` on whitespace whose word sits on a FOLLOWING line: vim merges
/// that line away too (9.1 probe: `daw` on the blank line of
/// ["foo","   ","bar"] leaves just ["foo"] — the word's line break goes with
/// it). The fingerprint is a charwise span starting on whitespace, containing
/// a newline, and ending right before the word's line break; both operators
/// extend the span over that `\n`.
fn merges_following_line_break(buf: &dyn VimBuffer, span: &OpSpan) -> bool {
    !span.linewise
        && buf.char_at(span.end) == Some('\n')
        && matches!(buf.char_at(span.start), Some(c) if c.is_whitespace())
        && buf.slice(span.start..span.end).contains('\n')
}

pub fn apply(
    vim: &mut VimState,
    ctx: &mut Ctx,
    op: Operator,
    span: &OpSpan,
    register: Option<char>,
) {
    if veto_readonly_write(vim, ctx, register) {
        return;
    }
    // `'[` / `']`: the first/last character of the last yanked or changed
    // text (audit E6 — the marks were unbound bells; `` `] `` resolves
    // through the ordinary mark table once set). Stored BEFORE the op runs
    // so the edit funnels keep them addressable through the change itself
    // (a delete folds `']` onto the deletion point, like vim's `dd`).
    if span.end > span.start || span.linewise {
        let mark_end = if span.linewise {
            let e = ctx.buf.line_range(last_line_of_span(ctx.buf, span)).end;
            ctx.buf.prev_char_offset(e.saturating_sub(1)).unwrap_or(span.start)
        } else {
            ctx.buf.prev_char_offset(span.end).unwrap_or(span.end - 1)
        };
        vim.marks.set('[', span.start);
        vim.marks.set(']', mark_end);
    }
    match op {
        // block `C`/`S` resolve in apply_block_operator (visual block mode
        // dispatches straight there); unreachable from here
        Operator::BlockChangeToEnd => {}
        // `!{motion}`: park the span and open the `!` prompt — the command
        // text arrives on Enter (audit B3)
        Operator::Filter => {
            vim.filter_span = Some(*span);
            vim.begin_cmdline('!');
        }
        // `={motion}`: reindent per the built-in rules — with no filetype
        // the only deterministic rule is `'lisp'` (audit B4); without it
        // vim has no rule either, so the text is left alone
        Operator::Reindent => {
            if !vim.options.lisp {
                return;
            }
            let buf = &*ctx.buf;
            let first_line = buf.offset_to_line(span.start);
            let last_line = last_line_of_span(buf, span);
            // paren stack over every line ABOVE the span: the innermost
            // unclosed `(` gives the lisp indent column (col + 1)
            let mut stack: Vec<usize> = Vec::new();
            for line in 0..first_line {
                let ls = buf.line_start(line);
                let le = buf.line_end(line);
                let mut col = 0usize;
                let mut o = ls;
                while o < le {
                    match buf.char_at(o) {
                        Some('(') => {
                            stack.push(col);
                            col += 1;
                        }
                        Some(')') => {
                            stack.pop();
                            col += 1;
                        }
                        Some(c) => col += crate::buffer::char_display_width(c),
                        None => break,
                    }
                    o += buf.char_at(o).map(|c| c.len_utf8()).unwrap_or(1);
                }
            }
            let mut lines: Vec<String> = Vec::new();
            for line in first_line..=last_line {
                let ls = buf.line_start(line);
                let le = buf.line_end(line);
                let text = buf.slice(ls..le);
                let trimmed = text.trim_start();
                let indent = stack.last().map(|c| c + 1).unwrap_or(0);
                lines.push(format!("{}{}", " ".repeat(indent), trimmed));
                // the line's own parens feed the NEXT line's indent
                let mut col = indent;
                for c in trimmed.chars() {
                    match c {
                        '(' => {
                            stack.push(col);
                            col += 1;
                        }
                        ')' => {
                            stack.pop();
                            col += 1;
                        }
                        other => col += crate::buffer::char_display_width(other),
                    }
                }
            }
            let mut new_text = lines.join("\n");
            let start = buf.line_start(first_line);
            let end = buf.line_range(last_line).end;
            // the replaced range includes the last line's own newline when
            // the span reaches the buffer end — keep it
            if buf.char_at(end.saturating_sub(1)) == Some('\n') {
                new_text.push('\n');
            }
            let gen = vim.edit_generation;
            vim.begin_edit();
            vim.edit_replace(ctx, start..end, &new_text);
            vim.end_edit();
            vim.cursor.offset = crate::buffer::clamp_cursor(ctx.buf, start);
            vim.cursor.desired_col = None;
            if vim.edit_generation != gen {
                vim.bump(ctx);
            }
        }
        Operator::Delete => {
            let mut span = *span;
            // object spans keep their exact bounds: vim's `daw` at a line
            // end leaves the following empty line alone (the join rule is a
            // MOTION-`dw` behavior — audit B2)
            if !vim.op_from_object && merges_following_line_break(ctx.buf, &span) {
                span.end += 1;
            }
            delete_span(vim, ctx, &span, register)
        }
        // visual `D`: charwise/linewise — delete the COVERED LINES (vim
        // `:h v_D` deletes whole lines from any non-block visual kind).
        // Blockwise is handled in apply_block_operator (per-row col..EOL).
        Operator::DeleteToEnd => {
            let first = ctx.buf.offset_to_line(span.start);
            let last = last_line_of_span(ctx.buf, span);
            let mut s2 = OpSpan {
                start: ctx.buf.line_start(first),
                end: ctx.buf.line_range(last).end,
                linewise: true,
            };
            if merges_following_line_break(ctx.buf, &s2) {
                s2.end += 1;
            }
            delete_span(vim, ctx, &s2, register)
        }
        Operator::Yank => yank_span(vim, ctx, span, register),
        Operator::Change => {
            // keep the indent of the first line when changing linewise —
            // gated on 'autoindent' like vim (9.1 probes: `cc` on "    abc"
            // with `noai` starts empty at col 0, with `ai` keeps the indent)
            let indent_text = if span.linewise && vim.options.autoindent {
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
                // `caw` shares the delete-side merge (see
                // [`merges_following_line_break`])
                let mut merged = *span;
                if merges_following_line_break(ctx.buf, &merged) {
                    merged.end += 1;
                }
                merged
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
            // the restored indent is pure autoindent: an untouched
            // whitespace-only line loses it at Esc (`cc<Esc>` → empty line,
            // vim 9.1 probe S8) — arm the same did_ai contract an `o`/`O`
            // open arms (see `VimState::insert_did_ai`). AFTER begin_insert,
            // which clears the flag at session start.
            if !indent_text.is_empty() {
                vim.insert_did_ai = true;
            }
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
            let mapped = case_mapped_text(op, &text);
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
        uppercase_char(c)
    } else {
        lowercase_char(c)
    }
}

/// `i_CTRL-K {c1}{c2}`: vim's default digraph table (a representative
/// Latin-1/general subset — `ss` → `ß` is the oracle-verified audit H3
/// case). Unknown pairs insert nothing, like vim's E-cancels.
pub(crate) fn digraph(a: char, b: char) -> String {
    const TABLE: &[(char, char, char)] = &[
        ('s', 's', 'ß'),
        ('o', '/', 'ø'),
        ('O', '/', 'Ø'),
        ('a', 'e', 'æ'),
        ('A', 'E', 'Æ'),
        ('o', 'e', 'œ'),
        ('O', 'E', 'Œ'),
        ('a', ':', 'ä'),
        ('o', ':', 'ö'),
        ('u', ':', 'ü'),
        ('A', ':', 'Ä'),
        ('O', ':', 'Ö'),
        ('U', ':', 'Ü'),
        ('e', '"', 'ë'),
        ('i', '"', 'ï'),
        ('E', '"', 'Ë'),
        ('a', '\'', 'á'),
        ('e', '\'', 'é'),
        ('i', '\'', 'í'),
        ('o', '\'', 'ó'),
        ('u', '\'', 'ú'),
        ('y', '\'', 'ý'),
        ('A', '\'', 'Á'),
        ('E', '\'', 'É'),
        ('I', '\'', 'Í'),
        ('O', '\'', 'Ó'),
        ('U', '\'', 'Ú'),
        ('Y', '\'', 'Ý'),
        ('a', '`', 'à'),
        ('e', '`', 'è'),
        ('i', '`', 'ì'),
        ('o', '`', 'ò'),
        ('u', '`', 'ù'),
        ('A', '`', 'À'),
        ('E', '`', 'È'),
        ('I', '`', 'Ì'),
        ('O', '`', 'Ò'),
        ('U', '`', 'Ù'),
        ('a', '^', 'â'),
        ('e', '^', 'ê'),
        ('i', '^', 'î'),
        ('o', '^', 'ô'),
        ('u', '^', 'û'),
        ('A', '^', 'Â'),
        ('E', '^', 'Ê'),
        ('I', '^', 'Î'),
        ('O', '^', 'Ô'),
        ('U', '^', 'Û'),
        ('c', ',', 'ç'),
        ('C', ',', 'Ç'),
        ('n', '~', 'ñ'),
        ('N', '~', 'Ñ'),
        ('a', '*', 'å'),
        ('A', '*', 'Å'),
        ('d', '-', 'ð'),
        ('D', '-', 'Ð'),
        ('t', 'h', 'þ'),
        ('T', 'H', 'Þ'),
        ('y', ':', 'ÿ'),
        ('c', 'o', '©'),
        ('r', 'g', '®'),
        ('D', 'G', '°'),
        ('+', '-', '±'),
        ('-', '-', '­'),
        ('1', '2', '½'),
        ('1', '4', '¼'),
        ('3', '4', '¾'),
        ('<', '<', '«'),
        ('>', '>', '»'),
        ('?', 'i', '¿'),
        ('!', 'I', '¡'),
        ('P', 'd', '£'),
        ('E', 'u', '€'),
        ('Y', 'e', '¥'),
        ('C', 'u', '¤'),
        ('x', ' ', '×'),
        (':', '-', '÷'),
        ('M', 'y', 'µ'),
        ('p', 'I', '¶'),
        ('S', 'E', '§'),
    ];
    for (x, y, out) in TABLE {
        if (*x == a && *y == b) || (*x == b && *y == a) {
            return (*out).to_string();
        }
    }
    String::new()
}

/// Uppercase one char the way vim 9.1 does: `ß` maps to the single char
/// `ẞ` (U+1E9E, oracle probe) — Rust's `to_uppercase` spells it `SS`,
/// which turned `gUU` on `ß` into `SS` (audit G4).
///
/// vim case-maps through its SIMPLE table: a char without a one-char
/// uppercase (ligature `ﬁ` → "FI", `ǰ` → "Jˇ") maps to ITSELF, while
/// Rust's full Unicode mapping expands it (audit C4 — `~` turned `ﬁx`
/// into `FIx`).
pub(crate) fn uppercase_char(c: char) -> String {
    if c == 'ß' {
        return '\u{1e9e}'.to_string();
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(one), None) => one.to_string(),
        _ => c.to_string(),
    }
}

/// Lowercase one char through vim's SIMPLE table: Rust spells `İ`
/// (U+0130) as `i` + combining dot, vim's table maps it to plain `i`
/// (oracle probe on `gul` — audit C4); multi-char expansions again leave
/// the char alone.
pub(crate) fn lowercase_char(c: char) -> String {
    if c == '\u{0130}' {
        return "i".to_owned();
    }
    let mut lo = c.to_lowercase();
    match (lo.next(), lo.next()) {
        (Some(one), None) => one.to_string(),
        _ => c.to_string(),
    }
}

/// Map every char of `text` through the case operator. Multi-char case
/// mappings (ß ↔ SS, İ → i̇) make each char a String; callers splice the
/// result back with `edit_replace`, which absorbs the changed byte length.
pub(crate) fn case_mapped_text(op: Operator, text: &str) -> String {
    text.chars()
        .map(|c| match op {
            Operator::Lowercase => lowercase_char(c),
            Operator::Uppercase => uppercase_char(c),
            _ => toggle_case(c),
        })
        .collect()
}

/// Shift one line left/right by `shiftwidth` — vim's COLUMN model: the
/// indent's display column (tabstop-aware) moves ±sw, then the new indent is
/// RE-EXPRESSED (as many whole tabs as fit under 'tabstop', the remainder in
/// spaces, all spaces under 'expandtab'). vim 9.1 probes, noet ts=8 sw=4:
/// `<<` on `\tx` → `    x`, on `\t\tx` → `\t    x`; `>>` on col-5 indent →
/// `\t x`; et sw=4: `>>` on `x` → `    x`. The old model peeled bytes (a tab
/// = a whole sw unit) and always inserted a raw tab, diverging on every
/// ts≠sw or mixed-indent buffer.
pub fn shift_line(vim: &mut VimState, ctx: &mut Ctx, line: usize, right: bool) {
    if line >= ctx.buf.line_count() {
        return;
    }
    let start = ctx.buf.line_start(line);
    let end = ctx.buf.line_end(line);
    if end == start {
        // an EMPTY line stays empty (vim skips it; 9.1 probe)
        return;
    }
    let (indent_bytes, _ws_only) = ctx.buf.line_indent(line);
    let ts = vim.options.tabstop.max(1);
    // display column of the first non-blank: a tab jumps to the next
    // tabstop boundary, anything else (indent is only ' '/'\t') is width 1
    let mut col = 0usize;
    let mut o = start;
    while o < start + indent_bytes {
        match ctx.buf.char_at(o) {
            Some('\t') => col = (col / ts + 1) * ts,
            Some(_) => col += 1,
            None => break,
        }
        o += ctx.buf.char_at(o).map_or(1, |c| c.len_utf8());
    }
    // vim: `shiftwidth=0` means "use 'tabstop'" (`:h shiftwidth`) — the old
    // `.max(1)` silently shifted by a single column instead (audit I2)
    let sw = if vim.options.shiftwidth == 0 {
        vim.options.tabstop.max(1)
    } else {
        vim.options.shiftwidth.max(1)
    };
    let target = if right {
        col.saturating_add(sw)
    } else {
        col.saturating_sub(sw)
    };
    if target == col {
        return;
    }
    let new_indent = if vim.options.expandtab {
        " ".repeat(target)
    } else {
        let tabs = target / ts;
        let mut s = "\t".repeat(tabs);
        s.push_str(&" ".repeat(target - tabs * ts));
        s
    };
    vim.edit_replace(ctx, start..start + indent_bytes, &new_indent);
}

/// `gq`/`gw`: reflow the span's lines to `textwidth` as one paragraph per
/// blank-line-separated group. The span's first-line indent becomes the
/// paragraph indent for all wrapped lines. `start` is a byte OFFSET (the
/// span start); `last_line` is the last affected LINE index.
pub fn format_lines(vim: &mut VimState, ctx: &mut Ctx, start: usize, last_line: usize) {
    let width = vim.options.textwidth.max(1);
    let span_end = ctx.buf.line_end(last_line);
    // gq formats WHOLE lines (vim 9.1 probe: `gq}` with the cursor mid-line
    // reformats from column 0). The raw span start may sit mid-line (charwise
    // motions like `gq}`, a charwise visual selection) — replacing from there
    // would keep the pre-cursor text in place AND repeat it inside the
    // reflowed paragraph. Anchor the edit at the first line's start.
    let start = ctx.buf.line_start(ctx.buf.offset_to_line(start));
    if start >= span_end {
        vim.cursor.offset = clamp_cursor(ctx.buf, start);
        return;
    }
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
            // the separator terminates the preceding paragraph — but an EMPTY
            // paragraph must not flush (an empty flush still emits one line,
            // and a run of blank lines then DOUBLES: vim 9.1 keeps blank
            // lines 1:1 through gq). The separator line itself is kept
            // verbatim — vim preserves whitespace-only lines too (probe:
            // `gq` over ["para", "   ", "tail"] keeps the three spaces).
            if !paragraph.is_empty() {
                flush_paragraph(&mut paragraph, &indent, width, &mut out);
            }
            out.push_str(&text);
            out.push('\n');
        } else {
            if paragraph.is_empty() {
                indent = text[..ind].to_owned();
            }
            for word in text[ind..].split_whitespace() {
                paragraph.push(word.to_owned());
            }
        }
    }
    // the last paragraph flushes with its terminating newline; a span that
    // ENDED on a blank line already has it — either way exactly one trailing
    // '\n' belongs to the buffer structure and is dropped
    if !paragraph.is_empty() {
        flush_paragraph(&mut paragraph, &indent, width, &mut out);
    }
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

/// vim refuses to route a yank/delete through the read-only registers:
/// `"%dd`/`".dd`/`":dd`/`"/dd` all raise E354 and leave the buffer alone
/// (audit C7 — the prefix itself is legal, the WRITE is not). The veto
/// lives at the two write funnels so operators and x/X/s share one rule.
fn veto_readonly_write(vim: &VimState, ctx: &mut Ctx, register: Option<char>) -> bool {
    if let Some(r) = register {
        if !crate::registers::is_write_valid(r) {
            ctx.host.status_message("E354: Invalid register name");
            ctx.host.bell();
            return true;
        }
    }
    false
}

/// `p` / `P`: paste a register. An unset register reports vim's feedback
/// (bell — vim shows E353: Nothing in register) instead of a silent no-op.
pub fn put(    vim: &mut VimState,
    ctx: &mut Ctx,
    register: char,
    count: usize,
    after: bool,
) {
    put_ex(vim, ctx, register, count, after, false)
}

/// `gp`/`gP` pass `leave_after = true`: the cursor lands just AFTER the new
/// text instead of on it (`:h gp` — audit G6; the charwise cursor goes one
/// past the last pasted char, the linewise one to the LAST pasted line's
/// first non-blank for `gp` / the FIRST for `gP`).
pub fn put_ex(
    vim: &mut VimState,
    ctx: &mut Ctx,
    register: char,
    count: usize,
    after: bool,
    leave_after: bool,
) {
    let Some(data) = vim.registers.get_for_paste(register, ctx.host) else {
        vim.note_bell(ctx);
        return;
    };
    // A linewise register always represents WHOLE lines, so an empty one is
    // one empty line (e.g. `yy` on the only line of an empty buffer): `p`
    // inserts an empty line rather than nothing (vim 9.1 parity). A charwise
    // register that is empty has nothing to place.
    let mut text = if data.text.is_empty() {
        if data.kind != RegisterKind::Linewise {
            return;
        }
        "\n".to_owned()
    } else {
        data.text.clone()
    };
    // A linewise register is newline-terminated by definition. Yanking a
    // trailing-newline-less tail line (`set noeol` files) stored it bare, and
    // `text.repeat(count)` then glued the copies HORIZONTALLY (`yy3p` on
    // noeol "abc" = "abc\nabcabcabc" — audit C2); normalize before repeating.
    if data.kind == RegisterKind::Linewise && !text.ends_with('\n') {
        text.push('\n');
    }
    let count = clamped_repeat_count(text.len(), count.max(1));

    if data.kind == RegisterKind::Blockwise {
        put_blockwise(vim, ctx, &text, count, after);
        return;
    }
    let repeated = text.repeat(count);
    let pasted_line_count = repeated.matches('\n').count().max(1);

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
            // last line without trailing newline: open a line for the FIRST
            // pasted line — and KEEP the register's own final `\n` so the
            // file stays newline-terminated exactly like vim's internal
            // buffer (`ddp` on "a\n" = "\na\n", not "\na"; an empty linewise
            // register on an empty buffer adds the line instead of vanishing
            // into the separator — audit C1. The old `strip_suffix` consumed
            // the terminator as if it were the separator and never paid it
            // back.)
            (ctx.buf.len(), format!("\n{repeated}"))
        } else {
            // above the current line: the register's lines go in verbatim
            // (linewise text is newline-terminated, so it concatenates
            // cleanly before the current line's first byte) — same one-\n
            // rule for a malformed register that lost its final newline.
            // An noeol EMPTY current line has no terminator of its own, so
            // the plain prepend makes the register's final `\n` absorb both
            // roles and a line disappears: `yyP` on the empty buffer must
            // end at TWO empty lines like vim (audit C1).
            let stripped = repeated.strip_suffix('\n').unwrap_or(&repeated);
            let pad = if !has_newline && ctx.buf.line_end(line) == ctx.buf.line_start(line) {
                "\n"
            } else {
                ""
            };
            (
                ctx.buf.line_start(line),
                format!("{stripped}\n{pad}"),
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
        let last_pasted = (cursor_line + pasted_line_count - 1).min(ctx.buf.line_count() - 1);
        // `'[`/`']` change marks: first pasted char .. last pasted char
        // (audit5 C-5 — do_put's b_op_start/b_op_end accounting). The `]`
        // lands on the last pasted CHAR's start — line_end-1 is the last
        // BYTE, which sits inside a multibyte tail (中) and the stored mark
        // then violates the addressability invariant (fuzz round-117)
        let mark_end = crate::buffer::prev_grapheme_offset(
            ctx.buf,
            ctx.buf.line_end(last_pasted),
        )
        .unwrap_or(cursor_at)
        .max(cursor_at);
        vim.marks.set('[', cursor_at);
        vim.marks.set(']', mark_end);
        if leave_after {
            // gp linewise: cursor just AFTER the pasted block — the line
            // FOLLOWING it (do_put MLINE `PUT_CURSEND: lnum+1`; audit5 C-8 —
            // the old "last pasted line" came from probes pinned at the
            // buffer end, where the clamp hides the difference); gP
            // linewise: ditto (`:h gp` "just after the new text")
            let line = cursor_line + pasted_line_count;
            vim.cursor.offset = ctx
                .buf
                .first_non_blank(line.min(ctx.buf.line_count() - 1));
        } else {
            vim.cursor.offset = ctx.buf.first_non_blank(cursor_line);
        }
    } else {
        let mut at = vim.cursor.offset;
        if after && !ctx.buf.at_line_end(at) {
            // step over the WHOLE cluster under the cursor: next_char_offset
            // would insert between a base char and its combining mark,
            // splitting the cluster (# + VS16 became # n VS16)
            at = crate::buffer::next_grapheme_offset(ctx.buf, at).unwrap_or(at);
        }
        vim.edit_insert(ctx, at, &repeated);
        // block cursor sits on the last pasted GRAPHEME start — `end - 1`
        // byte arithmetic parks mid-char on multibyte tails, and
        // prev_char_offset parks mid-cluster when the paste ends with a
        // combining mark / ZWJ member (the next `x` would split it)
        let end = at + repeated.len();
        // `'[`/`']`: first pasted char .. one past the last (audit5 C-5)
        vim.marks.set('[', at);
        vim.marks.set(']', end);
        // a MULTILINE charwise paste puts the cursor back on the FIRST
        // pasted character (do_put MCHAR multi-line arm restores b_op_start;
        // audit5 C-9/C-10 — the engine parked on the last pasted char like
        // the single-line shape)
        let multiline = repeated.contains('\n');
        vim.cursor.offset = if multiline {
            clamp_cursor(ctx.buf, at)
        } else if leave_after {
            // gp/gP charwise: just AFTER the last pasted char — one past
            // `end`, clamped like every cursor (the line end parks it on
            // the line's last char, vim's same clamp)
            clamp_cursor(ctx.buf, end)
        } else {
            clamp_cursor(
                ctx.buf,
                crate::buffer::prev_grapheme_offset(ctx.buf, end).unwrap_or(at),
            )
        };
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
/// against vim 9.1 (round 28 PTY): the FIRST row lands at the target column
/// on the cursor line (`p`: one column right of the cursor char; `P`: at it,
/// short lines padded with spaces up to the column). Every following row
/// lands at the SAME display column on the EXISTING line below — merged
/// into it, padding the line with spaces first when it is short. Only rows
/// past the buffer's last line open padded NEW lines at the tail (the old
/// "push down, never merge" model came from probes that all sat on the
/// buffer's last line, where the two models coincide). The cursor sits on
/// the first pasted character. `count` repeats each row horizontally (vim's
/// blockwise count).
fn put_blockwise(vim: &mut VimState, ctx: &mut Ctx, text: &str, count: usize, after: bool) {
    // NO trailing-newline trim: the block yank stores rows joined by `\n`
    // with no terminator, but an EMPTY last row legitimately produces a
    // trailing `\n` ("cde\n" = rows ["cde", ""]) — trimming it dropped the
    // row on paste (round15; reachable through the host block API)
    let rows: Vec<String> = text.split('\n').map(|row| row.repeat(count)).collect();
    let cur_col = crate::buffer::display_column(ctx.buf, vim.cursor.offset);
    let col = if after { cur_col + 1 } else { cur_col };

    let line = ctx.buf.offset_to_line(vim.cursor.offset);
    let line_count = ctx.buf.line_count();
    // rows 2..: bottom-up over the lines below the cursor line, then the
    // overflow (targets past the buffer end) appends at the tail in order
    let merged = (rows.len() - 1).min(line_count.saturating_sub(line + 1));
    if rows.len() > merged + 1 {
        let indent = " ".repeat(col);
        let mut extra = String::new();
        for row in &rows[merged + 1..] {
            extra.push('\n');
            extra.push_str(&indent);
            extra.push_str(row);
        }
        // before the LAST line's newline: the overflow rows become the new
        // tail lines (a trailing `\n`-less buffer gets its separator from
        // the same `\n` prefix — line_end == len there)
        let at = ctx.buf.line_end(line_count - 1);
        vim.edit_insert(ctx, at, &extra);
    }
    for i in (0..merged).rev() {
        insert_block_row_at_column(vim, ctx, line + 1 + i, col, &rows[i + 1]);
    }
    // first row: byte boundary at display column `col` on the cursor line;
    // short lines get padded with spaces up to the column first
    let line_start = ctx.buf.line_start(line);
    let line_end = ctx.buf.line_end(line);
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

/// Insert `row` at display column `col` of `target`, padding the line with
/// spaces up to `col` first when the line is shorter (vim's blockwise put
/// merge: "cccc" + row at col 1 → "c<row>ccc"; a 2-char line at col 3 pads
/// to "bb " before the row). Shared by the normal-mode put (`put_blockwise`)
/// and the visual-block put's overflow rows.
pub(crate) fn insert_block_row_at_column(
    vim: &mut VimState,
    ctx: &mut Ctx,
    target: usize,
    col: usize,
    row: &str,
) {
    let le = ctx.buf.line_end(target);
    let width = crate::buffer::display_column(ctx.buf, le);
    // col == width is an APPEND (byte point = line end), not the clamped
    // "last char" landing offset_for_display_column gives — `X` + row at
    // col 1 must make "X<row>", not "<row>X"
    let (at, pad) = if col < width {
        (crate::buffer::offset_for_display_column(ctx.buf, target, col), 0)
    } else {
        (le, col - width)
    };
    let text = format!("{}{}", " ".repeat(pad), row);
    if !text.is_empty() {
        vim.edit_insert(ctx, at, &text);
    }
}

/// `J` / `gJ`: join `count` lines (at least one join). `literal` = `gJ`
/// (no separator, keep the next line's indent). Returns the number of
/// joins actually performed — a shortfall means the buffer ran out of
/// lines (callers turn that into vim's bell; Ex callers stay silent).
pub fn join_lines(vim: &mut VimState, ctx: &mut Ctx, count: usize, literal: bool) -> usize {
    let joins = count.max(2) - 1;
    let mut performed = 0usize;
    for _ in 0..joins {
        let line = ctx.buf.offset_to_line(vim.cursor.offset);
        if line + 1 >= ctx.buf.line_count() {
            break;
        }
        performed += 1;
        let join_at = ctx.buf.line_end(line);
        let next_start = ctx.buf.line_start(line + 1);

        if literal {
            vim.edit_delete(ctx, join_at..next_start);
        } else {
            let next_end = ctx.buf.line_end(line + 1);
            let next_content = ctx.buf.slice(next_start..next_end);
            let next_trimmed = next_content.trim_start();
            let next_indent_len = next_content.len() - next_trimmed.len();

            // no extra space when the line already ends with whitespace,
            // either side of the seam is empty, or the next line starts
            // with `)` (vim 9.1: `J` on ["", "def"] gives "def" — the old
            // code produced " def"; ["def", ""] stays "def" too)
            let cur_tail = ctx.buf.slice(ctx.buf.line_start(line)..join_at);
            let separator = if cur_tail.is_empty()
                || cur_tail.ends_with(' ')
                || cur_tail.ends_with('\t')
                || next_trimmed.is_empty()
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
        // clamp_cursor snaps the landing onto a cluster start: the seam byte
        // can be a CONTINUATION char — `gJ` merging a line whose first char
        // is a combining mark glues that mark onto the char before the seam,
        // and a raw `join_at` parked the cursor mid-cluster (fuzz round 25)
        vim.cursor.offset = clamp_cursor(ctx.buf, join_at);
    }
    vim.cursor.desired_col = None;
    performed
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

/// `x` / `X` / `s` / `<Del>`: delete `count` chars under/before the cursor.
/// `register` carries an explicit `"{reg}` prefix (vim: `"ax` deletes into
/// register a; without one the deleted text lands in the usual delete
/// targets via `delete_span`'s `None`).
pub fn delete_chars(
    vim: &mut VimState,
    ctx: &mut Ctx,
    count: usize,
    backward: bool,
    register: Option<char>,
) {
    if veto_readonly_write(vim, ctx, register) {
        return;
    }
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
        register,
    );
}

/// `r{char}`: replace `count` chars with `char` (never crosses the line end).
pub fn replace_chars(vim: &mut VimState, ctx: &mut Ctx, ch: char, count: usize) -> bool {
    let count = count.max(1);
    let start = vim.cursor.offset;
    let line_end = ctx.buf.line_end(ctx.buf.offset_to_line(start));
    let mut o = start;
    let mut replaced = 0usize;
    for _ in 0..count {
        if o >= line_end {
            // `3rx` with only two chars left on the line: vim cancels the
            // whole replace instead of partially filling it
            return false;
        }
        let next = advance_graphemes(ctx.buf, o, 1, line_end);
        if next == o {
            break;
        }
        o = next;
        replaced += 1;
    }
    if replaced == 0 {
        // no cluster was consumed (the cursor cluster ran into the line end):
        // nothing to replace, and the cursor-landing arithmetic below would
        // underflow on a multi-byte replacement char (probe: `r中` on the
        // base of a ZWJ-at-line-end cluster used to panic the debug build)
        return false;
    }
    let replacements = if ch == '\n' {
        // N<CR> collapses to ONE break: ":h r" — "5r<CR> replaces five
        // characters with a single line break" (9.1 byte probe: `3r<CR>` on
        // "abcdef" → one empty line + "def", not three). The autoindent the
        // doc mentions nets out to the plain break too: the indent stays on
        // the FIRST line and the new line's untouched ai-indent is stripped
        // at Esc — exactly what a bare `\n` here produces.
        "\n".to_owned()
    } else {
        ch.to_string().repeat(replaced)
    };
    vim.edit_replace(ctx, start..o, &replacements);
    if ch == '\n' {
        // `r<CR>` splits the line: vim parks the cursor on the FIRST
        // character of the new next line (9.1 probe: 'abc' + r<CR> → cursor
        // (2,1)) — there is no replaced char to sit on
        vim.cursor.offset = clamp_cursor(ctx.buf, start + 1);
    } else {
        vim.cursor.offset = clamp_cursor(ctx.buf, start + replacements.len() - ch.len_utf8());
    }
    vim.cursor.desired_col = None;
    true
}

/// `r{char}` with `CTRL-E`/`CTRL-Y` as the char (`:h r`): each replaced
/// character takes the char from the line BELOW (`C-e`) / ABOVE (`C-y`) at
/// the same display column — `10r<C-E>` copies 10 characters from the line
/// below. Either side running out (count past the cursor line's end, the
/// neighbor line shorter than the start column or the count) cancels the
/// whole replace, like every other failed `r`. Returns false when cancelled.
pub fn replace_chars_from_neighbor(
    vim: &mut VimState,
    ctx: &mut Ctx,
    below: bool,
    count: usize,
) -> bool {
    let count = count.max(1);
    let start = vim.cursor.offset;
    let line = ctx.buf.offset_to_line(start);
    let cursor_line_end = ctx.buf.line_end(line);
    let neighbor = if below {
        if line + 1 >= ctx.buf.line_count() {
            return false;
        }
        line + 1
    } else {
        if line == 0 {
            return false;
        }
        line - 1
    };
    // same DISPLAY column on the neighbor line (wide chars align like the
    // terminal shows them, mirroring i_CTRL-E)
    let mut col = crate::buffer::display_column(ctx.buf, start);
    let src = crate::buffer::offset_for_display_column(ctx.buf, neighbor, col);
    let neighbor_end = ctx.buf.line_end(neighbor);

    // 逐位配对：光标行的第 i 个字素取邻行同显示列起连续第 i 个字符。vim
    // 逐位处理、某位在邻行取不到字符时**跳过**（ins_copychar 返回 NUL →
    // 仅推进光标，normal.c nv_replace「will be decremented further
    // down」）——短邻行不放弃整个命令。9.1 oracle：`abcdef` + `abX` 上
    // `0ll2r<C-E>` → `abXdef`（'c'←'X'，'d' 跳过），光标落最后处理位
    // 'd'。全部跳过 = 无操作（无编辑、无铃声、光标不动）。
    //
    // 越列判定：`offset_for_display_column` 对越列查询钳回最后字素起点
    // （那是 `j`/`|` 的运动落点语义），不能当「该列有字符」用——要用
    // 覆盖判定（字符的显示列区间盖住目标列）。
    let mut replacements = String::new();
    let mut end = start;
    let mut last_pos_new_start = start;
    let mut any_replaced = false;
    let mut nsrc = src;
    for _ in 0..count {
        if end >= cursor_line_end {
            return false;
        }
        let next = advance_graphemes(ctx.buf, end, 1, cursor_line_end);
        if next == end {
            return false;
        }
        last_pos_new_start = start + replacements.len();
        let covered = nsrc < neighbor_end
            && match ctx.buf.char_at(nsrc) {
                Some(c) => {
                    let cs = crate::buffer::display_column(ctx.buf, nsrc);
                    let w = crate::buffer::char_display_width(c);
                    let on = cs <= col && col < cs + w;
                    if on {
                        replacements.push(c);
                        nsrc += c.len_utf8();
                        any_replaced = true;
                    }
                    on
                }
                None => false,
            };
        if !covered {
            // 跳过位保留光标行原字符（拼进替换串 = 文本不变）
            replacements.push_str(&ctx.buf.slice(end..next));
        }
        end = next;
        // 下一位的邻行配对列 = 下一个光标字符自己的显示列（宽字符对齐
        // 终端列；vim 的 ++col 是字节制 quirk，这里按显示列泛化）
        col = crate::buffer::display_column(ctx.buf, end);
    }
    if !any_replaced {
        return true;
    }
    vim.edit_replace(ctx, start..end, &replacements);
    // cursor on the last processed position's start in the NEW text (the
    // pre-edit `end` no longer exists when the replacement is shorter —
    // the old `end - last_len` landed inside the NEXT multibyte char,
    // fuzz round35)
    vim.cursor.offset = clamp_cursor(ctx.buf, last_pos_new_start);
    vim.cursor.desired_col = None;
    true
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
            // bottom-up so earlier row offsets stay valid; fill counts
            // CLUSTERS (a composed char takes one replacement char)
            for range in block.rows.iter().rev() {
                if range.is_empty() {
                    continue;
                }
                let n = crate::buffer::grapheme_count(ctx.buf, range.clone());
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
                let n = crate::buffer::grapheme_count(ctx.buf, ls..le);
                vim.edit_replace(ctx, ls..le, &replacement.repeat(n));
            }
            vim.cursor.offset = ctx.buf.first_non_blank(first.min(ctx.buf.line_count() - 1));
        }
        VisualKind::Char => {
            let Some(span) = span_from_visual(vim, ctx.buf) else {
                return;
            };
            let n = crate::buffer::grapheme_count(ctx.buf, span.start..span.end);
            vim.edit_replace(ctx, span.start..span.end, &replacement.repeat(n));
            vim.cursor.offset = clamp_cursor(ctx.buf, span.start);
        }
    }
    vim.cursor.desired_col = None;
}

/// `~`: toggle case of `count` chars, cursor ends on the last one.
///
/// Each step consumes one GRAPHEME cluster but only the base char's case is
/// mapped — the cluster's tail (combining marks, ZWJ-joined emoji) is carried
/// over verbatim. Writing back only `toggle_case(base)` deleted the tail
/// silently (`"e"+U+0301` → `"E"`, the accent gone; an emoji family lost its
/// members — vim 9.1 keeps both), so the tail is sliced out of the consumed
/// span and re-appended.
pub fn toggle_chars(vim: &mut VimState, ctx: &mut Ctx, count: usize) -> bool {
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
        // the cluster tail after the base char (0 for a plain char)
        let tail = ctx.buf.slice(o + c.len_utf8()..next);
        mapped.push_str(&tail);
        o = next;
    }
    if mapped.is_empty() {
        return false;
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
    true
}
