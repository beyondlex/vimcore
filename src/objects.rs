//! Text objects: `iw aw iW aW is as ip ap`, quotes and bracket blocks.
//!
//! A text object yields a span; in Operator-Pending it feeds an operator, in
//! Visual mode it extends the selection.

use crate::buffer::VimBuffer;
use crate::word;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextObject {
    Word {
        inner: bool,
        big: bool,
    },
    Sentence {
        inner: bool,
    },
    Paragraph {
        inner: bool,
    },
    Quote {
        inner: bool,
        quote: char,
    },
    Block {
        inner: bool,
        open: char,
        close: char,
    },
    Tag {
        inner: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectRange {
    pub start: usize,
    pub end: usize,
    pub linewise: bool,
}

impl ObjectRange {
    fn charwise(start: usize, end: usize) -> Self {
        ObjectRange {
            start,
            end,
            linewise: false,
        }
    }
}

/// Map the single-letter aliases (`b`, `B`) to their bracket pairs.
pub fn resolve_alias(c: char, inner: bool) -> Option<TextObject> {
    match c {
        'b' => Some(TextObject::Block {
            inner,
            open: '(',
            close: ')',
        }),
        'B' => Some(TextObject::Block {
            inner,
            open: '{',
            close: '}',
        }),
        _ => None,
    }
}

pub fn range(buf: &dyn VimBuffer, offset: usize, obj: TextObject, count: usize) -> Option<ObjectRange> {
    match obj {
        TextObject::Word { inner, big } => word_range(buf, offset, inner, big),
        TextObject::Sentence { inner } => sentence_range(buf, offset, inner, count),
        TextObject::Paragraph { inner } => paragraph_range(buf, offset, inner),
        TextObject::Quote { inner, quote } => quote_range(buf, offset, inner, quote),
        TextObject::Block { inner, open, close } => {
            block_range(buf, offset, inner, open, close, count)
        }
        TextObject::Tag { inner } => tag_range(buf, offset, inner),
    }
}

fn word_range(buf: &dyn VimBuffer, offset: usize, inner: bool, big: bool) -> Option<ObjectRange> {
    let line = buf.offset_to_line(offset);
    let line_start = buf.line_start(line);
    let line_end = buf.line_end(line);

    // One classifier serves both sizes: within a line (this scan never
    // crosses the line bounds) `word::class_at` maps blanks→Blank,
    // everything else→Word for `aw`/`iW`, and word/punct apart for `iw`.
    let class_of = |o: usize| word::class_at(buf, o, big);

    // A cursor past the line's content end (insert mode parks there) or
    // before the line start is pulled ONTO the line and FLOORED to the
    // char boundary of the last char. The old probe `line_end - 1` is a
    // raw byte step: on a CJK-tailed line it lands mid-character, where
    // `class_at` reads None — the empty-line branch then fired and `iw`
    // silently selected the whole line.
    let offset = if offset >= line_end || offset < line_start {
        crate::buffer::floor_to_char_boundary(buf, line_end.saturating_sub(1)).max(line_start)
    } else {
        offset
    };
    let Some(class) = class_of(offset) else {
        // EMPTY line (cursor sits on the newline): `aw` spans the line's
        // newline plus — when the next line has content — that line's first
        // word (final newline included when the word is line-final; 9.1
        // oracles: ["foo","","bar"] → "\nbar\n", ["foo","","bar baz"] →
        // "\nbar", ["foo","","","bar"] → just "\n"). INNER word objects
        // FAIL on an empty line — there is no word to select — and a failed
        // object cancels the operator with vim's bell (audit L5); the old
        // empty-selection fallback made `diw`/`yiw` silently no-op.
        if !inner {
            return empty_line_aw(buf, big, line_start);
        }
        return None;
    };

    // expand to the run of the same class within this line
    let mut start = offset;
    while start > line_start {
        let Some(prev) = buf.prev_char_offset(start) else {
            break;
        };
        if class_of(prev) != Some(class) {
            break;
        }
        start = prev;
    }
    let mut end = buf.next_char_offset(offset).unwrap_or(offset);
    while end < line_end {
        if class_of(end) != Some(class) {
            break;
        }
        end += buf.char_at(end).map(|c| c.len_utf8()).unwrap_or(1);
    }

    if inner || class != word::Class::Blank {
        if !inner {
            // aw: prefer trailing whitespace, fall back to leading
            let mut extend = end;
            while extend < line_end {
                match buf.char_at(extend) {
                    Some(c) if c.is_whitespace() => extend += c.len_utf8(),
                    _ => break,
                }
            }
            if extend > end {
                end = extend;
            } else {
                while start > line_start {
                    let Some(prev) = buf.prev_char_offset(start) else {
                        break;
                    };
                    match buf.char_at(prev) {
                        Some(c) if c.is_whitespace() => start = prev,
                        _ => break,
                    }
                }
            }
        }
        return Some(ObjectRange::charwise(start, end));
    }

    // Cursor on whitespace and the object is `aw`: the whitespace run PLUS
    // the next word (see [`blank_run_plus_next_word`]). A failed object (the
    // run trails into EOF with no word) cancels the operator.
    blank_run_plus_next_word(buf, big, start)
}

/// The class-run end at `offset` (word run for Word/Punct, blank run for
/// Blank — `word::class_at` with the same size).
fn class_run_end(buf: &dyn VimBuffer, big: bool, offset: usize) -> usize {
    let Some(class) = word::class_at(buf, offset, big) else {
        return offset;
    };
    let mut end = offset + buf.char_at(offset).map(|c| c.len_utf8()).unwrap_or(1);
    while end < buf.len() {
        if word::class_at(buf, end, big) == Some(class) {
            end += buf.char_at(end).map(|c| c.len_utf8()).unwrap_or(1);
        } else {
            break;
        }
    }
    end
}

/// `aw` with the cursor ON an EMPTY line (audit B2): the line's newline,
/// plus the next line's first word when that line has content — its final
/// newline joins when the word is line-final (oracle: ["foo","","bar"] →
/// "\nbar\n", ["foo","","bar baz"] → "\nbar"). Consecutive EMPTY lines are
/// NOT swallowed here (`yaw` on ["foo","","","bar"] from the first empty
/// line is just "\n"); the deeper swallow belongs to the count-extension
/// re-probe in `ops::object_span_count`. `None` = the object fails (noeol
/// tail with no newline to span).
fn empty_line_aw(buf: &dyn VimBuffer, big: bool, line_start: usize) -> Option<ObjectRange> {
    let line = buf.offset_to_line(line_start);
    let line_end = buf.line_end(line);
    // the empty line needs its terminator to span anything
    if buf.line_range(line).end <= line_end {
        return None;
    }
    let mut end = line_end + 1;
    let next_line = buf.offset_to_line(end);
    let next_end = buf.line_end(next_line);
    if next_end > end {
        let w = class_run_end(buf, big, end);
        // a line-final word carries its newline ("\nbar\n")
        if w == next_end && buf.line_range(next_line).end > next_end {
            end = w + 1;
        } else {
            end = w;
        }
    }
    Some(ObjectRange::charwise(line_start, end))
}

/// The count-extension re-probe for `aw` when the scan lands ON a line's
/// terminator (`d2aw` over a line break — audit B1): vim's object spans the
/// newline, swallows consecutive EMPTY lines, then takes the next word WITH
/// its trailing blanks (the final newline joins when the word is line-final):
/// `d2aw` on "ab cd\nef gh" covers " cd\nef "; on "foo\n\nbar" the second
/// object is "\n\nbar\n" and the buffer empties. `None` when nothing
/// follows.
pub(crate) fn newline_word_span(
    buf: &dyn VimBuffer,
    big: bool,
    nl_offset: usize,
) -> Option<ObjectRange> {
    let mut end = nl_offset + 1;
    loop {
        let l = buf.offset_to_line(end);
        let le = buf.line_end(l);
        if le == end && buf.line_range(l).end > le {
            end += 1; // swallow the empty line's newline
            continue;
        }
        break;
    }
    if end >= buf.len() {
        return Some(ObjectRange::charwise(nl_offset, buf.len()));
    }
    let l = buf.offset_to_line(end);
    let le = buf.line_end(l);
    let mut w = class_run_end(buf, big, end);
    while w < le {
        match buf.char_at(w) {
            Some(c) if c.is_whitespace() => w += c.len_utf8(),
            _ => break,
        }
    }
    if w == le && buf.line_range(l).end > le {
        w += 1; // line-final word carries its newline
    }
    Some(ObjectRange::charwise(nl_offset, w))
}

/// The `aw` object when the cursor is on whitespace (audit B2, vim 9.1
/// daw/yaw oracle matrix, byte-counted):
/// * a word later ON THE SAME LINE joins the run (`yaw` on the gap of
///   "foo   bar" yanks "   bar");
/// * the run reaching the line end crosses the line's own newline. When a
///   content line follows DIRECTLY, its word joins (+ trailing blanks; a
///   line-final word carries its newline — `daw` on "a   \nb" spans
///   "   \nb\n", leaving "a");
/// * from a WHITESPACE-ONLY cursor line the span additionally swallows
///   following EMPTY lines whole and STOPS at the next content char, word
///   excluded (`daw` on "a   \n\nb" spans "   \n", leaving "a\nb\n"; on
///   "foo\n   \n\nbar" it spans "   \n\n", leaving "foo\nbar\n");
/// * nothing but blanks after the span → the object FAILS (`None`):
///   `daw` on "a   \n" is a no-op with the bell.
fn blank_run_plus_next_word(
    buf: &dyn VimBuffer,
    big: bool,
    run_start: usize,
) -> Option<ObjectRange> {
    let line = buf.offset_to_line(run_start);
    let line_start = buf.line_start(line);
    let line_end = buf.line_end(line);
    let mut o = run_start;
    while o < line_end {
        match buf.char_at(o) {
            Some(c) if c.is_whitespace() => o += c.len_utf8(),
            _ => break,
        }
    }
    // a word follows on the SAME line: run + word
    if o < line_end {
        return Some(ObjectRange::charwise(run_start, class_run_end(buf, big, o)));
    }
    // the run reached the line end — nothing more without a terminator
    if buf.line_range(line).end <= line_end {
        return None;
    }
    let mut end = line_end + 1; // past the newline = the next line's start
    // a WHITESPACE-ONLY cursor line swallows following EMPTY lines whole
    let cursor_line_ws_only = buf.slice(line_start..line_end).chars().all(char::is_whitespace);
    if cursor_line_ws_only {
        loop {
            let l = buf.offset_to_line(end);
            let le = buf.line_end(l);
            if le == end && buf.line_range(l).end > le {
                end += 1; // the empty line's newline joins the span
                continue;
            }
            break;
        }
    }
    // nothing but blanks from the span end to EOF → the object FAILS
    // (`daw` on "a   \n" is a no-op with the bell — audit B2)
    let mut content_found = false;
    let mut probe = end;
    while probe < buf.len() {
        let l = buf.offset_to_line(probe);
        let le = buf.line_end(l);
        if probe < le {
            content_found = true;
            break; // content ahead
        }
        let range_end = buf.line_range(l).end;
        if range_end <= le {
            return None; // noeol tail with no content
        }
        probe = range_end;
    }
    if !content_found {
        return None;
    }
    // a content line directly at `end` contributes its word (+ trailing
    // blanks; line-final words carry their newline) — but only when NO
    // empty line was swallowed in between: vim keeps the word when an
    // empty line separates ("a   \n\nb" daw leaves "a\nb\n")
    let swallowed = end > line_end + 1;
    if end < buf.len() && !swallowed {
        let l = buf.offset_to_line(end);
        let le = buf.line_end(l);
        if le > end {
            let mut e = class_run_end(buf, big, end);
            while e < le {
                match buf.char_at(e) {
                    Some(c) if c.is_whitespace() => e += c.len_utf8(),
                    _ => break,
                }
            }
            if e == le && buf.line_range(l).end > le {
                e += 1;
            }
            end = e;
        }
    }
    Some(ObjectRange::charwise(run_start, end))
}

fn sentence_range(
    buf: &dyn VimBuffer,
    offset: usize,
    inner: bool,
    count: usize,
) -> Option<ObjectRange> {
    // trailing-whitespace cap shared by the outer forms: sentence + the
    // whitespace run after its terminator, capped at a paragraph boundary
    // and at the buffer-final newline (`:h as`; the caps are 9.1-pinned in
    // the original audit)
    let outer_end = |term: usize| -> usize {
        let mut o = term;
        while let Some(c) = buf.char_at(o) {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n' && o + 1 == buf.len() {
                break;
            }
            let line = buf.offset_to_line(o);
            // a blank line is a boundary: stop before consuming any of it
            if buf.line_is_blank(line) {
                break;
            }
            o += c.len_utf8();
        }
        o
    };
    let find_term = |from: usize, to: usize| -> usize {
        (from..to)
            .find(|&o| matches!(buf.char_at(o), Some('.' | '!' | '?')))
            .map(|o| o + buf.char_at(o).map_or(1, |c| c.len_utf8()))
            .unwrap_or(to)
    };
    // one PAST the cursor, at the next char boundary — a raw `offset + 1`
    // lands mid-character on a multi-byte cursor char, where
    // `prev_char_offset` is None and `prev_sentence` silently collapsed the
    // sentence start to 0 (`dis` on "Hi. 你好 ok" deleted "Hi." instead of
    // the sentence under the cursor)
    let mut start = word::prev_sentence(
        buf,
        crate::buffer::next_grapheme_offset(buf, offset).unwrap_or(offset),
    );
    let next = word::next_sentence(buf, offset);
    // `is` stops AT the sentence terminator — vim 9.1: `dis` on "Aaa. Bbb."
    // deletes "Aaa." and keeps the trailing space in the next sentence's
    // leading whitespace.
    let mut term_end = find_term(start, next);
    // audit5 B-11: the cursor sitting on the whitespace AFTER a sentence
    // (past its terminator, before the next sentence's first char) makes
    // `as` anchor on the NEXT sentence — that whitespace is the next
    // sentence's LEADING run in vim's model. The engine used to delete the
    // previous sentence from there.
    let mut range_start = start;
    let mut leading_ws = false;
    let c_len = |buf: &dyn VimBuffer, o: usize| -> usize {
        buf.char_at(o).map_or(1, |c| c.len_utf8())
    };
    let on_gap = offset >= term_end
        && buf.char_at(offset).is_some_and(|c| c.is_whitespace());
    if !inner && on_gap && next < buf.len() {
        // the gap belongs to the anchored sentence's leading run: the `as`
        // range starts where the previous sentence's terminator ended
        range_start = term_end;
        // the whitespace run rides at the FRONT of the anchored range; the
        // sentence's own trailing run stays outside (oracle: `das` on the
        // gap of "? And one more. And no…" deletes " And one more." and
        // keeps BOTH spaces)
        leading_ws = true;
        let mut first = range_start;
        while matches!(buf.char_at(first), Some(c) if c.is_whitespace()) {
            first += c_len(buf, first);
        }
        start = first;
        term_end = find_term(start, word::next_sentence(buf, start));
    }
    // COUNT semantics (9.1 oracle matrix on "Aa. Bb. Cc. Dd." +
    // "A sentence.  A sentence?  A sentence!"): the boundary ALTERNATES
    // between post-terminator and post-gap positions, one boundary per
    // count step. `N-is`: N=1 → term0, N=2 → S1.start, N=3 → term1,
    // N=4 → S2.start … (`3yis` = "A sentence.  A sentence?" — the last
    // boundary EXCLUDES the gap). `N-as` lands on post-gap positions only:
    // `N-as` end = S_N.start (`2yas` = "…A sentence?  ").
    let count = count.max(1);
    let count_end = if inner {
        let mut end = term_end;
        let mut at_term = true;
        let mut sent = start;
        for _ in 1..count {
            if at_term {
                sent = word::next_sentence(buf, sent);
                end = sent;
            } else {
                end = find_term(sent, word::next_sentence(buf, sent));
            }
            at_term = !at_term;
            if end >= buf.len() {
                end = buf.len();
                break;
            }
        }
        end
    } else {
        // N-as ends on the N-th sentence's terminator; the trailing
        // whitespace run is added by outer_end below (1yas = "A sentence.
        //  " — the gap; B-11's anchored shape stops after "more.")
        let mut sent = start;
        for _ in 1..count {
            sent = word::next_sentence(buf, sent);
        }
        let r = find_term(sent, word::next_sentence(buf, sent)).min(buf.len());
        r
    };
    let _ = term_end;
    if !inner {
        // outer (`as`): sentence + TRAILING whitespace, capped at a
        // paragraph-boundary line and the buffer-final newline. When there
        // is NO trailing whitespace, the LEADING run is taken instead
        // (`:h as`) — single-sentence shape only (the count shapes are not
        // pinned for the leading fallback).
        let o = if leading_ws { count_end } else { outer_end(count_end) };
        if o > count_end {
            return Some(ObjectRange::charwise(range_start, o));
        }
        if count > 1 {
            return Some(ObjectRange::charwise(range_start, count_end.max(range_start)));
        }
        // the leading fallback stays WITHIN the sentence's own line
        let mut lead = start;
        let line_start = buf.line_start(buf.offset_to_line(start));
        while lead > line_start {
            let Some(prev) = buf.prev_char_offset(lead) else { break };
            match buf.char_at(prev) {
                Some(c) if c.is_whitespace() => lead = prev,
                _ => break,
            }
        }
        return Some(ObjectRange::charwise(lead.min(range_start), term_end.max(lead.min(range_start))));
    }
    // inner: `is` excludes LEADING whitespace too (`:h is`)
    let mut trimmed = start;
    while let Some(c) = buf.char_at(trimmed) {
        if c.is_whitespace() {
            trimmed += c.len_utf8();
        } else {
            break;
        }
    }
    if count > 1 {
        return Some(ObjectRange::charwise(trimmed, count_end.max(trimmed)));
    }
    Some(ObjectRange::charwise(trimmed, term_end.max(trimmed)))
}

/// First terminator at/after `start` (its end), falling back to the next
/// sentence's start — the single-sentence inner end.
fn term_end_of(start: usize, buf: &dyn VimBuffer) -> usize {
    let next = word::next_sentence(buf, start);
    (start..next)
        .find(|&o| matches!(buf.char_at(o), Some('.' | '!' | '?')))
        .map(|o| o + buf.char_at(o).map_or(1, |c| c.len_utf8()))
        .unwrap_or(next)
}

fn paragraph_range(buf: &dyn VimBuffer, offset: usize, inner: bool) -> Option<ObjectRange> {
    let line = buf.offset_to_line(offset);
    // find the start: first line of this paragraph block (the run of lines
    // whose blankness matches `line`'s — `ip`/`ap` never cross a boundary)
    let mut first = line;
    while first > 0 && buf.line_is_blank(first - 1) == buf.line_is_blank(line) {
        first -= 1;
    }
    // find the end: last line of the block
    let mut last = line;
    while last + 1 < buf.line_count() && buf.line_is_blank(last + 1) == buf.line_is_blank(line) {
        last += 1;
    }
    if !inner {
        // `ap` extends by whitespace, directionally (vim 9.1 probes:
        // [aaa,bbb,"","",ccc] `2Gdap` -> [ccc] — ALL trailing blanks go;
        // [aaa,"","","bbb",ccc] `2Gdap` on a BLANK line -> [aaa] — the
        // blank block plus the ENTIRE following paragraph, but NOT that
        // paragraph's own trailing blanks ([aaa,"",bbb,"",x] `2Gdap` ->
        // [aaa,"",x])). The old `last += 1` swallowed exactly one blank in
        // both shapes, leaving stray lines behind.
        if buf.line_is_blank(line) {
            // blank block + the whole next paragraph (its trailing blanks
            // stay)
            if last + 1 < buf.line_count() {
                let mut para_last = last + 1;
                while para_last + 1 < buf.line_count() && !buf.line_is_blank(para_last + 1) {
                    para_last += 1;
                }
                last = para_last;
            }
        } else {
            // text line: swallow every following blank line
            let before = last;
            while last + 1 < buf.line_count() && buf.line_is_blank(last + 1) {
                last += 1;
            }
            // no trailing blanks to take (the paragraph runs to EOF): vim
            // takes the LEADING blank run instead (`:h ap`; probe 9.1:
            // [para1,"",para2] `3Gdap` -> [para1] — the separator line goes
            // with the deleted paragraph, leaving no stray blank behind)
            if last == before && first > 0 && buf.line_is_blank(first - 1) {
                while first > 0 && buf.line_is_blank(first - 1) {
                    first -= 1;
                }
            }
        }
    }
    Some(ObjectRange {
        start: buf.line_start(first),
        end: buf.line_range(last).end,
        linewise: true,
    })
}

fn quote_positions(buf: &dyn VimBuffer, offset: usize, quote: char) -> Vec<usize> {
    let line = buf.offset_to_line(offset);
    let mut positions = Vec::new();
    let mut o = buf.line_start(line);
    let end = buf.line_end(line);
    while o < end {
        if buf.char_at(o) == Some(quote) {
            // an escaped quote \" is text, not a delimiter — but the escape
            // itself counts only when ODD: `\\"` is a literal backslash
            // followed by a REAL closing quote. Walk the backslash run
            // behind the quote and take its length mod 2 (checking just the
            // single preceding char got `"a\\"` wrong: the closing quote was
            // treated as escaped, the line lost its pairing entirely).
            let line_start = buf.line_start(line);
            let mut backslashes = 0usize;
            let mut probe = o;
            while probe > line_start {
                let Some(prev) = buf.prev_char_offset(probe) else {
                    break;
                };
                if buf.char_at(prev) != Some('\\') {
                    break;
                }
                backslashes += 1;
                probe = prev;
            }
            if backslashes.is_multiple_of(2) {
                positions.push(o);
            }
        }
        match buf.next_char_offset(o) {
            Some(next) => o = next,
            None => break,
        }
    }
    positions
}

fn quote_range(
    buf: &dyn VimBuffer,
    offset: usize,
    inner: bool,
    quote: char,
) -> Option<ObjectRange> {
    let positions = quote_positions(buf, offset, quote);
    // Quotes have no nesting, so occurrences pair up in order (1st-2nd,
    // 3rd-4th, ...). An ODD tail occurrence has no partner and is dropped
    // by `chunks(2)` — vim behaves the same for an unmatched quote.
    //
    // Preference (9.1 probes on `say "hi" then "bye"`):
    //   - the pair containing the cursor (cursor on/inside a string);
    //   - else the quote LEFT of the cursor acts as an opener and pairs
    //     with the next quote — `ci"` at `the|n` replaces ` then ` →
    //     `say "hi"X"bye"` (NOT a no-op, NOT the following string);
    //   - else (no quote left of the cursor) the first pair after it.
    let mut chosen: Option<(usize, usize)> = None;
    for pair in positions.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let (open, close) = (pair[0], pair[1]);
        if open <= offset && offset <= close {
            chosen = Some((open, close));
            break;
        }
    }
    if chosen.is_none() {
        let left = positions.iter().rev().find(|&&p| p < offset).copied();
        match left.and_then(|l| {
            let right = positions.iter().copied().find(|&p| p > l);
            right.map(|r| (l, r))
        }) {
            Some(pair) => chosen = Some(pair),
            // no left quote at all — OR a left quote with no right partner
            // (a dangling closer past the cursor): vim falls back to the
            // FIRST pair of the line. 9.1 probe: `say "hi" then "bye" end`
            // with the cursor after `"bye"` → `ci"` replaces `hi`. The old
            // code only scanned pairs opening AFTER the cursor here, which
            // found nothing past the last quote and belled.
            None => {
                // "first pair of the line" — only a complete pair qualifies
                if let Some(&[a, b]) = positions.first_chunk::<2>() {
                    chosen = Some((a, b));
                }
            }
        }
    }
    let (open, close) = chosen?;
    let range = if inner {
        ObjectRange::charwise(open + quote.len_utf8(), close)
    } else {
        let end = close + quote.len_utf8();
        // `a"` includes surrounding whitespace: the TRAILING run wins; with
        // none on the same line the LEADING run is taken (audit5 B-8 /
        // testdir Test_textobj_quote: `ya'` on "some    'special'    s"
        // yanks "'special'    ", on "…'special'string" it yanks
        // "    'special'"). The engine used to take the bare string both
        // ways.
        let mut trail = end;
        while matches!(buf.char_at(trail), Some(' ') | Some('\t')) {
            trail += 1;
        }
        if trail > end {
            ObjectRange::charwise(open, trail)
        } else {
            let line_start = buf.line_start(buf.offset_to_line(open));
            let mut lead = open;
            while lead > line_start
                && matches!(
                    buf.prev_char_offset(lead).and_then(|p| buf.char_at(p)),
                    Some(' ') | Some('\t')
                )
            {
                lead = buf.prev_char_offset(lead).unwrap();
            }
            ObjectRange::charwise(lead, end)
        }
    };
    // inner may be EMPTY (`ci"` on `""` inserts between the quotes, like
    // vim); only the outer form — which always spans both quotes — is
    // required to be non-empty
    if !inner && range.start >= range.end {
        None
    } else {
        Some(range)
    }
}

fn block_range(
    buf: &dyn VimBuffer,
    offset: usize,
    inner: bool,
    open: char,
    close: char,
    count: usize,
) -> Option<ObjectRange> {
    // an OPEN bracket under the cursor opens the block (vim): scan forward
    // from the NEXT char. A close under the cursor must not count — the
    // block containing the cursor extends past it.
    let opened_here = buf.char_at(offset) == Some(open);
    let open_pos = if opened_here {
        Some(offset)
    } else {
        // scan backwards for the innermost unmatched `open`
        let mut depth = 0i32;
        let mut open_pos = None;
        let mut o = offset;
        while let Some(prev) = buf.prev_char_offset(o) {
            o = prev;
            match buf.char_at(o) {
                Some(c) if c == close => depth += 1,
                Some(c) if c == open => {
                    if depth == 0 {
                        open_pos = Some(o);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        match open_pos {
            Some(p) => Some(p),
            None => {
                // vim (`:h i(` "when the cursor is not in a block"): search
                // FORWARD for the first unmatched `open` (audit5 B-4..B-7 —
                // `0di)` on "foo (bar (baz (quux)))" used to fail outright;
                // surplus closers in front of it don't count). The COUNT
                // counts unmatched opens forward — `02di)` enters the
                // SECOND nesting level (testdir: → "foo (bar ())"), so the
                // whole count is consumed here; the outer climb in
                // object_span_count must not re-apply (it doesn't: the
                // forward-found span's start is past the cursor)
                let mut surplus = 0i32;
                let mut o = offset;
                let mut found = 0usize;
                loop {
                    match buf.char_at(o) {
                        Some(c) if c == close => surplus += 1,
                        Some(c) if c == open => {
                            if surplus == 0 {
                                found += 1;
                                if found >= count.max(1) {
                                    break;
                                }
                            } else {
                                surplus -= 1;
                            }
                        }
                        Some(_) => {}
                        None => return None,
                    }
                    o = buf.next_char_offset(o)?;
                }
                Some(o)
            }
        }
    };
    let open_pos = open_pos?;

    // scan forwards for the innermost unmatched `close`. A forward-found
    // open starts the scan after itself (the cursor sits BEFORE the block,
    // so scanning from it would double-count the open)
    let mut depth = 0i32;
    let mut close_pos = None;
    let mut o = if opened_here {
        buf.next_char_offset(offset)?
    } else if open_pos > offset {
        open_pos + open.len_utf8()
    } else {
        offset
    };
    while let Some(c) = buf.char_at(o) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth < 0 {
                close_pos = Some(o);
                break;
            }
        }
        o = buf.next_char_offset(o)?;
    }
    let close_pos = close_pos?;

    if inner {
        // empty pairs are fine: `ci(` on `()` must be able to insert between
        // the brackets (zero-width inner range)
        let start = open_pos + open.len_utf8();
        // cross-line inner: vim takes the line AFTER the open through the
        // line BEFORE the close — both surrounding newlines survive, so the
        // lines never join (audit5 B-15: `di[` on " \n[\none [two]\nthre\n]"
        // leaves " \n[\n]"; `di(` on the multi-line call leaves "x = (\n)")
        let start_line = buf.offset_to_line(start);
        let end_line = buf.offset_to_line(close_pos);
        if end_line > start_line + 1 || (end_line == start_line + 1 && close_pos == buf.line_start(end_line)) {
            let inner_start = buf.line_start(start_line + 1);
            let inner_end = buf.line_range(end_line - 1).end;
            if inner_start <= inner_end {
                return Some(ObjectRange::charwise(inner_start, inner_end));
            }
        }
        Some(ObjectRange::charwise(start, close_pos))
    } else {
        Some(ObjectRange::charwise(
            open_pos,
            close_pos + close.len_utf8(),
        ))
    }
}

/// `it` / `at`: the element range around an HTML/XML tag. Whole-buffer
/// design: tags can span many lines, so instead of a windowed scan we
/// collect every `<...>` occurrence once, then match open/close pairs with
/// a name stack (self-closing `<br/>` never opens).
fn tag_range(buf: &dyn VimBuffer, offset: usize, inner: bool) -> Option<ObjectRange> {
    let text = buf.slice(0..buf.len());
    let bytes = text.as_bytes();

    // collect tag spans
    let mut tags: Vec<(usize, usize, &str, bool)> = Vec::new(); // (start, end_after_close, name, is_open)
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // scan to the tag's REAL closing `>`: a `>` inside a quoted
            // attribute value does not close the tag (audit5 B-14 — the
            // bare `find('>')` truncated `<div attr="attr >> foo ">` at
            // the first `>`, so `yit` yanked attribute text)
            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            let close_rel = loop {
                if j >= bytes.len() {
                    break None;
                }
                let b = bytes[j];
                match quote {
                    Some(q) => {
                        if b == q {
                            quote = None;
                        }
                    }
                    None => {
                        if b == b'"' || b == b'\'' {
                            quote = Some(b);
                        } else if b == b'>' {
                            break Some(j - i);
                        }
                    }
                }
                j += 1;
            };
            if let Some(close_rel) = close_rel {
                let inner_text = &text[i + 1..i + close_rel];
                let (name, is_open) = if let Some(rest) = inner_text.strip_prefix('/') {
                    (
                        rest.trim_end_matches('/')
                            .split_whitespace()
                            .next()
                            .unwrap_or(""),
                        false,
                    )
                } else {
                    (
                        inner_text.split_whitespace().next().unwrap_or(""),
                        !inner_text.ends_with('/'),
                    )
                };
                tags.push((i, i + close_rel + 1, name, is_open));
                i += close_rel + 1;
                continue;
            } else {
                // no `>` ahead of this `<` means none ahead of any later
                // `<` either — stop instead of rescanning the tail per `<`
                // (a template fragment of bare `<`s made this quadratic)
                break;
            }
        }
        i += 1;
    }

    // find the innermost pair containing the cursor. A closing tag pops
    // the matching open (rposition = innermost same-name) and truncates the
    // stack below it — children of a matched pair were balanced by their
    // own closes already. Containment follows the vim 9.1 probes: the block
    // spans `open_start <= offset < close_end` — a cursor on the OPENING
    // tag (its `<` included) selects THAT tag's block, a cursor on the
    // CLOSING tag's `<` selects the ENCLOSING one (its own block already
    // ended at close_start... i.e. the closer's `<` is one past its own
    // range). The old strict `open_start < offset` skipped the open tag and
    // climbed to the parent — `dit` on `<div><p>x</p></div>` col 5 deleted
    // `<p>x</p>` where vim 9.1 deletes just `x`.
    // (open_start, open_end, name) — the open tag's END rides along: the
    // inner range starts there, and recomputing it with a bare `find('>')`
    // re-introduced the quoted-`>` truncation (audit5 B-14)
    let mut stack: Vec<(usize, usize, &str)> = Vec::new();
    // (open_start, close_start, close_end) of the best pair — close_start
    // is the closer's own `<`: `it` must end there, never swallow the tag
    let mut best: Option<(usize, usize, usize, usize)> = None;
    for (start, end, name, is_open) in tags {
        if is_open {
            stack.push((start, end, name));
        } else if let Some(pos) = stack.iter().rposition(|(_, _, n)| n.eq_ignore_ascii_case(name)) {
            let (open_start, open_end, _) = stack[pos];
            stack.truncate(pos);
            if open_start <= offset && offset < end {
                let better = match best {
                    Some((bs, _, _, _)) => open_start > bs,
                    None => true,
                };
                if better {
                    best = Some((open_start, open_end, start, end));
                }
            }
        }
    }
    let (open_start, open_end, close_start, close_end) = best?;
    if inner {
        // empty elements (`<p></p>`) yield a zero-width inner range
        Some(ObjectRange::charwise(open_end, close_start))
    } else {
        Some(ObjectRange::charwise(open_start, close_end))
    }
}
