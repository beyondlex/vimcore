//! The host-command Ex registry (`:h user-commands` in spirit).
//!
//! The engine keeps the whole `:` state machine (prompt editing, ranges,
//! history, `|` splitting); hosts register their PRODUCT commands
//! (`:hunk-next`, `:config`, `:w`-with-a-twist…) into one registry and the
//! engine dispatches them like built-ins. Dispatch order:
//!
//! 1. the leading `[range]` is parsed by the engine and handed over
//!    resolved (the handler never parses addresses);
//! 2. host commands are matched BEFORE the built-in table — an explicit
//!    registration is the host saying "this one is mine";
//! 3. a miss falls through to the built-in spellings and finally E492.
//!
//! Command-name matching follows vim's boundary rule, generalized: a
//! registration matches when it equals the first whitespace-delimited word
//! (`:w file`), or when it is a prefix of the line and the NEXT character
//! is not a name character (`s` matches `s/a/b/` but not `save-all` — the
//! latter stays with the built-ins). The longest match wins.

use std::rc::Rc;

use crate::state::{Ctx, VimState};

/// One host-registered Ex command invocation, as the handler sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExInvocation {
    /// The registered name that matched (not the typed alias).
    pub name: String,
    /// The trailing `!` (`:q!`, `:e!`). The engine strips it; a host that
    /// cares about force-semantics reads it here.
    pub bang: bool,
    /// Everything after the command name, leading whitespace trimmed. For a
    /// glued match (`:s/a/b/` against a registration named `s`) this keeps
    /// its leading separator (`/a/b/`) — the handler parses its own syntax.
    pub arg: String,
    /// The resolved `[range]` as 0-based INCLUSIVE line bounds, or `None`
    /// when none was typed (commands apply their own default).
    pub range: Option<(usize, usize)>,
    /// How many addresses the user typed (`:5j` = 1, `:2,5j` = 2).
    pub addr_count: usize,
}

impl ExInvocation {
    /// The range clamped to the buffer, or the command's default when none
    /// was typed: a single line = the cursor line, two = the whole buffer.
    pub fn resolved_range(
        &self,
        vim: &VimState,
        buf: &dyn crate::buffer::VimBuffer,
    ) -> (usize, usize) {
        match self.range {
            Some(r) => {
                let last = buf.line_count().saturating_sub(1);
                (r.0.min(last), r.1.min(last))
            }
            None => {
                let cursor = buf.offset_to_line(vim.cursor.offset);
                (cursor, cursor)
            }
        }
    }
}

/// A host command handler. Receives the engine and the context mutably —
/// buffer edits should go through the engine's own funnels so undo groups,
/// marks and `.` replay stay coherent. Return `false` to fail the command:
/// the engine bells and aborts a surrounding replay, like a built-in error.
pub type ExHandler = Rc<dyn Fn(&mut VimState, &mut Ctx, &ExInvocation) -> bool>;

/// Argument-completion candidate source: given the argument text typed so
/// far, return the candidates (the widget inserts the common prefix).
pub type ExCompletion = Rc<dyn Fn(&str) -> Vec<String>>;

/// One registered command.
pub struct ExEntry {
    pub name: String,
    pub aliases: Vec<String>,
    pub completion: Option<ExCompletion>,
    pub handler: ExHandler,
}

/// The registry. Small by construction (a host registers a handful of
/// product commands), so lookup is a linear scan.
#[derive(Default)]
pub struct ExRegistry {
    entries: Vec<ExEntry>,
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

impl ExRegistry {
    /// Find the entry for a typed command line (the part after the range).
    /// Returns the entry, the matched length and whether a `!` follows.
    pub(crate) fn lookup(&self, line: &str) -> Option<(&ExEntry, usize, bool)> {
        let word_end = line.find(char::is_whitespace).unwrap_or(line.len());
        let word = &line[..word_end];
        let mut best: Option<(usize, &ExEntry, usize, bool)> = None;
        for entry in &self.entries {
            let spellings = std::iter::once(&entry.name).chain(entry.aliases.iter());
            for spelling in spellings {
                // 1) whole-word: `:w file` / `:q!`
                let (word_match, rest_of_word) = if let Some(stem) = word.strip_suffix('!') {
                    (stem == spelling.as_str(), "!")
                } else {
                    (word == spelling.as_str(), "")
                };
                if word_match {
                    let bang = rest_of_word == "!";
                    let taken = spelling.len() + if bang { 1 } else { 0 };
                    if best.is_none_or(|(len, _, _, _)| spelling.len() > len) {
                        best = Some((spelling.len(), entry, taken, bang));
                    }
                    continue;
                }
                // 2) glued: `s/a/b/` — only at a non-name-character boundary
                // (`w` must not swallow `write`)
                if line.starts_with(spelling.as_str()) {
                    let after = &line[spelling.len()..];
                    let next_ok = after.chars().next().is_some_and(|c| !is_name_char(c));
                    if next_ok {
                        // `cmd!` (nothing after the bang) parses as bang;
                        // anything else keeps the bang in its argument text
                        let (taken, bang) = if after == "!" {
                            (spelling.len() + 1, true)
                        } else {
                            (spelling.len(), false)
                        };
                        if best.is_none_or(|(len, _, _, _)| spelling.len() > len) {
                            best = Some((spelling.len(), entry, taken, bang));
                        }
                    }
                }
            }
        }
        best.map(|(_, entry, taken, bang)| (entry, taken, bang))
    }

    /// Completion candidates for a `:` line typed so far (the raw line
    /// INCLUDING any range prefix — the command word is the first
    /// whitespace token). A whole-word command match followed by typed
    /// argument text routes to the entry's argument completion; anything
    /// else completes command names/aliases by prefix.
    pub fn completions(&self, line: &str) -> Vec<String> {
        let trimmed = line.trim_start();
        let word_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let word = &trimmed[..word_end];
        // the whitespace the word was followed by (evidence the user moved
        // on to the argument): `config <Tab>` completes arguments,
        // `conf<Tab>` completes names
        let had_space = word_end < trimmed.len();
        let rest = trimmed[word_end..].trim_start();
        if had_space {
            if let Some((entry, _, _)) = self.lookup(&trimmed[..word_end]) {
                return entry
                    .completion
                    .as_ref()
                    .map(|f| f(rest))
                    .unwrap_or_default();
            }
        }
        let mut names: Vec<String> = self
            .entries
            .iter()
            .flat_map(|e| std::iter::once(e.name.clone()).chain(e.aliases.iter().cloned()))
            .filter(|n| n.starts_with(word))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ExEntry> {
        self.entries.iter()
    }
}

impl VimState {
    /// Register a host Ex command. Re-registering an existing name replaces
    /// it. Handlers run BEFORE the built-in command table; see
    /// [`ExRegistry`] for the matching rules.
    pub fn register_ex(
        &mut self,
        name: &str,
        aliases: &[&str],
        completion: Option<ExCompletion>,
        handler: ExHandler,
    ) {
        let owned: Vec<String> = aliases.iter().map(|s| s.to_string()).collect();
        let entry = ExEntry {
            name: name.to_string(),
            aliases: owned,
            completion,
            handler,
        };
        if let Some(slot) = self
            .ex_registry
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name))
        {
            self.ex_registry.entries[slot] = entry;
        } else {
            self.ex_registry.entries.push(entry);
        }
    }

    /// Completion candidates for the `:` line typed so far (host commands
    /// only — built-ins are the widget's own list). See
    /// [`ExRegistry::completions`].
    pub fn ex_completions(&self, line: &str) -> Vec<String> {
        self.ex_registry.completions(line)
    }
}
