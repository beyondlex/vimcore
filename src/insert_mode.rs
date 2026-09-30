//! Insert-mode key handling.
//!
//! Printable characters intentionally return [`KeyResult::Unknown`] (unless an
//! insert-mode mapping wants them): the host's IME path owns text input, so
//! composition (Chinese/Japanese input, accents, ...) keeps working. Text
//! arrives back through [`VimState::insert_text_at_cursor`].

use crate::key::{Key, KeyKind};
use crate::mode::Mode;
use crate::motions::Motion;
use crate::state::{Ctx, ProcessOutcome, VimState};
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

        // exit insert: <Esc>, <C-[>, <C-c>
        if key == Key::escape() || key == Key::ctrl_char('[') || key == Key::ctrl_char('c') {
            self.exit_insert(ctx);
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
                        if let Some(c) = ctx.buf.char_at(at) {
                            // deleting the newline would merge the typing row
                            // into its neighbor — same corruption as a
                            // vertical move (block session invariants)
                            if c == '\n' && self.in_block_insert() {
                                ctx.host.bell();
                                return ProcessOutcome::Consumed;
                            }
                            self.begin_edit();
                            self.edit_delete(ctx, at..at + c.len_utf8());
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
                        self.goto_motion(ctx, Motion::PageUp, 1);
                        return ProcessOutcome::Consumed;
                    }
                    "pagedown" => {
                        if self.in_block_insert() {
                            ctx.host.bell();
                            return ProcessOutcome::Consumed;
                        }
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

        // printable text: let the host/IME decide *unless* an insert mapping
        // is interested (e.g. `jk` -> Esc)
        if let Some(c) = key.printable_char() {
            self.pending_unknown_char = Some(c);
            return ProcessOutcome::Unknown;
        }

        ProcessOutcome::Unknown
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

        // Replace mode: BS restores the overwritten character and steps
        // back (vim `R` + BS). `None` entries were APPENDED past the line
        // end — there is nothing to restore, so they plain-delete. The
        // position guard comes FIRST: at offset 0 there is nothing to
        // restore, and consuming a stack entry there would desync the
        // remaining entries from their positions.
        if self.mode == Mode::Replace && at > 0 {
            if let Some(orig) = self.replace_overwritten.pop() {
                if at > line_start {
                    if let Some(prev) = ctx.buf.prev_char_offset(at) {
                        self.begin_edit();
                        match orig {
                            Some(c) => self.edit_replace(ctx, prev..at, &c.to_string()),
                            None => self.edit_delete(ctx, prev..at),
                        }
                        self.cursor.offset = prev;
                        ctx.host.changed();
                    }
                } else {
                    // crossed the line start: join with the previous line
                    self.begin_edit();
                    self.edit_delete(ctx, at - 1..at);
                    self.cursor.offset = at - 1;
                    ctx.host.changed();
                }
                return;
            }
        }

        if at > line_start {
            if let Some(prev) = ctx.buf.prev_char_offset(at) {
                self.begin_edit();
                self.edit_delete(ctx, prev..at);
                self.cursor.offset = prev;
                // a block-insert session replicates block.text onto the other
                // rows on exit: a backspace that undoes typed text must
                // shrink it too, or the replicas carry the deleted char
                self.block_backspace_undo(at);
                ctx.host.changed();
            }
        } else if at > 0 {
            // join with the previous line
            self.begin_edit();
            self.edit_delete(ctx, at - 1..at);
            self.cursor.offset = at - 1;
            ctx.host.changed();
        }
    }

    fn insert_tab(&mut self, ctx: &mut Ctx) {
        if self.options.expandtab {
            let sw = self.options.tabstop.max(1);
            // align by DISPLAY column (wide chars cover two cells): vim pads
            // to the next multiple of 'tabstop' of the virtual column (9.1
            // probe: '中文' + Tab at display col 4, ts=4 → four spaces; the
            // old BYTE-column math (6 % 4) gave two and broke the grid)
            let col = crate::buffer::display_column(ctx.buf, self.cursor.offset);
            let spaces = sw - (col % sw);
            self.insert_text_at_cursor(ctx, &" ".repeat(spaces));
        } else {
            self.insert_text_at_cursor(ctx, "\t");
        }
    }

    /// `<C-w>`: delete the word before the cursor, like typing `b` then
    /// deleting. Word classes follow normal mode (`(`, `bar`, `)` are three
    /// separate words). A deletion that would cross the line start JOINS the
    /// line with its predecessor instead — vim deletes exactly the newline
    /// (9.1 probe: `i<C-w>` at (2,1) of ['aaaa','bbbb'] → ['aaaabbbb'] and
    /// ['aaaa','  bbbb','cc'] at (3,1) keeps both lines' text). The block-
    /// session lock: replica offsets assume every row stays a row, so the
    /// join is refused with a bell (the same rule as the line-start BS and
    /// `<C-u>` below).
    fn insert_delete_word_before(&mut self, ctx: &mut Ctx) {
        let at = self.cursor.offset;
        let line_start = ctx.buf.line_start(ctx.buf.offset_to_line(at));
        let target = word::prev_word_start(ctx.buf, at, false);
        if target >= line_start {
            if target < at {
                // words/indent on THIS row only — the row survives as a row,
                // so block sessions are unaffected
                self.begin_edit();
                self.edit_delete(ctx, target..at);
                self.cursor.offset = target;
                ctx.host.changed();
            }
            return;
        }
        if self.in_block_insert() {
            ctx.host.bell();
            return;
        }
        if at > 0 {
            // join with the previous line: delete the newline (the same edit
            // the line-start BS does)
            self.begin_edit();
            self.edit_delete(ctx, at - 1..at);
            self.cursor.offset = at - 1;
            ctx.host.changed();
        }
    }

    /// `<C-u>`: delete to the line start; at the line start vim JOINS with
    /// the previous line (removes the newline — 9.1 probe: `i<C-u>` at (2,1)
    /// of ['aaaa','bbbb'] → ['aaaabbbb']), so the dead end does the same
    /// edit the line-start BS does. Block sessions refuse the join (the
    /// replica offsets assume every row stays a row).
    fn insert_delete_to_line_start(&mut self, ctx: &mut Ctx) {
        let at = self.cursor.offset;
        let line_start = ctx.buf.line_start(ctx.buf.offset_to_line(at));
        if at > line_start {
            self.begin_edit();
            self.edit_delete(ctx, line_start..at);
            self.cursor.offset = line_start;
            ctx.host.changed();
            return;
        }
        if self.in_block_insert() {
            ctx.host.bell();
            return;
        }
        if at > 0 {
            self.begin_edit();
            self.edit_delete(ctx, at - 1..at);
            self.cursor.offset = at - 1;
            ctx.host.changed();
        }
    }

    /// Called when insert mode is left implicitly (host navigation etc.).
    pub fn ensure_normal_mode(&mut self, ctx: &mut Ctx) {
        if matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.exit_insert(ctx);
        }
    }
}
