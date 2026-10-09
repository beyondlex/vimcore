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
    /// Typed `1%`: the file-percentage form. `1G` has the same
    /// absent-vs-typed-1 ambiguity and is rewritten to `gg`; here the
    /// rewrite lands on this variant (state.rs) — by construction it only
    /// ever carries the typed-`1` spelling, so the percentage formula
    /// computes the 1% line directly (`ceil(total/100)`, matching the
    /// `count*total` formula of the `MatchBracket` count arm at count=1).
    /// Counts 2..=100 take the MatchBracket percentage arm instead.
    GoToFilePercent,
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
        /// `g*`/`g#`: the word becomes a SUBSTRING pattern (audit F2 — the
        /// old keys degraded to whole-word `*`/`#`)
        substring: bool,
    }, // * / # / g* / g#
    /// `go`: jump to byte [count] (1-based; `:h go`, audit B9)
    GoToByte,
    /// `gm`: half a screen row across (`:h gm`, audit B5 — the engine has no
    /// soft wrap, so this is display column `columns/2` of the current line)
    ScreenRowMiddle,
    /// `gj` / `gk`: one SCREEN row down/up (`:h gj`, audit B6). With no
    /// soft wrap modeled these move within the line by one `'columns'`
    /// multiple and fall back to j/k at the line's screen edges.
    ScreenLine {
        down: bool,
    },
    /// `gd` / `gD`: jump to the first whole-word declaration of the word
    /// `gd`/`gD`: jump to the (first whole-word match at/after the anchor
    /// of the) declaration — `gD` anchors line 1; `gd` runs the `[[`
    /// function-start search with blank-line backoff first (`:h gd`,
    /// audit F1 + 2026-10-09 oracle 矩阵)
    SearchDeclaration {
        whole_file: bool,
    },
    /// `]]`/`[[`/`][`/`[]` (sections: `{`/`}` in COLUMN 1) and `]m`/`[m`/`]M`/
    /// `[M` (methods: `{`/`}` NOT in column 1) — audit A3; the whole family
    /// used to be unbound
    Section {
        ch: char,
        backward: bool,
        /// method form: the brace must NOT sit in column 1
        method: bool,
    },
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
    /// `_`: down `count - 1` lines, first non-blank (`:h _`) — audit K4
    Underscore,
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
            | Motion::GoToFilePercent
            | Motion::ScreenTop
            | Motion::ScreenMiddle
            | Motion::ScreenBottom
            | Motion::PageUp
            | Motion::PageDown
            | Motion::LineDownFirstNonBlank
            | Motion::LineUpFirstNonBlank
            | Motion::Underscore => MotionKind::Linewise,
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
                | Motion::SearchDeclaration { .. }
                | Motion::GoToByte
                | Motion::Section { .. }
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
                let o = crate::buffer::offset_for_display_column_with(
                    buf,
                    target_line,
                    desired,
                    vim.options.tabstop,
                );
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
                    // start of the LAST GRAPHEME (byte arithmetic `end - 1`
                    // lands mid-char on multibyte tails; prev_char_offset
                    // lands mid-cluster on trailing combining marks / ZWJ
                    // families — `中文` + `$` sits on 文, `"e\u{0301}"` + `$`
                    // sits on e)
                    let last_start = crate::buffer::prev_grapheme_offset(buf, end)
                        .unwrap_or(end - 1)
                        .max(buf.line_start(line));
                    MotionResult::new(last_start, MotionKind::Inclusive)
                }
            }
            // g_: last NON-blank char of the count-th line
            Motion::LastLineNonBlank => {
                // g_: to the last non-blank of the (count-th) line; trailing
                // blanks are skipped, and the landing spot is a character
                // start (same multibyte hazard as `$`). On a whitespace-only
                // line the scan walks down to line_start: vim 9.1 probes on
                // `   ` delete from line start THROUGH the cursor
                // (`dg_` from c1/c2/c3 removes 1/2/3 chars — an inclusive
                // span anchored at line_start). An EMPTY line never enters
                // the loop (o == end) and takes the exclusive no-op branch,
                // which keeps `d$`-on-empty-line semantics for `dg_` too.
                let line =
                    (buf.offset_to_line(vim.cursor.offset) + count - 1).min(buf.line_count() - 1);
                let end = buf.line_end(line);
                let mut o = end;
                while let Some(prev) = buf.prev_char_offset(o) {
                    if prev < buf.line_start(line) {
                        break;
                    }
                    match buf.char_at(prev) {
                        // width-0 chars (combining marks) are not landing
                        // spots: they attach to the base before them, and a
                        // cursor parked there would let `x` split the cluster.
                        // Blank check is is_whitespace (U+3000 trailing pad
                        // is blank to vim's g_ too)
                        Some(c) if !c.is_whitespace() && crate::buffer::char_display_width(c) > 0 => {
                            o = prev;
                            break;
                        }
                        _ => o = prev,
                    }
                }
                if o == end {
                    // EMPTY line: no last char to be inclusive of. An
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
                    let next = word::next_word_start(buf, o, big);
                    // buffer end or a self-returning step stops the count
                    // (see the paragraph loops below for the spin hazard)
                    if next == o || next >= buf.len() {
                        o = next;
                        break;
                    }
                    o = next;
                }
                MotionResult::new(o.min(buf.len()), MotionKind::Exclusive)
            }
            // e / E: end of the current-or-next word run (inclusive)
            Motion::WordEnd { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::next_word_end(buf, o, big);
                    // fixed point (cursor on the buffer-final word char):
                    // stop counting — `999999999e` used to run the full
                    // word scan a billion times instead of bell-returning
                    // like vim's motion loop does (WordEndBack's guard, twin)
                    if next == o {
                        break;
                    }
                    o = next;
                }
                MotionResult::new(o, MotionKind::Inclusive)
            }
            // b / B: start of the previous word run
            Motion::WordBack { big } => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::prev_word_start(buf, o, big);
                    if next == o {
                        break;
                    }
                    o = next;
                }
                if o == vim.cursor.offset {
                    // stuck at the buffer start: vim's nv_bcmd beeps
                    // (WordEndBack's twin — the ge arm got this first,
                    // audit5 B-2: b/B at the origin were silently "fine")
                    MotionResult::stuck(o)
                } else {
                    MotionResult::new(o, MotionKind::Exclusive)
                }
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
            // file instead (`50%` = halfway down, first non-blank); the
            // typed `1%` form arrives as [`Motion::GoToFilePercent`].
            //
            // vim's formula is ceil on the 1-based line: `(count*total+99)/100`
            // (nv_percent). The naive floor `count*total/100` was one line
            // low whenever the product was an exact multiple of 100 (9.1:
            // `2%` on 200 lines = line 4, not 5; `50%` = 100, not 101).
            // Counts above 100 are rejected by vim (clearopbeep).
            Motion::MatchBracket if count > 1 => {
                if count > 100 {
                    return MotionResult::stuck(vim.cursor.offset);
                }
                let total = buf.line_count() as u64;
                let line = ((count as u64 * total).div_ceil(100).clamp(1, total) - 1) as usize;
                MotionResult::new(buf.line_start(line), MotionKind::Linewise)
            }
            Motion::GoToFilePercent => {
                let total = buf.line_count().max(1) as u64;
                let line = ((total).div_ceil(100).clamp(1, total) - 1) as usize;
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
            //
            // The repeat loops below all stop at a fixed point: at buffer
            // end / offset 0 every further step returns the same offset and
            // a huge count used to spin the full line walk per iteration
            // (`999999999}` on a 1MB buffer ≈ forever; vim's motion loop
            // gives up at the first non-advancing step).
            Motion::ParaNext => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::next_paragraph(buf, o);
                    if next == o {
                        break;
                    }
                    o = next;
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // {: previous paragraph boundary
            Motion::ParaPrev => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::prev_paragraph(buf, o);
                    if next == o {
                        break;
                    }
                    o = next;
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // ): next sentence end. Divergence from vim: only `.!?` single
            // chars end sentences here — no `...` or newline-follow rules.
            Motion::SentenceNext => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::next_sentence(buf, o);
                    if next == o {
                        break;
                    }
                    o = next;
                }
                MotionResult::new(o, MotionKind::Exclusive)
            }
            // (: previous sentence end (same divergence as `)`)
            Motion::SentencePrev => {
                let mut o = vim.cursor.offset;
                for _ in 0..count {
                    let next = word::prev_sentence(buf, o);
                    if next == o {
                        break;
                    }
                    o = next;
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
                        // a remembered offset (`/pat/e+2`) re-applies on
                        // EVERY repeat, like vim (`:h search-offset` — the
                        // field doc promised it; audit F3)
                        let target = match vim.search.offset.clone() {
                            Some(off) => {
                                search::apply_search_offset(vim, buf, &off);
                                vim.cursor.offset
                            }
                            None => o,
                        };
                        MotionResult::new(target, MotionKind::Exclusive)
                    }
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // * / #: word under cursor becomes the pattern, then jump. No
            // word on this line → stuck (bell), NOT a jump with the stale
            // pattern from a previous search. `{count}*` skips count-1
            // further matches (9.1: `2*` lands on the second next one).
            Motion::StarSearch { forward, substring } => {
                if !search::search_word_under_cursor(vim, buf, ctx.host, forward, !substring) {
                    return MotionResult::stuck(vim.cursor.offset);
                }
                match search::jump_to_match(vim, buf, forward, count) {
                    Some(o) => MotionResult::new(o, MotionKind::Exclusive),
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // go: byte [count] (1-based), floored to a char boundary
            Motion::GoToByte => {
                let target = count.max(1).saturating_sub(1).min(buf.len());
                let target = crate::buffer::floor_to_char_boundary(buf, target);
                MotionResult::new(target, MotionKind::Exclusive)
            }
            // gm: half a screen row across (audit B5)
            Motion::ScreenRowMiddle => {
                let columns = ctx.host.columns();
                let want = columns.max(2) / 2;
                let line = buf.offset_to_line(vim.cursor.offset);
                let target = crate::buffer::offset_for_display_column_with(
                    buf,
                    line,
                    want,
                    vim.options.tabstop,
                );
                MotionResult::new(target, MotionKind::Exclusive)
            }
            // gj / gk: one screen row down/up. Without a wrap model these
            // walk `'columns'`-wide display rows WITHIN the line and fall
            // back to the plain j/k at the edges (audit B6)
            Motion::ScreenLine { down } => {
                let columns = ctx.host.columns().max(1);
                let here = vim.cursor.offset;
                let line = buf.offset_to_line(here);
                let col = vim.display_column_ts(buf, here);
                let row = col / columns;
                let le = buf.line_end(line);
                let width = vim.display_column_ts(buf, le);
                if down {
                    let want = (row + 1) * columns;
                    if want <= width {
                        let target = crate::buffer::offset_for_display_column_with(
                            buf,
                            line,
                            want,
                            vim.options.tabstop,
                        );
                        MotionResult::new(target, MotionKind::Exclusive)
                    } else {
                        // past this line's last screen row: plain j fallback
                        // (computed inline — the buf borrow spans the arm)
                        let next = line + 1;
                        if next >= buf.line_count() {
                            return MotionResult::stuck(here);
                        }
                        let target = crate::buffer::offset_for_display_column_with(
                            buf,
                            next,
                            col,
                            vim.options.tabstop,
                        );
                        MotionResult::new(target, MotionKind::Linewise)
                    }
                } else {
                    if row > 0 {
                        let want = (row - 1) * columns;
                        let target = crate::buffer::offset_for_display_column_with(
                            buf,
                            line,
                            want,
                            vim.options.tabstop,
                        );
                        MotionResult::new(target, MotionKind::Exclusive)
                    } else {
                        if line == 0 {
                            return MotionResult::stuck(here);
                        }
                        let target = crate::buffer::offset_for_display_column_with(
                            buf,
                            line - 1,
                            col,
                            vim.options.tabstop,
                        );
                        MotionResult::new(target, MotionKind::Linewise)
                    }
                }
            }
            // gd / gD: jump to the declaration of the word under the cursor.
            // gd scans from the START of the current line backward, then
            // forward from the cursor; gD takes the first match in the file
            // (`:h gd` — audit F1; unbound, the `g` miss used to re-feed the
            // `d` and arm the delete operator)
            Motion::SearchDeclaration { whole_file } => {
                let here = vim.cursor.offset;
                let Some((start, end)) = search::word_bounds_at(buf, here) else {
                    return MotionResult::stuck(here);
                };
                let literal = buf.slice(start..end);
                if literal.is_empty() {
                    return MotionResult::stuck(here);
                }
                let pattern = format!(r"\b{}\b", regex::escape(&literal));
                let matches = search::all_matches(vim, buf, &pattern);
                if matches.is_empty() {
                    return MotionResult::stuck(here);
                }
                // `:h gd` 的锚定（2026-10-09 oracle 矩阵 T1-T5）：gD 锚在
                // 行 1；gd 先向 [[ 找上一个第 1 列的 `{`——找不到就停在行 1
                // 且**不**做空行回退，找到了才从 `{` 行向上越过非空行（空行
                // 或行 1 截止）——然后从锚点行起向前搜全词匹配的第一个。
                // 旧实现「光标前最近一次 / 光标后第一个」是另一套模型：
                // 无函数结构的常见形状 vim 落行 1 的首个出现，旧实现落
                // 光标前最近的一次。
                let anchor = if whole_file {
                    0
                } else {
                    let cur_line = buf.offset_to_line(here);
                    match (0..cur_line)
                        .rev()
                        .find(|&l| buf.char_at(buf.line_start(l)) == Some('{'))
                    {
                        None => 0,
                        Some(brace_line) => {
                            let mut anchor = brace_line;
                            while anchor > 0 && !buf.line_is_blank(anchor - 1) {
                                anchor -= 1;
                            }
                            anchor
                        }
                    }
                };
                let found = matches
                    .iter()
                    .find(|m| buf.offset_to_line(m.start) >= anchor)
                    .cloned();
                match found {
                    Some(m) => MotionResult::new(m.start, MotionKind::Exclusive),
                    None => MotionResult::stuck(here),
                }
            }
            // '{char} / `{char}: the mark name arrives as a char argument
            Motion::MarkJump { linewise } => {
                let Some(name) = vim.char_arg else {
                    return MotionResult::stuck(vim.cursor.offset);
                };
                match vim.marks.resolve(name, buf) {
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
                            // `]` lands ON the stored mark with no step-back
                            // (2026-10-09 oracle: after `0lliXYZ<Esc>` the
                            // `] jump parks at byte 5 — one PAST 'Z', the
                            // ins_esc b_op_end value verbatim; nv_brackets
                            // has no dec here despite `:h ']
                            // "last character" wording)
                            MotionResult::new(o, MotionKind::Exclusive)
                        }
                    }
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // ]] / [[ / ][ / []: the next/previous COLUMN-1 brace — sections
            // land at the line start, linewise for operators (`:h ]]`).
            // ]m / [m / ]M / [M: methods, the brace NOT in column 1, landed
            // ON the brace char (audit A3 — the family was unbound)
            Motion::Section {
                ch,
                backward,
                method,
            } => {
                let want = count.max(1);
                // the METHOD flavors ([m/]m/[M/]M) and the SECTION flavors
                // (]]/[[/][/[]) follow different models:
                //
                // SECTIONS: the next/previous COLUMN-1 brace (`:h ]]`),
                // linewise-style line starts.
                //
                // METHODS (oracle matrix, BUG_AUDIT3 挂账 2 — vim 9.1
                // probes): `]m`/`[m` land on the first brace (ANY kind, ANY
                // column) after/before the cursor; `]M`/`[M` land on the
                // first brace too, UNLESS it is an opener whose
                // nesting-match exists with braces beyond the match — then
                // they land on the MATCH (the method's end). `[count]`
                // iterates the rule. (The C source's nv_bracket_block
                // PHASE1/PHASE2 + findmatchlimit produce these shapes; the
                // closed form here is the observable contract.)
                let open = if ch == '{' { '}' } else { '{' };
                let mut braces: Vec<usize> = Vec::new();
                {
                    let here = vim.cursor.offset;
                    // one walk over the buffer, keeping braces strictly
                    // after (forward) / strictly before (backward) the
                    // cursor, in scan order (backward = reverse)
                    let mut collected: Vec<usize> = Vec::new();
                    let mut o = 0usize;
                    while o < buf.len() {
                        if let Some(c) = buf.char_at(o) {
                            if (c == '{' || c == '}')
                                && ((backward && o < here) || (!backward && o > here))
                            {
                                collected.push(o);
                            }
                            o += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    if backward {
                        collected.reverse();
                    }
                    braces = collected;
                }
                let mut found: Option<usize> = None;
                if !method {
                    // sections: column-1 braces only, in line order
                    let ls_only: Vec<usize> = braces
                        .iter()
                        .copied()
                        .filter(|o| {
                            buf.char_at(*o) == Some(ch)
                                && buf.line_start(buf.offset_to_line(*o)) == *o
                        })
                        .collect();
                    found = ls_only.get(want - 1).copied();
                } else if ch == open {
                    // ]m / [m: the [count]-th brace, any kind
                    found = braces.get(want - 1).copied();
                } else {
                    // ]M / [M: first brace; an opener whose match exists
                    // with braces beyond the match lands the MATCH
                    let mut idx = 0usize;
                    for _ in 0..want {
                        let Some(b1) = braces.get(idx).copied() else {
                            break;
                        };
                        if buf.char_at(b1) == Some(ch) {
                            // the flavor's own kind: land it
                            found = Some(b1);
                            idx += 1;
                        } else {
                            // the other kind: nesting-match from b1
                            let mut depth = 1usize;
                            let mut m: Option<usize> = None;
                            for (j, o) in braces.iter().enumerate().skip(idx + 1) {
                                let c = buf.char_at(*o);
                                let is_open = c == Some(open);
                                depth = if is_open { depth + 1 } else { depth - 1 };
                                if depth == 0 {
                                    m = Some(*o);
                                    idx = j + 1;
                                    break;
                                }
                            }
                            match m {
                                Some(m_pos) if braces[idx..].iter().any(|o| *o > m_pos) => {
                                    // the method closes AND more braces
                                    // follow: land the method's end
                                    found = Some(m_pos);
                                }
                                _ => {
                                    // unterminated method (or nothing past
                                    // its end): land the opener/closer b1
                                    found = Some(b1);
                                    idx += 1;
                                }
                            }
                        }
                    }
                }
                match found {
                    // both section flavors are EXCLUSIVE motions (`:h ]]`,
                    // `:h ]m` — audit B11/B14; the column-1 rule (a) then
                    // promotes operator spans exactly like vim). The old
                    // Linewise/Inclusive kinds made `d]]` wipe whole lines
                    // and `d]m` swallow the brace.
                    Some(o) => MotionResult::new(o, MotionKind::Exclusive),
                    None => MotionResult::stuck(vim.cursor.offset),
                }
            }
            // |: display column `count` (wide chars cover two cells, TAB
            // expands per 'tabstop' — audit A1: `5|` on "ab\tc" parks ON
            // the tab, the old width-1 model landed past it)
            Motion::Column => {
                let line = buf.offset_to_line(vim.cursor.offset);
                let col = count.max(1) - 1;
                MotionResult::new(
                    crate::buffer::offset_for_display_column_with(
                        buf,
                        line,
                        col,
                        vim.options.tabstop,
                    ),
                    MotionKind::Exclusive,
                )
            }
            // H / M / L: viewport-relative rows
            Motion::ScreenTop | Motion::ScreenMiddle | Motion::ScreenBottom => {
                let (first, last) = ctx.host.viewport();
                // saturating: a buggy host may report an inverted/extreme
                // viewport; `first + count` must clamp, not panic (audit A1)
                let line = match self {
                    Motion::ScreenTop => first.saturating_add(count - 1).min(last),
                    Motion::ScreenMiddle => first.saturating_add(last) / 2,
                    _ => last.saturating_sub(count - 1).max(first),
                };
                let line = line.min(buf.line_count() - 1);
                MotionResult::new(buf.line_start(line), MotionKind::Linewise)
            }
            Motion::ScrollHalfDown | Motion::ScrollHalfUp | Motion::PageDown | Motion::PageUp => {
                let (first, last) = ctx.host.viewport();
                let visible = last.saturating_sub(first).max(1);
                // half-page for <C-d>/<C-u>, full page for <C-f>/<C-b>;
                // a typed count scales the scroll (`2<C-f>` = two pages —
                // audit F9; the old code dropped it)
                let base = if matches!(self, Motion::PageDown | Motion::PageUp) {
                    visible
                } else {
                    visible / 2
                };
                let step = base.saturating_mul(count.max(1));
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
                let o = crate::buffer::offset_for_display_column_with(
                    buf,
                    line,
                    desired,
                    vim.options.tabstop,
                );
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
                        // last GRAPHEME start (not `end - 1` bytes: a multi-byte
                        // final char would put the cursor mid-character; and
                        // prev_char_offset would sit on a trailing combining
                        // mark of the match's final cluster)
                        let last = crate::buffer::prev_grapheme_offset(buf, range.end)
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
                    MotionResult::new(buf.first_non_blank(cur - count), MotionKind::Linewise)
                }
            }
            // `_`: down count-1 lines, first non-blank (`:h _`) — audit K4
            Motion::Underscore => {
                let cur = buf.offset_to_line(vim.cursor.offset);
                let line = cur + count - 1;
                if line > buf.line_count() - 1 {
                    MotionResult::stuck(vim.cursor.offset)
                } else {
                    MotionResult::new(buf.first_non_blank(line), MotionKind::Linewise)
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
                // grapheme-aware stop: prev/next CHAR would park on a
                // combining mark adjacent to the hit (mid-cluster)
                let stop = if forward {
                    crate::buffer::prev_grapheme_offset(buf, hit)
                } else {
                    crate::buffer::next_grapheme_offset(buf, hit)
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
            // (grapheme-aware, as in the fresh path above)
            let target_offset = if forward {
                crate::buffer::next_grapheme_offset(buf, o)
            } else {
                crate::buffer::prev_grapheme_offset(buf, o)
            };
            return match target_offset {
                Some(t) => MotionResult::till(o, t),
                None => MotionResult::stuck(start),
            };
        }
        // `till` always returned above (skip / repeat paths); plain `f`/`F`
        // land INCLUSIVE on the matched char
        MotionResult::new(o, MotionKind::Inclusive)
    }
}
