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
            last_matches: Vec::new(),
            last_found_match: None,
            matches_generation: None,
        }
    }
}

/// Regex options come from the engine's case options; the builder itself
/// never fails to construct (only `.build()` can reject a bad pattern).
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
/// by the caller for `N`). Also used after `*`.
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

    let cursor = vim.cursor.offset;
    let start_index = if forward {
        matches.iter().position(|m| m.start > cursor).unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|m| m.start < cursor)
            .unwrap_or(matches.len() - 1)
    };
    // `count` steps in the search DIRECTION from the cursor's neighbor
    // match, wrapping around the list — `2N` is two matches BACKWARD (vim
    // 9.1: from the last of five matches, 1N→4th, 2N→3rd, 3N→2nd; the old
    // code walked +count-1 and went FORWARD). u64 math keeps a huge typed
    // count from overflowing usize in debug builds.
    let len = matches.len() as u64;
    let count = count.max(1) as u64;
    let index = if forward {
        (start_index as u64 + (count - 1)) as usize
    } else {
        let back = (count - 1) % len;
        (start_index as u64 + len - back) as usize % len as usize
    };
    Some(matches.get(index % matches.len())?.start)
}

/// `*` / `#`: search for the text at the cursor. Vim's fallback chain
/// (probe 9.1, round 14): the WORD under the cursor → the first non-blank
/// char after it on the line, taken LITERALLY (cursor on `!` searches the
/// escaped `!` — the old code scanned only for word chars, so a line of
/// `foo !` fell through to the stale-pattern bell). Returns false when the
/// line offers nothing (whitespace only) — the caller must not jump with a
/// stale pattern.
pub fn search_word_under_cursor(
    vim: &mut VimState,
    buf: &dyn VimBuffer,
    host: &mut dyn crate::host::VimHost,
    forward: bool,
) -> bool {
    let offset = vim.cursor.offset;
    // not on a word char: scan forward to the next NON-BLANK within this line
    let offset = if !matches!(buf.char_at(offset), Some(c) if is_word_char(c)) {
        let line_end = buf.line_end(buf.offset_to_line(offset));
        let mut o = offset;
        while o < line_end && matches!(buf.char_at(o), Some(c) if c.is_whitespace()) {
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
        let pattern = format!(r"\b{escaped}\b");
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
    let matches = all_matches(vim, ctx.buf, pattern);
    // the "current" preview is the match vim's incsearch would JUMP to on
    // Enter: the first one at/after the cursor, wrapping to the buffer's
    // first — marking the buffer's first match misrendered the preview
    // whenever the cursor sat below it
    let current = matches
        .iter()
        .find(|m| m.start >= vim.cursor.offset)
        .cloned()
        .or_else(|| matches.first().cloned());
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
