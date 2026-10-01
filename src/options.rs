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
            autoindent: true,
            ignorecase: true,
            smartcase: true,
            hlsearch: true,
            incsearch: true,
            showmode: true,
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
            _ => unreachable!("BOOL_TABLE and this match are out of sync"),
        }
    }

    /// `:set name=value` for numeric options. Values are capped: tabstop and
    /// shiftwidth feed `" ".repeat(n)`-shaped indent synthesis (insert Tab,
    /// `>>`), and a typo like `:set sw=99999999999` must not hand the host a
    /// gigabyte allocation. 1e6 columns is far past any real use; vim caps
    /// these options at 2^31-1 instead (documented divergence).
    pub fn set_value(&mut self, name: &str, value: &str) -> bool {
        const MAX_OPTION_VALUE: usize = 1_000_000;
        let Some(canon) = canonical_value(name) else {
            return false;
        };
        // every numeric option is a usize today, so one parse serves all
        let Ok(v) = value.parse::<usize>() else {
            return false;
        };
        let v = v.min(MAX_OPTION_VALUE);
        match canon {
            "tabstop" => self.tabstop = v,
            "shiftwidth" => self.shiftwidth = v,
            "textwidth" => self.textwidth = v,
            "scrolloff" => self.scrolloff = v,
            _ => unreachable!("VALUE_TABLE and this match are out of sync"),
        }
        true
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

    fn value_of(&self, canon: &str) -> usize {
        match canon {
            "tabstop" => self.tabstop,
            "shiftwidth" => self.shiftwidth,
            "textwidth" => self.textwidth,
            "scrolloff" => self.scrolloff,
            _ => unreachable!("VALUE_TABLE and this match are out of sync"),
        }
    }

    /// `:set name?` feedback in vim's rendering: `ignorecase` /
    /// `noignorecase` / `tabstop=4`. The `no` spelling queries too
    /// (`noic?` → `noignorecase`). `None` when the name is unknown.
    pub fn describe(&self, name: &str) -> Option<String> {
        let (name, negated) = match name.strip_prefix("no") {
            Some(stripped) if canonical_bool(stripped).is_some() => (stripped, true),
            _ => (name, false),
        };
        if let Some(canon) = canonical_bool(name) {
            let on = self.bool_option(name)? != negated;
            return Some(if on {
                canon.to_owned()
            } else {
                format!("no{canon}")
            });
        }
        if negated {
            return None; // `notabstop` is not a thing
        }
        let canon = canonical_value(name)?;
        Some(format!("{}={}", canon, self.value_of(canon)))
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
