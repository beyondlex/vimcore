//! Parser for `~/.vimcorerc` style configuration — the IdeaVim-compatible
//! subset: `set` options, `:map`-family mappings (with `<Leader>`), `"`
//! comments and `source`. Anything else (Lua, functions, autocmds, plugin
//! managers) is collected into `Config::ignored` and skipped, like IdeaVim.

use crate::key::{parse_key_sequence, Key, KeyKind};
use crate::keymap::ModeClass;
use std::path::PathBuf;

/// One `set` item. `On`/`Off`/`Toggle` are booleans; `Value` is `name=value`;
/// `Reset` is the `name&` default-reset form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    On(String),
    Off(String),
    Toggle(String),
    Value(String, String),
    Reset(String),
}

/// One mapping from the `:map` family, resolved against the file's
/// `mapleader` (so `<Leader>` in the LHS is already a concrete key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigMapping {
    pub class: ModeClass,
    pub lhs: Vec<Key>,
    pub rhs: Vec<Key>,
    pub noremap: bool,
}

/// A parsed configuration file.
#[derive(Debug, Default)]
pub struct Config {
    pub settings: Vec<Setting>,
    pub mappings: Vec<ConfigMapping>,
    /// `source <path>` directives, in file order. The loader applies them.
    pub sources: Vec<PathBuf>,
    /// Lines that were not understood (Lua, functions, autocmds, ...).
    pub ignored: Vec<String>,
}

/// Counts reported by [`VimState::apply_config`](crate::state::VimState::apply_config).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigStats {
    pub options: usize,
    pub mappings: usize,
    pub ignored: usize,
}

fn default_leader() -> Key {
    Key::char('\\')
}

/// Bind one `let mapleader = "x"`-style line into `target`. The variable
/// name must end at the assignment (`let mapleader2` is an unrelated
/// variable, not a leader binding); a matched-but-malformed line consumes
/// nothing so it falls through to `ignored` with a trace.
fn bind_let_leader(line: &str, prefix: &str, target: &mut Key) -> bool {
    let Some(rest) = line.strip_prefix(prefix) else {
        return false;
    };
    // word boundary + an actual `=` (after optional spaces)
    let boundary_ok = rest
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        && rest.trim_start().starts_with('=');
    if !boundary_ok {
        return false;
    }
    // let mapleader = " " / "," / "<Space>" — a bare space parses to
    // Char(' ') via parse_key_sequence, and `<Space>` normalizes to
    // Char(' ') in Key::parse_angle, so both spellings converge.
    if let Some(value) = rest.split('=').nth(1) {
        let value = value.trim().trim_matches('"');
        if let Some(key) = parse_key_sequence(value).first() {
            *target = key.clone();
        }
    }
    true
}

/// Parse a configuration text. `mapleader` (via `let mapleader = "x"`)
/// affects subsequent `<Leader>` occurrences, like vim.
pub fn parse(text: &str) -> Config {
    let mut config = Config::default();
    let mut leader = default_leader();
    let mut local_leader = default_leader();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        // strip a leading colon (`:map ...` in the user's example)
        let line = line.strip_prefix(':').unwrap_or(line);
        if line.is_empty() || line.starts_with('"') {
            continue;
        }

        // `let mapleader` / `let g:mapleader` / `let maplocalleader` /
        // `let g:maplocalleader` bind the two leader keys. The variable name
        // must END at the `=`: the old bare strip_prefix also swallowed
        // `let mapleader2 = ";"` — an unrelated variable silently rebinding
        // every `<Leader>` mapping with no trace in `ignored` (round 14).
        if bind_let_leader(line, "let mapleader", &mut leader)
            || bind_let_leader(line, "let g:mapleader", &mut leader)
            || bind_let_leader(line, "let maplocalleader", &mut local_leader)
            || bind_let_leader(line, "let g:maplocalleader", &mut local_leader)
        {
            // a leader binding never reaches the map family
            continue;
        }

        if let Some(rest) = line
            .strip_prefix("set")
            .filter(|r| r.is_empty() || r.starts_with(' ') || r.starts_with('\t'))
        {
            for arg in rest.split_whitespace() {
                // a `"` starts a comment to end of line (vim: `set ts=4 "
                // note` sets quietly — the old parser emitted garbage
                // On("\"")/On("note") settings that inflated
                // ConfigStats::ignored at apply time)
                if arg.starts_with('"') {
                    break;
                }
                // Branch order matters twice over: `=` must win over `no`
                // (`set no=3`? unlikely, but `name=value` is never a negation),
                // and a leading `no` is only negation when the remainder names
                // a real boolean option — otherwise `set number` would parse
                // as `Off("mber")`.
                let setting = if let Some((name, value)) = arg.split_once('=') {
                    Setting::Value(name.to_owned(), value.to_owned())
                } else if let Some(name) = arg.strip_suffix('!') {
                    Setting::Toggle(name.to_owned())
                } else if let Some(name) = arg
                    .strip_suffix("&vim")
                    .or_else(|| arg.strip_suffix("&vi"))
                    .or_else(|| arg.strip_suffix('&'))
                {
                    // `set ts&` resets to the default (ex_set parity)
                    Setting::Reset(name.to_owned())
                } else if arg.ends_with('?') {
                    // a query (`set ic?`) DISPLAYS a value; a config file has
                    // no message channel, so the token is dropped quietly
                    // instead of becoming a garbage On("ts?") setting
                    continue;
                } else if let Some(name) = arg
                    .strip_prefix("no")
                    .filter(|n| crate::options::is_bool_option(n))
                {
                    Setting::Off(name.to_owned())
                } else {
                    Setting::On(arg.to_owned())
                };
                config.settings.push(setting);
            }
            continue;
        }

        if let Some(path) = line
            .strip_prefix("source")
            .filter(|r| r.starts_with(' ') || r.starts_with('\t'))
        {
            // tilde expansion is the loader's job (parse stays pure)
            config.sources.push(PathBuf::from(path.trim()));
            continue;
        }

        // the :map family — the optional leading `:` was already stripped
        let (noremap, classes, after_cmd) =
            if let Some(rest) = strip_map_cmd(line, "noremap").map(str::trim_start) {
                (true, vec![ModeClass::Normal, ModeClass::Visual], rest)
            } else if let Some(rest) = strip_map_cmd(line, "nnoremap").map(str::trim_start) {
                (true, vec![ModeClass::Normal], rest)
            } else if let Some(rest) = strip_map_cmd(line, "vnoremap").map(str::trim_start) {
                (true, vec![ModeClass::Visual], rest)
            } else if let Some(rest) = strip_map_cmd(line, "inoremap").map(str::trim_start) {
                (true, vec![ModeClass::Insert], rest)
            } else if let Some(rest) = strip_map_cmd(line, "nmap").map(str::trim_start) {
                (false, vec![ModeClass::Normal], rest)
            } else if let Some(rest) = strip_map_cmd(line, "vmap").map(str::trim_start) {
                (false, vec![ModeClass::Visual], rest)
            } else if let Some(rest) = strip_map_cmd(line, "imap").map(str::trim_start) {
                (false, vec![ModeClass::Insert], rest)
            } else if let Some(rest) = strip_map_cmd(line, "map").map(str::trim_start) {
                (false, vec![ModeClass::Normal, ModeClass::Visual], rest)
            } else {
                config.ignored.push(raw_line.to_owned());
                continue;
            };

        let (lhs_str, rhs_str) = match split_ws2(after_cmd) {
            Some((lhs, rhs)) if !lhs.is_empty() && !rhs.trim().is_empty() => {
                (lhs, rhs.trim_start())
            }
            _ => {
                config.ignored.push(raw_line.to_owned());
                continue;
            }
        };
        let lhs = parse_with_leader(lhs_str, &leader, &local_leader);
        if lhs.is_empty() {
            config.ignored.push(raw_line.to_owned());
            continue;
        }
        let rhs = parse_with_leader(rhs_str, &leader, &local_leader);
        for class in classes {
            config.mappings.push(ConfigMapping {
                class,
                lhs: lhs.clone(),
                rhs: rhs.clone(),
                noremap,
            });
        }
    }
    config
}

fn strip_map_cmd<'a>(line: &'a str, cmd: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(cmd)?;
    // command boundary: end of line or whitespace ("map" must not match
    // "mapping"). TAB is whitespace too — an rc line like `nnoremap\tx y`
    // used to be silently dropped into `ignored`
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
        Some(rest)
    } else {
        None
    }
}

/// Split `{lhs}{sep}{rhs}` on the first space OR tab (vimrcs use both).
fn split_ws2(s: &str) -> Option<(&str, &str)> {
    let i = s.find([' ', '\t'])?;
    Some((&s[..i], &s[i + 1..]))
}

/// Parse a mapping side, resolving the `<Leader>`/`<leader>` token against
/// the file's `mapleader`. The substitution happens on the PARSED sequence
/// (`<Leader>` parses to a `Named("leader")` marker key), never by string
/// replacement: re-parsing `leader.notation()` shatters multi-char keys —
/// a `<Space>` leader would come back as S,p,a,c,e.
fn parse_with_leader(seq: &str, leader: &Key, local_leader: &Key) -> Vec<Key> {
    parse_key_sequence(seq)
        .into_iter()
        .map(|k| match &k.kind {
            // `<Leader>`/`<LocalLeader>` are marker keys from the angle
            // notation; resolve them against the file's two let-variables.
            // An unresolved `<LocalLeader>` used to stay Named("localleader")
            // — a key no keyboard can produce, so the mapping silently never
            // fired.
            KeyKind::Named(n) if n == "leader" => leader.clone(),
            KeyKind::Named(n) if n == "localleader" => local_leader.clone(),
            _ => k,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_number_is_on_not_off_mber() {
        // regression: `no` used to be stripped before anything else, so
        // `set number` parsed as `Off("mber")`
        let config = parse("set number");
        assert_eq!(config.settings, vec![Setting::On("number".to_owned())]);
    }

    #[test]
    fn set_no_prefix_negates_known_boolean_options() {
        let config = parse("set nonumber\nset nonu");
        assert_eq!(
            config.settings,
            vec![
                Setting::Off("number".to_owned()),
                Setting::Off("nu".to_owned()),
            ]
        );
    }

    #[test]
    fn set_value_and_toggle_forms() {
        let config = parse("set tabstop=8\nset number!");
        assert_eq!(
            config.settings,
            vec![
                Setting::Value("tabstop".to_owned(), "8".to_owned()),
                Setting::Toggle("number".to_owned()),
            ]
        );
    }

    #[test]
    fn set_unknown_option_falls_through_to_on() {
        // `wrap` is not in the engine's option subset: it parses as `On` and
        // is silently ignored at apply time (like IdeaVim's unknown sets)
        let config = parse("set wrap");
        assert_eq!(config.settings, vec![Setting::On("wrap".to_owned())]);
    }
}
