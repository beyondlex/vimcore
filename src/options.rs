//! The option subset the engine reads.
//!
//! Options are plain data on the engine (vim's "global" scope only for v1).
//! Hosts can expose `:set`-style UI later; the engine only needs the values.

/// Boolean options and their vim aliases — the single source of truth for
/// `set_boolean`, `bool_option` and the `no<name>` prefix check in the rc
/// parser (`set nonumber` must negate `number`, not parse as `Off("mber")`).
const BOOL_TABLE: &[(&str, &[&str])] = &[
    ("number", &["nu"]),
    ("relativenumber", &["rnu"]),
    ("expandtab", &["et"]),
    ("autoindent", &["ai"]),
    ("ignorecase", &["ic"]),
    ("smartcase", &["scs"]),
    ("hlsearch", &["hls"]),
    ("incsearch", &["is"]),
    ("showmode", &["smd"]),
    ("showcmd", &["sc"]),
    ("lisp", &[]),
];

/// Numeric `name=value` options and their aliases.
const VALUE_TABLE: &[(&str, &[&str])] = &[
    ("tabstop", &["ts"]),
    ("shiftwidth", &["sw"]),
    ("textwidth", &["tw"]),
    ("scrolloff", &["so"]),
];

/// Resolve `name` (canonical or alias) to the canonical boolean option name.
fn canonical_bool(name: &str) -> Option<&'static str> {
    BOOL_TABLE
        .iter()
        .find(|(canon, aliases)| *name == **canon || aliases.contains(&name))
        .map(|(canon, _)| *canon)
}

/// Resolve `name` (canonical or alias) to the canonical numeric option name.
fn canonical_value(name: &str) -> Option<&'static str> {
    VALUE_TABLE
        .iter()
        .find(|(canon, aliases)| *name == **canon || aliases.contains(&name))
        .map(|(canon, _)| *canon)
}

/// True if `name` (canonical or alias) names a boolean option. The rc parser
/// consults this before treating a leading `no` as negation.
pub fn is_bool_option(name: &str) -> bool {
    canonical_bool(name).is_some()
}

/// Defaults for `:set {name}&` — mirrors [`Options::default`]. Keep the two
/// in sync (a drift makes `ts&` reset to a non-default value).
const VALUE_DEFAULTS: &[(&str, usize)] = &[
    ("tabstop", 4),
    ("shiftwidth", 4),
    ("textwidth", 78),
    ("scrolloff", 4),
];

#[derive(Clone, Debug)]
pub struct Options {
    pub number: bool,
    pub relativenumber: bool,
    pub scrolloff: usize,
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub textwidth: usize,
    pub expandtab: bool,
    pub autoindent: bool,
    pub ignorecase: bool,
    pub smartcase: bool,
    pub hlsearch: bool,
    pub incsearch: bool,
    pub showmode: bool,
    pub showcmd: bool,
    /// `'lisp'` — the built-in indent rule `=` uses without a filetype
    pub lisp: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            number: false,
            relativenumber: false,
            scrolloff: 4,
            tabstop: 4,
            shiftwidth: 4,
            textwidth: 78,
            expandtab: true,
            autoindent: false,
            ignorecase: true,
            smartcase: true,
            hlsearch: true,
            incsearch: true,
            showmode: true,
            lisp: false,
            showcmd: true,
        }
    }
}

impl Options {
    /// Case sensitivity for a query, honoring `ignorecase` + `smartcase`.
    pub fn case_insensitive_for(&self, query: &str) -> bool {
        self.ignorecase && !(self.smartcase && query.chars().any(|c| c.is_uppercase()))
    }

    /// `:set <name>` / `:set no<name>` support for boolean options.
    pub fn set_boolean(&mut self, name: &str, value: bool) -> bool {
        let Some(canon) = canonical_bool(name) else {
            return false;
        };
        match canon {
            "number" => self.number = value,
            "relativenumber" => self.relativenumber = value,
            "expandtab" => self.expandtab = value,
            "autoindent" => self.autoindent = value,
            "ignorecase" => self.ignorecase = value,
            "smartcase" => self.smartcase = value,
            "hlsearch" => self.hlsearch = value,
            "incsearch" => self.incsearch = value,
            "showmode" => self.showmode = value,
            "showcmd" => self.showcmd = value,
            "lisp" => self.lisp = value,
            _ => unreachable!("BOOL_TABLE and this match are out of sync"),
        }
        true
    }

    /// Current value of a boolean option (`:set name!` toggling).
    pub fn bool_option(&self, name: &str) -> Option<bool> {
        match canonical_bool(name)? {
            "number" => Some(self.number),
            "relativenumber" => Some(self.relativenumber),
            "expandtab" => Some(self.expandtab),
            "autoindent" => Some(self.autoindent),
            "ignorecase" => Some(self.ignorecase),
            "smartcase" => Some(self.smartcase),
            "hlsearch" => Some(self.hlsearch),
            "incsearch" => Some(self.incsearch),
            "showmode" => Some(self.showmode),
            "showcmd" => Some(self.showcmd),
            "lisp" => Some(self.lisp),
            _ => unreachable!("BOOL_TABLE and this match are out of sync"),
        }
    }

    /// `:set name=value` for numeric options. Values are capped: tabstop and
    /// shiftwidth feed `" ".repeat(n)`-shaped indent synthesis (insert Tab,
    /// `>>`), and a typo like `:set sw=99999999999` must not hand the host a
    /// gigabyte allocation. 1e6 columns is far past any real use; vim caps
    /// these options at 2^31-1 instead (documented divergence).
    ///
    /// Errors carry vim's message: `ts=0` is E487 "Argument must be
    /// positive" (audit5 A-13 — the engine silently accepted tabstop=0 and
    /// every column computation then leaned on `.max(1)`), and a tabstop
    /// past its ceiling is E474 "Invalid argument" instead of a silent clamp
    /// to another value (audit5 A-14). `sw=0`/`tw=0` stay legal like vim.
    ///
    /// Parsing follows vim's `do_set` grammar exactly: an optional leading
    /// `-` then digits, with the WHOLE string consumed — so a leading `+`
    /// (`:set ts=+4`) or any other stray byte is E521, not a silently
    /// accepted value (the old `parse::<usize>` read `+4` as `4`). A NEGATIVE
    /// value is E487 "must be positive" for every one of these options
    /// (9.1 probes: `:set sw=-1`/`tw=-1`/`so=-1` all E487, and `-0` counts as
    /// 0, so `:set ts=-0` E487s while `:set sw=-0` is accepted). The message
    /// quotes the name AS TYPED (alias or canonical), like vim
    /// (`:set tabstop=-1` → "tabstop=-1", `:set ts=-1` → "ts=-1").
    pub fn set_value(&mut self, name: &str, value: &str) -> Result<(), String> {
        const MAX_OPTION_VALUE: usize = 1_000_000;
        // vim's real ceiling for 'tabstop' is 9999 (E474 past it: 9.1 probe
        // `:set ts=10000`); the other numeric options accept very large
        // values, so the engine keeps its own 1e6 safety cap for them
        // (documented divergence — no allocation rides on those).
        const MAX_TABSTOP: usize = 9999;
        let Some(canon) = canonical_value(name) else {
            return Err(format!("E518: Unknown option: {name}"));
        };
        // optional sign then digits, whole string: `+4`/`4x`/`4.5`/`` → E521
        let digits = value.strip_prefix('-').unwrap_or(value);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("E521: Number required after =: {name}={value}"));
        }
        // saturate an overflowing digit string on the correct SIDE of zero:
        // vim reports E487 for a huge negative and E474 for a huge positive
        let n: i64 = if value.starts_with('-') {
            value.parse::<i64>().unwrap_or(i64::MIN)
        } else {
            value.parse::<i64>().unwrap_or(i64::MAX)
        };
        if n < 0 {
            return Err(format!("E487: Argument must be positive: {name}={value}"));
        }
        let v = n as usize;
        if canon == "tabstop" {
            if v == 0 {
                return Err(format!("E487: Argument must be positive: {name}={value}"));
            }
            if v > MAX_TABSTOP {
                return Err(format!("E474: Invalid argument: {name}={value}"));
            }
        }
        let v = v.min(MAX_OPTION_VALUE);
        match canon {
            "tabstop" => self.tabstop = v,
            "shiftwidth" => self.shiftwidth = v,
            "textwidth" => self.textwidth = v,
            "scrolloff" => self.scrolloff = v,
            _ => unreachable!("VALUE_TABLE and this match are out of sync"),
        }
        Ok(())
    }

    /// True when `name` (canonical or alias) names a numeric option —
    /// `:set ts` (bare, no `=`) must QUERY it, not try a boolean set.
    pub fn is_value_option(&self, name: &str) -> bool {
        canonical_value(name).is_some()
    }

    /// `:set {name}&` — reset a numeric option to its default.
    /// False when the name is unknown.
    pub fn reset_value(&mut self, name: &str) -> bool {
        let Some(canon) = canonical_value(name) else {
            return false;
        };
        let Some((_, default)) = VALUE_DEFAULTS.iter().find(|(n, _)| *n == canon) else {
            return false;
        };
        match canon {
            "tabstop" => self.tabstop = *default,
            "shiftwidth" => self.shiftwidth = *default,
            "textwidth" => self.textwidth = *default,
            "scrolloff" => self.scrolloff = *default,
            _ => unreachable!("VALUE_TABLE and this match are out of sync"),
        }
        true
    }

    /// `:set {bool}&` — reset a boolean option to its default (audit H1;
    /// the `&` suffix used to fall through to E518 for booleans because only
    /// the numeric reset existed). False when the name is unknown.
    pub fn reset_boolean(&mut self, name: &str) -> bool {
        let Some(canon) = canonical_bool(name) else {
            return false;
        };
        let defaults = Options::default();
        match canon {
            "number" => self.number = defaults.number,
            "relativenumber" => self.relativenumber = defaults.relativenumber,
            "expandtab" => self.expandtab = defaults.expandtab,
            "autoindent" => self.autoindent = defaults.autoindent,
            "ignorecase" => self.ignorecase = defaults.ignorecase,
            "smartcase" => self.smartcase = defaults.smartcase,
            "hlsearch" => self.hlsearch = defaults.hlsearch,
            "incsearch" => self.incsearch = defaults.incsearch,
            "showmode" => self.showmode = defaults.showmode,
            "showcmd" => self.showcmd = defaults.showcmd,
            "lisp" => self.lisp = defaults.lisp,
            _ => unreachable!("BOOL_TABLE and this match are out of sync"),
        }
        true
    }

    fn value_of(&self, canon: &str) -> usize {
        match canon {
            "tabstop" => self.tabstop,
            "shiftwidth" => self.shiftwidth,
            "textwidth" => self.textwidth,
            "scrolloff" => self.scrolloff,
            _ => unreachable!("VALUE_TABLE and this match are out of sync"),
        }
    }

    /// `:set name?` feedback in vim's rendering: two-space indent, then
    /// `ignorecase` / `noignorecase` / `tabstop=4` (9.1: `:set ts?` prints
    /// `  tabstop=8` — audit H2; the bare unpadded form was engine-shaped).
    /// The `no` spelling queries too and reports the option's REAL state —
    /// `:set noic?` prints `ignorecase` when 'ignorecase' is ON (9.1 probe:
    /// the prefix never inverts the shown value; the old `negated` flag
    /// printed the opposite, audit8 A-1). `None` when the name is unknown.
    pub fn describe(&self, name: &str) -> Option<String> {
        let describe_bool = |name: &str| -> Option<String> {
            let canon = canonical_bool(name)?;
            let on = self.bool_option(name)?;
            Some(if on {
                format!("  {canon}")
            } else {
                format!("  no{canon}")
            })
        };
        if let Some(text) = describe_bool(name) {
            return Some(text);
        }
        // `no{bool}` is a valid query spelling of `{bool}?`; and the prefix
        // simply resolves away for a VALUE option too — `:set notabstop?`
        // prints the tabstop state (9.1 probe with `ts=6`: "  tabstop=6",
        // audit8 A-6; the engine reported an unknown option)
        if let Some(stripped) = name.strip_prefix("no") {
            if let Some(text) = describe_bool(stripped) {
                return Some(text);
            }
            if let Some(canon) = canonical_value(stripped) {
                return Some(format!("  {}={}", canon, self.value_of(canon)));
            }
            return None;
        }
        let canon = canonical_value(name)?;
        Some(format!("  {}={}", canon, self.value_of(canon)))
    }

    /// `:set all&` — reset EVERY option to its default (vim 9.1 probe:
    /// `:set ts=9` + `:set all&` → `tabstop=8`; the engine used to LIST the
    /// options instead of resetting them, audit8 A-2).
    pub fn reset_all(&mut self) {
        let defaults = Options::default();
        self.number = defaults.number;
        self.relativenumber = defaults.relativenumber;
        self.expandtab = defaults.expandtab;
        self.autoindent = defaults.autoindent;
        self.ignorecase = defaults.ignorecase;
        self.smartcase = defaults.smartcase;
        self.hlsearch = defaults.hlsearch;
        self.incsearch = defaults.incsearch;
        self.showmode = defaults.showmode;
        self.showcmd = defaults.showcmd;
        self.lisp = defaults.lisp;
        self.tabstop = defaults.tabstop;
        self.shiftwidth = defaults.shiftwidth;
        self.textwidth = defaults.textwidth;
        self.scrolloff = defaults.scrolloff;
    }

    /// `:set` with no arguments: every option, rendered like [`describe`].
    pub fn describe_all(&self) -> Vec<String> {
        let mut out: Vec<String> = BOOL_TABLE
            .iter()
            .filter_map(|(canon, _)| self.describe(canon))
            .collect();
        out.extend(
            VALUE_TABLE
                .iter()
                .filter_map(|(canon, _)| self.describe(canon)),
        );
        out
    }
}
