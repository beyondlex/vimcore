//! Character classes and word/sentence/paragraph scanning.
//!
//! Shared by motions (`w`, `b`, `e`, ...) and text objects (`iw`, `aw`, ...).
//! A "class run" never spans a newline; blank lines act as boundaries for
//! `w`-family motions, exactly as they do in vim.

use crate::buffer::VimBuffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Blank,
    Word,
    Punct,
}

pub fn char_class(c: char) -> Class {
    // vim's utf_class blanks are ONLY ' ' and '\t': a bare CR (^M inside a
    // line) is punctuation, and `w` must stop on it (audit B8 — the
    // `char::is_whitespace` blanket put it in the Blank class and word
    // motions sailed across)
    if c == ' ' || c == '\t' {
        Class::Blank
    } else if c == '\r' {
        Class::Punct
    } else if c.is_whitespace() {
        Class::Blank
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

pub fn is_word_char(c: char) -> bool {
    char_class(c) == Class::Word
}

/// Class used by the w/b/e family. With `big` (W/B/E), only blanks separate
/// runs. A newline always ends a run.
///
/// Width-0 chars (combining marks, VS16) and ZWJ are cluster CONTINUATIONS:
/// they inherit the class of the base char they attach to, so a run scan
/// treats `base + marks` as one unit. Classifying a mark on its own (Punct —
/// it is neither alphanumeric nor whitespace) made `w` stop mid-cluster on
/// `"b\u{0301}"` and parked the cursor on the mark. A mark with no base
/// behind it (line start — its base was deleted) is its own degenerate run.
pub fn class_at(buf: &dyn VimBuffer, offset: usize, big: bool) -> Option<Class> {
    let c = buf.char_at(offset)?;
    if c == '\n' {
        return None; // run boundary
    }
    if c == '\u{200D}' || crate::buffer::char_display_width(c) == 0 {
        // walk back over continuation chars to the cluster's base — the walk
        // STOPS at a newline (GB4 hard boundary): a width-0 char at a line
        // start has no base on its own line and is its own degenerate run.
        // Falling through to classify('\n') = Blank glued the mark onto the
        // previous line's blank class instead.
        let mut back = offset;
        while let Some(prev) = buf.prev_char_offset(back) {
            match buf.char_at(prev) {
                Some(pc) if pc != '\n' && is_cont(pc) => {
                    back = prev;
                }
                Some('\n') | None => break,
                Some(base) => {
                    return Some(classify(base, big));
                }
            }
        }
        return Some(classify(c, big)); // standalone mark: degenerate run
    }
    Some(classify(c, big))
}

fn classify(c: char, big: bool) -> Class {
    match char_class(c) {
        Class::Blank => Class::Blank,
        _ if big => Class::Word,
        other => other,
    }
}

/// Is `c` a cluster-continuation char (combining mark, VS16, ZWJ)? Such
/// chars never start or end a run: they ride on the base before them.
fn is_cont(c: char) -> bool {
    c == '\u{200D}' || crate::buffer::char_display_width(c) == 0
}

/// A truly EMPTY line (no characters before the newline). vim's `w`/`b`
/// family parks on empty lines only — a whitespace-only line ("   ") is
/// skipped freely (9.1 probe: `w` over `abc | "   " | def` lands on `d`).
fn is_empty_line(buf: &dyn VimBuffer, line: usize) -> bool {
    matches!(buf.char_at(buf.line_start(line)), None | Some('\n'))
}

/// The next class run's start at or after `offset`, crossing lines freely.
fn next_non_blank(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    let mut o = offset;
    loop {
        match buf.char_at(o) {
            None => return None,
            Some('\n') => o += 1,
            Some(c) if c.is_whitespace() => o += c.len_utf8(),
            Some(_) => return Some(o),
        }
    }
}

/// `w` / `W`: start of the next word. Stops on blank lines, never lands on a
/// newline.
pub fn next_word_start(buf: &dyn VimBuffer, offset: usize, big: bool) -> usize {
    let mut o = offset;
    // consume the remainder of the current run (if we are on one)
    if let Some(class) = class_at(buf, o, big) {
        if class != Class::Blank {
            while let Some(next) = buf.next_char_offset(o) {
                match class_at(buf, next, big) {
                    Some(c) if c == class => o = next,
                    _ => {
                        o = next;
                        break;
                    }
                }
            }
        }
    }
    // now step over blanks: spaces first, then whole lines. The skip is
    // `is_whitespace` (NOT just ' '/'\t'): a U+3000 ideographic space is
    // Blank to `class_at` and to vim's `w`, and stopping on it would park
    // the cursor ON whitespace — the one place `w` never rests in vim.
    //
    // Continuation chars ride along with the blank they attach to: a mark
    // reached here has a BLANK base (a word base would have ended the skip
    // above — its class would differ), and resting on the mark parks the
    // cursor on an invisible char where `x` splits the cluster. This is the
    // forward mirror of `prev_word_start`'s `is_cont` arm (round 24 fixed
    // `b` but missed `w`: `a \u{0301}bc` parked mid-cluster).
    loop {
        // skip BLANK-class chars on this line (class_at, not the
        // is_whitespace blanket — a bare \r is Punct and `w` must stop on
        // it, audit B8)
        while let Some(c) = buf.char_at(o) {
            if c != '\n' && (class_at(buf, o, big) == Some(Class::Blank) || is_cont(c)) {
                o += c.len_utf8();
            } else {
                break;
            }
        }
        match buf.char_at(o) {
            Some('\n') => {
                o += 1;
                let line = buf.offset_to_line(o);
                // stop on EMPTY lines only, otherwise continue skipping indent
                if is_empty_line(buf, line) {
                    return buf.line_start(line);
                }
            }
            None => return o,
            Some(_) => return o,
        }
    }
}

/// Operator-pending `w`/`W` — vim's `fwd_word(count, bigword, eol=TRUE)`
/// (9.1 textobject.c). The LAST count iteration stops one step EARLIER than
/// the bare motion: on the newline when the word scan runs off the line end,
/// and on column 0 when the blank skip crosses a line break. That landing IS
/// the exclusive operator span end — it is why `dw` on the last char of a
/// word deletes exactly one char and why `d2w` on "  ind1\n  ind2" deletes
/// "  ind1" charwise (9.1 oracle matrix, see `span_from_motion`).
/// `None` = the motion fails (started at the last char of the file) and the
/// operator cancels with a bell.
pub fn fwd_word_operator(
    buf: &dyn VimBuffer,
    offset: usize,
    big: bool,
    count: usize,
) -> Option<usize> {
    let mut o = offset;
    for iter in (0..count.max(1)).rev() {
        let last_iter = iter == 0;
        let sclass = op_class(buf, o, big);
        let started_last_line = buf.offset_to_line(o) + 1 >= buf.line_count();
        let (n, code) = op_inc(buf, o);
        o = n;
        match code {
            Inc::Eof => return None,
            Inc::Eol | Inc::Crossed if started_last_line => return None,
            Inc::Eol | Inc::Crossed if last_iter => return Some(o),
            _ => {}
        }
        // past the end of the current run (if we started in one)
        while op_class(buf, o, big) == sclass {
            let (n2, code2) = op_inc(buf, o);
            o = n2;
            if code2 == Inc::Eof || (matches!(code2, Inc::Eol | Inc::Crossed) && last_iter) {
                return Some(o);
            }
        }
        // next non-blank; the run stops on a truly EMPTY line's column 0
        while op_class(buf, o, big) == Class::Blank {
            let line = buf.offset_to_line(o);
            if o == buf.line_start(line) && is_empty_line(buf, line) {
                break;
            }
            let (n2, code2) = op_inc(buf, o);
            o = n2;
            if code2 == Inc::Eof || (matches!(code2, Inc::Eol | Inc::Crossed) && last_iter) {
                return Some(o);
            }
        }
    }
    Some(o)
}

/// vim's `cls()` for the operator-pending walk: the line-terminating
/// newline (and past-EOF) reads as BLANK, ending a run.
fn op_class(buf: &dyn VimBuffer, offset: usize, big: bool) -> Class {
    match buf.char_at(offset) {
        Some('\n') | None => Class::Blank,
        _ => class_at(buf, offset, big).unwrap_or(Class::Blank),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inc {
    /// moved to the next char on the same line
    Within,
    /// moved ONTO the line's newline (vim's i == 2)
    Eol,
    /// crossed the newline, landing on the next line's col 0 (vim's i == 1)
    Crossed,
    /// at the end of the file, no move (vim's i == -1)
    Eof,
}

fn op_inc(buf: &dyn VimBuffer, offset: usize) -> (usize, Inc) {
    match buf.char_at(offset) {
        None => (offset, Inc::Eof),
        Some('\n') => {
            let line = buf.offset_to_line(offset);
            if line + 1 < buf.line_count() {
                (buf.line_start(line + 1), Inc::Crossed)
            } else {
                (offset, Inc::Eof)
            }
        }
        Some(_) => match buf.next_char_offset(offset) {
            Some(n) => (
                n,
                if buf.at_line_end(n) {
                    Inc::Eol
                } else {
                    Inc::Within
                },
            ),
            None => (buf.len(), Inc::Eol),
        },
    }
}

/// vim's `dec()`: back one position. `Within` stays on the line; `Crossed`
/// lands ON the previous line's terminator (vim's `col == len` eol position,
/// which `dec` visits before its last character); `Sof` = already at the
/// first character of the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dec {
    Within,
    Crossed,
    Sof,
}

fn op_dec(buf: &dyn VimBuffer, offset: usize) -> (usize, Dec) {
    let line = buf.offset_to_line(offset);
    let line_start = buf.line_start(line);
    if offset > line_start {
        match buf.prev_char_offset(offset) {
            Some(p) => (p, Dec::Within),
            None => (offset, Dec::Sof),
        }
    } else if line > 0 {
        (buf.line_end(line - 1), Dec::Crossed)
    } else {
        (offset, Dec::Sof)
    }
}

/// vim's `incl()`: `inc`, but a step ONTO a line's terminator is not a stop —
/// the walk crosses to the next line's column 0 (at the end of the file the
/// cursor stays parked on the terminator, which is where vim leaves it too).
fn op_incl(buf: &dyn VimBuffer, offset: usize) -> (usize, Inc) {
    let (next, code) = op_inc(buf, offset);
    if code == Inc::Eol {
        return op_inc(buf, next);
    }
    (next, code)
}

/// vim's `decl()`: `dec`, but a landing on the previous line's terminator is
/// skipped when that line has content (an EMPTY line is visited at column 0).
fn op_decl(buf: &dyn VimBuffer, offset: usize) -> usize {
    let (next, code) = op_dec(buf, offset);
    if code == Dec::Crossed && next != buf.line_start(buf.offset_to_line(next)) {
        return op_dec(buf, next).0;
    }
    next
}

/// vim's `back_in_line()`: step back to the start of the class run under the
/// cursor, stopping at the line's first column.
fn back_in_line(buf: &dyn VimBuffer, big: bool, pos: &mut usize) {
    let sclass = op_class(buf, *pos, big);
    loop {
        if *pos == buf.line_start(buf.offset_to_line(*pos)) {
            return;
        }
        *pos = op_dec(buf, *pos).0;
        if op_class(buf, *pos, big) != sclass {
            *pos = op_inc(buf, *pos).0;
            return;
        }
    }
}

/// vim's `skip_chars(class, FORWARD)`: walk off a run of `class`, returning
/// the landing position. `None` = the end of the file was reached mid-walk
/// (vim's TRUE, which the callers turn into FAIL).
fn skip_run(buf: &dyn VimBuffer, mut offset: usize, big: bool, class: Class) -> Option<usize> {
    while op_class(buf, offset, big) == class {
        let (next, code) = op_inc(buf, offset);
        if code == Inc::Eof {
            return None;
        }
        offset = next;
    }
    Some(offset)
}

/// vim's `fwd_word(1, bigword, eol = TRUE)` as the text-object walk uses it:
/// one word forward, the last step allowed to stop ON a line's terminator.
/// Returns `(position, completed)` — a walk that hits the last character of
/// the file FAILs but leaves the cursor where it stopped, and `current_word`
/// reads that position either way.
fn fwd_word_step(buf: &dyn VimBuffer, offset: usize, big: bool) -> (usize, bool) {
    let mut o = offset;
    let sclass = op_class(buf, o, big);
    let started_last_line = buf.offset_to_line(o) + 1 >= buf.line_count();
    let (next, code) = op_inc(buf, o);
    o = next;
    if code == Inc::Eof || (matches!(code, Inc::Eol | Inc::Crossed) && started_last_line) {
        return (o, false);
    }
    if matches!(code, Inc::Eol | Inc::Crossed) {
        return (o, true);
    }
    if sclass != Class::Blank {
        while op_class(buf, o, big) == sclass {
            let (next, code) = op_inc(buf, o);
            o = next;
            if code == Inc::Eof || matches!(code, Inc::Eol | Inc::Crossed) {
                return (o, true);
            }
        }
    }
    while op_class(buf, o, big) == Class::Blank {
        let line = buf.offset_to_line(o);
        // the blank run stops on a truly EMPTY line's column 0
        if o == buf.line_start(line) && is_empty_line(buf, line) {
            return (o, true);
        }
        let (next, code) = op_inc(buf, o);
        o = next;
        if code == Inc::Eof || matches!(code, Inc::Eol | Inc::Crossed) {
            return (o, true);
        }
    }
    (o, true)
}

/// vim's `end_word(1, bigword, stop = TRUE, empty = TRUE)`: finish the current
/// run, or cross the blanks to the end of the next one; a blank run that
/// reaches an EMPTY line stops there without stepping back. `None` = FAIL
/// (end of file).
fn end_word_step(buf: &dyn VimBuffer, offset: usize, big: bool) -> Option<usize> {
    let mut o = offset;
    let sclass = op_class(buf, o, big);
    let (next, code) = op_inc(buf, o);
    if code == Inc::Eof {
        return None;
    }
    o = next;
    if op_class(buf, o, big) == sclass && sclass != Class::Blank {
        // started mid-run: finish it
        o = skip_run(buf, o, big, sclass)?;
    } else if sclass == Class::Blank {
        // stop == TRUE: only a blank start crosses over to the next run
        while op_class(buf, o, big) == Class::Blank {
            let line = buf.offset_to_line(o);
            if o == buf.line_start(line) && is_empty_line(buf, line) {
                return Some(o);
            }
            let (next, code) = op_inc(buf, o);
            if code == Inc::Eof {
                return None;
            }
            o = next;
        }
        let class = op_class(buf, o, big);
        o = skip_run(buf, o, big, class)?;
    }
    // overshot by one character — step back
    Some(op_dec(buf, o).0)
}

/// vim's `current_word()`: the `aw`/`iw`/`aW`/`iW` object, walked in vim's
/// position model where a position may sit ON a line's terminator (that is
/// what makes a blank run cross a line break, and why the walk can stop on
/// column 0). `include` selects the outer (`a`) object; `count` extends the
/// selection by further word/blank runs.
///
/// Returns `(start, end, inclusive)`: with `inclusive` the selection ends ON
/// `end`, otherwise `end` is the first character PAST the selection (possibly
/// a line's terminator). `None` = the object fails, which cancels the
/// operator with vim's bell.
pub fn current_word(
    buf: &dyn VimBuffer,
    offset: usize,
    count: usize,
    include: bool,
    big: bool,
) -> Option<(usize, usize, bool)> {
    // a cursor past the last byte (the phantom line a `w` walk can leave) is
    // not a position vim can select from; read it as the line's terminator
    let mut pos = if buf.char_at(offset).is_some() {
        offset
    } else {
        let line = buf.offset_to_line(offset.min(buf.len()));
        buf.line_end(line)
    };
    let mut inclusive = true;
    let mut include_white = false;
    let mut count = count.max(1);

    back_in_line(buf, big, &mut pos);
    let start_pos = pos;

    if (op_class(buf, pos, big) == Class::Blank) == include {
        // on a blank with `a`, or on a word with `i`: take the run's end
        pos = end_word_step(buf, pos, big)?;
    } else {
        // on a word with `a`, or on a blank with `i`: take the run's start,
        // then step back onto it (a landing on column 0 is a single-character
        // word at the line start — back up to the line's terminator instead)
        pos = fwd_word_step(buf, pos, big).0;
        pos = if pos == buf.line_start(buf.offset_to_line(pos)) {
            op_decl(buf, pos)
        } else {
            op_dec(buf, pos).0
        };
        if include {
            include_white = true;
        }
    }
    count -= 1;

    while count > 0 {
        inclusive = true;
        let (next, code) = op_incl(buf, pos);
        if code == Inc::Eof {
            return None;
        }
        pos = next;
        if include != (op_class(buf, pos, big) == Class::Blank) {
            let (landed, ok) = fwd_word_step(buf, pos, big);
            pos = landed;
            if !ok && count > 1 {
                return None;
            }
            // a landing one past a newline must not grab the next line's first
            // character: park on the stop and drop the inclusiveness
            let (back, code) = op_dec(buf, pos);
            if code == Dec::Sof {
                inclusive = false;
            } else {
                pos = back;
            }
        } else {
            pos = end_word_step(buf, pos, big)?;
        }
        count -= 1;
    }

    let mut start = start_pos;
    if include_white
        && (op_class(buf, pos, big) != Class::Blank
            || (pos == buf.line_start(buf.offset_to_line(pos)) && !inclusive))
    {
        // the selection ends on a WORD with no trailing blanks — pull the
        // START back over the blanks in front of it instead (vim's `daw` on a
        // sentence's last word)
        let end_pos = pos;
        pos = start;
        let (back, code) = op_dec(buf, pos);
        if code != Dec::Sof {
            pos = back;
            back_in_line(buf, big, &mut pos);
            if op_class(buf, pos, big) == Class::Blank
                && pos != buf.line_start(buf.offset_to_line(pos))
            {
                start = pos;
            }
        }
        pos = end_pos;
    }
    Some((start, pos, inclusive))
}

/// `e` / `E`: end of the current word, or of the next one when already at a
/// run end. Returns the offset *of the last character* (inclusive motion) —
/// the last GRAPHEME start: a run ending on a continuation char (a mark
/// glued to the base before it) reports the base, never the mark itself.
pub fn next_word_end(buf: &dyn VimBuffer, offset: usize, big: bool) -> usize {
    let mut o = offset;
    // finish the current run first
    if let Some(class) = class_at(buf, o, big) {
        if class != Class::Blank {
            while let Some(next) = buf.next_char_offset(o) {
                match class_at(buf, next, big) {
                    Some(c) if c == class => o = next,
                    _ => break,
                }
            }
            if o != offset {
                return pull_off_continuation(buf, o, offset);
            }
        }
    }
    // on a run end / blank: jump to the end of the next run. On the last
    // word's end the FIXED POINT (returning `offset` unchanged) is correct
    // vim behavior, NOT a failure: `ye`/`de` there yank/delete exactly the
    // cursor character (9.1 probe: `ye` at the end of the last word yanks
    // just that char; the inclusive-span pipeline treats the unchanged
    // landing as a 1-char span). Returning `buf.len()` instead made them
    // swallow the trailing newline. Callers that repeat with a count guard
    // the fixed point themselves (motions.rs WordEnd).
    let Some(start) = next_non_blank(buf, buf.next_char_offset(offset).unwrap_or(offset)) else {
        return offset;
    };
    // a mark right after the whitespace belongs to the blank cluster (its
    // base IS the blank) — the next real run starts at the next cluster
    let start = skip_continuation(buf, start);
    let class = class_at(buf, start, big).unwrap_or(Class::Word);
    let mut o = start;
    while let Some(next) = buf.next_char_offset(o) {
        match class_at(buf, next, big) {
            Some(c) if c == class => o = next,
            _ => break,
        }
    }
    pull_off_continuation(buf, o, offset)
}

/// Walk `o` back off any trailing continuation chars onto the cluster's
/// base — `e`'s landing spot must be a cluster START (width ≥ 1), or the
/// inclusive span / cursor would sit mid-cluster. Stops at `floor` (the
/// original offset): a degenerate run that IS a lone mark keeps its spot.
fn pull_off_continuation(buf: &dyn VimBuffer, mut o: usize, floor: usize) -> usize {
    while o > floor {
        match buf.char_at(o) {
            Some(c) if is_cont(c) => match buf.prev_char_offset(o) {
                Some(p) => o = p,
                None => break,
            },
            _ => break,
        }
    }
    o
}

/// Step `o` forward over continuation chars onto the next cluster start.
fn skip_continuation(buf: &dyn VimBuffer, mut o: usize) -> usize {
    while let Some(c) = buf.char_at(o) {
        if c != '\n' && is_cont(c) {
            o += c.len_utf8();
        } else {
            break;
        }
    }
    o
}

/// `b` / `B`: start of the current or previous word.
pub fn prev_word_start(buf: &dyn VimBuffer, offset: usize, big: bool) -> usize {
    let mut o = offset;
    // if mid-run, walk back to its start
    if let Some(class) = class_at(buf, o, big) {
        if class != Class::Blank {
            while let Some(prev) = buf.prev_char_offset(o) {
                match class_at(buf, prev, big) {
                    Some(c) if c == class => o = prev,
                    _ => break,
                }
            }
            if o != offset {
                return o;
            }
        }
    }
    // on a run start / blank: walk back over blanks, then to the previous
    // run's start. A continuation char reached here rides on a BLANK base
    // (a word base would have been consumed by the run walk above) — skip
    // it with the blank cluster, like the blank itself.
    loop {
        let Some(prev) = buf.prev_char_offset(o) else {
            return o;
        };
        match buf.char_at(prev) {
            Some('\n') => {
                // crossing a line upwards: EMPTY lines are parked on,
                // whitespace-only ones are skipped freely (mirror of `w`)
                let line = buf.offset_to_line(prev);
                o = prev;
                if is_empty_line(buf, line) {
                    return buf.line_start(line);
                }
            }
            Some(c) if c.is_whitespace() || is_cont(c) => o = prev,
            _ => {
                o = prev;
                // walk to the start of this run
                if let Some(class) = class_at(buf, o, big) {
                    while let Some(prev) = buf.prev_char_offset(o) {
                        match class_at(buf, prev, big) {
                            Some(c) if c == class => o = prev,
                            _ => break,
                        }
                    }
                }
                return o;
            }
        }
    }
}

/// `ge` / `gE`: end of the previous word (inclusive).
///
/// A word-end position is a non-blank char whose following char starts a
/// different run (or the buffer end). We search backwards for the largest
/// such position before `offset`; when none exists the offset is unchanged.
pub fn prev_word_end(buf: &dyn VimBuffer, offset: usize, big: bool) -> usize {
    let mut o = offset;
    while let Some(prev) = buf.prev_char_offset(o) {
        o = prev;
        let Some(c) = buf.char_at(o) else { return o };
        if c.is_whitespace() || is_cont(c) {
            // blanks AND continuation chars are never run ends: a mark
            // glued to the base before it would otherwise report itself
            // as `ge`'s landing spot (mid-cluster parking)
            continue;
        }
        let at_run_end = match buf.next_char_offset(o) {
            Some(next) => class_at(buf, next, big) != class_at(buf, o, big),
            None => true,
        };
        if at_run_end {
            return o;
        }
    }
    o
}

/// `f`/`t`/`F`/`T` helpers: find `target` on the current line. The `till`
/// forms stop one char BEFORE/AFTER the hit — that stop position must still
/// be on the SAME line, or the motion fails (vim: `tx` with the only `x` at
/// column 0 beeps instead of parking the cursor on the previous line's `\n`).
pub fn find_char_forward(
    buf: &dyn VimBuffer,
    offset: usize,
    target: char,
    till: bool,
) -> Option<usize> {
    let line = buf.offset_to_line(offset);
    let line_start = buf.line_start(line);
    let mut o = buf.next_char_offset(offset).unwrap_or(offset);
    let end = buf.line_end(line);
    while o < end {
        if buf.char_at(o) == Some(target) {
            if till {
                return match crate::buffer::prev_grapheme_offset(buf, o) {
                    Some(prev) if prev >= line_start => Some(prev),
                    // hit is the first char of the line: no room to stop
                    _ => None,
                };
            }
            return Some(o);
        }
        o = buf.next_char_offset(o)?;
    }
    None
}

pub fn find_char_backward(
    buf: &dyn VimBuffer,
    offset: usize,
    target: char,
    till: bool,
) -> Option<usize> {
    let line = buf.offset_to_line(offset);
    let mut o = offset;
    let start = buf.line_start(line);
    let line_end = buf.line_end(line);
    while o > start {
        let Some(prev) = buf.prev_char_offset(o) else {
            break;
        };
        o = prev;
        if buf.char_at(o) == Some(target) {
            if till {
                return match crate::buffer::next_grapheme_offset(buf, o) {
                    // the stop char must be ON the line (hit can't be the
                    // last content char — there'd be nothing to stop on);
                    // grapheme-aware: the stop never lands on a trailing
                    // combining mark of the hit's own cluster
                    Some(next) if next < line_end => Some(next),
                    _ => None,
                };
            }
            return Some(o);
        }
    }
    None
}

const OPENERS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];

fn bracket_pair(c: char) -> Option<(char, char, bool)> {
    // (char, other, is_opener)
    for (open, close) in OPENERS {
        if c == *open {
            return Some((c, *close, true));
        }
        if c == *close {
            return Some((c, *open, false));
        }
    }
    None
}

/// `%`: find the match of the bracket under the cursor, scanning the whole
/// buffer with nesting. Falls back to the next bracket on the line.
pub fn match_bracket(buf: &dyn VimBuffer, offset: usize) -> Option<usize> {
    // locate a bracket at or after the cursor (same line first, like vim)
    let line = buf.offset_to_line(offset);
    let mut start = None;
    let mut o = offset;
    let line_end = buf.line_end(line);
    while o <= line_end {
        // stop at the end of the buffer — NOT `?`, which would read as
        // "scan error" instead of "no bracket on this line"
        let Some(c) = buf.char_at(o) else { break };
        if bracket_pair(c).is_some() {
            start = Some(o);
            break;
        }
        let Some(next) = buf.next_char_offset(o) else {
            break;
        };
        o = next;
    }
    let start = start?;

    let (ch, other, is_opener) = bracket_pair(buf.char_at(start)?)?;
    let mut depth = 0i32;
    let mut o = start;
    if is_opener {
        while let Some(c) = buf.char_at(o) {
            if c == ch {
                depth += 1;
            } else if c == other {
                depth -= 1;
                if depth == 0 {
                    return Some(o);
                }
            }
            let Some(next) = buf.next_char_offset(o) else {
                break;
            };
            o = next;
        }
    } else {
        while let Some(prev) = buf.prev_char_offset(o) {
            o = prev;
            let Some(c) = buf.char_at(o) else { break };
            if c == ch {
                depth += 1;
            } else if c == other {
                depth -= 1;
                if depth < 0 {
                    return Some(o);
                }
            }
        }
    }
    None
}

/// `}`: start of the next paragraph boundary, buffer end. vim's paragraph
/// boundary is a truly EMPTY line only — a whitespace-only line does NOT
/// stop `}` (9.1 probe: `}` over `aaa | "   " | bbb` lands at buffer end;
/// the old `line_is_blank` check parked on the whitespace line instead).
pub fn next_paragraph(buf: &dyn VimBuffer, offset: usize) -> usize {
    let line = buf.offset_to_line(offset);
    let mut line = line + 1;
    while line < buf.line_count() {
        if is_empty_line(buf, line) {
            return buf.line_start(line);
        }
        line += 1;
    }
    buf.len()
}

/// `{`: start of the previous paragraph boundary (see [`next_paragraph`]).
pub fn prev_paragraph(buf: &dyn VimBuffer, offset: usize) -> usize {
    let line = buf.offset_to_line(offset);
    let mut line = line.saturating_sub(1);
    while line > 0 {
        if is_empty_line(buf, line) {
            return buf.line_start(line);
        }
        line -= 1;
    }
    buf.line_start(0)
}

/// `(`: beginning of the current or previous sentence.
pub fn prev_sentence(buf: &dyn VimBuffer, offset: usize) -> usize {
    let mut o = offset;
    while let Some(prev) = buf.prev_char_offset(o) {
        o = prev;
        if matches!(buf.char_at(o), Some('.' | '!' | '?')) {
            // skip the whitespace run after the terminator
            let mut after = buf.next_char_offset(o).unwrap_or(o);
            while let Some(c) = buf.char_at(after) {
                if c.is_whitespace() {
                    after += c.len_utf8();
                } else {
                    break;
                }
            }
            if after < offset {
                return after;
            }
        }
    }
    0
}

/// `)`: beginning of the next sentence.
pub fn next_sentence(buf: &dyn VimBuffer, offset: usize) -> usize {
    let mut o = offset;
    while let Some(c) = buf.char_at(o) {
        if matches!(c, '.' | '!' | '?') {
            // skip whitespace run; sentence starts at next non-blank
            let mut after = buf.next_char_offset(o).unwrap_or(o);
            while let Some(w) = buf.char_at(after) {
                if w.is_whitespace() {
                    after += w.len_utf8();
                } else {
                    break;
                }
            }
            if buf.char_at(after).is_some() {
                return after;
            }
        }
        o += c.len_utf8();
    }
    buf.len()
}
