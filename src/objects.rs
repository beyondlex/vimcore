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

pub fn range(buf: &dyn VimBuffer, offset: usize, obj: TextObject) -> Option<ObjectRange> {
    match obj {
        TextObject::Word { inner, big } => word_range(buf, offset, inner, big),
        TextObject::Sentence { inner } => sentence_range(buf, offset, inner),
        TextObject::Paragraph { inner } => paragraph_range(buf, offset, inner),
        TextObject::Quote { inner, quote } => quote_range(buf, offset, inner, quote),
        TextObject::Block { inner, open, close } => block_range(buf, offset, inner, open, close),
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
        // EMPTY line (cursor sits on the newline): `iw`/`ip` keep the line
        // itself, but `aw` treats it as one blank run reaching into the
        // next line's word (9.1: `daw` on ["foo","","bar"] leaves ["foo"])
        if !inner {
            return Some(blank_run_plus_next_word(buf, big, line_start));
        }
        return Some(ObjectRange::charwise(line_start, line_end));
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
    // the next word (see [`blank_run_plus_next_word`]).
    Some(blank_run_plus_next_word(buf, big, start))
}

/// The `aw` object when the cursor is on whitespace (or an empty line): the
/// whitespace run PLUS the next word — the run extends across newlines
/// (blank lines belong to it), matching vim (9.1 probes: `yaw` on the gap of
/// "foo   bar" yanks "   bar"; on a whitespace-only/empty line the object
/// reaches the next line's word, so `daw` merges that line away).
fn blank_run_plus_next_word(buf: &dyn VimBuffer, big: bool, run_start: usize) -> ObjectRange {
    let mut o = run_start;
    while let Some(c) = buf.char_at(o) {
        if c.is_whitespace() {
            o += c.len_utf8();
        } else {
            break;
        }
    }
    // `o` is the first char of the next word (or EOF); consume its run
    let mut word_end = o;
    if let Some(class) = word::class_at(buf, word_end, big) {
        word_end += buf.char_at(word_end).map(|c| c.len_utf8()).unwrap_or(1);
        let mut probe = o;
        while let Some(next) = buf.next_char_offset(probe) {
            if word::class_at(buf, next, big) == Some(class) {
                word_end = next + buf.char_at(next).map(|c| c.len_utf8()).unwrap_or(1);
                probe = next;
            } else {
                break;
            }
        }
    }
    ObjectRange::charwise(run_start, word_end)
}

fn sentence_range(buf: &dyn VimBuffer, offset: usize, inner: bool) -> Option<ObjectRange> {
    let start = word::prev_sentence(buf, (offset + 1).min(buf.len()));
    let next = word::next_sentence(buf, offset);
    // `is` stops AT the sentence terminator — vim 9.1: `dis` on "Aaa. Bbb."
    // deletes "Aaa." and keeps the trailing space in the next sentence's
    // leading whitespace. `next_sentence` already skips that whitespace run
    // (it returns the NEXT sentence's first char), so the inner end is the
    // first terminator at/after the sentence start; a sentence without one
    // runs to end-of-buffer.
    let term_end = (start..next)
        .find(|&o| matches!(buf.char_at(o), Some('.' | '!' | '?')))
        .map(|o| o + buf.char_at(o).map_or(1, |c| c.len_utf8()))
        .unwrap_or(next);
    if !inner {
        // outer (`as`): sentence + TRAILING whitespace, capped at a
        // paragraph-boundary line (blank lines stay — vim 9.1: `das` on
        // ["Aaa.","","Bbb."] keeps the empty line) and at the buffer-final
        // newline (the last line's terminator is not "following white
        // space": vim 9.1 `das` on "Aaa. Bbb." deletes " Bbb." — the
        // leading fallback fires, and the buffer stays a proper single
        // line). When there is NO trailing whitespace, the LEADING run is
        // taken instead (`:h as`).
        let mut o = term_end;
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
        if o > term_end {
            return Some(ObjectRange::charwise(start, o));
        }
        // the leading fallback stays WITHIN the sentence's own line (vim
        // 9.1: `das` on the tail of ["Aaa.","","Bbb."] removes the sentence
        // only — the blank line and its newlines stay put; `das` on the mid-
        // line tail of "Aaa. Bbb." takes the single leading space)
        let mut lead = start;
        let line_start = buf.line_start(buf.offset_to_line(start));
        while lead > line_start {
            let Some(prev) = buf.prev_char_offset(lead) else { break };
            match buf.char_at(prev) {
                Some(c) if c.is_whitespace() => lead = prev,
                _ => break,
            }
        }
        return Some(ObjectRange::charwise(lead, term_end.max(lead)));
    }
    // inner: `is` excludes LEADING whitespace too (`:h is`) — only reachable
    // for a first sentence that starts after buffer-start indentation, since
    // `prev_sentence` already skips the run after a terminator
    let mut trimmed = start;
    while let Some(c) = buf.char_at(trimmed) {
        if c.is_whitespace() {
            trimmed += c.len_utf8();
        } else {
            break;
        }
    }
    Some(ObjectRange::charwise(trimmed, term_end.max(trimmed)))
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
        ObjectRange::charwise(open, close + quote.len_utf8())
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
        open_pos
    };
    let open_pos = open_pos?;

    // scan forwards from the cursor for the innermost unmatched `close`
    let mut depth = 0i32;
    let mut close_pos = None;
    let mut o = if opened_here {
        buf.next_char_offset(offset)?
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
            if let Some(close_rel) = text[i..].find('>') {
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
    // own closes already. Strict `open_start < offset` bounds mean the
    // cursor must be strictly BETWEEN the tags: sitting exactly on `<` of
    // the open tag does NOT select (documented quirk of this engine).
    let mut stack: Vec<(usize, &str)> = Vec::new();
    let mut best: Option<(usize, usize)> = None;
    for (start, end, name, is_open) in tags {
        if is_open {
            stack.push((start, name));
        } else if let Some(pos) = stack.iter().rposition(|(_, n)| *n == name) {
            let (open_start, _) = stack[pos];
            stack.truncate(pos);
            if open_start < offset && offset < end {
                let better = match best {
                    Some((bs, _)) => open_start > bs,
                    None => true,
                };
                if better {
                    best = Some((open_start, end));
                }
            }
        }
    }
    let (open_start, close_end) = best?;
    if inner {
        // empty elements (`<p></p>`) yield a zero-width inner range
        let after_open = text[open_start..].find('>')? + open_start + 1;
        Some(ObjectRange::charwise(after_open, close_end))
    } else {
        Some(ObjectRange::charwise(open_start, close_end))
    }
}
