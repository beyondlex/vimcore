//! Search: `/` `?` `n` `N` `*` `#`, incremental highlighting.
//!
//! Patterns use Rust `regex` syntax, which covers common vim "magic" patterns
//! (documented divergence: `\bsomething` style lookarounds follow RE2 rules).

use crate::buffer::VimBuffer;
use crate::state::{Ctx, VimState};
use crate::word::is_word_char;
use regex::RegexBuilder;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchState {
    pub pattern: Option<String>,
    pub forward: bool,
    /// The active search offset (`/pat/e+2`) — reapplied by `n`/`N` like
    /// vim (`:h search-offset`; audit F3).
    pub offset: Option<SearchOffset>,
    /// The match under the cursor for `n`/`N` stepping.
    pub last_matches: Vec<Range<usize>>,
    /// The range `gn`/`dgn` last selected, for the operator span.
    pub(crate) last_found_match: Option<Range<usize>>,
    /// Buffer edit generation `last_matches` was computed at. Every edit
    /// funnels through `VimState::edit_*` (or engine-driven undo/redo), which
    /// bumps the generation, so consecutive `n`/`N` keystrokes walk the
    /// cached list instead of re-scanning the buffer per keypress. An empty
    /// `last_matches` is never trusted from the cache: `:noh` empties it
    /// without an edit, and `n` must still jump (and re-arm highlights).
    pub matches_generation: Option<u64>,
}

impl Default for SearchState {
    fn default() -> Self {
        SearchState {
            pattern: None,
            forward: true,
            offset: None,
            last_matches: Vec::new(),
            last_found_match: None,
            matches_generation: None,
        }
    }
}

/// Regex options come from the engine's case options; the builder itself
/// never fails to construct (only `.build()` can reject a bad pattern).
/// Parsed `/pat/{offset}` tail: which match end anchors the cursor, a
/// CHARACTER shift from that anchor and a LINE shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOffset {
    pub anchor: OffsetAnchor,
    /// Characters to move from the anchor (`/pat/e2`, `/pat/s-2` — the
    /// count after a letter anchor counts CHARS, audit F5; the bare `e`
    /// parks ON the last char, encoded as -1 from the exclusive end).
    pub char_shift: i64,
    /// Lines to shift from the match line (`/pat/+2`, bare `2` — audit F5;
    /// the old parser folded the letter counts in here).
    pub line_shift: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffsetAnchor {
    /// `s`/`b`: start of the match
    Start,
    /// `e`: the match's EXCLUSIVE end byte
    End,
}

/// Parse the offset tail of a search (`:h search-offset`): `e[+-N]`/`s[+-N]`/
/// `b[+-N]` anchor at the match end/start with a CHARACTER shift, a bare
/// `[+-]N`/`N` is a LINE shift from the match line. Returns None on a
/// malformed tail.
pub fn parse_search_offset(spec: &str) -> Option<SearchOffset> {
    let mut anchor: Option<OffsetAnchor> = None;
    let mut rest = spec;
    match spec.chars().next() {
        Some('e') => {
            anchor = Some(OffsetAnchor::End);
            rest = &spec[1..];
        }
        Some('s') | Some('b') => {
            anchor = Some(OffsetAnchor::Start);
            rest = &spec[1..];
        }
        Some(c) if c.is_ascii_digit() || c == '+' || c == '-' => {}
        _ => return None,
    }
    let (sign, digits) = match rest.strip_prefix('-') {
        Some(d) => (-1i64, d),
        None => match rest.strip_prefix('+') {
            Some(d) => (1i64, d),
            None => (1i64, rest),
        },
    };
    let mut char_shift = 0i64;
    let mut line_shift = 0i64;
    match anchor {
        Some(a) => {
            char_shift = if digits.is_empty() {
                if rest.is_empty() {
                    // bare letter: `e` parks ON the match's last char
                    // (exclusive end - 1); `b`/`s` at the start
                    if a == OffsetAnchor::End {
                        -1
                    } else {
                        0
                    }
                } else {
                    // a bare sign counts one char
                    sign
                }
            } else {
                sign * digits.parse::<i64>().ok()?
            };
        }
        None => {
            line_shift = if digits.is_empty() {
                if rest.is_empty() {
                    0
                } else {
                    sign
                }
            } else {
                sign * digits.parse::<i64>().ok()?
            };
        }
    }
    Some(SearchOffset {
        anchor: anchor.unwrap_or(OffsetAnchor::Start),
        char_shift,
        line_shift,
    })
}

pub fn compile(vim: &VimState, pattern: &str) -> RegexBuilder {
    let mut builder = RegexBuilder::new(pattern);
    builder
        .case_insensitive(vim.options.case_insensitive_for(pattern))
        .multi_line(true);
    builder
}

/// Find all matches of `pattern`, with a limit guard on pathological input.
pub fn all_matches(vim: &VimState, buf: &dyn VimBuffer, pattern: &str) -> Vec<Range<usize>> {
    let Ok(re) = compile(vim, pattern).build() else {
        return Vec::new();
    };

    // NOTE: scanning per-line through the VimBuffer trait was measured at
    // ~20,000x SLOWER than one whole-buffer slice + scan (20k line_range
    // trait calls + allocations dwarf one contiguous memcpy), so the
    // whole-buffer scan stays. For per-edit cost control on huge files,
    // hosts use set_hlsearch_live_update(false) + refresh_highlights.
    let text = buf.slice(0..buf.len());
    let ends_with_newline = text.ends_with('\n');
    let len = text.len();
    re.find_iter(&text)
        .filter(|m| {
            // zero-width matches are real positions for search (`/^$` finds
            // empty lines — vim 9.1), EXCEPT the phantom position after the
            // final newline, which is no line at all
            !(m.is_empty() && ends_with_newline && m.start() == len)
        })
        .map(|m| m.start()..m.end())
        .take(10_000)
        .collect()
}

/// Record a pattern (from the command line or `*`) and publish highlights.
pub fn set_pattern(vim: &mut VimState, ctx: &mut Ctx, pattern: String, forward: bool) {
    set_pattern_inner(vim, ctx.buf, ctx.host, pattern, forward);
}

/// [`set_pattern`] with split borrows, for callers that already hold `buf`.
pub fn set_pattern_inner(
    vim: &mut VimState,
    buf: &dyn VimBuffer,
    host: &mut dyn crate::host::VimHost,
    pattern: String,
    forward: bool,
) {
    let matches = all_matches(vim, buf, &pattern);
    vim.search.pattern = Some(pattern.clone());
    vim.search.forward = forward;
    vim.search.last_matches = matches;
    vim.search.matches_generation = Some(vim.edit_generation);
    // vim mirrors every search into `@/` (`<C-r>/` in insert, `:reg`)
    vim.registers.store_search(pattern);
    if vim.options.hlsearch {
        host.set_search_highlights(&vim.search.last_matches, None);
    } else {
        host.set_search_highlights(&[], None);
    }
}

/// Clear highlights (`:noh`).
pub fn clear_highlights(vim: &mut VimState, ctx: &mut Ctx) {
    vim.search.last_matches.clear();
    vim.search.matches_generation = None;
    ctx.host.set_search_highlights(&[], None);
}

/// `n` / `N`: move to the next match in `forward` direction (already flipped
/// by the caller for `N`). Also used after `*` and the `/`/`?` prompts.
///
/// vim's acceptance rule (9.1 oracle matrix, 2026-10-08): candidates are
/// scanned in the search direction starting at the match CONTAINING the
/// cursor; a candidate is accepted only when its offset-applied landing
/// strictly ADVANCES the cursor (`/foo` from a match start skips to the
/// next match — landing == origin doesn't count; `/foo/e` accepts the
/// cursor's own match because the `e` anchor moves it to the match end).
/// The scan wraps (`wrapscan`), and `count` counts accepted landings.
pub fn jump_to_match(
    vim: &mut VimState,
    buf: &dyn VimBuffer,
    forward: bool,
    count: usize,
) -> Option<usize> {
    let pattern = vim.search.pattern.clone()?;
    // The full-buffer scan runs only when the text changed since the matches
    // were computed; a run of `n`/`N` keystrokes walks the cached list, which
    // keeps per-keypress cost O(1) instead of O(buffer) on large files.
    if vim.search.matches_generation != Some(vim.edit_generation) {
        let matches = all_matches(vim, buf, &pattern);
        vim.search.matches_generation = Some(vim.edit_generation);
        vim.search.last_matches = matches;
    }
    let matches = &vim.search.last_matches;
    if matches.is_empty() {
        return None;
    }

    let mut origin = vim.cursor.offset;
    let len = matches.len();
    // first candidate: the match containing the origin (its end past it),
    // else the next match in the direction; when NO match lies ahead the
    // scan WRAPS to the far end ('wrapscan') and that first candidate is
    // already a wrap step
    let (start_idx, first_is_wrap) = if forward {
        match matches.iter().position(|m| m.end > origin) {
            Some(i) => (i, false),
            None => (0, true),
        }
    } else {
        match matches.iter().rposition(|m| m.start <= origin) {
            Some(i) => (i, false),
            None => (len - 1, true),
        }
    };
    let offset = vim.search.offset;
    // A huge count degenerates into a bounded number of sequential passes:
    // the acceptance walk is deterministic per pass, so the landing stays
    // defined (and the arithmetic bounded) however large the typed count is
    // (the u64-overflow contract of the round-24 probe).
    let passes = count.max(1).saturating_mul(len.max(1)).min(len * 8 + 8) / len.max(1) + 1;
    let want = count.max(1);
    let mut accepted = 0usize;
    let mut last_accepted: Option<usize> = None;
    // one full wrap is enough: a second pass re-tests the same candidates
    for pass in 0..passes {
        let step_base = pass * len;
        for step in step_base..step_base + len {
            let (idx, wrapped_raw) = if forward {
                let raw = start_idx + step;
                let i = raw % len;
                (i, raw >= len)
            } else {
                // walk downward with wraparound past index 0
                let off = step % len;
                if off <= start_idx {
                    (start_idx - off, step >= start_idx + 1)
                } else {
                    (start_idx + len - off, true)
                }
            };
            let wrapped = wrapped_raw || (step == 0 && first_is_wrap && pass == 0);
            let m = &matches[idx];
            // the offset landing decides acceptance; a match whose landing does
            // not advance (plain search from its own start) is passed over —
            // EXCEPT once the scan has WRAPPED: vim's 'wrapscan' takes the
            // origin match itself when the scan comes around (`n` from the last
            // match lands on the first whatever the direction)
            vim.search.last_found_match = Some(m.clone());
            let landing = offset
                .and_then(|off| offset_target(vim, buf, &off))
                .unwrap_or(m.start);
            let advances = if forward { landing > origin } else { landing < origin };
            if !advances && !wrapped {
                continue;
            }
            accepted += 1;
            last_accepted = Some(m.start);
            if accepted >= want {
                return Some(m.start);
            }
            // count repeats are SEQUENTIAL searches: the next acceptance is
            // measured from the landing just accepted
            origin = landing;
        }
    }
    last_accepted
}

/// Compute where a search offset lands: anchor at the match start/end, add
/// the CHARACTER shift, then shift LINES from the match line — the line form
/// lands in COLUMN 1 of the shifted line (vim 9.1 probe, audit F6; the old
/// code kept the match column and counted letter anchors as whole lines).
pub fn offset_target(vim: &VimState, buf: &dyn VimBuffer, offset: &SearchOffset) -> Option<usize> {
    let m = vim.search.last_found_match.clone()?;
    let anchor = match offset.anchor {
        OffsetAnchor::Start => m.start,
        OffsetAnchor::End => m.end,
    };
    if offset.line_shift != 0 {
        let line = buf.offset_to_line(m.start) as i64 + offset.line_shift;
        let line = line.clamp(0, buf.line_count() as i64 - 1) as usize;
        return Some(buf.line_start(line));
    }
    let moved = anchor as i64 + offset.char_shift;
    if moved < 0 {
        return Some(0);
    }
    let mut target = moved as usize;
    if target > buf.len() {
        target = buf.len();
    }
    let target = crate::buffer::floor_to_char_boundary(buf, target);
    Some(crate::buffer::clamp_to_line_end(buf, target))
}

/// Apply a search offset to the last-found match (audit F3/F5/F6).
pub fn apply_search_offset(vim: &mut VimState, buf: &dyn VimBuffer, offset: &SearchOffset) {
    if let Some(target) = offset_target(vim, buf, offset) {
        vim.cursor.offset = target;
        vim.cursor.desired_col = None;
    }
}

/// `*` / `#`: search for the text at the cursor. Vim's fallback chain
/// (probe 9.1, round 14): the WORD under the cursor → the first non-blank
/// char after it on the line, taken LITERALLY (cursor on `!` searches the
/// escaped `!` — the old code scanned only for word chars, so a line of
/// `foo !` fell through to the stale-pattern bell). Returns false when the
/// line offers nothing (whitespace only) — the caller must not jump with a
/// stale pattern.
///
/// `whole_word` selects between `*` (`\bword\b`) and `g*` (`g#`: the same
/// word as a SUBSTRING pattern — `:h g*`, audit F2; the old keys degraded
/// to plain `*`/`#`).
pub fn search_word_under_cursor(
    vim: &mut VimState,
    buf: &dyn VimBuffer,
    host: &mut dyn crate::host::VimHost,
    forward: bool,
    whole_word: bool,
) -> bool {
    let offset = vim.cursor.offset;
    // not on a word char: scan forward to the next NON-BLANK within this
    // line. Continuation chars (combining marks) count as blank here —
    // they are invisible cluster tails (glued to the blank behind them on
    // a whitespace-only line) and searching for one parks the cursor
    // mid-cluster
    let offset = if !matches!(buf.char_at(offset), Some(c) if is_word_char(c)) {
        let line_end = buf.line_end(buf.offset_to_line(offset));
        let mut o = offset;
        while o < line_end
            && matches!(buf.char_at(o), Some(c) if c.is_whitespace() || c == '\u{200D}' || crate::buffer::char_display_width(c) == 0)
        {
            o += buf.char_at(o).map(|c| c.len_utf8()).unwrap_or(1);
        }
        o
    } else {
        offset
    };
    // a word char at the probe: whole-word pattern, like vim's `*`
    if let Some((start, end)) = word_bounds_at(buf, offset) {
        let literal = buf.slice(start..end);
        if literal.is_empty() {
            return false;
        }
        let escaped = regex::escape(&literal);
        let pattern = if whole_word {
            format!(r"\b{escaped}\b")
        } else {
            escaped
        };
        set_pattern_inner(vim, buf, host, pattern, forward);
        return true;
    }
    // a non-word, non-blank char: search it literally (no word boundaries —
    // vim's pattern for `*` on `!` is just the escaped char)
    match buf.char_at(offset) {
        Some(c) if !c.is_whitespace() => {
            let pattern = regex::escape(&c.to_string());
            set_pattern_inner(vim, buf, host, pattern, forward);
            true
        }
        _ => false,
    }
}

/// Word bounds around `offset` (the run of word chars containing it).
/// Returns `None` when `offset` sits on a non-word char.
pub fn word_bounds_at(buf: &dyn VimBuffer, offset: usize) -> Option<(usize, usize)> {
    if !matches!(buf.char_at(offset), Some(c) if is_word_char(c)) {
        return None;
    }
    let mut start = offset;
    while let Some(prev) = buf.prev_char_offset(start) {
        match buf.char_at(prev) {
            Some(p) if is_word_char(p) => start = prev,
            _ => break,
        }
    }
    let mut end = start;
    while let Some(next) = buf.next_char_offset(end) {
        match buf.char_at(next) {
            Some(c) if is_word_char(c) => end = next,
            _ => break,
        }
    }
    // `end` is the last char OF the word; the bound is exclusive
    Some((
        start,
        end + buf.char_at(end).map(|c| c.len_utf8()).unwrap_or(1),
    ))
}

/// Publish incremental highlights while the user types in the command line.
pub fn publish_incsearch(vim: &mut VimState, ctx: &mut Ctx, pattern: &str) {
    if !vim.options.incsearch {
        return;
    }
    if pattern.is_empty() {
        // the prompt was erased (typing then <BS> to empty): the empty
        // pattern matches zero-width at EVERY byte, so scanning it would
        // publish thousands of `i..i` ranges. vim's preview falls back to
        // the last ACCEPTED search's highlights here — same set
        // `cancel_cmdline` restores.
        if vim.options.hlsearch {
            let matches = vim.search.last_matches.clone();
            ctx.host.set_search_highlights(&matches, None);
        } else {
            ctx.host.set_search_highlights(&[], None);
        }
        return;
    }
    let matches = all_matches(vim, ctx.buf, pattern);
    // the "current" preview is the match vim's incsearch would JUMP to on
    // Enter: forward prompts take the first match at/after the cursor
    // (wrapping to the buffer's first); BACKWARD prompts (`?pat`) take the
    // last match BEFORE the cursor, wrapping to the buffer's last (audit
    // F7 — the old preview always marked the forward-nearest match)
    let backward = matches!(vim.mode, crate::mode::Mode::CommandLine { prompt: '?' });
    let current = if backward {
        matches
            .iter()
            .rev()
            .find(|m| m.start < vim.cursor.offset)
            .cloned()
            .or_else(|| matches.last().cloned())
    } else {
        matches
            .iter()
            .find(|m| m.start >= vim.cursor.offset)
            .cloned()
            .or_else(|| matches.first().cloned())
    };
    ctx.host.set_search_highlights(&matches, current);
}

/// The match to select for `gn`/`gN`: the match CONTAINING `offset` if there
/// is one, else the first match starting after it (`backward`: before it) —
/// vim's gn prefers the current match, and `cgn` + `.` relies on "next after
/// the cursor" afterwards. Returns None when there is no pattern or no match
/// (the caller reports E35/E486).
///
/// `strict` skips the "match containing offset" preference: `2gN`'s second
/// step must find the match BEFORE the one just selected — with the
/// containing-preference the backward iteration kept re-selecting the same
/// match (`range.start` is always contained in it).
pub fn find_match_from(
    vim: &mut VimState,
    buf: &dyn VimBuffer,
    offset: usize,
    backward: bool,
    strict: bool,
) -> Option<Range<usize>> {
    let pattern = vim.search.pattern.clone()?;
    if vim.search.matches_generation != Some(vim.edit_generation) {
        let matches = all_matches(vim, buf, &pattern);
        vim.search.matches_generation = Some(vim.edit_generation);
        vim.search.last_matches = matches;
    }
    let matches = &vim.search.last_matches;
    let containing = if strict {
        None
    } else {
        matches.iter().find(|m| m.contains(&offset))
    };
    // past-the-end steps wrap like vim's wrapscan (`2gN` on the FIRST
    // match lands on the last one)
    let found = match containing {
        Some(m) => Some(m.clone()),
        None if backward => matches
            .iter()
            .rev()
            .find(|m| m.end <= offset)
            .cloned()
            .or_else(|| matches.last().cloned()),
        None => matches
            .iter()
            .find(|m| m.start > offset)
            .cloned()
            .or_else(|| matches.first().cloned()),
    };
    found
}
