//! Command-line mode: `/` `?` search prompts and `:` Ex commands, with
//! per-prompt history.

use std::collections::HashMap;

use crate::key::{Key, KeyKind};
use crate::mode::Mode;
use crate::search;
use crate::state::{Ctx, KeyResult, VimState};

/// Platforms may deliver Enter, Backspace and Tab as bare control characters
/// through the text-input path (`"\n"`, `"\x7f"`, `"\t"`). Normalize them so
/// the prompt treats them like their named keys instead of pattern text.
/// C-h folds to Backspace too (crossterm-style hosts deliver it raw; vim
/// binds C-h = BS at the prompt).
fn normalize_control_char(key: Key) -> Key {
    if !key.modifiers.is_plain() {
        return key;
    }
    if let KeyKind::Char(c) = key.kind {
        return match c {
            '\n' | '\r' => Key::enter(),
            '\x7f' => Key::backspace(),
            '\x08' => Key::backspace(),
            '\t' => Key::tab(),
            _ => key,
        };
    }
    key
}

/// vim's Ex abbreviation rule: every unambiguous prefix of the full name is
/// accepted. Hand-rolled static lists (greppable); `:s`/`substitute…` live in
/// [`VimState::ex_substitute`], `:ju[mp]` is deliberately absent (a DIFFERENT
/// vim command — NOTES 分歧 #26).
const DELETE_SPELLINGS: &[&str] = &["d", "de", "del", "dele", "delet", "delete"];
const YANK_SPELLINGS: &[&str] = &["y", "ya", "yan", "yank"];
const JOIN_SPELLINGS: &[&str] = &["j", "jo", "joi", "join"];
const SORT_SPELLINGS: &[&str] = &["sor", "sort"];

/// `parse_range` 的返回体：范围（`None` = 未写范围前缀）+ 剥离范围后的命令。
/// A parsed `[range]`: the clamped `(first, last)` lines, how many addresses
/// the user actually TYPED (a bare `:5j` is one address and joins with the
/// next line; `:2,2j` typed two equal ones and is a no-op — vim), and the
/// command text that follows the range.
type ParsedRange<'a> = (Option<(usize, usize)>, usize, &'a str);

/// Command-line input buffer + per-prompt history.
#[derive(Default)]
pub struct Cmdline {
    pub buffer: String,
    /// History per prompt: `/` and `?` SHARE the search history (vim), `:`
    /// has its own. Keys are canonicalized through [`history_key`].
    pub history: HashMap<char, Vec<String>>,
    /// Position while browsing history with Up/Down; None = typing.
    pub history_pos: Option<usize>,
    /// In-progress input stashed while browsing history.
    pub stash: Option<String>,
    /// Last executed Ex command line (`@:` replays it).
    pub last_command: Option<String>,
    /// The last substitute command line (`s/pat/rep/flags`), for `&` and the
    /// bare `:s` repeat. Stored when the command PARSES (even on E486 — vim
    /// retries the same command and reports the same miss).
    pub last_substitute: Option<String>,
    /// Count typed before the `/`/`?` prompt (`3/foo` = 3rd match from the
    /// cursor — vim 9.1 probe). Consumed by the executing Enter; a cancelled
    /// prompt drops it (probe: `2/x<Esc>` then `x` deletes ONE char — the
    /// count must not leak into the next command). 0 = no count.
    pub search_count: usize,
}

impl Cmdline {
    /// `/` and `?` share one search history, like vim.
    fn history_key(prompt: char) -> char {
        if prompt == ':' {
            ':'
        } else {
            '/'
        }
    }

    fn history_for(&mut self, prompt: char) -> &mut Vec<String> {
        self.history.entry(Self::history_key(prompt)).or_default()
    }
}

impl VimState {
    pub(crate) fn begin_cmdline(&mut self, prompt: char) {
        self.cmdline.buffer.clear();
        self.cmdline.history_pos = None;
        self.mode = Mode::CommandLine { prompt };
    }

    /// Execute the current pattern and jump to the first match.
    fn execute_search(&mut self, ctx: &mut Ctx, pattern: String, forward: bool) {
        self.mode = Mode::Normal;
        if pattern.is_empty() {
            // empty pattern: re-use the last one, like vim — but in the
            // direction of the CURRENT prompt (`?` + Enter repeats BACKWARD,
            // `/` + Enter forward; verified against vim 9.1)
            match self.search.pattern.clone() {
                Some(last) => {
                    search::set_pattern(self, ctx, last, forward);
                    let count = std::mem::take(&mut self.cmdline.search_count).max(1);
                    self.jump_to_current_match(ctx, forward, count);
                    ctx.host.changed();
                }
                // vim reports E35 instead of a silent no-op (probe:
                // `/<CR>` with no previous search sets v:errmsg to E35)
                None => {
                    ctx.host
                        .status_message("E35: No previous regular expression");
                    ctx.host.bell();
                }
            }
            return;
        }
        self.cmdline.history_pos = None;
        self.cmdline.stash = None;
        search::set_pattern(self, ctx, pattern, forward);
        let count = std::mem::take(&mut self.cmdline.search_count).max(1);
        self.jump_to_current_match(ctx, forward, count);
        ctx.host.changed();
    }

    /// Move to the nearest match for the active pattern in `forward`.
    fn jump_to_current_match(&mut self, ctx: &mut Ctx, forward: bool, count: usize) {
        if let Some(offset) = search::jump_to_match(self, ctx.buf, forward, count) {
            let origin = self.cursor.offset;
            self.cursor.offset = offset;
            self.cursor.desired_col = None;
            self.record_jump(origin, offset);
            // publish with the current match marked (respecting `hlsearch`).
            // `jump_to_match` returns a `.start` from `last_matches` (it may
            // have re-scanned into it), so no second scan is needed here.
            if self.options.hlsearch {
                let matches = self.search.last_matches.clone();
                let current = matches.iter().find(|m| m.start == offset).cloned();
                ctx.host.set_search_highlights(&matches, current);
            } else {
                ctx.host.set_search_highlights(&[], None);
            }
            ctx.host.scroll_to_line(ctx.buf.offset_to_line(offset));
        } else {
            // vim reports the miss, not just a bell: E35 before any search
            // was entered, E486 with the pattern otherwise
            match &self.search.pattern {
                Some(pattern) => {
                    ctx.host
                        .status_message(&format!("E486: Pattern not found: {pattern}"));
                }
                None => ctx
                    .host
                    .status_message("E35: No previous regular expression"),
            }
            ctx.host.bell();
        }
    }

    /// Esc at the prompt: a visual `:` returns to the intact selection
    /// (vim semantics) and a second Esc from there exits visual; a plain
    /// search/ex prompt aborts back to normal mode. Highlights are restored
    /// to the pre-prompt set — incremental highlighting is preview-only and
    /// must not leak as an accepted pattern.
    fn cancel_cmdline(&mut self, ctx: &mut Ctx) {
        if let Some((kind, anchor, _prompt_cursor)) = self.cmdline_visual.take() {
            self.mode = Mode::Visual { kind };
            self.visual_anchor = Some(anchor);
        } else {
            self.mode = Mode::Normal;
        }
        self.discard_change_record();
        self.cmdline.buffer.clear();
        self.cmdline.search_count = 0;
        // restore the previous highlight set — but only when `hlsearch` may
        // show one at all: with hlsearch OFF the pre-prompt set is empty,
        // and republishing `last_matches` here would leave permanent
        // highlights behind an incsearch-only preview (hlsearch=false +
        // incsearch=true: cancel must clear, like vim)
        if self.options.hlsearch {
            let matches = self.search.last_matches.clone();
            ctx.host.set_search_highlights(&matches, None);
        } else {
            ctx.host.set_search_highlights(&[], None);
        }
        ctx.host.changed();
    }

    /// One keystroke at the prompt. Printable chars append to the buffer
    /// (and drive incremental search on `/`/`?`); Enter executes and pushes
    /// to this prompt's history (consecutive duplicates deduped); Up/Down
    /// browse history with an in-progress stash; Esc cancels. Everything
    /// else is swallowed — a cmdline is never `Unknown` to the host.
    pub(crate) fn cmdline_key(&mut self, ctx: &mut Ctx, key: Key) -> KeyResult {
        let Mode::CommandLine { prompt } = self.mode else {
            return KeyResult::Consumed;
        };
        let key = normalize_control_char(key);
        // <C-c> cancels the prompt (vim's interrupt), not just <Esc>
        if key == Key::ctrl_char('c') {
            self.cancel_cmdline(ctx);
            return KeyResult::Consumed;
        }
        // <C-w>/<C-u> edit the prompt line itself (vim's cmdline window is
        // out of scope, but the two delete chords every rc file trains into
        // muscle memory are not): C-w kills the last word, C-u clears.
        if key == Key::ctrl_char('w') {
            let no_tail_ws = self.cmdline.buffer.trim_end_matches(' ');
            let cut = no_tail_ws
                .char_indices()
                .rev()
                .take_while(|(_, c)| !c.is_whitespace())
                .last()
                .map(|(i, _)| i)
                .unwrap_or_else(|| no_tail_ws.len());
            self.cmdline.buffer.truncate(cut);
            ctx.host.changed();
            return KeyResult::Consumed;
        }
        if key == Key::ctrl_char('u') {
            self.cmdline.buffer.clear();
            ctx.host.changed();
            return KeyResult::Consumed;
        }
        match &key.kind {
            KeyKind::Char(c) if key.modifiers.is_plain() => {
                self.cmdline.buffer.push(*c);
                self.cmdline.history_pos = None;
                // incremental search applies to the search prompts only —
                // a half-typed `:set` line is not a pattern
                if self.options.incsearch && prompt != ':' {
                    let pattern = self.cmdline.buffer.clone();
                    search::publish_incsearch(self, ctx, &pattern);
                }
                ctx.host.changed();
                KeyResult::Consumed
            }
            KeyKind::Named(name) if key.modifiers.is_plain() => match name.as_str() {
                "enter" => {
                    // record into this prompt's history (dedup consecutive
                    // repeats; an empty entry reuses the previous value)
                    let entry = std::mem::take(&mut self.cmdline.buffer);
                    if !entry.is_empty() {
                        // vim relocates an older duplicate to the end
                        // (`/foo /bar /foo` browses as bar → foo, the older
                        // "foo" is gone)
                        let history = self.cmdline.history_for(prompt);
                        history.retain(|h| h != &entry);
                        history.push(entry.clone());
                    }
                    self.cmdline.history_pos = None;
                    self.cmdline.stash = None;
                    if prompt == ':' {
                        self.mode = Mode::Normal;
                        if !entry.is_empty() {
                            self.cmdline.last_command = Some(entry.clone());
                        }
                        let gen_before = self.edit_generation;
                        self.execute_ex(ctx, &entry);
                        // A non-mutating Ex command (`:5`, `:noh`, `:reg`,
                        // E492…) must not glue itself onto the NEXT change's
                        // `.` recording: vim repeats only the change (9.1
                        // probe: `:2` `x` `gg` `.` repeats just the `x` at the
                        // current line, not "jump to line 2 then delete").
                        // Mutating commands (`:s`/`:d`/`:j`/`:sort`) commit
                        // inside execute_ex — their edit generation moved.
                        if self.edit_generation == gen_before {
                            self.discard_change_record();
                        }
                        // executing a visual `:` command ends visual mode
                        // (marks written, anchor cleared), like vim
                        if let Some((kind, anchor, prompt_cursor)) = self.cmdline_visual.take() {
                            self.close_visual_after_cmdline(ctx.buf, kind, anchor, prompt_cursor);
                        }
                    } else {
                        self.execute_search(ctx, entry, prompt == '/');
                    }
                    KeyResult::Consumed
                }
                "escape" => {
                    self.cancel_cmdline(ctx);
                    KeyResult::Consumed
                }
                "backspace" => {
                    // an empty prompt KEEPS cmdline mode on <BS> (vim probe:
                    // mode() stays 'c') — only a real deletion edits text
                    if self.cmdline.buffer.pop().is_some() {
                        if self.options.incsearch && prompt != ':' {
                            let pattern = self.cmdline.buffer.clone();
                            search::publish_incsearch(self, ctx, &pattern);
                        }
                        ctx.host.changed();
                    }
                    KeyResult::Consumed
                }
                "up" | "down" => {
                    let Some(history) = self.cmdline.history.get(&Cmdline::history_key(prompt))
                    else {
                        return KeyResult::Consumed;
                    };
                    if history.is_empty() {
                        return KeyResult::Consumed;
                    }
                    let pos = match self.cmdline.history_pos {
                        None => {
                            if name == "up" {
                                self.cmdline.stash = Some(self.cmdline.buffer.clone());
                                history.len() - 1
                            } else {
                                return KeyResult::Consumed;
                            }
                        }
                        // stored positions are always in range and history
                        // only grows, so browsing up is just "one earlier"
                        Some(pos) if name == "up" => pos.saturating_sub(1),
                        Some(pos) => pos + 1,
                    };
                    if pos >= history.len() {
                        // past the newest entry: back to typing
                        self.cmdline.history_pos = None;
                        self.cmdline.buffer = self.cmdline.stash.take().unwrap_or_default();
                    } else {
                        self.cmdline.history_pos = Some(pos);
                        self.cmdline.buffer = history[pos].clone();
                    }
                    ctx.host.changed();
                    KeyResult::Consumed
                }
                _ => KeyResult::Consumed,
            },
            _ => KeyResult::Consumed,
        }
    }

    /// Close out a visual selection after its `:'<,'>` command ran: write
    /// the `<`/`>` marks from the PROMPT-TIME selection (vim keeps the
    /// executed range — marks written from the post-command cursor would
    /// rewrite `'<`/`'>` wherever the command left the cursor and break a
    /// later `gv`), drop the anchor and return to normal mode. The engine
    /// cursor is NOT moved here — vim lets the executed command decide.
    fn close_visual_after_cmdline(
        &mut self,
        buf: &dyn crate::buffer::VimBuffer,
        kind: crate::mode::VisualKind,
        anchor: usize,
        prompt_cursor: usize,
    ) {
        let cursor = prompt_cursor;
        let (lo, hi) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        // the command may have shrunk the buffer: the prompt-time bounds are
        // pre-edit bytes and must be floored/clamped onto the CURRENT text
        // before they become stored marks (the engine's addressability
        // invariant — fuzz caught `last_visual` past the end of an emptied
        // buffer)
        let lo = crate::buffer::floor_to_char_boundary(buf, lo);
        let hi = crate::buffer::floor_to_char_boundary(buf, hi);
        let (lo, hi) = (lo.min(hi), hi.max(lo));
        self.marks.set('<', lo);
        self.marks.set('>', hi);
        // exclusive end one CHAR past the last covered char — `hi + 1` bytes
        // would sit inside a multi-byte cursor char (same rule as
        // `exit_visual`; consumers floor it, but stored bounds stay clean)
        let end = buf.next_char_offset(hi).unwrap_or(hi);
        // one source of truth (kind included) — `gv` and `'<`/`'>` read it
        self.marks.last_visual = Some((lo, end, kind));
        self.visual_anchor = None;
        self.marks.active_visual = None;
        self.mode = Mode::Normal;
    }

    // ---- `:` Ex commands ---------------------------------------------------

    /// Execute a `:` command line. Supported in v1: `:noh[lsearch]`,
    /// `:set` (booleans, `no`/`!`/`name?` forms, `name=value` numerics, bare
    /// listing), `:[%]s/pat/rep/[g]`, `:[range]d[elete]`, `:[range]y[ank]`,
    /// `:[range]sor[t]`, `:[range]j[oin]`, `:reg[isters]`, `:marks`,
    /// `:w`, `:q`/`:q!`, `:wq`/`:x`, `:bn[ext]`/`:bp[revious]`, and
    /// IdeaVim's `:action <id>` bridge. Unknown commands get E492 and return
    /// to normal mode (mode is already Normal here).
    pub(crate) fn execute_ex(&mut self, ctx: &mut Ctx, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        // a leading `"` comments the whole line out (vim: `: " scratch` is a
        // silent no-op, not E492)
        if line.starts_with('"') {
            return;
        }
        // the range prefix is parsed off before command dispatch; `None`
        // means "no range typed" — commands then apply their own default
        let (range, addr_count, line) = match Self::parse_range(self, ctx, line) {
            Ok(parsed) => parsed,
            Err(message) => {
                ctx.host.status_message(&message);
                ctx.host.bell();
                return;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            // a range with no command moves the cursor to the range's LAST
            // address (`:5` = line 5, `:2,5` = line 5 — 9.1 probe: the old
            // code jumped to `first`, so `:2,5<CR>` landed on line 1); a
            // plain `:` (no range) is a no-op
            if let Some((_, addr)) = range {
                let line_no = addr.min(ctx.buf.line_count().saturating_sub(1));
                self.cursor.offset = ctx.buf.first_non_blank(line_no);
                self.cursor.desired_col = None;
                ctx.host.scroll_to_line(line_no);
            }
            return;
        }
        match line {
            "noh" | "nohl" | "nohls" | "nohlsearch" | "noh!" | "nohl!" | "nohls!"
            | "nohlsearch!" => {
                search::clear_highlights(self, ctx);
                return;
            }
            "w" | "write" | "w!" | "write!" => {
                ctx.host.save();
                return;
            }
            "q" | "quit" => {
                // not forced: the host may refuse (vim E37 on modified)
                ctx.host.request_close_forced(false);
                return;
            }
            "q!" | "quit!" => {
                ctx.host.request_close_forced(true);
                return;
            }
            // `:x` writes even when the buffer is unmodified here; vim only
            // writes when changes were made (`:h :x`). The host's `save` is
            // free to no-op for clean buffers — the engine has no modified
            // flag of its own, so the clean/dirty decision is the host's.
            "wq" | "x" | "xit" | "wq!" | "x!" | "xit!" => {
                ctx.host.save();
                ctx.host.request_close_forced(true);
                return;
            }
            // :bf[irst]/:brewind — the host owns the buffer list (NOTES.md
            // 第八轮「悬而未决」闭环); a refusal rings like :bN's.
            "bfirst" | "bf" | "brewind" => {
                if !ctx.host.first_buffer() {
                    ctx.host.bell();
                }
                return;
            }
            "blast" | "bl" => {
                if !ctx.host.last_buffer() {
                    ctx.host.bell();
                }
                return;
            }
            // :reg[isters] — one status line per populated register
            "reg" | "registers" => {
                for (name, reg) in self.registers.items() {
                    ctx.host.status_message(&Self::register_line(name, &reg));
                }
                return;
            }
            // :marks — named marks plus the specials, `mark  line  col  text`
            "marks" => {
                for (name, offset) in self.marks.items() {
                    ctx.host
                        .status_message(&Self::mark_line(ctx.buf, name, offset));
                }
                // items() already carries a user-set `.` mark (m.) — only
                // append the engine-tracked row when it is not listed yet,
                // or the listing shows two rows for one mark
                if let Some(offset) = self.marks.last_change {
                    if !self.marks.items().iter().any(|(n, _)| *n == '.') {
                        ctx.host
                            .status_message(&Self::mark_line(ctx.buf, '.', offset));
                    }
                }
                if let Some(offset) = self.marks.last_insert_exit {
                    ctx.host
                        .status_message(&Self::mark_line(ctx.buf, '^', offset));
                }
                if let Some((lo, _, _)) = self.marks.last_visual {
                    ctx.host.status_message(&Self::mark_line(ctx.buf, '<', lo));
                }
                if let Some(offset) = self.marks.last_jump {
                    ctx.host
                        .status_message(&Self::mark_line(ctx.buf, '\'', offset));
                }
                return;
            }
            _ => {}
        }
        if let Some(rest) = line
            .strip_prefix("set")
            .filter(|rest| rest.is_empty() || rest.starts_with(' '))
        {
            self.ex_set(ctx, rest.trim_start());
            return;
        }
        if line == "bnext" || line == "bn" {
            if !ctx.host.cycle_buffer(true) {
                ctx.host.bell();
            }
            return;
        }
        if line == "bprev" || line == "bprevious" || line == "bp" || line == "bN" {
            if !ctx.host.cycle_buffer(false) {
                ctx.host.bell();
            }
            return;
        }
        // IdeaVim's host-action bridge: :action SomeId dispatches the host
        // application action by id (typically the RHS of a :map)
        if let Some(id) = line.strip_prefix("action").filter(|r| r.starts_with(' ')) {
            let id = id.trim();
            if id.is_empty() {
                ctx.host.bell();
            } else {
                // strict = the layer wants misses reported (host-specific
                // rc); lenient = shared-rc misses should pass silently
                ctx.host
                    .dispatch_host_action_hinted(id, !self.lenient_actions);
            }
            return;
        }
        // `:s`/`:d`/`:y`/`:j` without a range default to the current line;
        // `addr_count` is how many addresses the user typed (from the range
        // parse above) and already carries the no-range case as 0
        let range = range.unwrap_or_else(|| {
            let current = ctx.buf.offset_to_line(self.cursor.offset);
            (current, current)
        });
        if self.ex_substitute(ctx, line, range) {
            return;
        }
        // :{range}d[elete] [x] [count] — delete the range's lines; the first
        // argument may name a REGISTER (`:1,2d a` stores into `"a`, vim) or
        // be a COUNT that re-anchors at the range's LAST line (vim: `:1,2d 3`
        // deletes lines 2-4; an EXPLICIT `:1,2d 1` deletes just line 2 —
        // any given count, even 1, re-anchors)
        if let Some(rest) = DELETE_SPELLINGS
            .iter()
            .find_map(|cmd| Self::boundary_cmd(line, cmd))
        {
            let (register, count) = Self::parse_reg_count(rest.trim());
            let (mut first, last) = range;
            if count >= 1 {
                first = last;
            }
            let last = last
                .saturating_add(count.saturating_sub(1))
                .min(ctx.buf.line_count().saturating_sub(1));
            self.ex_delete_lines(ctx, (first, last), register);
            return;
        }
        // :{range}y[ank] [x] [count] — yank the range's lines into a register
        if let Some(rest) = YANK_SPELLINGS
            .iter()
            .find_map(|cmd| Self::boundary_cmd(line, cmd))
        {
            self.ex_yank_lines(ctx, range, rest.trim());
            return;
        }
        // :{range}sor[t] [!] [i] [u] — sort the range's lines (before `s`:
        // `sort` starts with an `s` and would otherwise hit the substitute
        // parser's alphanumeric-separator guard)
        if let Some(rest) = SORT_SPELLINGS
            .iter()
            .find_map(|cmd| Self::boundary_cmd(line, cmd))
        {
            self.ex_sort(ctx, range, rest.trim());
            return;
        }
        // :{range}j[oin][!] [count] — join the range's lines; a ONE-ADDRESS
        // range joins with the NEXT line (vim's bare `:j`), `!` removes all
        // whitespace. A trailing COUNT joins count lines starting at the
        // range's LAST line (`:2j 3` joins lines 2-4, vim 9.1 probe — the
        // engine used to ignore it and join only two). `:ju` is NOT here:
        // vim's `:ju[mp]` is a different command (deliberately unsupported —
        // NOTES 分歧 #26).
        if let Some(rest) = JOIN_SPELLINGS
            .iter()
            .find_map(|cmd| Self::boundary_cmd(line, cmd))
        {
            self.ex_join(ctx, range, rest.trim(), addr_count);
            return;
        }
        ctx.host
            .status_message(&format!("E492: Not an editor command: {line}"));
        ctx.host.bell();
    }

    /// `cmd` at the line start with a command boundary (end, space, `!`).
    fn boundary_cmd<'a>(line: &'a str, cmd: &str) -> Option<&'a str> {
        line.strip_prefix(cmd)
            .filter(|rest| rest.is_empty() || rest.starts_with(' ') || rest.starts_with('!'))
    }

    /// [`Self::substitute_without_flags`] for callers outside this module
    /// (the `&` command lives in state.rs).
    pub(crate) fn strip_substitute_flags_for_repeat(cmd: &str) -> String {
        Self::substitute_without_flags(cmd)
    }

    /// `&` and bare `:s` repeat the last substitute WITHOUT its flags (vim
    /// 9.1 probe: after `s/a/B/g`, both on "xaxax" replace only the first
    /// match — the `g` is dropped; `:h :&` calls this out explicitly).
    /// Rebuild `s{sep}pat{sep}rep{sep}` from the stored command line,
    /// leaving a missing-replacement form (`s/pat`) as-is.
    fn substitute_without_flags(cmd: &str) -> String {
        let Some(after_s) = cmd.strip_prefix('s') else {
            return cmd.to_owned();
        };
        let Some(sep) = after_s.chars().next() else {
            return cmd.to_owned();
        };
        if sep.is_alphanumeric() {
            return cmd.to_owned();
        }
        let mut parts = after_s[sep.len_utf8()..].split(sep);
        let pattern = parts.next().unwrap_or("");
        match parts.next() {
            Some(rep) => format!("s{sep}{pattern}{sep}{rep}{sep}"),
            None => format!("s{sep}{pattern}"),
        }
    }

    /// One `:registers` listing line: `"x  c|l|b  text` with embedded
    /// newlines shown as `^J` (vim's rendering) and the tail elided.
    fn register_line(name: char, reg: &crate::registers::Register) -> String {
        const MAX_TEXT: usize = 50;
        let flat = reg.text.trim_end_matches('\n').replace('\n', "^J");
        let mut shown: String = flat.chars().take(MAX_TEXT).collect();
        if flat.chars().count() > MAX_TEXT {
            shown.push('…');
        }
        let kind = match reg.kind {
            crate::registers::RegisterKind::Charwise => "c",
            crate::registers::RegisterKind::Linewise => "l",
            crate::registers::RegisterKind::Blockwise => "b",
        };
        format!("\"{name}  {kind}  {shown}")
    }

    /// One `:marks` listing line: `mark  line  col  text`.
    fn mark_line(buf: &dyn crate::buffer::VimBuffer, name: char, offset: usize) -> String {
        let line = buf.offset_to_line(offset);
        let col = crate::buffer::display_column(buf, offset) + 1;
        let text: String = buf.line_content(line).trim().chars().take(40).collect();
        format!("{name}  line {}  col {}  {}", line + 1, col, text)
    }

    /// Parse an Ex range prefix: `%`, `.`, `$`, `'`, numbers, each with an
    /// optional `+n`/`-n` offset, joined by `,` or `;`. Returns the resolved
    /// inclusive line range (`None` when no prefix was typed — the command
    /// then decides its own default) and the remainder of the line (the
    /// command), or the vim error text (`E16` syntax, `E20` unset mark —
    /// probed: `:'<,'>d` with no prior visual is `E20: Mark '< not set`).
    /// Note: vim's `;` sets the cursor to each intermediate address; here
    /// `,` and `;` are treated alike (documented divergence).
    fn parse_range<'a>(
        vim: &VimState,
        ctx: &Ctx,
        line: &'a str,
    ) -> Result<ParsedRange<'a>, String> {
        enum Base {
            /// `%` is unusable as a per-address base (handled by the caller).
            Unusable,
            Line(usize),
        }
        fn base_line(spec: &str, vim: &VimState, ctx: &Ctx) -> Result<Base, String> {
            let unset_mark = |name: &str| format!("E20: Mark {name} not set");
            match spec {
                "." | "" => Ok(Base::Line(ctx.buf.offset_to_line(vim.cursor.offset))),
                "%" => Ok(Base::Unusable),
                "$" => Ok(Base::Line(ctx.buf.line_count().saturating_sub(1))),
                "'<" => vim
                    .marks
                    .active_visual()
                    .map(|(a, _)| Base::Line(ctx.buf.offset_to_line(a)))
                    .or_else(|| {
                        vim.marks.resolve('<').map(|off| {
                            let off = crate::buffer::floor_to_char_boundary(ctx.buf, off);
                            Base::Line(ctx.buf.offset_to_line(off))
                        })
                    })
                    .ok_or_else(|| unset_mark("'<")),
                "'>" => {
                    vim.marks
                        .active_visual()
                        .map(|(_, b)| {
                            Base::Line(ctx.buf.offset_to_line(
                                crate::buffer::floor_to_char_boundary(ctx.buf, b.saturating_sub(1)),
                            ))
                        })
                        .or_else(|| {
                            vim.marks.resolve('>').map(|off| {
                                // '<'> hi is exclusive; floor(hi-1) = last char
                                let off = crate::buffer::floor_to_char_boundary(
                                    ctx.buf,
                                    off.saturating_sub(1),
                                );
                                Base::Line(ctx.buf.offset_to_line(off))
                            })
                        })
                        .ok_or_else(|| unset_mark("'>"))
                }
                // `'a`-style marks: resolve through the mark table (the range
                // scanner accepts any `'x`; dropping them here made
                // `:'a,'b d` fail with E16 even though they parsed)
                other if other.starts_with('\'') && other.len() > 1 => {
                    // the name is one full char — a multi-byte char must not
                    // be split (`:'中` panicking `split_at` was round12's
                    // user-triggerable crash); vim reports E78 for unknown
                    // mark *names*, E20 for unset ones.
                    let name = other[1..].chars().next().unwrap_or('\'');
                    if !name.is_ascii_alphanumeric()
                        && !matches!(name, '"' | '<' | '>' | '[' | ']' | '(' | ')' | '{' | '}' | '.' | '^' | '\'' | '`')
                    {
                        return Err("E78: Unknown mark".to_owned());
                    }
                    vim.marks
                        .resolve(name)
                        .map(|off| {
                            let off = crate::buffer::floor_to_char_boundary(ctx.buf, off);
                            Base::Line(ctx.buf.offset_to_line(off))
                        })
                        .ok_or_else(|| unset_mark(other))
                }
                other => other
                    .parse::<usize>()
                    .ok()
                    .map(|n| Base::Line(n.saturating_sub(1)))
                    .ok_or_else(|| "E16: Invalid range".to_owned()),
            }
        }
        fn with_offset(base: usize, spec: &str) -> usize {
            // Chained offsets accumulate (`:5+2+1d` = line 8 — 9.1 probe);
            // only the first run of `+`/`-` segments belongs to this address.
            let mut value = base;
            let mut rest = spec;
            while !rest.is_empty() {
                let (sign, digits) = match rest.strip_prefix('-') {
                    Some(d) => (false, d),
                    None => match rest.strip_prefix('+') {
                        Some(d) => (true, d),
                        None => break,
                    },
                };
                let end = digits
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(digits.len());
                let n: usize = digits[..end].parse().unwrap_or(0);
                value = if sign {
                    value.saturating_add(n)
                } else {
                    value.saturating_sub(n)
                };
                rest = &digits[end..];
            }
            value
        }
        // split off the range part: a command starts at the first letter
        // that is not part of a `'<` / `'>` mark spec. Scan the allowed
        // range alphabet manually, char-by-char so a multi-byte char after
        // `'` (e.g. `:'中d`) can never push `range_end` into the middle of
        // it — that made `line.split_at` panic (round12, user-triggerable).
        let mut range_end = 0usize;
        let mut i = 0usize;
        while i < line.len() {
            let c = line[i..].chars().next().unwrap();
            if c == '\'' {
                // mark spec: ' + one full char
                i += 1 + line[i + 1..].chars().next().map_or(0, char::len_utf8);
                range_end = i.min(line.len());
                continue;
            }
            // `>` is NOT a range character: `'>` arrives through the mark
            // branch above, and letting a bare `>` into the range made
            // `:5>` (the shift command, E492 here) parse as a bogus range
            // and report a misleading E16
            if c.is_ascii_digit()
                || matches!(c, '.' | '$' | '%' | ',' | ';' | '+' | '-' | ' ')
            {
                i += 1;
                range_end = i;
                continue;
            }
            break;
        }
        let (range_part, rest) = line.split_at(range_end);
        if range_part.is_empty() {
            // no prefix: the command applies its own default
            return Ok((None, 0, line));
        }
        if range_part.trim_end() == "%" {
            let last = ctx.buf.line_count().saturating_sub(1);
            return Ok((Some((0, last)), 1, rest.trim_start()));
        }
        let last = ctx.buf.line_count().saturating_sub(1);
        let cursor_line = ctx.buf.offset_to_line(vim.cursor.offset);
        // vim keeps only the LAST TWO addresses of a multi-address range
        // (9.1 probe: `:1,2,3d` on ['a','b','c','d'] deletes lines 2-3).
        let mut addresses: Vec<usize> = Vec::new();
        for part in range_part.split([',', ';']) {
            let part = part.trim();
            let (base_str, off_str) = part
                .find(['+', '-'])
                .map(|i| part.split_at(i))
                .unwrap_or((part, ""));
            // An EMPTY address defaults to the cursor line, on either side
            // of the comma (`:,3d` = `.,3d`, `:2,d` = `2,.d` — 9.1 probes).
            // A LEADING `+n`/`-n` with no address bases at the CURSOR line
            // (vim's get_address: `:5,+1d` is lines 5..cursor+1 — the old
            // comment claimed previous-address, which would have invited a
            // "fix" of the correct code). Whitespace before the offset
            // (`:5 +2d`) is skipped like vim does.
            let value = match base_line(base_str.trim_end(), vim, ctx)? {
                Base::Line(base) => with_offset(base, off_str),
                // mid-range `%` = `1,$` collapsed to its LAST line: only the
                // final two addresses survive a longer chain anyway, so
                // `:1,%d` deletes the whole file exactly like vim
                Base::Unusable if base_str == "%" => last,
                Base::Unusable => {
                    let base = addresses.last().copied().unwrap_or(cursor_line);
                    with_offset(base, off_str)
                }
            };
            addresses.push(value.min(last));
        }
        match addresses.as_slice() {
            [] => Err("E16: Invalid range".to_owned()),
            [only] => Ok((Some((*only, *only)), 1usize, rest.trim_start())),
            _ => {
                let (a, b) = (
                    addresses[addresses.len() - 2],
                    addresses[addresses.len() - 1],
                );
                Ok((Some((a.min(b), a.max(b))), addresses.len(), rest.trim_start()))
            }
        }
    }

    /// `:{range}j[oin][!] [count]` — join the range's lines into one (single
    /// undo step). Without `!` the J separator logic applies (one space at
    /// the seam); with `!` the lines are concatenated verbatim (vim's `:j!` ≈
    /// `gJ`). A ONE-ADDRESS range joins with the NEXT line, so the bare `:j`
    /// works — but a range with TWO EQUAL addresses (`:2,2j`) does nothing
    /// (vim 9.1, `:h :j`: "If a [range] has equal start and end values, this
    /// command does nothing"). A COUNT re-anchors the range to count lines
    /// starting at its LAST line (`:2j 3` joins lines 2-4).
    fn ex_join(&mut self, ctx: &mut Ctx, (first, last): (usize, usize), args: &str, addr_count: usize) {
        let bang = args.starts_with('!');
        let count = args
            .trim_start_matches('!')
            .trim()
            .parse::<usize>()
            .unwrap_or(0);
        let last = last.min(ctx.buf.line_count().saturating_sub(1));
        let mut first = first.min(last);
        if count >= 1 {
            first = last;
        }
        let last = last
            .saturating_add(count.saturating_sub(1))
            .min(ctx.buf.line_count().saturating_sub(1));
        if addr_count >= 2 && first == last {
            // `:2,2j` — vim: nothing happens, not even a seam
            return;
        }
        let lines = last - first + 1;
        // a single-address range still joins one seam (`:5j` = join 5 and 6)
        let joins = if lines < 2 { 1 } else { lines - 1 };
        self.cursor.offset = ctx.buf.line_start(first);
        self.begin_edit();
        crate::ops::join_lines(self, ctx, joins + 1, bang);
        self.end_edit();
        // vim parks the cursor on the joined line's FIRST NON-BLANK after
        // `:j` (probes: `:1,2j` on ['    aaaa','bbbb'] → col 5, bare `:j` →
        // col 1) — the seam landing is normal-mode `J`'s behavior. The cursor
        // moves BEFORE `bump` so the changelist / `.` mark record the join
        // site, not wherever the command's cursor happened to be before.
        self.cursor.offset = ctx.buf.first_non_blank(first);
        self.cursor.desired_col = None;
        self.bump(ctx);
        ctx.host.changed();
        self.commit_change_record();
    }

    /// `:{range}sor[t][!] [flags]` — sort the range's lines. Flags (subset
    /// of vim's): `!` reverse, `i` ignore case, `u` dedupe AFTER sorting.
    /// Other vim flags (`n` numeric, `x`/`o`/`b`…) are ignored.
    fn ex_sort(&mut self, ctx: &mut Ctx, (first, last): (usize, usize), flags: &str) {
        let last = last.min(ctx.buf.line_count().saturating_sub(1));
        let first = first.min(last);
        let reverse = flags.contains('!');
        let ignore_case = flags.contains('i');
        let unique = flags.contains('u');
        if first == last {
            // a one-line range has nothing to reorder OR dedupe (`:sort u`
            // on a single line can't drop a consecutive duplicate either) —
            // and the replace below must not run: it would announce an undo
            // group for a no-op edit, burning one `u` on unchanged text
            return;
        }
        let mut lines: Vec<String> = (first..=last).map(|l| ctx.buf.line_content(l)).collect();
        let lower = |s: &str| {
            if ignore_case {
                s.to_lowercase()
            } else {
                s.to_owned()
            }
        };
        lines.sort_by_key(|s| lower(s));
        if unique && ignore_case {
            // vim folds the dedup when `i` is set (`%sort iu` on
            // [foo,FOO,bar] → [bar,foo], probe). The stable case-insensitive
            // sort puts the original-first variant ahead, so plain dedup_by
            // keeps exactly the survivor vim keeps.
            lines.dedup_by(|a, b| lower(a) == lower(b));
        } else if unique {
            lines.dedup();
        }
        if reverse {
            lines.reverse();
        }
        let start = ctx.buf.line_start(first);
        let end = ctx.buf.line_range(last).end;
        // the range includes the last line's terminating newline — the join
        // must give it back or `:%sort` shaves the buffer's final newline
        let had_trailing_newline = ctx.buf.slice(start..end).ends_with('\n');
        let mut new_text = lines.join("\n");
        if had_trailing_newline {
            new_text.push('\n');
        }
        self.begin_edit();
        self.edit_replace(ctx, start..end, &new_text);
        self.end_edit();
        // cursor BEFORE bump: the changelist / `.` mark record the sorted
        // range's first line, not the pre-command cursor
        self.cursor.offset = ctx.buf.first_non_blank(first);
        self.cursor.desired_col = None;
        self.bump(ctx);
        ctx.host.changed();
        self.commit_change_record();
    }

    /// Parse the `:d` / `:y` argument list: a lone numeric first argument is
    /// a COUNT (`:2y 3`), an alphabetic one names a REGISTER with an
    /// optional count after it (`:y a 2`). Special register chars work too —
    /// `:d _` must land in the black hole (not `"1`), `:d +` in the
    /// clipboard (9.1 semantics). Returns `(register, count)`, count 0 =
    /// absent.
    fn parse_reg_count(args: &str) -> (Option<char>, usize) {
        const SPECIALS: &str = "_+\".-:%#*";
        let mut register = None;
        let mut count = 0usize;
        let mut parts = args.split_whitespace();
        if let Some(head) = parts.next() {
            if let Some(c) = head.chars().next() {
                if c.is_ascii_digit() {
                    // a NON-numeric tail (`:1,2d 3x`) is garbage, not a
                    // count of 1: defaulting to 1 re-anchored the range at
                    // its last line and deleted the WRONG line set — vim
                    // reports E488 and deletes nothing. 0 = no count.
                    count = head.parse::<usize>().unwrap_or(0);
                } else if c.is_ascii_alphanumeric() || SPECIALS.contains(c) {
                    register = Some(c);
                    // the count may follow the register (`:y a 2`)
                    if let Some(n) = parts.next().and_then(|s| s.parse::<usize>().ok()) {
                        count = n;
                    }
                }
            }
        }
        (register, count)
    }

    /// `:{range}y[ank] [x] [count]` — yank the range's lines into a register
    /// (default: the unnamed/yank path like `yy`). The second argument is a
    /// COUNT of lines starting at the range's LAST line (`:2y a 3` = three
    /// lines into `"a`, `:1,2y 3` = lines 2-4, vim 9.1 probe — the engine
    /// used to extend the range instead); a lone numeric first argument is a
    /// count (`:2y 3`). The buffer is untouched.
    fn ex_yank_lines(&mut self, ctx: &mut Ctx, (first, last): (usize, usize), args: &str) {
        let (register, count) = Self::parse_reg_count(args);
        let (mut first, last) = (first, last);
        // any EXPLICIT count (even 1) re-anchors at the range's last line
        // (vim: `:1,2y 1` yanks just line 2)
        if count >= 1 {
            first = last;
        }
        let last = last
            .saturating_add(count.saturating_sub(1))
            .min(ctx.buf.line_count().saturating_sub(1));
        let first = first.min(last);
        let span = crate::ops::OpSpan {
            start: ctx.buf.line_start(first),
            end: ctx.buf.line_range(last).end,
            linewise: true,
        };
        crate::ops::yank_span(self, ctx, &span, register);
    }

    /// `:{range}d` — delete the lines of the range (single undo step),
    /// storing them into `register` when named (`:1,2d a`, vim 9.1), else
    /// the normal delete registers. Cursor to the first non-blank of the
    /// line that took their place.
    fn ex_delete_lines(
        &mut self,
        ctx: &mut Ctx,
        (first, last): (usize, usize),
        register: Option<char>,
    ) {
        let last = last.min(ctx.buf.line_count().saturating_sub(1));
        let start = ctx.buf.line_start(first);
        let end = ctx.buf.line_range(last).end;
        if start >= end {
            ctx.host.bell();
            return;
        }
        self.begin_edit();
        // delete_span (not the raw buffer write) routes the deleted lines
        // through the register file — the whole point of `:d a`
        crate::ops::delete_span(
            self,
            ctx,
            &crate::ops::OpSpan {
                start,
                end,
                linewise: true,
            },
            register,
        );
        // cursor BEFORE bump: the changelist / `.` mark record the line that
        // took the deleted lines' place, not the pre-command cursor
        let below = ctx.buf.line_count().saturating_sub(1);
        let line = first.min(below);
        self.cursor.offset = ctx.buf.first_non_blank(line);
        self.cursor.desired_col = None;
        self.bump(ctx);
        ctx.host.changed();
        self.commit_change_record();
    }

    /// `:set` with space-separated items: `name`, `noname`, `name!`,
    /// `name=value`, `name?` (report the current value on the status
    /// channel). Stops at the first unknown item (bell).
    fn ex_set(&mut self, ctx: &mut Ctx, args: &str) {
        if args.is_empty() {
            // vim lists all options here — report ours line by line
            for line in self.options.describe_all() {
                ctx.host.status_message(&line);
            }
            return;
        }
        let mut applied = false;
        let mut search_rules_changed = false;
        for arg in args.split_whitespace() {
            // a `"` starts a comment to end of line (vim: `:set ts=4 " note`
            // sets quietly); stop parsing so the words after it don't error
            if arg.starts_with('"') {
                break;
            }
            // `name?` is a query: report and move on, values untouched
            if let Some(name) = arg.strip_suffix('?') {
                match self.options.describe(name) {
                    Some(text) => ctx.host.status_message(&text),
                    None => {
                        ctx.host.bell();
                        return;
                    }
                }
                continue;
            }
            let search_rule = arg.trim_end_matches(['!', '?', '='])
                .split('=')
                .next()
                .map(|n| {
                    let n = n.strip_prefix("no").unwrap_or(n);
                    matches!(n, "ic" | "ignorecase" | "isc" | "smartcase" | "hls" | "hlsearch")
                })
                .unwrap_or(false);
            let ok = if let Some(name) = arg.strip_suffix('!') {
                match self.options.bool_option(name) {
                    Some(current) => self.options.set_boolean(name, !current),
                    None => false,
                }
            } else if let Some(name) = arg.strip_prefix("no") {
                self.options.set_boolean(name, false)
            } else if let Some((name, value)) = arg.split_once('=') {
                self.options.set_value(name, value)
            } else {
                self.options.set_boolean(arg, true)
            };
            if !ok {
                ctx.host.bell();
                return;
            }
            applied = true;
            if search_rule {
                search_rules_changed = true;
            }
        }
        // search options (ic/isd/…) change how the next scan must run: drop
        // the cached match list so `n` re-scans under the new options — and
        // re-publish the LIVE highlights under the new rules (vim: `:set
        // noic` immediately re-renders the hlsearch match set)
        if applied {
            self.search.matches_generation = None;
        }
        if search_rules_changed {
            if let Some(pattern) = self.search.pattern.clone() {
                let matches = search::all_matches(self, ctx.buf, &pattern);
                self.search.last_matches = matches.clone();
                self.search.matches_generation = Some(self.edit_generation);
                if self.options.hlsearch {
                    ctx.host.set_search_highlights(&matches, None);
                } else {
                    ctx.host.set_search_highlights(&[], None);
                }
            }
        }
    }

    /// `:[%]s{sep}pattern{sep}replacement{sep}[flags]` — over the current
    /// line, or every line with `%`. `g` replaces all matches per line
    /// (default: first match per line). Replacement follows Rust regex
    /// expansion (`$1`, documented divergence from vim's `\1`). The bare
    /// `:s` repeats the last substitute (vim; same on the current line).
    /// `:[range]s[ubstitute]` — the full spelling normalizes to the `s`
    /// spelling (vim's one-word alias). Returns false when `line` is not a
    /// substitute command at all.
    fn ex_substitute(&mut self, ctx: &mut Ctx, line: &str, range: (usize, usize)) -> bool {
        // `s` + rest below; `substitute` and every unambiguous prefix
        // (`:su`, `:sub`, …) are accepted spellings of `s` (vim's
        // abbreviation rule)
        const SUBST_ABBREVS: &[&str] = &[
            "substitute", "substitut", "substitu", "substit", "substi", "subst", "subs", "sub",
            "su",
        ];
        let normalized;
        let line = if let Some((rest, _spelling)) = SUBST_ABBREVS.iter().find_map(|sp| {
            line.strip_prefix(sp)
                .filter(|r| r.is_empty() || !r.chars().next().is_some_and(char::is_alphanumeric))
                .map(|r| (r, sp))
        }) {
            normalized = format!("s{rest}");
            &normalized
        } else {
            line
        };
        let Some(after_s) = line.strip_prefix('s') else {
            return false;
        };
        if after_s.is_empty() {
            // bare `:s` = repeat the last substitute WITHOUT the previous
            // flags (vim 9.1; same as `&`), on the GIVEN range — the default
            // is the current line, but `:3,4s` after `:2,4s/a/b/` re-runs it
            // on lines 3-4 (9.1 probe; the old replay dropped the range and
            // always landed on the cursor line)
            return match self.cmdline.last_substitute.clone() {
                Some(last) => {
                    let last = Self::substitute_without_flags(&last);
                    self.ex_substitute(ctx, &last, range);
                    true
                }
                None => {
                    // vim: E33 "No previous substitute regular expression"
                    ctx.host
                        .status_message("E33: No previous substitute regular expression");
                    ctx.host.bell();
                    true
                }
            };
        }
        let Some(sep) = after_s.chars().next() else {
            ctx.host.bell();
            return true;
        };
        if sep.is_alphanumeric() {
            // words starting with `s` that are not commands of this engine
            // (`:sort` is handled before this parser) get the standard
            // unknown-command report instead of a bare bell
            return false;
        }
        let mut parts = after_s[sep.len_utf8()..].split(sep);
        let Some(pattern) = parts.next() else {
            ctx.host.bell();
            return true;
        };
        // a MISSING replacement (`:s/foo`, nothing after the pattern) is an
        // EMPTY one — vim deletes the match (9.1 probe), it does not reuse
        // the last command's replacement
        let replacement = parts.next().unwrap_or("");
        let flags = parts.next().unwrap_or("");
        if parts.next().is_some() {
            ctx.host.bell();
            return true;
        }

        // an empty pattern reuses the last search, like vim
        let pattern = if pattern.is_empty() {
            match self.search.pattern.clone() {
                Some(p) => p,
                None => {
                    // same message channel as the `n` miss: a bare bell hid
                    // WHY nothing happened
                    ctx.host
                        .status_message("E35: No previous regular expression");
                    ctx.host.bell();
                    return true;
                }
            }
        } else {
            pattern.to_owned()
        };
        let Some(mut builder) = search::compile(self, &pattern) else {
            ctx.host.bell();
            return true;
        };
        // `i` forces case-insensitive for this substitution, `I` forces
        // case-sensitive (overriding ignorecase/smartcase, like vim)
        if flags.contains('i') {
            builder.case_insensitive(true);
        } else if flags.contains('I') {
            builder.case_insensitive(false);
        }
        let Ok(re) = builder.build() else {
            ctx.host.bell();
            return true;
        };
        // remember for `&` / bare `:s` — only after the pattern COMPILED, or
        // `&` would re-run a command that never worked (and even on a later
        // E486, vim retries the same command and reports the same miss)
        self.cmdline.last_substitute = Some(line.to_owned());
        let global = flags.contains('g');

        let (first_line, last_line) = range;

        let range_start = ctx.buf.line_start(first_line);
        let range_end = ctx.buf.line_end(last_line);
        let mut joined: Vec<String> = Vec::new();
        let mut total = 0usize;
        // the cursor lands on the LAST SUBSTITUTED line, at its first
        // non-blank (vim 9.1 probes: `%s` over lines where the tail has no
        // matches ends on the last line that changed). Line numbers are
        // stable — replacements can never introduce newlines.
        let mut last_sub_line: Option<usize> = None;
        for line_no in first_line..=last_line {
            let ls = ctx.buf.line_start(line_no);
            let le = ctx.buf.line_end(line_no);
            let text = ctx.buf.slice(ls..le);
            let mut hits = 0usize;
            // one counting replacer serves both modes: `replace` stops after
            // the first match, `replace_all` runs to the end of the line.
            // Zero-width matches (`` :s/^/x/ ``, `` :s/$/x/ ``, `a*` on
            // "bbb") count and expand like any other — vim substitutes at
            // them (row-prefix/suffix insertion idiom); treating them as
            // no-ops made those commands falsely report E486. The regex
            // crate advances past empty matches itself, so replace_all
            // cannot loop on them.
            let count_replacements = |caps: &regex::Captures| -> String {
                hits += 1;
                let mut out = String::new();
                caps.expand(replacement, &mut out);
                out
            };
            let replaced = if global {
                re.replace_all(&text, count_replacements)
            } else {
                re.replace(&text, count_replacements)
            }
            .to_string();
            if hits > 0 {
                total += hits;
                last_sub_line = Some(line_no);
            }
            joined.push(replaced);
        }

        if total == 0 {
            ctx.host
                .status_message(&format!("E486: Pattern not found: {pattern}"));
            ctx.host.bell();
            return true;
        }

        self.begin_edit();
        let new_text = joined.join("\n");
        self.edit_replace(ctx, range_start..range_end, &new_text);
        self.end_edit();
        // the substitute pattern enters the search state (vim: after
        // `:s/x/Y/`, `@/` is `x`, `n` finds the next one and hlsearch
        // highlights it — 9.1 probes; the engine kept the OLD pattern).
        // After the edit, so the published matches sit on the new text.
        search::set_pattern(self, ctx, pattern.clone(), true);
        // cursor BEFORE the bump: the changelist / `.` mark must record the
        // LAST SUBSTITUTED line (vim probe: `:4s` then g; lands on line 4),
        // not wherever the cursor happened to sit before the command
        if let Some(line_no) = last_sub_line {
            let line = line_no.min(ctx.buf.line_count() - 1);
            self.cursor.offset =
                crate::buffer::clamp_to_line_end(ctx.buf, ctx.buf.first_non_blank(line));
            self.cursor.desired_col = None;
        }
        self.bump(ctx);
        ctx.host.status_message(&format!("{total} substitutions"));
        // `.` repeats the substitution at the cursor's line
        self.commit_change_record();
        true
    }
}
