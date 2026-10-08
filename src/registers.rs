//! Registers: `"a`-`"z`, the unnamed register, the yank register `"0`,
//! the numbered delete ring `"1`-`"9`, the small-delete register `"-`,
//! the clipboard register `"+` and the blackhole `"_`.

use crate::host::VimHost;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterKind {
    Charwise,
    Linewise,
    /// Reserved for visual-block puts.
    Blockwise,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub kind: RegisterKind,
}

/// The register file. `"x` reads/writes register `x`; the unnamed register is
/// the register named `"` itself and mirrors every write (vim semantics).
#[derive(Clone, Debug, Default)]
pub struct Registers {
    /// Bumped on EVERY register write (audit D4): the macro player compares
    /// it against its per-register sync mark to notice `"ayy`-style
    /// overwrites without losing unserializable recorded keys.
    pub(crate) write_gen: u64,
    named: HashMap<char, Register>,
    /// Last written register for unnamed access.
    last: Option<Register>,
}

pub const UNNAMED: char = '"';
pub const BLACKHOLE: char = '_';
pub const CLIPBOARD: char = '+';
pub const SMALL_DELETE: char = '-';
pub const YANK: char = '0';
/// `".` — the last inserted text (audit D1)
pub const LAST_INSERT: char = '.';
/// `":` — the last Ex command line (audit D3)
pub const LAST_COMMAND: char = ':';
/// `"=` — the expression register result (audit C9). The prompt evaluates a
/// constant arithmetic expression; the decimal result pastes charwise.
pub const EXPRESSION: char = '=';

/// Registers the `"{reg}` prefix accepts for READING but refuses as a
/// yank/delete TARGET (audit C7 — `"%dd`/`".dd`/`":dd`/`"/dd` all error
/// E354 in vim and leave the buffer untouched; the oracle also shows an
/// intervening motion like `".j` ends the prefix so a LATER `dd` deletes
/// normally, which the engine gets from `end_command` clearing the
/// register after every command). `=` has no engine evaluator yet and is
/// paste-only, so writes refuse too.
pub fn is_write_valid(name: char) -> bool {
    !matches!(name, LAST_INSERT | '%' | LAST_COMMAND | '/' | '=')
}

/// `text.repeat(count)` under a hard byte ceiling. Counts arrive unvalidated
/// from the keyboard and `99999999p` must clamp the pasted volume instead of
/// multiplying the register into an OOM (vim dies the same way; the engine
/// refuses to). Pairs with [`crate::ops::clamped_repeat_count`].
pub(crate) fn clamped_repeat(text: &str, count: usize) -> String {
    const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;
    let count = count.min((MAX_PASTE_BYTES / text.len().max(1)).max(1));
    text.repeat(count)
}

/// Push an explicit `"+` store to the host side (`VimHost::clipboard_write`).
/// The register file has no host access, so the yank/delete funnels call this
/// right after storing; without it `"+yy` wrote a named slot that
/// [`Registers::get_for_paste`] (which reads the HOST clipboard) never saw,
/// and `"+p` pasted stale external content instead.
pub fn sync_clipboard_host(host: &mut dyn VimHost, name: Option<char>, text: &str) {
    if name == Some(CLIPBOARD) {
        host.clipboard_write(text);
    }
}

impl Registers {
    pub fn get(&self, name: char) -> Option<&Register> {
        match name {
            UNNAMED => self.last.as_ref(),
            BLACKHOLE => None,
            LAST_INSERT => self.named.get(&LAST_INSERT),
            LAST_COMMAND => self.named.get(&LAST_COMMAND),
            EXPRESSION => self.named.get(&EXPRESSION),
            // `"A` reads register `a` — the uppercase spelling is the append
            // form, not a separate slot
            c if c.is_ascii_uppercase() => self.named.get(&c.to_ascii_lowercase()),
            _ => self.named.get(&name),
        }
    }

    /// `(name, register)` pairs for `:registers` listings: the unnamed
    /// register first (when set), then the named ones sorted by name.
    pub fn items(&self) -> Vec<(char, Register)> {
        let mut out: Vec<(char, Register)> =
            self.named.iter().map(|(c, r)| (*c, r.clone())).collect();
        out.sort_by_key(|(c, _)| *c);
        if let Some(r) = &self.last {
            out.insert(0, (UNNAMED, r.clone()));
        }
        out
    }

    /// Register contents for a paste, resolving the clipboard register (and
    /// the `clipboard=unnamed` behavior through the host).
    pub fn get_for_paste(&self, name: char, host: &dyn VimHost) -> Option<Register> {
        match name {
            // `"%` is the HOST filename, read live (audit D2)
            '%' => {
                let name = host.buffer_name();
                (!name.is_empty()).then(|| Register {
                    text: name.to_owned(),
                    kind: RegisterKind::Charwise,
                })
            }
            CLIPBOARD => host
                .clipboard_read()
                .filter(|text| !text.is_empty())
                .map(|text| Register {
                    kind: if text.contains('\n') {
                        RegisterKind::Linewise
                    } else {
                        RegisterKind::Charwise
                    },
                    text,
                })
                // a host whose clipboard is a no-op (or was empty at read
                // time) still gets the `"+yy` → `"+p` roundtrip through the
                // named-slot mirror that [`crate::ops`] keeps in sync — the
                // old code read ONLY the host side, so a yank into `+`
                // vanished into a slot no paste could reach
                .or_else(|| self.get(CLIPBOARD).cloned()),
            _ => self.get(name).cloned(),
        }
    }

    /// Write to a register. `name` of `UNNAMED` writes to the unnamed slot
    /// only. Every write also updates the unnamed mirror (except the
    /// blackhole), matching vim. Uppercase `A`-`Z` APPEND to the lowercase
    /// register instead of writing a separate slot (vim's `"Ayy`).
    /// Store the last search pattern into `"/` (`@/`): a plain named slot —
    /// it must never touch the unnamed register, unlike [`Self::store`].
    pub fn store_search(&mut self, pattern: String) {
        self.named.insert(
            '/',
            Register {
                text: pattern,
                kind: RegisterKind::Charwise,
            },
        );
    }

    /// `".` — record the text the last insert session typed (audit D1;
    /// mirrors the search slot: no unnamed write).
    pub fn store_last_insert(&mut self, text: String) {
        self.named.insert(
            LAST_INSERT,
            Register {
                text,
                kind: RegisterKind::Charwise,
            },
        );
    }

    /// `":` — record the last Ex command line (audit D3).
    pub fn store_last_command(&mut self, cmd: String) {
        self.named.insert(
            LAST_COMMAND,
            Register {
                text: cmd,
                kind: RegisterKind::Charwise,
            },
        );
    }

    /// A letter register's raw text, if set (audit D4 — the macro/register
    /// unification reads it).
    pub fn named_text(&self, name: char) -> Option<String> {
        self.named.get(&name).map(|r| r.text.clone())
    }

    /// Write a letter register directly WITHOUT touching the unnamed/yank
    /// bookkeeping (the macro recorder's register mirror, audit D4b).
    pub fn store_named_plain(&mut self, name: char, text: String) {
        self.write_gen += 1;
        self.named.insert(
            name,
            Register {
                text,
                kind: RegisterKind::Charwise,
            },
        );
    }

    /// `"=` — store the expression prompt's evaluated result (audit C9).
    /// Read-only through the write funnels like the other specials.
    pub fn store_expression(&mut self, text: String) {
        self.named.insert(
            EXPRESSION,
            Register {
                text,
                kind: RegisterKind::Charwise,
            },
        );
    }

    pub fn store(&mut self, name: char, text: String, kind: RegisterKind) {
        self.store_ext(name, text, kind, false);
    }

    /// `unnamed_new_piece`: a DELETE append (`"Add`) points the unnamed
    /// register at the NEW piece while a YANK append (`"Ayy`) points it at
    /// the MERGED register — vim is deliberately asymmetric here (9.1
    /// oracle: `p` after `"Ayy` pastes both lines, after `"Add` only the
    /// just-deleted text — audit C8).
    pub fn store_ext(&mut self, name: char, text: String, kind: RegisterKind, unnamed_new_piece: bool) {
        self.write_gen += 1;
        let register = Register {
            text: text.clone(),
            kind,
        };
        if name == BLACKHOLE {
            return;
        }
        if name == UNNAMED {
            self.last = Some(register);
            return;
        }
        if name.is_ascii_uppercase() {
            self.append_to_named(name, text, kind, unnamed_new_piece);
            return;
        }
        self.named.insert(name, register.clone());
        self.last = Some(register);
    }

    /// Uppercase-register append: concatenate onto the lowercase register.
    /// Either side linewise makes the result linewise (text re-joined on
    /// line boundaries); same-kind appends concatenate byte for byte.
    ///
    /// The UNNAMED register points at the NEW piece, not the merged result
    /// (vim: `"Add` then `p` pastes only the just-deleted text — audit C8;
    /// routing through [`Self::store`] re-pointed `last` at the merge).
    fn append_to_named(
        &mut self,
        name: char,
        text: String,
        kind: RegisterKind,
        unnamed_new_piece: bool,
    ) {
        let lower = name.to_ascii_lowercase();
        let (merged, merged_kind) = match self.named.get(&lower) {
            Some(existing) => {
                let mut kind = kind;
                let mut text = text.clone();
                if existing.kind == RegisterKind::Linewise || kind == RegisterKind::Linewise {
                    kind = RegisterKind::Linewise;
                    if !existing.text.ends_with('\n') {
                        text.insert(0, '\n');
                    }
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                }
                (format!("{}{}", existing.text, text), kind)
            }
            None => (text.clone(), kind),
        };
        self.named.insert(
            lower,
            Register {
                text: merged.clone(),
                kind: merged_kind,
            },
        );
        self.last = Some(if unnamed_new_piece {
            Register { text, kind }
        } else {
            Register {
                text: merged,
                kind: merged_kind,
            }
        });
    }

    /// Yank semantics: explicit register, else `"0` + unnamed.
    pub fn store_yank(&mut self, explicit: Option<char>, text: String, kind: RegisterKind) {
        match explicit {
            // `"_yy` writes NOTHING: vim keeps both the blackhole slot empty
            // and the unnamed register untouched (9.1 probe: `"_yy` then `p`
            // pastes nothing; `:h quote_`). The old fall-through re-pointed
            // `last` at the yanked text anyway, so `p` resurrected it.
            Some(BLACKHOLE) => {}
            Some(name) if name != UNNAMED => {
                self.store(name, text, kind);
                // an uppercase append re-points `last` at the MERGED register
                // inside `store`; lowercase named stores do the same — no
                // separate `last` write is needed or wanted here
            }
            _ => self.store(YANK, text, kind),
        }
    }

    /// Delete semantics: explicit register, else the numbered ring for
    /// multi-line deletes and `"-` for small deletes.
    pub fn store_delete(&mut self, explicit: Option<char>, text: String, kind: RegisterKind) {
        match explicit {
            Some(name) => self.store_ext(name, text, kind, true),
            None => {
                let multi_line = text.contains('\n');
                if kind == RegisterKind::Linewise || multi_line {
                    // shift 1..8 -> 2..9, then record into "1
                    for i in (2..=9usize).rev() {
                        let from = char::from(b'0' + (i - 1) as u8);
                        let to = char::from(b'0' + i as u8);
                        if let Some(r) = self.named.get(&from) {
                            let r = r.clone();
                            self.named.insert(to, r);
                        }
                    }
                    self.store('1', text, kind);
                } else {
                    self.store(SMALL_DELETE, text, kind);
                }
            }
        }
    }
}
