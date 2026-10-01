//! Motions: pure cursor-target computations.
//!
//! `Motion::target` computes where a motion lands and what *kind* of span it
//! describes (`Exclusive`/`Inclusive`/`Linewise`). Turning a target into an
//! operator range — including vim's special cases (`dw` never joins lines,
//! `cw` acts like `ce`, the column-1 rules) — lives in [`crate::ops`].

use crate::buffer::VimBuffer;
use crate::search;
use crate::state::{Ctx, VimState};
use crate::word;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    FirstNonBlank,
    LineEnd,
    LastLineNonBlank, // g_
    WordStart {
        big: bool,
    },
    WordEnd {
        big: bool,
    },
    WordBack {
        big: bool,
    },
    WordEndBack {
        big: bool,
    },
    FindChar {
        forward: bool,
        till: bool,
    },
    RepeatFind {
        reverse: bool,
    }, // ; ,
    MatchBracket, // %
    GoToLine {
        first: bool,
    }, // gg / G
    ParaNext,
    ParaPrev,
    SentenceNext,
    SentencePrev,
    SearchNext {
        forward: bool,
    }, // n / N
    StarSearch {
        forward: bool,
    }, // * / #
    MarkJump {
        linewise: bool,
    }, // '{char} / `{char} as operator target
    Column, // | (to count column)
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    ScrollHalfDown,
    ScrollHalfUp,
    PageDown,
    PageUp,
    LineDownFirstNonBlank, // enter / +
    LineUpFirstNonBlank,   // -
    /// `gn`/`gN` as an operator target (`dgn`): the span is the match
    /// itself, not cursor..target — the operator arm reads
    /// `search.last_found_match`.
    SelectMatch {
        backward: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionResult {
    pub offset: usize,
    pub kind: MotionKind,
    /// False when the motion could not move (e.g. `h` at buffer start).
    pub moved: bool,
    /// Till motions only: the matched target the cursor is parked against
    /// (`offset` is the STOP — one char this side of the target). Operator
    /// spans must END at the target, not the stop: vim `d2t3` on
    /// `a1b2c3d3e` deletes `a1b2c3d` (span [0, target)), while a bare `t`
    /// parks the cursor on the stop (vim 9.1 probes, round 14).
    pub till_target: Option<usize>,
}

impl MotionResult {
    fn new(offset: usize, kind: MotionKind) -> Self {
        MotionResult {
            offset,
            kind,
            moved: true,
            till_target: None,
        }
    }
    fn stuck(offset: usize) -> Self {
        MotionResult {
            offset,
            kind: MotionKind::Exclusive,
            moved: false,
            till_target: None,
        }
    }
    /// A till result: the cursor parks at `stop`, operators span through
    /// `target`.
    fn till(stop: usize, target: usize) -> Self {
        MotionResult {
            offset: stop,
            kind: MotionKind::Exclusive,
            moved: true,
            till_target: Some(target),
        }
    }
}

impl Motion {
    pub fn kind(&self) -> MotionKind {
        match self {
            Motion::Up
            | Motion::Down
            | Motion::GoToLine { .. }
            | Motion::ScreenTop
            | Motion::ScreenMiddle
            | Motion::ScreenBottom
            | Motion::PageUp
            | Motion::PageDown
            | Motion::LineDownFirstNonBlank
            | Motion::LineUpFirstNonBlank => MotionKind::Linewise,
            Motion::LineEnd
            | Motion::LastLineNonBlank
            | Motion::WordEnd { .. }
            | Motion::WordEndBack { .. }
            | Motion::MatchBracket
            | Motion::FindChar { till: false, .. }
            | Motion::RepeatFind { .. }
            | Motion::SelectMatch { .. } => MotionKind::Inclusive,
            _ => MotionKind::Exclusive,
        }
    }

    /// Whether this motion is a "jump" in vim's sense: it belongs on the
    /// jumplist reachable with `C-o` / `C-i`.
    pub fn is_jump(self) -> bool {
        matches!(
            self,
            Motion::GoToLine { .. }
                | Motion::SearchNext { .. }
                | Motion::StarSearch { .. }
                | Motion::ParaNext
                | Motion::ParaPrev
                | Motion::SentenceNext
                | Motion::SentencePrev
                | Motion::MatchBracket
                | Motion::ScreenTop
                | Motion::ScreenMiddle
                | Motion::ScreenBottom
                | Motion::ScrollHalfDown
                | Motion::ScrollHalfUp
                | Motion::PageUp
                | Motion::PageDown
                | Motion::MarkJump { .. }
        )
    }

    /// Compute the landing offset. Applies `count` where it makes sense
    /// (an absent count arrives as 1). Never moves the cursor — callers
    /// decide — but `SearchNext` may re-publish highlights as a side
    /// effect, so this takes `&mut VimState`.
    pub fn target(&self, vim: &mut VimState, ctx: &mut Ctx, count: usize) -> MotionResult {
        let buf = &*ctx.buf;
        let count = count.max(1);
        match *self {
            // h: count graphemes left, stopping at the line start
            Motion::Left => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    match crate::buffer::prev_grapheme_offset(buf, o) {
                        Some(prev) if prev >= buf.line_start(buf.offset_to_line(o)) => o = prev,
                        _ => break,
                    }
                }
                if o == vim.cursor.offset {
                    MotionResult::stuck(o)
                } else {
                    MotionResult::new(o, MotionKind::Exclusive)
                }
            }
            // l: count graphemes right, never stepping onto the newline
            Motion::Right => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let line = buf.offset_to_line(o);
                    let end = buf.line_end(line);
                    if end == buf.line_start(line) || o >= end {
                        break;
                    }
                    // `next < end`: never step onto the newline position
                    match crate::buffer::next_grapheme_offset(buf, o) {
                        Some(next) if next < end => o = next,
                        _ => break,
                    }
                }
                if o == vim.cursor.offset {
                    MotionResult::stuck(o)
                } else {
                    MotionResult::new(o, MotionKind::Exclusive)
                }
            }
            // j/k: keep the desired display column (`desired_col` memoizes
            // it across intermediate lines, like vim's wv_col)
            Motion::Up | Motion::Down => {
                let dir = if matches!(self, Motion::Up) { -1i64 } else { 1 };
                let start_line = buf.offset_to_line(vim.cursor.offset) as i64;
                let target_line = (start_line + dir * count as i64)
                    .clamp(0, buf.line_count() as i64 - 1)
                    as usize;
                let desired = vim.desired_column(buf);
                let o = crate::buffer::offset_for_display_column(buf, target_line, desired);
                let moved = target_line != start_line as usize;
                if moved {
                    MotionResult::new(o, MotionKind::Linewise)
                } else {
                    MotionResult::stuck(vim.cursor.offset)
                }
            }
            // 0: byte offset of the line start
            Motion::LineStart => MotionResult::new(
                buf.line_start(buf.offset_to_line(vim.cursor.offset)),
                MotionKind::Exclusive,
            ),
            // ^: first non-blank char of the current line
            Motion::FirstNonBlank => MotionResult::new(
                buf.first_non_blank(buf.offset_to_line(vim.cursor.offset)),
                MotionKind::Exclusive,
            ),
            Motion::LineEnd => {
                // `count$` ends at the END of the count-th line down (`2$`,
                // `d2$` cross the newline like vim)
                let line =
                    (buf.offset_to_line(vim.cursor.offset) + count - 1).min(buf.line_count() - 1);
                let end = buf.line_end(line);
                if end == buf.line_start(line) {
                    // EMPTY line: no last char to be inclusive of. Returning
                    // the newline position with Inclusive made `d$` swallow
                    // the line break and merge two lines (9.1 probe: `d$` on
                    // an empty line is a no-op). Park at the line start with
                    // an empty exclusive span instead.
                    MotionResult::new(buf.line_start(line), MotionKind::Exclusive)
                } else {
                    // start of the LAST character: byte arithmetic `end - 1`
                    // lands mid-char when the line ends with a multibyte
                    // char (`中文` + `$` must sit on 文, not inside it)
                    let last_start = buf
                        .prev_char_offset(end)
                        .unwrap_or(end - 1)
                        .max(buf.line_start(line));
                    MotionResult::new(last_start, MotionKind::Inclusive)
                }
            }
            // g_: last NON-blank char of the count-th line
            Motion::LastLineNonBlank => {
                // g_: to the last non-blank of the (count-th) line; trailing
                // blanks are skipped, and the landing spot is a character
                // start (same multibyte hazard as `$`)
                let line =
                    (buf.offset_to_line(vim.cursor.offset) + count - 1).min(buf.line_count() - 1);
                let end = buf.line_end(line);
                let mut o = end;
                while let Some(prev) = buf.prev_char_offset(o) {
                    if prev < buf.line_start(line) {
                        break;
                    }
                    match buf.char_at(prev) {
                        Some(c) if c != ' ' && c != '\t' => {
                            o = prev;
                            break;
                        }
                        _ => o = prev,
                    }
                }
                if o == end {
                    // whitespace-only line: no non-blank to land on; an
                    // Inclusive span at the newline position would swallow
                    // the line break (same hazard as `d$` on an empty line)
                    MotionResult::new(buf.line_start(line), MotionKind::Exclusive)
                } else {
                    MotionResult::new(o, MotionKind::Inclusive)
                }
            }
            // w / W: start of the next word run
            Motion::WordStart { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::next_word_start(buf, o, big);
                    if o >= buf.len() {
                        break;
                    }
                }
                MotionResult::new(o.min(buf.len()), MotionKind::Exclusive)
            }
            // e / E: end of the current-or-next word run (inclusive)
            Motion::WordEnd { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::next_word_end(buf, o, big);
                }
                MotionResult::new(o, MotionKind::Inclusive)
            }
            // b / B: start of the previous word run
            Motion::WordBack { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::prev_word_start(buf, o, big);
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // ge / gE: end of the previous word run
            Motion::WordEndBack { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::prev_word_end(buf, o, big);
                    if next == o {
                        break;
                    }
                    o = next;
                }
                if o == vim.cursor.offset {
                    MotionResult::stuck(o)
                } else {
                    MotionResult::new(o, MotionKind::Inclusive)
                }
            }
            // f/t/F/T: reuse the last find's target char (for `;`/`,`).
            // The char for a FRESH find arrives as a char argument.
            Motion::FindChar { forward, till } => {
                let Some((target_char, _, _)) = vim.last_find else {
                    return MotionResult::stuck(vim.cursor.offset);
                };
                // a fresh char-argument find never skips: the cursor may
                // legitimately sit right before the target (`tx` with the
                // target adjacent must still succeed and let `dx` delete it)
                Self::find_from(vim, buf, target_char, forward, till, count, false)
            }
            // ; / ,: repeat the last find, `,` mirroring its direction
            Motion::RepeatFind { reverse } => {
                let Some((target_char, forward, till)) = vim.last_find else {
                    return MotionResult::stuck(vim.cursor.offset);
                };
                let (forward, till) = if reverse {
                    (!forward, till)
                } else {
                    (forward, till)
                };
                Self::find_from(vim, buf, target_char, forward, till, count, true)
            }
            // %: jump to the bracket matching the one under the cursor.
            // With an explicit count vim jumps to that PERCENTAGE of the
            // file instead (`50%` = halfway down, first non-blank).
            Motion::MatchBracket if count > 1 => {
                let total = buf.line_count() as u64;
                let line = (count as u64 * total / 100).clamp(0, total - 1) as usize;
                MotionResult::new(buf.line_start(line), MotionKind::Linewise)
            }
            Motion::MatchBracket => match word::match_bracket(buf, vim.cursor.offset) {
                Some(o) => MotionResult::new(o, MotionKind::Inclusive),
                None => MotionResult::stuck(vim.cursor.offset),
            },
            // gg / G: an explicit count (`1G`, `2gg`) is the line number;
            // the bare forms go to first/last line. execute_command rewrites
            // an explicit `1G` to `first=true` before the count collapses.
            Motion::GoToLine { first } => {
                let line = if count > 1 {
                    (count - 1).min(buf.line_count() - 1)
                } else if first {
                    0
                } else {
                    buf.line_count() - 1
                };
                MotionResult::new(buf.line_start(line), MotionKind::Linewise)
            }
            // }: next paragraph boundary (blank line)
            Motion::ParaNext => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::next_paragraph(buf, o);
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // {: previous paragraph boundary
            Motion::ParaPrev => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::prev_paragraph(buf, o);
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // ): next sentence end. Divergence from vim: only `.!?` single
            // chars end sentences here — no `...` or newline-follow rules.
            Motion::SentenceNext => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::next_sentence(buf, o);
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // (: previous sentence end (same divergence as `)`)
            Motion::SentencePrev => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    o = word::prev_sentence(buf, o);
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // n / N: jump to the count-th next/previous match of the
            // active pattern. The table's `forward` flag is the REPEAT
            // polarity (`n` = same direction as the last search, `N` =
            // opposite), not the direction itself: after `?foo` a plain `n`
            // must keep searching BACKWARD, like vim.
            Motion::SearchNext { forward } => {
                let dir = if forward {
                    vim.search.forward
                } else {
                    !vim.search.forward
                };
                match search::jump_to_match(vim, buf, dir, count) {
                    Some(o) => {
                        // re-publish the matches: after Esc dismissed the
                        // highlights (`:noh` semantics) `n`/`N` re-arms them
                        if vim.options.hlsearch {
                            let current = vim
                                .search
                                .last_matches
                                .iter()
                                .find(|m| m.start == o)
                                .cloned();
                            ctx.host
                                .set_search_highlights(&vim.search.last_matches, current);
                        }
                        MotionResult::new(o, MotionKind::Exclusive)
                    }
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // * / #: word under cursor becomes the pattern, then jump. No
            // word on this line → stuck (bell), NOT a jump with the stale
            // pattern from a previous search. `{count}*` skips count-1
            // further matches (9.1: `2*` lands on the second next one).
            Motion::StarSearch { forward } => {
                if !search::search_word_under_cursor(vim, buf, ctx.host, forward) {
                    return MotionResult::stuck(vim.cursor.offset);
                }
                match search::jump_to_match(vim, buf, forward, count) {
                    Some(o) => MotionResult::new(o, MotionKind::Exclusive),
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // '{char} / `{char}: the mark name arrives as a char argument
            Motion::MarkJump { linewise } => {
                let Some(name) = vim.char_arg else {
                    return MotionResult::stuck(vim.cursor.offset);
                };
                match vim.marks.resolve(name) {
                    Some(o) => {
                        // a length-preserving replace can leave a stored
                        // mark mid-character; floor before cursor math
                        let o = crate::buffer::floor_to_char_boundary(buf, o);
                        if linewise {
                            MotionResult::new(
                                buf.line_start(buf.offset_to_line(o)),
                                MotionKind::Linewise,
                            )
                        } else {
                            MotionResult::new(o, MotionKind::Exclusive)
                        }
                    }
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // |: display column `count` (wide chars cover two cells)
            Motion::Column => {
                let line = buf.offset_to_line(vim.cursor.offset);
                // `|` counts display columns (wide chars cover two)
                let col = count.max(1) - 1;
                MotionResult::new(
                    crate::buffer::offset_for_display_column(buf, line, col),
                    MotionKind::Exclusive,
                )
            }
            // H / M / L: viewport-relative rows
            Motion::ScreenTop | Motion::ScreenMiddle | Motion::ScreenBottom => {
                let (first, last) = ctx.host.viewport();
                let line = match self {
                    Motion::ScreenTop => (first + count - 1).min(last),
                    Motion::ScreenMiddle => (first + last) / 2,
                    _ => last.saturating_sub(count - 1).max(first),
                };
                let line = line.min(buf.line_count() - 1);
                MotionResult::new(buf.line_start(line), MotionKind::Linewise)
            }
            Motion::ScrollHalfDown | Motion::ScrollHalfUp | Motion::PageDown | Motion::PageUp => {
                let (first, last) = ctx.host.viewport();
                let visible = last.saturating_sub(first).max(1);
                // half-page for <C-d>/<C-u>, full page for <C-f>/<C-b>
                let step = if matches!(self, Motion::PageDown | Motion::PageUp) {
                    visible
                } else {
                    visible / 2
                };
                let dir = if matches!(self, Motion::ScrollHalfDown | Motion::PageDown) {
                    1i64
                } else {
                    -1
                };
                let line = buf.offset_to_line(vim.cursor.offset) as i64 + dir * step as i64;
                let line = line.clamp(0, buf.line_count() as i64 - 1) as usize;
                // keep the DISPLAY column, exactly like j/k: `desired` counts
                // cells, so on CJK/Tab rows it must be mapped back to a byte
                // offset (a plain `line_start + desired` would land mid-char)
                let desired = vim.desired_column(buf);
                let o = crate::buffer::offset_for_display_column(buf, line, desired);
                MotionResult::new(o, MotionKind::Linewise)
            }
            // gn / gN as operator target: the match containing the cursor,
            // else the next one in `backward`'s opposite direction
            Motion::SelectMatch { backward } => {
                let mut from = vim.cursor.offset;
                let mut found = None;
                for step in 0..count {
                    // the first step prefers the match containing the cursor;
                    // further counts step strictly past the one just found
                    // (otherwise `2gN` re-selected the same match forever)
                    match search::find_match_from(vim, buf, from, backward, step > 0) {
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
                match found {
                    Some(range) => {
                        vim.search.last_found_match = Some(range.clone());
                        // last char START (not `end - 1` bytes: a multi-byte
                        // final char would put the cursor mid-character)
                        let last = buf
                            .prev_char_offset(range.end)
                            .unwrap_or(range.start)
                            .max(range.start);
                        MotionResult::new(last, MotionKind::Inclusive)
                    }
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // + / -: adjacent line, first non-blank char. At the buffer edge
            // there is no adjacent line: the motion FAILS (9.1 probe: `d-`
            // on line 1 and `d+` on the last line are no-ops, they don't
            // delete the current line).
            Motion::LineDownFirstNonBlank => {
                let cur = buf.offset_to_line(vim.cursor.offset);
                let line = cur + count;
                if line > buf.line_count() - 1 {
                    MotionResult::stuck(vim.cursor.offset)
                } else {
                    MotionResult::new(buf.first_non_blank(line), MotionKind::Linewise)
                }
            }
            Motion::LineUpFirstNonBlank => {
                let cur = buf.offset_to_line(vim.cursor.offset);
                if cur < count {
                    MotionResult::stuck(vim.cursor.offset)
                } else {
                    MotionResult::new(
                        buf.first_non_blank(cur - count),
                        MotionKind::Linewise,
                    )
                }
            }
        }
    }

    fn find_from(
        vim: &VimState,
        buf: &dyn VimBuffer,
        target: char,
        forward: bool,
        till: bool,
        count: usize,
        is_repeat: bool,
    ) -> MotionResult {
        let start = vim.cursor.offset;
        let mut o = start;
        // A `;`/`,` repeat of a till motion re-runs the search from the
        // cursor — which after a `t` sits right before the target, so the
        // scan would immediately re-hit the same character and the repeat
        // would silently succeed without moving (vim: `tx;` advances to the
        // NEXT x, `t,;;` walks target by target). vim skips that adjacent
        // target ONCE, up front; the count then counts find attempts, which
        // may re-hit (probe 9.1: `t3` then `2;` advances exactly ONE target,
        // `t3;;;` walks one per `;`).
        if till && is_repeat {
            if forward {
                if let Some(next) = buf.next_char_offset(o) {
                    if buf.char_at(next) == Some(target) {
                        o = next;
                    }
                }
            } else if let Some(prev) = buf.prev_char_offset(o) {
                if buf.char_at(prev) == Some(target) {
                    o = prev;
                }
            }
        }
        if !is_repeat {
            // A fresh char-argument find never skips (the adjacent target
            // still counts — `t3` beside a 3 succeeds in place), and count
            // counts DISTINCT targets: resume each find from the target
            // itself so one occurrence is never counted twice (probe 9.1:
            // `2t3` lands before the SECOND 3 both from beside the first
            // and from afar). The till stop is applied once, to the final
            // hit — the intermediate stops are irrelevant to the count.
            let mut hit = o;
            for _ in 0..count {
                let found = if forward {
                    word::find_char_forward(buf, o, target, false)
                } else {
                    word::find_char_backward(buf, o, target, false)
                };
                let Some(h) = found else {
                    return MotionResult::stuck(start);
                };
                hit = h;
                o = h;
            }
            if till {
                let stop = if forward {
                    buf.prev_char_offset(hit)
                } else {
                    buf.next_char_offset(hit)
                };
                // the stop char must sit ON the hit's line (same rule as
                // find_char_forward/backward's till branch)
                let on_line = match stop {
                    Some(s) => {
                        let line = buf.offset_to_line(hit);
                        if forward {
                            s >= buf.line_start(line)
                        } else {
                            s < buf.line_end(line)
                        }
                    }
                    None => false,
                };
                if !on_line {
                    return MotionResult::stuck(start);
                }
                return MotionResult::till(stop.unwrap(), hit);
            }
            return MotionResult::new(hit, MotionKind::Inclusive);
        }
        for _ in 0..count {
            let found = if forward {
                word::find_char_forward(buf, o, target, till)
            } else {
                word::find_char_backward(buf, o, target, till)
            };
            match found {
                Some(hit) => o = hit,
                None => return MotionResult::stuck(start),
            }
        }
        if till {
            // the stop the loop parked on sits one char from the matched
            // target — record it so operator spans reach the target
            let target_offset = if forward {
                buf.next_char_offset(o)
            } else {
                buf.prev_char_offset(o)
            };
            return match target_offset {
                Some(t) => MotionResult::till(o, t),
                None => MotionResult::stuck(start),
            };
        }
        MotionResult::new(
            o,
            if till {
                MotionKind::Exclusive
            } else {
                MotionKind::Inclusive
            },
        )
    }
}
