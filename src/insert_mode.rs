//! Insert-mode key handling.
//!
//! Printable characters intentionally return [`KeyResult::Unknown`] (unless an
//! insert-mode mapping wants them): the host's IME path owns text input, so
//! composition (Chinese/Japanese input, accents, ...) keeps working. Text
//! arrives back through [`VimState::insert_text_at_cursor`].

use crate::key::{Key, KeyKind};
use crate::mode::Mode;
use crate::motions::Motion;
use crate::state::{Ctx, ProcessOutcome, QuotePending, VimState};
use crate::word;

impl VimState {
    pub(crate) fn insert_key(&mut self, ctx: &mut Ctx, key: Key) -> ProcessOutcome {
        // finish a pending <C-r>{register}
        if self.insert_register_pending {
            self.insert_register_pending = false;
            if let Some(name) = key.printable_char() {
                if let Some(data) = self.registers.get_for_paste(name, ctx.host) {
                    self.insert_text_at_cursor(ctx, &data.text);
                }
            }
            return ProcessOutcome::Consumed;
        }

        // i_CTRL-V / i_CTRL-K pendings must run BEFORE the <Esc> exit:
        // inside a quote the Esc is the QUOTED byte; inside a digraph it
        // just cancels the digraph (`:h i_CTRL-K`; audit H1)
        if self.insert_quote.is_none()
            && self.insert_digraph_first.is_none()
            && key == Key::ctrl_char('v')
        {
            self.insert_quote = Some(QuotePending::Single);
            return ProcessOutcome::Consumed;
        }
        if self.insert_quote.is_none() && self.insert_digraph_first.is_none()
            && key == Key::ctrl_char('k')
        {
            self.insert_digraph_first = Some('\0');
            return ProcessOutcome::Consumed;
        }
        if let Some(state) = self.insert_quote.take() {
            // Single + a numeric prefix letter starts the code collector;
            // the prefix letter itself is CONSUMED (it is not a digit of
            // the code — re-feeding it emitted a NUL, audit H2)
            match state {
                QuotePending::Single => {
                    if let Some(c) = key.printable_char() {
                        match c {
                            'u' => {
                                self.insert_quote =
                                    Some(QuotePending::Hex { max_digits: 4, buffer: String::new() });
                                return ProcessOutcome::Consumed;
                            }
                            'U' => {
                                self.insert_quote =
                                    Some(QuotePending::Hex { max_digits: 8, buffer: String::new() });
                                return ProcessOutcome::Consumed;
                            }
                            'x' | 'X' => {
                                self.insert_quote =
                                    Some(QuotePending::Hex { max_digits: 2, buffer: String::new() });
                                return ProcessOutcome::Consumed;
                            }
                            'o' | 'O' => {
                                self.insert_quote = Some(QuotePending::Radix {
                                    base: 8,
                                    max_digits: 3,
                                    buffer: String::new(),
                                });
                                return ProcessOutcome::Consumed;
                            }
                            d if d.is_ascii_digit() => {
                                self.insert_quote = Some(QuotePending::Radix {
                                    base: 10,
                                    max_digits: 3,
                                    buffer: d.to_string(),
                                });
                                return ProcessOutcome::Consumed;
                            }
                            _ => {}
                        }
                    }
                    self.insert_quote_step(ctx, state, key);
                }
                state => {
                    self.insert_quote_step(ctx, state, key);
                }
            }
            return ProcessOutcome::Consumed;
        }
        if self.insert_digraph_first.is_some() {
            let Some(c) = key.printable_char() else {
                // any non-printable cancels the digraph, like vim's <Esc>
                self.insert_digraph_first = None;
                return ProcessOutcome::Consumed;
            };
            let first = self.insert_digraph_first.take().unwrap();
            if first == '\0' {
                self.insert_digraph_first = Some(c);
                return ProcessOutcome::Consumed;
            }
            let text = crate::ops::digraph(first, c);
            self.insert_text_at_cursor(ctx, &text);
            return ProcessOutcome::Consumed;
        }

        // exit insert: <Esc>, <C-[>, <C-c>
        if key == Key::escape() || key == Key::ctrl_char('[') || key == Key::ctrl_char('c') {
            self.exit_insert(ctx);
            return ProcessOutcome::Consumed;
        }


        // <C-h> = Backspace (vim binds them identically in insert). Only the
        // RAW `\x08` byte reached the backspace path before; gpui-style hosts
        // deliver the normalized chord and it fell through as Unknown —
        // swallowed, nothing deleted.
        if key == Key::ctrl_char('h') {
            self.insert_backspace(ctx);
            return ProcessOutcome::Consumed;
        }

        if key.modifiers.is_plain() {
            if let KeyKind::Named(name) = &key.kind {
                match name.as_str() {
                    "enter" => {
                        // a newline mid-session moves the cursor to another
                        // row, desyncing the block session's replica offsets
                        // the same way a vertical move would (locked above)
                        if self.in_block_insert() {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        }
                        self.insert_text_at_cursor(ctx, "\n");
                        return ProcessOutcome::Consumed;
                    }
                    "backspace" => {
                        self.insert_backspace(ctx);
                        return ProcessOutcome::Consumed;
                    }
                    "tab" => {
                        self.insert_tab(ctx);
                        return ProcessOutcome::Consumed;
                    }
                    "delete" => {
                        let at = self.cursor.offset;
                        // delete the whole GRAPHEME cluster ahead (the twin of
                        // the BS path's cluster rule, vim delcombine=off: a
                        // single-char delete on `e\u{0301}` left the bare mark
                        // behind, re-attaching it to the previous char)
                        if let Some(end) = crate::buffer::next_grapheme_offset(ctx.buf, at) {
                            let c = ctx.buf.char_at(at);
                            // deleting the newline would merge the typing row
                            // into its neighbor — same corruption as a
                            // vertical move (block session invariants)
                            if c == Some('\n') && self.in_block_insert() {
                                ctx.host.bell();
                                return ProcessOutcome::Consumed;
                            }
                            self.begin_edit();
                            self.edit_delete(ctx, at..end);
                            // every other edit path republishes the hlsearch
                            // scan; skipping it here left `last_matches`
                            // pointing mid-character (fuzz round 10) — the
                            // stale cache is what `cancel_cmdline` republishes
                            self.republish_search(ctx);
                            ctx.host.changed();
                        }
                        return ProcessOutcome::Consumed;
                    }
                    "up" | "down" => {
                        // vertical moves are locked out mid-block-insert: the
                        // row replication assumes all typing landed on the
                        // session's typing row, and a page motion would point
                        // it at another row (drifting the replica offsets).
                        // vim allows vertical moves here (extending the
                        // block), which this engine does not model.
                        if self.in_block_insert() {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        }
                        let motion = if name == "up" {
                            Motion::Up
                        } else {
                            Motion::Down
                        };
                        // the new line owns no typed text yet: its C-w/C-u
                        // floor re-arms at the first typed byte there
                        self.insert_typed_start = None;
                        self.goto_motion(ctx, motion, 1);
                        return ProcessOutcome::Consumed;
                    }
                    "left" | "right" => {
                        let motion = if name == "left" {
                            Motion::Left
                        } else {
                            Motion::Right
                        };
                        self.goto_motion(ctx, motion, 1);
                        return ProcessOutcome::Consumed;
                    }
                    "home" => {
                        self.cursor.offset = ctx
                            .buf
                            .line_start(ctx.buf.offset_to_line(self.cursor.offset));
                        return ProcessOutcome::Consumed;
                    }
                    "end" => {
                        let line = ctx.buf.offset_to_line(self.cursor.offset);
                        self.cursor.offset = ctx.buf.line_end(line);
                        return ProcessOutcome::Consumed;
                    }
                    "pageup" => {
                        if self.in_block_insert() {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        }
                        self.insert_typed_start = None;
                        self.goto_motion(ctx, Motion::PageUp, 1);
                        return ProcessOutcome::Consumed;
                    }
                    "pagedown" => {
                        if self.in_block_insert() {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        }
                        self.insert_typed_start = None;
                        self.goto_motion(ctx, Motion::PageDown, 1);
                        return ProcessOutcome::Consumed;
                    }
                    _ => {}
                }
            }
        }

        // Ctrl chords inside insert. Only alt/platform (cmd) disqualify a
        // chord — `is_plain()` additionally excludes control itself, which
        // made this whole block unreachable once.
        if key.modifiers.control && !key.modifiers.alt && !key.modifiers.platform {
            match &key.kind {
                KeyKind::Char('w') | KeyKind::Char('W') => {
                    self.insert_delete_word_before(ctx);
                    return ProcessOutcome::Consumed;
                }
                KeyKind::Char('u') => {
                    self.insert_delete_to_line_start(ctx);
                    return ProcessOutcome::Consumed;
                }
                KeyKind::Char('r') => {
                    self.insert_register_pending = true;
                    return ProcessOutcome::Consumed;
                }
                _ => return ProcessOutcome::Unknown,
            }
        }

        // <S-Tab> in insert: vim 9.1 leaves the line alone (oracle probe:
        // `A<S-Tab>` appends nothing) — the old fall-through delivered the
        // chord as Unknown and somewhere synthesized two spaces (audit K2)
        if key.modifiers.shift
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && matches!(&key.kind, KeyKind::Named(n) if n == "tab")
        {
            return ProcessOutcome::Consumed;
        }

        // printable text: let the host/IME decide *unless* an insert mapping
        // is interested (e.g. `jk` -> Esc)
        if let Some(c) = key.printable_char() {
            self.pending_unknown_chars.push(c);
            return ProcessOutcome::Unknown;
        }

        ProcessOutcome::Unknown
    }

    /// One key inside a pending `i_CTRL-V`: returns `true` when the key
    /// RESOLVED the quote (the char was inserted), `false` when it was
    /// consumed as another digit of a numeric code.
    fn insert_quote_step(&mut self, ctx: &mut Ctx, state: QuotePending, key: Key) -> bool {
        match state {
            QuotePending::Single => {
                let c = self.quote_key_char(key);
                self.insert_text_at_cursor(ctx, &c);
                true
            }
            QuotePending::Radix {
                base,
                max_digits,
                mut buffer,
            } => {
                if let Some(d) = key.printable_char().filter(|c| c.is_digit(base)) {
                    buffer.push(d);
                    if buffer.chars().count() < max_digits {
                        self.insert_quote = Some(QuotePending::Radix {
                            base,
                            max_digits,
                            buffer,
                        });
                        return false;
                    }
                }
                let value = u32::from_str_radix(&buffer, base).unwrap_or(0);
                if let Some(c) = char::from_u32(value) {
                    let s = c.to_string();
                    self.insert_text_at_cursor(ctx, &s);
                }
                true
            }
            QuotePending::Hex {
                max_digits,
                mut buffer,
            } => {
                if let Some(d) = key.printable_char().filter(|c| c.is_ascii_hexdigit()) {
                    buffer.push(d);
                    if buffer.chars().count() < max_digits {
                        self.insert_quote = Some(QuotePending::Hex { max_digits, buffer });
                        return false;
                    }
                }
                let value = u32::from_str_radix(&buffer, 16).unwrap_or(0);
                if let Some(c) = char::from_u32(value) {
                    let s = c.to_string();
                    self.insert_text_at_cursor(ctx, &s);
                }
                true
            }
        }
    }

    /// The literal char a quoted key produces: control chords map to their
    /// control byte (C-a → 0x01), Esc/Tab/CR to the raw bytes, named keys
    /// produce nothing (vim inserts their termcap sequence; the engine has
    /// no terminal model).
    fn quote_key_char(&self, key: Key) -> String {
        if let Some(c) = key.printable_char() {
            return c.to_string();
        }
        if key.modifiers.control && !key.modifiers.alt && !key.modifiers.platform {
            if let KeyKind::Char(c) = key.kind {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    return char::from_u32(upper as u32 - 'A' as u32 + 1)
                        .map(|b| b.to_string())
                        .unwrap_or_default();
                }
                if c == '@' {
                    return '\0'.to_string();
                }
            }
        }
        match &key.kind {
            KeyKind::Named(name) => match name.as_str() {
                "escape" => '\u{1b}'.to_string(),
                "tab" => '\t'.to_string(),
                "enter" => '\r'.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    fn insert_backspace(&mut self, ctx: &mut Ctx) {
        let at = self.cursor.offset;
        let line_start = ctx.buf.line_start(ctx.buf.offset_to_line(at));

        // A block session's replica offsets assume every line of the block
        // stays a separate line: a BS that JOINS the typing row with its
        // neighbor invalidates `typing_line`/`rows` (fuzz round 8: the
        // replication then inserts past the buffer end). Bell like the
        // locked vertical moves.
        if self.in_block_insert() && at <= line_start && at > 0 {
            ctx.host.bell();
            return;
        }
        // Block session BS deletes only the LAST TYPED char at the typed end
        // (shrinking the replica with it — vim 9.1 probes: `ab<BS>` gives
        // "a" + replica "a"; after `<Left>` the BS is a NO-OP, it never eats
        // into the row's own content: `ab<Left><BS>` keeps "ab" everywhere).
        // Mid-row BS would delete pre-existing text the replica can't model —
        // the exit-side delta guard then skips replication entirely, so make
        // the keypress match vim instead and simply refuse it.
        if self.in_block_insert() && self.block_typed_end() != Some(at) {
            ctx.host.bell();
            return;
        }

        // Replace mode: BS restores the overwritten character and steps
        // back — but ONLY when the top stack entry IS the position being
        // backspaced. The vim 9.1 probe matrix (round 14) is exact:
        //   `Rab<BS>`     → restores `x` (straight back over typing);
        //   `Rabc<Left><BS>` / `Rab<Left><BS>` → text UNCHANGED, BS is a
        //   total no-op (the top entry belongs to a later position; vim
        //   neither restores nor moves).
        // So the raw LIFO is gone: entries carry their offsets and a
        // mismatch makes the whole keypress inert. `None` entries were
        // APPENDED past the line end — matched, they plain-delete.
        if self.mode == Mode::Replace && at > 0 {
            if at > line_start {
                let Some(prev) = ctx.buf.prev_char_offset(at) else {
                    return;
                };
                match self.replace_overwritten.last() {
                    Some((pos, _)) if *pos == prev => {
                        let (_, orig) = self.replace_overwritten.pop().unwrap();
                        self.begin_edit();
                        match orig {
                            Some(c) => self.edit_replace(ctx, prev..at, &c.to_string()),
                            None => self.edit_delete(ctx, prev..at),
                        }
                        self.republish_search(ctx);
                        self.cursor.offset = prev;
                        ctx.host.changed();
                    }
                    // position mismatch (or empty stack): vim leaves
                    // everything alone — no restore, no cursor move
                    _ => {}
                }
                return;
            }
            // crossed the line start: join with the previous line; the top
            // entry belongs to a position at/after the join point and is
            // discarded to keep the remaining stack position-consistent
            self.replace_overwritten.pop();
            self.begin_edit();
            self.edit_delete(ctx, at - 1..at);
            self.republish_search(ctx);
            self.cursor.offset = at - 1;
            ctx.host.changed();
            return;
        }

        // autoindent: BS at the end of an untouched whitespace-only line
        // deletes the WHOLE autoindent in one stroke, not one char (vim 9.1
        // probe S5: `o<BS>` leaves the line empty). Only fires while the
        // line's indent is pure autoindent (`insert_did_ai`) — typing
        // anything disarms it and BS degrades to per-char deletes.
        if self.insert_did_ai && at > line_start {
            let le = ctx.buf.line_end(ctx.buf.offset_to_line(at));
            if at == le
                && ctx
                    .buf
                    .slice(line_start..le)
                    .chars()
                    .all(char::is_whitespace)
            {
                self.begin_edit();
                self.edit_delete(ctx, line_start..at);
                self.cursor.offset = line_start;
                self.republish_search(ctx);
                ctx.host.changed();
                return;
            }
        }

        if at > line_start {
            // delete the whole GRAPHEME cluster behind the cursor (vim's
            // default delcombine=off: BS removes the composed char, not just
            // its combining tail)
            if let Some(prev) = crate::buffer::prev_grapheme_offset(ctx.buf, at) {
                self.begin_edit();
                self.edit_delete(ctx, prev..at);
                self.cursor.offset = prev;
                // a block-insert session replicates block.text onto the other
                // rows on exit: a backspace that undoes typed text must
                // shrink it too, or the replicas carry the deleted char
                self.block_backspace_undo(at);
                self.republish_search(ctx);
                ctx.host.changed();
            }
        } else if at > 0 {
            // join with the previous line
            self.begin_edit();
            self.edit_delete(ctx, at - 1..at);
            self.republish_search(ctx);
            self.cursor.offset = at - 1;
            ctx.host.changed();
        }
    }

    fn insert_tab(&mut self, ctx: &mut Ctx) {
        if self.options.expandtab {
            // align by DISPLAY column (wide chars cover two cells): vim pads
            // to the next multiple of 'tabstop' of the virtual column (9.1
            // probe: '中文' + Tab at display col 4, ts=4 → four spaces; the
            // old BYTE-column math (6 % 4) gave two and broke the grid)
            let ts = self.options.tabstop.max(1);
            let col = crate::buffer::display_column(ctx.buf, self.cursor.offset);
            let spaces = ts - (col % ts);
            self.insert_text_at_cursor(ctx, &" ".repeat(spaces));
        } else {
            self.insert_text_at_cursor(ctx, "\t");
        }
    }

    /// `<C-w>`: delete the word before the cursor, like typing `b` then
    /// deleting. Word classes follow normal mode (`(`, `bar`, `)` are three
    /// separate words). A deletion whose word run crosses the line start
    /// JOINS the line with its predecessor instead — vim deletes exactly
    /// the newline (`backspace=eol` shape, the common config; a glued word
    /// from a line-start combining mark reaches here). At the line start
    /// with nothing typed the stroke is a NO-OP — round 30's typeahead
    /// probes REVERSED the round-9 claim of a join: vim 9.1 deletes nothing
    /// when there is no character before the cursor on this line (bs default
    /// and bs=start probes agree), so the engine no longer joins there.
    ///
    /// The deletion never backs over the line's TYPING START
    /// ([`VimState::insert_typed_start`]) — vim deletes newly entered
    /// characters only (9.1 probes, round 30: insert mid-word, type `X`,
    /// `<C-w>` leaves the pre-existing word alone; a second `<C-w>` with the
    /// cursor back at the anchor is a no-op). With nothing typed on this
    /// line the floor is the line start (the `backspace=start` shape most
    /// hosts configure). Replace mode restores overwritten characters across
    /// the deleted span.
    fn insert_delete_word_before(&mut self, ctx: &mut Ctx) {
        let at = self.cursor.offset;
        let line_start = ctx.buf.line_start(ctx.buf.offset_to_line(at));
        // whitespace between the line start and the cursor goes in ONE
        // stroke (vim 9.1 probes: `i<C-w>` from c2/c3 of `  y` wipes the run
        // back to col 1 — the autoindent wipe; the old code fell through to
        // the JOIN branch because prev_word_start crosses the line, and then
        // peeled exactly one byte per press). A JOIN stays a col-1-only
        // stroke: mid-run presses never merge lines. The wipe floors at the
        // typing start like every other stroke, so pre-existing indent
        // behind the session's first typed character survives.
        if at > line_start
            && ctx
                .buf
                .slice(line_start..at)
                .chars()
                .all(char::is_whitespace)
        {
            // floor the anchor defensively: a host text swap can land it
            // mid-character without passing through the edit funnels
            let floor = self
                .insert_typed_start
                .map(|t| crate::buffer::floor_to_char_boundary(ctx.buf, t))
                .unwrap_or(line_start)
                .max(line_start);
            if at > floor {
                self.insert_delete_typed_span(ctx, floor..at);
                self.cursor.offset = floor;
            }
            return;
        }
        let floor = self
            .insert_typed_start
            .map(|t| crate::buffer::floor_to_char_boundary(ctx.buf, t))
            .unwrap_or(line_start)
            .max(line_start);
        let target = word::prev_word_start(ctx.buf, at, false).max(floor);
        if target >= line_start {
            if target < at {
                // words/indent on THIS row only — the row survives as a row,
                // so block sessions are unaffected
                self.insert_delete_typed_span(ctx, target..at);
                self.cursor.offset = target;
            }
            return;
        }
        if self.in_block_insert() {
            ctx.host.bell();
            return;
        }
        if at > 0 {
            // join with the previous line: delete the NEWLINE at the line
            // boundary — never the byte before the cursor. `at` can sit
            // mid-line here (the word's run start was found on the previous
            // line, e.g. the cursor word is glued across the join point by
            // a line-start combining mark): `at-1..at` then deleted an
            // arbitrary byte — mid-character on a multi-byte tail (host
            // panic: fuzz round 25) or a real char with no join at all.
            // Reachable only with the floor at the line start (nothing
            // typed past it), so no typed-span bookkeeping applies.
            let join_at = line_start - 1;
            self.begin_edit();
            self.edit_delete(ctx, join_at..line_start);
            self.cursor.offset = join_at;
            self.republish_search(ctx);
            ctx.host.changed();
        }
    }

    /// `<C-u>`: delete the newly typed characters on this line — vim's
    /// "all entered characters before the cursor" rule (round-30 probes:
    /// pre-existing text before the typing start survives). For a session
    /// that has not typed on this line the stroke falls back to the line
    /// start (`backspace=start` shape; P7 probe: cursor col 2 of "hello
    /// world", `i<C-u>` → "o world"). At the line start with nothing typed
    /// vim deletes nothing — the round-9 claim of a join was a mis-probe
    /// (round 30 typeahead re-check: `i<C-u>` at (2,1) of ['aaaa','bbbb']
    /// → ['aaaa','bbbb'] unchanged) — so the dead end is inert here too.
    /// Replace mode restores the overwritten characters inside the deleted
    /// span.
    fn insert_delete_to_line_start(&mut self, ctx: &mut Ctx) {
        let at = self.cursor.offset;
        let line_start = ctx.buf.line_start(ctx.buf.offset_to_line(at));
        let floor = match self.insert_typed_start {
            Some(t) if t < at => crate::buffer::floor_to_char_boundary(ctx.buf, t).max(line_start),
            _ => line_start,
        };
        if at > floor {
            self.insert_delete_typed_span(ctx, floor..at);
            self.cursor.offset = floor;
        }
    }

    /// The shared C-w/C-u deletion stroke. Replace mode RESTORES: characters
    /// inside `range` that replaced existing text come back as their
    /// originals (9.1 round-30 probes: `RXX<C-w>` on "one two" → "one two"
    /// with the cursor at the typing start; `Rab cd<C-u>Z` → "Zbcdefg" —
    /// C-u restored `abcde` and Z overwrote `a`). Text appended past the
    /// line end (`None` entries) deletes plainly. Insert mode plain-deletes.
    fn insert_delete_typed_span(&mut self, ctx: &mut Ctx, range: std::ops::Range<usize>) {
        if range.start >= range.end {
            return;
        }
        if self.mode == Mode::Replace {
            // rebuild the span with the stashed originals substituted back
            // in — one edit, because restoring multi-byte originals shifts
            // everything after the first substitution
            let span = ctx.buf.slice(range.clone());
            let mut out = String::new();
            for (i, c) in span.char_indices() {
                let pos = range.start + i;
                match self.replace_overwritten.iter().find(|(p, _)| *p == pos) {
                    Some((_, Some(orig))) => out.push(*orig),
                    _ => out.push(c),
                }
            }
            let delta = out.len() as isize - (range.end - range.start) as isize;
            self.begin_edit();
            self.edit_replace(ctx, range.clone(), &out);
            // entries inside the span are consumed (their originals are back
            // in the buffer); entries past it shift by the length delta
            self.replace_overwritten
                .retain(|(p, _)| !range.contains(p));
            for (p, _) in &mut self.replace_overwritten {
                if *p >= range.end {
                    *p = (*p as isize + delta).max(0) as usize;
                }
            }
        } else {
            let removed = range.end - range.start;
            let append_end = range.end;
            self.begin_edit();
            self.edit_delete(ctx, range);
            // a block session's replica mirrors the typed text: a deletion
            // that shaves a suffix off the session's append (the common
            // C-w/C-u right after typing) must shave the replica too, or the
            // exit-side delta guard sees a divergence and silently skips
            // replication. Non-suffix strokes (after an arrow move) leave
            // the replica alone — the guard skips replication for them.
            self.block_shave_typed_suffix(append_end, removed);
        }
        self.republish_search(ctx);
        ctx.host.changed();
    }

    /// Called when insert mode is left implicitly (host navigation etc.).
    pub fn ensure_normal_mode(&mut self, ctx: &mut Ctx) {
        if matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.exit_insert(ctx);
        }
    }
}
