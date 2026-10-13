//! audit8_a: parity regression for Ex / `:s` / `:set` / insert indent (independent audit, domain A)
//!
//! Cases come from black-box parity: the same (initial buffer, keystrokes) is fed to
//! the engine and to the local `/usr/bin/vim` (9.1 patches 1-1752,
//! `-Nu NONE -N -i NONE -n`, with a leading `:set ic scs ts=4 tw=78 et` to pin the
//! engine's deliberately divergent defaults into oracle premises); after `:wq` writes
//! the buffer back, the text is compared byte by byte. The `want` column below is
//! exactly vim's measured result.
//!
//! Keys are stored as **raw bytes** (`<CR>` already expanded to `\r`, `<C-h>` to
//! `\u{8}`); [`feed`] turns the bytes back into engine key names — no extra DSL
//! round-trip. The table has no case that types text in insert mode: those characters
//! are the host's text channel (the engine only collects them into
//! `pending_unknown_chars` waiting for the host to settle them); writing one would
//! require calling `type_text` like a real host (the lesson of this round's `0<C-d>`
//! case).
//!
//! Of the 161 cases, 10 are known divergences and stay out of the table: `:g`/`:v`/
//! `:normal` (divergence ledger #12), `:copy`/`:move` (the whole `:t`/`:co`/`:m` family
//! unimplemented; red probes in `tests/audit6_a.rs`'s `copy_command_missing`/
//! `move_command_missing`), `:<C-h>d` (backspace on an empty command line — the `-s`
//! and expect PTY channels give contradictory vim results; see the NOTES open-items
//! ledger).
//!
//! For 4 cases (`cmd_u`/`range_pct`/`range_dot_dollar`/`hist_prefix`) both sides' text
//! is the empty string — the classic shape of a false negative from `-s` channel
//! timeout, already re-verified via expect PTY to confirm the command really executed
//! (oracle discipline: bell/arrow-key classes go through PTY).

mod common;
use common::Fixture;

/// Raw key bytes → re-fed as engine key names (same byte stream the parity script feeds vim).
fn feed(f: &mut Fixture, keys: &str) {
    let bytes = keys.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // arrow keys are ESC + [ + letter and must be checked before a bare ESC
        let arrow = match bytes.get(i..i + 3) {
            Some(b"\x1b[A") => Some(("<Up>", 3)),
            Some(b"\x1b[B") => Some(("<Down>", 3)),
            Some(b"\x1b[C") => Some(("<Right>", 3)),
            Some(b"\x1b[D") => Some(("<Left>", 3)),
            _ => None,
        };
        if let Some((name, len)) = arrow {
            f.feed([name]);
            i += len;
            continue;
        }
        let name: String = match b {
            0x0d | 0x0a => "<CR>".to_owned(),
            0x09 => "<Tab>".to_owned(),
            0x7f => "<BS>".to_owned(),
            0x1b => "<Esc>".to_owned(),
            0x08 => "<C-h>".to_owned(),
            0x01..=0x07 | 0x0e..=0x18 | 0x1a..=0x1f => {
                format!("<C-{}>", (b + b'A' - 1) as char).to_lowercase()
            }
            _ => {
                // printables go through KEY events (same source as the parity script):
                // under the prompt `:` must be a key; type_text's IME channel would
                // feed the command line askew
                let ch = keys[i..].chars().next().unwrap();
                f.feed([ch.to_string()]);
                i += ch.len_utf8();
                continue;
            }
        };
        f.feed([name.as_str()]);
        i += 1;
    }
}

fn run(init: &str, keys: &str) -> String {
    let mut f = Fixture::new(init);
    feed(&mut f, keys);
    f.text()
}

fn statuses(init: &str, keys: &str) -> Vec<String> {
    let mut f = Fixture::new(init);
    feed(&mut f, keys);
    f.host.statuses.clone()
}

fn last_status(init: &str, keys: &str) -> String {
    statuses(init, keys)
        .pop()
        .unwrap_or_else(|| "<no message>".to_owned())
}

/// (label, initial buffer, key bytes, buffer measured in vim 9.1)
const CASES: &[(&str, &str, &str, &str)] = &[
        ("set_nonu_q", "x\n", ":set nonumber?\r", "x\n"),
        ("set_nu_q", "x\n", ":set number?\r", "x\n"),
        ("set_all_reset", "x\n", ":set ts=9\r:set all&\r", "x\n"),
        ("s_n_glob", "aa\nba\naa\n", ":%s/a/X/n\r", "aa\nba\naa\n"),
        ("s_n_globg", "aa\nba\naa\n", ":%s/a/X/gn\r", "aa\nba\naa\n"),
        ("s_lead_ws_flag", "aa\nbb\n", ":s/a/b/ e\r", "aa\nbb\n"),
        ("s_digit_g", "aa\nbb\ncc\n", ":s/a/b/2g\r", "aa\nbb\ncc\n"),
        ("star_word", "foo bar foo\n", "*", "foo bar foo\n"),
        ("hash_word", "foo bar foo\n", "2l#", "foo bar foo\n"),
        ("n_after_slash", "foo bar foo\n", "/bar\rn", "foo bar foo\n"),
        ("N_after_slash", "foo bar foo\n", "/bar\rN", "foo bar foo\n"),
        ("qmark_search", "foo bar foo\n", "?foo\rn", "foo bar foo\n"),
        ("count_search", "a a a a\n", "3/a\r", "a a a a\n"),
        ("search_e_offset", "foo bar\nbaz\n", "/bar/e\rx", "foo ba\nbaz\n"),
        ("search_b_offset", "foo bar\nbaz\n", "/bar/b\rx", "foo ar\nbaz\n"),
        ("search_line_offset", "foo bar\nbaz\n", "/bar/+1\rx", "foo bar\naz\n"),
        ("s_amp", "xx\n", ":s/x/&X/\r", "xXx\n"),
        ("s_tilde_noprev", "xx\n", ":s/x/~/\r", "x\n"),
        ("s_upper", "xx\n", ":s/x/\\u&/\r", "Xx\n"),
        ("s_cr_split", "foo bar\n", ":s/foo/F\\rB/\r", "F\nB bar\n"),
        ("s_gg", "xaxax\n", ":s/a/B/gg\r", "xBxax\n"),
        ("s_empty_repl", "xax\n", ":s/a//g\r", "xx\n"),
        ("d_range", "aa\nbb\ncc\ndd\n", ":1,3d\r", "dd\n"),
        ("y_count_put", "aa\nbb\ncc\ndd\n", ":1,2y 3\r:put\r", "aa\nbb\ncc\ndd\nbb\ncc\ndd\n"),
        ("join_count", "aa\nbb\ncc\ndd\n", ":2j 3\r", "aa\nbb cc dd\n"),
        ("join_bang", "aa\nbb\ncc\n", ":1,2j!\r", "aabb\ncc\n"),
        ("sort_bang", "a\nc\nb\n", ":sort!\r", "c\nb\na\n"),
        ("sort_n", "x\n5\na\n-3\n", ":sort n\r", "x\na\n-3\n5\n"),
        ("put_reg", "aa\nbb\n", ":1,1y\r:put a\r", "aa\nbb\n"),
        ("marks_a", "aa\nbb\n", ":2\rma\r", "aa\nbb\n"),
        ("delm", "aa\nbb\n", "ma\r:delm a\r", "aa\nbb\n"),
        ("reg_arg", "aa\nbb\n", ":1y\r:reg 0\r", "aa\nbb\n"),
        ("ce_width", "aa\n", ":set tw=10\r:ce\r", "    aa\n"),
        ("ri_width", "aa\n", ":set tw=10\r:ri\r", "        aa\n"),
        ("le_indent", "  aa\n", ":le\r", "aa\n"),
        ("retab", "\ta\n", ":set et\r:retab 4\r", "    a\n"),
        ("undo_redo", "aa\n", "x\r:undo\r:redo\r", "a\n"),
        ("amp_amp", "xax\nyay\n", ":1s/a/B/\r:2&&\r", "xBx\nyBy\n"),
        ("amp", "xax\nyay\n", ":1s/a/B/\rj&\r", "xBx\nyBy\n"),
        ("tilde_cmd", "xax\nyay\n", ":1s/a/B/\rj:~\r", "xBx\nyBy\n"),
        ("s_bad_flag", "aa\n", ":s/a/b/z\r", "aa\n"),
        ("s_count0", "aa\n", ":s/a/b/0\r", "aa\n"),
        ("set_bad", "x\n", ":set foo\r", "x\n"),
        ("set_num_bad", "x\n", ":set ts=x\r", "x\n"),
        ("set_ts0", "x\n", ":set ts=0\r", "x\n"),
        ("cmd_w", "x\n", ":set ts=4\u{17}\r", "x\n"),
        ("cmd_u", "x\n", ":set ts=4\u{15}d\r", ""),
        ("hist_up", "a\nb\nc\n", ":1d\r:2d\r:\u{1b}[A\r", "b\n"),
        ("visual_colon", "aa\nbb\n", "Vj:s/a/X/\r", "Xa\nbb\n"),
        ("range_plus", "aa\nbb\ncc\n", ":+d\r", "aa\ncc\n"),
        ("range_pct", "aa\nbb\ncc\n", ":%d\r", ""),
        ("range_dollar", "aa\nbb\ncc\n", ":$d\r", "aa\nbb\n"),
        ("range_dot_dollar", "aa\nbb\ncc\n", ":.,$d\r", ""),
        ("range_reverse", "aa\nbb\ncc\n", ":3,1d\r", "aa\nbb\ncc\n"),
        ("range_oob", "aa\nbb\ncc\n", ":9d\r", "aa\nbb\ncc\n"),
        ("range_bare_oob", "aa\nbb\ncc\n", ":9\r", "aa\nbb\ncc\n"),
        ("set_inv", "x\n", ":set invic\r", "x\n"),
        ("set_reset_bool", "x\n", ":set ic\r:set ic&\r", "x\n"),
        ("s_multiple_flags", "aa\nbb\n", ":s/a/b/gi\r", "bb\nbb\n"),
        ("s_case_I", "Aa\naa\n", ":s/a/X/gI\r", "AX\naa\n"),
        ("search_star_cjk", "中文 中文\n", "*", "中文 中文\n"),
        ("search_n_reapply_e", "foo bar\nbaz bar\n", "/bar/e\rnx", "foo bar\nbaz ba\n"),
        ("search_N_back", "foo bar\nbaz bar\n", "/bar\rNx", "foo bar\nbaz ar\n"),
        ("search_qmark_n", "foo bar\nbaz bar\n", "?bar\rnx", "foo ar\nbaz bar\n"),
        ("star_gstar", "foobar foo\n", "g*", "foobar foo\n"),
        ("star_mid", "abc foo abc\n", "4l*", "abc foo abc\n"),
        ("s_UL_case", "abc\n", ":s/abc/\\U&\\E/\r", "ABC\n"),
        ("s_Lcase", "ABC\n", ":s/ABC/\\L&/\r", "abc\n"),
        ("s_lfirst", "abc\n", ":s/abc/\\l&/\r", "abc\n"),
        ("s_prev_repl", "ax\nbx\n", ":1s/a/NN/\r:2s/b/~/\r", "NNx\nNNx\n"),
        ("s_amp_escape", "ax\n", ":s/a/\\&X/\r", "&Xx\n"),
        ("s_tilde_escape", "ax\n", ":s/a/\\~X/\r", "~Xx\n"),
        ("s_backslash", "ax\n", ":s/a/x\\\\y/\r", "x\\yx\n"),
        ("s_sep_hash", "a#b\n", ":s#a#X#\r", "X#b\n"),
        ("s_sep_comma", "a,b\n", ":s,a,X,\r", "X,b\n"),
        ("s_esc_sep", "a/b\n", ":s/a\\/b/X/\r", "X\n"),
        ("s_tab", "ab\n", ":s/a/\\t/\r", "\tb\n"),
        ("range_mark", "aa\nbb\ncc\n", "2Gma\r:'a\rd\r", "aa\n"),
        ("range_marks_pair", "aa\nbb\ncc\n", "1Gma2Gmb\r:'a,'bd\r", "cc\n"),
        ("range_search_fwd", "aa\nbb\ncc\ndd\n", ":/bb/\rd\r", "aa\ndd\n"),
        ("range_search_back", "aa\nbb\ncc\ndd\n", "G:?bb?\rd\r", "aa\ndd\n"),
        ("range_search_pair", "aa\nbb\ncc\ndd\n", ":/bb/,/cc/d\r", "aa\ndd\n"),
        ("range_comma_open", "aa\nbb\ncc\n", "2G:,d\r", "aa\ncc\n"),
        ("range_semicolon", "aa\nbb\ncc\n", ":1;2d\r", "cc\n"),
        ("range_plus2", "aa\nbb\ncc\n", ":1+2d\r", "aa\nbb\n"),
        ("range_2search", "aa\nbb\ncc\ndd\n", ":1/cc/d\r", "aa\nbb\ndd\n"),
        ("range_bare_mark", "aa\nbb\n", "2G:'\r", "aa\nbb\n"),
        ("colon_print", "aa\nbb\n", ":1p\r", "aa\nbb\n"),
        ("colon_num", "aa\nbb\n", ":2\rx", "aa\nb\n"),
        ("colon_numrange", "aa\nbb\ncc\n", ":1,2\rx", "aa\nb\ncc\n"),
        ("k_cmd", "aa\nbb\n", ":ka\r:'ad\r", "bb\n"),
        ("put_zero", "aa\nbb\n", ":1y\r:0pu\r", "aa\naa\nbb\n"),
        ("put_above", "aa\nbb\n", ":1y\r:2pu!\r", "aa\naa\nbb\n"),
        ("put_named", "aa\nbb\n", "\"ayy\r:2pu a\r", "aa\nbb\naa\n"),
        ("y_into_a", "aa\nbb\n", ":1y a\r", "aa\nbb\n"),
        ("d_into_a", "aa\nbb\n", ":1d a\r", "bb\n"),
        ("d_bang", "aa\nbb\n", ":1d!\r", "aa\nbb\n"),
        ("y_bang", "aa\nbb\n", ":1y!\r", "aa\nbb\n"),
        ("d_count", "aa\nbb\ncc\ndd\n", ":1d 2\r", "cc\ndd\n"),
        ("y_count", "aa\nbb\ncc\ndd\n", ":1y 2\r", "aa\nbb\ncc\ndd\n"),
        ("d_reg_count", "aa\nbb\ncc\ndd\n", ":1d a 2\r", "cc\ndd\n"),
        ("join_plain", "aa\nbb\ncc\n", ":1j\r", "aa bb\ncc\n"),
        ("join_two_addr", "aa\nbb\ncc\n", ":1,1j\r", "aa\nbb\ncc\n"),
        ("join_count1", "aa\nbb\ncc\n", ":1j 1\r", "aa\nbb\ncc\n"),
        ("retab_noet", "\ta\n", ":retab\r", "    a\n"),
        ("retab_bang", "        a\n", ":set noet ts=8\r:retab! 4\r", "\t\ta\n"),
        ("sort_u", "b\na\na\nb\n", ":sort u\r", "a\nb\n"),
        ("sort_i", "b\nA\na\n", ":sort i\r", "A\na\nb\n"),
        ("sort_iu", "b\nA\na\n", ":sort iu\r", "A\nb\n"),
        ("le_num", "    aa\n", ":le 2\r", "  aa\n"),
        ("ce_num", "aa\n", ":ce 10\r", "    aa\n"),
        ("ri_num", "aa\n", ":ri 10\r", "        aa\n"),
        ("set_noic", "x\n", ":set noic\r", "x\n"),
        ("set_ic_toggle", "x\n", ":set ic!\r", "x\n"),
        ("set_inv_ts", "x\n", ":set invts\r", "x\n"),
        ("set_query_sp", "x\n", ":set ts ?\r", "x\n"),
        ("set_bare_num", "x\n", ":set ts\r", "x\n"),
        ("set_all_list", "x\n", ":set all\r", "x\n"),
        ("set_comment", "x\n", ":set ts=4 \" note\r", "x\n"),
        ("set_unknown_eq", "x\n", ":set foo=1\r", "x\n"),
        ("set_reset_ts", "x\n", ":set ts=9\r:set ts&\r", "x\n"),
        ("cmd_r_reg", "aa\nbb\n", "\"ayy\r:\"x\r", "aa\nbb\n"),
        ("cmd_cr_reg", "aa\nbb\n", "\"ayy\r:\u{12}a\r", "aa\nbb\n"),
        ("cmd_cr_eq", "x\n", ":\"\u{12}=1+2\r\r", "x\n"),
        ("cmd_literal_cv", "x\n", ":e\u{16}\u{1b}x\r", "x\n"),
        ("hist_prefix", "a\nb\nc\n", ":1d\r:2d\r:1\u{1b}[A\r", ""),
        ("hist_ctrl_p", "a\nb\nc\n", ":1d\r:2d\r:\u{10}\r", "b\n"),
        ("delm_range", "aa\nbb\n", "ma\rmb\r:delm a-b\r", "aa\nbb\n"),
        ("delm_bang", "aa\nbb\n", "ma\rmb\r:delm!\r", "aa\nbb\n"),
        ("delm_bare", "aa\nbb\n", ":delm\r", "aa\nbb\n"),
        ("reg_bare", "aa\nbb\n", "\"ayy\r:reg\r", "aa\nbb\n"),
        ("noh", "aa\nbb\n", "/a\r:noh\r", "aa\nbb\n"),
        ("s_g_flag_upper", "aa\n", ":s/a/X/gI\r", "XX\n"),
        ("s_only_pattern", "aaa\n", ":s/a\r", "aa\n"),
        ("s_empty_pat", "aaa\n", "/a\r:s//X/\r", "Xaa\n"),
        ("s_repeat_bare", "ax\nyx\n", ":1s/a/B/\r:2s\r", "Bx\nyx\n"),
        ("sub_count_span", "a1\na2\na3\na4\n", ":1s/a/X/ 3\r", "X1\nX2\nX3\na4\n"),
        ("undo_ex", "aa\n", "x\r:undo\r", "aa\n"),
        ("redo_ex", "aa\n", "x\ru\r:redo\r", "a\n"),
        ("wq_forced", "aa\n", ":wq\r", "aa\n"),
        ("buf_first_last", "aa\n", ":bf\r", "aa\n"),
        ("action_cmd", "aa\n", ":action SomeId\r", "aa\n"),
        ("s_noic", "Aa\n", ":set noic\r:s/a/X/\r", "AX\n"),
        ("s_ic_scs_upper", "aA\n", ":set ic scs\r:s/A/X/\r", "aX\n"),
        ("s_noic_n", "Aa\n", ":set noic\r:s/a/X/n\r", "Aa\n"),
        ("s_noic_I_flag", "Aa\n", ":set ic\r:s/a/X/I\r", "AX\n"),
        ("s_leading_ws_count", "aa\nbb\ncc\n", ":s/a/b/ 2\r", "ba\nbb\ncc\n"),
        ("s_flag_g_then_ws", "aa\n", ":s/a/b/g 2\r", "bb\n"),
        ("s_huge_count", "aa\n", ":s/a/b/99999999999999999999\r", "aa\n"),
        ("s_i32max", "aa\n", ":s/a/b/2147483647\r", "aa\n"),
        ("s_i32max1", "aa\n", ":s/a/b/2147483646\r", "ba\n"),
];

/// Parity table: the engine text must equal vim 9.1's measured write-back byte for byte.
#[test]
fn parity_with_vim_oracle() {
    let mut bad: Vec<String> = Vec::new();
    for (label, init, keys, want) in CASES {
        let got = run(init, keys);
        if got != *want {
            bad.push(format!("{label}: want {want:?} got {got:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} cases diverged:\n{}",
        bad.len(),
        CASES.len(),
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The message side (the half invisible in buffer text) — each item maps to a fix
// from this round
// ---------------------------------------------------------------------------

/// A-1: the `no` prefix never inverts the displayed value — `:set noic?` reports
/// `ignorecase` while 'ignorecase' is ON (9.1 redir evidence: `:set ic` +
/// `:set noic?` → "  ignorecase"; `:set noic` + `:set ic?` → "noignorecase"). The old
/// implementation carried a `negated` flag and printed the opposite state.
#[test]
fn set_query_never_inverts_the_no_prefix() {
    assert_eq!(last_status("x\n", ":set ic\r:set noic?\r"), "  ignorecase");
    assert_eq!(
        last_status("x\n", ":set noic\r:set ic?\r"),
        "  noignorecase"
    );
    assert_eq!(last_status("x\n", ":set ic\r:set ic?\r"), "  ignorecase");
}

/// A-6 (a new finding in the closing re-check): `no{value-option}?` resolves to the
/// value option itself in vim (9.1 evidence: `:set ts=6` + `:set notabstop?` →
/// "  tabstop=6"); the old implementation claimed `notabstop` does not exist.
#[test]
fn set_query_resolves_no_prefix_on_value_options() {
    assert_eq!(
        last_status("x\n", ":set ts=6\r:set notabstop?\r"),
        "  tabstop=6"
    );
    assert_eq!(last_status("x\n", ":set ts=6\r:set nots?\r"), "  tabstop=6");
}

/// A-2: `:set all&` is a **reset**, not a listing (9.1 evidence: `:set ts=9` +
/// `:set all&` + `:set ts?` → "  tabstop=8"). The engine's reset target is the engine's
/// own default ('ts'=4 is divergence ledger #2); what is pinned here is the semantics
/// "the value returns to the default".
#[test]
fn set_all_amp_resets_every_option() {
    assert_eq!(
        last_status("x\n", ":set ts=9\r:set all&\r:set ts?\r"),
        "  tabstop=4"
    );
    assert_eq!(
        last_status("x\n", ":set noic\r:set all&\r:set ic?\r"),
        "  ignorecase"
    );
    // `:set all` (without &) still lists item by item
    let listed = statuses("x\n", ":set all\r");
    assert!(
        listed.len() > 1,
        "`:set all` should list items one by one, got {listed:?}"
    );
    assert!(listed.iter().any(|s| s == "  tabstop=4"));
}

/// A-3: `:s`'s flag string must hug the third delimiter; whitespace before it is legal
/// only when "the whole stretch is a pure count" (9.1 evidence: `:s/a/b/ e` →
/// "E488: Trailing characters: e" with the text unmoved; `:s/a/b/2g` → E488 quoting
/// "g"). The old implementation accepted `/ e` and performed the substitution.
#[test]
fn sub_flags_must_hug_the_delimiter() {
    assert_eq!(
        last_status("aa\nbb\n", ":s/a/b/ e\r"),
        "E488: Trailing characters: e"
    );
    assert_eq!(run("aa\nbb\n", ":s/a/b/ e\r"), "aa\nbb\n");
    assert_eq!(
        last_status("aa\nbb\ncc\n", ":s/a/b/2g\r"),
        "E488: Trailing characters: g"
    );
    assert_eq!(run("aa\nbb\ncc\n", ":s/a/b/2g\r"), "aa\nbb\ncc\n");
    // whitespace is allowed before a pure count, which also extends the range by count lines
    assert_eq!(run("a1\na2\na3\na4\n", ":1s/a/X/ 3\r"), "X1\nX2\nX3\na4\n");
    assert_eq!(run("aa\n", ":s/a/b/g 2\r"), "bb\n");
}

/// A-3 addendum: count 0 raises E939; overflow raises E1510 reported with the
/// **clamped value** (9.1 evidence: `:s/a/b/999…` →
/// "E1510: Value too large: 9223372036854775807", `:s/a/b/2147483647` →
/// "E1510: Value too large: 2147483647", while `:s/a/b/2147483646` is legal and really
/// substitutes out "ba").
#[test]
fn sub_count_error_messages_match_vim() {
    assert_eq!(
        last_status("aa\n", ":s/a/b/0\r"),
        "E939: Positive count required"
    );
    assert_eq!(
        last_status("aa\n", ":s/a/b/99999999999999999999\r"),
        "E1510: Value too large: 9223372036854775807"
    );
    assert_eq!(
        last_status("aa\n", ":s/a/b/2147483647\r"),
        "E1510: Value too large: 2147483647"
    );
    assert_eq!(run("aa\n", ":s/a/b/2147483646\r"), "ba\n");
}

/// A-4: the `n` flag counts by "what the substitution would do" — without `g` each line
/// counts once (9.1 evidence: `:s/a/X/n` on "aa" → "1 match on 1 line"; `:%s/a/X/n` on
/// three lines with one hit each → "3 matches on 3 lines"; `:%s/a/X/gn` →
/// "5 matches on 3 lines"). The old implementation always counted every occurrence.
#[test]
fn sub_n_counts_like_the_substitution_would() {
    assert_eq!(last_status("aa\n", ":s/a/X/n\r"), "1 match on 1 line");
    assert_eq!(
        last_status("aa\nba\naa\n", ":%s/a/X/n\r"),
        "3 matches on 3 lines"
    );
    assert_eq!(
        last_status("aa\nba\naa\n", ":%s/a/X/gn\r"),
        "5 matches on 3 lines"
    );
    assert_eq!(last_status("ab\nab\n", ":%s/a/X/n\r"), "2 matches on 2 lines");
}

/// A-5: outside `i`/`I` the case settings are **not touched** — with no flags it follows
/// 'ignorecase'/'smartcase' (9.1 evidence: `:set noic` + `:s/a/X/` on "Aa" → "AX",
/// hitting only the lowercase a; `:set ic` + `:s/a/X/gI` → "AX"). The old implementation
/// unconditionally called `case_insensitive(true)`, making `:set noic` a no-op.
#[test]
fn sub_without_flags_follows_ignorecase() {
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/\r"), "AX\n");
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/n\r"), "Aa\n");
    assert_eq!(
        last_status("Aa\n", ":set noic\r:s/a/X/n\r"),
        "1 match on 1 line"
    );
    assert_eq!(run("Aa\n", ":set ic\r:s/a/X/gI\r"), "AX\n");
    assert_eq!(run("Aa\n", ":set noic\r:s/a/X/gi\r"), "XX\n");
}

// ---------------------------------------------------------------------------
// insert `<C-t>`/`<C-d>`: vim ROUNDs to a multiple of shiftwidth, not ±shiftwidth
// ---------------------------------------------------------------------------
//
// All shapes are vim 9.1 evidence (`:set et ts=4 sw=4 noai`; key sequences are always
// anchored with `0` — under the `-s` channel the starting cursor lands at end of line,
// so without anchoring the results would be false).

/// Type an `X` after `<C-t>`/`<C-d>` to pin both the new indent and the cursor landing.
fn shift_then_type(init: &str, col: usize, key: &str) -> (String, usize) {
    let mut f = Fixture::new(init);
    f.feed(["0"]);
    for _ in 0..col {
        f.feed(["l"]);
    }
    f.feed(["i"]);
    f.feed([key]);
    f.type_text("X");
    f.feed(["<Esc>"]);
    (f.text(), f.cursor())
}

/// The rising arm of `get_new_sw_indent(round=TRUE)`: round up to the next shiftwidth
/// multiple (9.1: indents 0/2/3/4/6/8 → 4/4/4/8/8/12). The old implementation did
/// `old + sw`, so on a non-multiple indent one step overshoots (2 spaces → 6).
#[test]
fn insert_ctrl_t_rounds_indent_up_to_a_shiftwidth_multiple() {
    // (indent, cursor col, new indent, byte position of X)
    for (indent, col, want_indent, want_byte) in [
        (0usize, 0usize, 4usize, 4usize),
        (2, 0, 4, 2),
        (2, 1, 4, 3),
        (2, 2, 4, 4),
        (3, 0, 4, 1),
        (4, 0, 8, 4),
        (6, 0, 8, 2),
        (8, 0, 12, 4),
    ] {
        let init = format!("{}foo\n", " ".repeat(indent));
        let (text, byte) = shift_then_type(&init, col, "<C-t>");
        let want = format!(
            "{}X{}foo\n",
            " ".repeat(want_byte),
            " ".repeat(want_indent - want_byte)
        );
        assert_eq!(text, want, "<C-t> on {indent}-space indent, col {col}");
        assert_eq!(byte, want_byte, "<C-t> cursor col afterwards ({indent} spaces, col {col})");
    }
}

/// The falling arm: round down to the previous shiftwidth multiple (9.1: indents
/// 2/4/6/8 → 0/0/4/4; when already a multiple it drops another notch, so 4 → 0 rather
/// than the in-place "false success" 4 → 4). When the cursor lands inside the indent it
/// is clamped to the start of the line (a negative column shift has no column to back
/// out of).
#[test]
fn insert_ctrl_d_rounds_indent_down_to_a_shiftwidth_multiple() {
    for (indent, col, want_indent) in [
        (2usize, 0usize, 0usize),
        (4, 0, 0),
        (4, 3, 0),
        (4, 4, 0),
        (6, 0, 4),
        (8, 0, 4),
    ] {
        let init = format!("{}foo\n", " ".repeat(indent));
        let (text, byte) = shift_then_type(&init, col, "<C-d>");
        // X always lands at byte 0 (cursor clamped to line start), followed by the new indent
        let want = format!("X{}foo\n", " ".repeat(want_indent));
        assert_eq!(text, want, "<C-d> on {indent}-space indent, col {col}");
        assert_eq!(byte, 0, "<C-d> clamps the cursor to line start ({indent} spaces, col {col})");
    }
}

/// `0<C-d>`: the leading `0` is a command char, not text, and the backspace **clears
/// all indent** (vim's ins_shift goes through change_indent's INDENT_SET arm) rather
/// than "minus one shiftwidth". 9.1 evidence: on 8-space indent, `i0<C-d>` → "foo".
#[test]
fn insert_zero_ctrl_d_clears_all_indent() {
    let mut f = Fixture::new("        foo\n");
    f.feed(["i"]);
    // insert-mode printables are the HOST's text channel (the engine declines
    // the raw key so an IME can compose), same as the `X` marker above
    f.type_text("0");
    f.feed(["<C-d>"]);
    f.feed(["<Esc>"]);
    assert_eq!(f.text(), "foo\n");
    assert_eq!(f.cursor(), 0);
}

/// TAB indents get rewritten entirely to spaces under `expandtab` (9.1 evidence: on
/// `\t\tfoo`, `0i<C-t>X` → 12-column-wide indent, X lands at byte 4).
#[test]
fn insert_shift_rewrites_tab_indent_under_expandtab() {
    let (text, byte) = shift_then_type("\t\tfoo\n", 0, "<C-t>");
    assert_eq!(text, "    X        foo\n");
    assert_eq!(byte, 4);
    let (text, byte) = shift_then_type("\t\tfoo\n", 0, "<C-d>");
    assert_eq!(text, "X    foo\n");
    assert_eq!(byte, 0);
}
