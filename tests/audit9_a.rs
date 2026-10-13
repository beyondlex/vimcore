//! Round-6 audit probes (Ex / command line / :s / :set / marks / registers domains).
//!
//! Every probe is first verified against `/usr/bin/vim` (VIM 9.1), then pinned as engine behavior;
//! it should fail before the fix and becomes a regression after. All green means this round's new
//! bugs are zeroed out.

mod common;

use common::Fixture;

/// Feed one Ex command line character by character into the prompt (each item of `f.feed` is "one key").
fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn last_status(f: &Fixture) -> String {
    f.host.statuses.last().cloned().unwrap_or_default()
}

// ======================================================== :set numeric syntax

// oracle (`vim -Nu NONE -N -i NONE -n -s`, `:set ts=-1`):
//   E487: Argument must be positive: ts=-1
// Old engine behavior: `value.parse::<usize>("-1")` failed → E521 (treated the minus sign as a syntax error).
// The parse grammar of numeric options = optional `-` + digits, whole string consumed; negative values always E487.
#[test]
fn set_negative_value_is_e487_not_e521() {
    for (opt, val) in [("ts", "-1"), ("sw", "-1"), ("tw", "-1"), ("so", "-1")] {
        let mut f = Fixture::new("a\n");
        ex(&mut f, &format!("set {opt}={val}"));
        assert_eq!(
            last_status(&f),
            format!("E487: Argument must be positive: {opt}={val}"),
            "`:{opt}={val}` should report E487 (oracle); actual {:?}",
            f.host.statuses
        );
    }
}

// oracle: `:set ts=+4` → E521: Number required after =: ts=+4
// Old engine behavior: Rust's `"+4".parse::<usize>()` = Ok(4), silently accepted.
#[test]
fn set_leading_plus_is_e521() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=+4");
    assert_eq!(
        last_status(&f),
        "E521: Number required after =: ts=+4",
        "`+4` is not a legal number (oracle E521): {:?}",
        f.host.statuses
    );
}

// oracle: `-0` parses as 0, so `:set ts=-0` → E487 (tabstop must be positive), while `:set sw=-0`
// → accepted (0 is legal). A sign-flag check would misjudge `-0` as negative.
#[test]
fn set_negative_zero_follows_zero_rules() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=-0");
    assert_eq!(
        last_status(&f),
        "E487: Argument must be positive: ts=-0",
        "`:set ts=-0` = ts=0 → E487: {:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set sw=-0");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E4")),
        "`:set sw=-0` = sw=0 → legal: {:?}",
        f.host.statuses
    );
}

// oracle: E487/E474 messages quote the name **the user typed** (alias or full name):
//   `:set tabstop=0` → "E487: Argument must be positive: tabstop=0"
// The old engine hardcoded the name as "ts".
#[test]
fn set_error_message_quotes_typed_name() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set tabstop=0");
    assert_eq!(
        last_status(&f),
        "E487: Argument must be positive: tabstop=0",
        "the message should quote the typed name tabstop: {:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set tabstop=10000");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument: tabstop=10000",
        "the message should quote the typed name tabstop: {:?}",
        f.host.statuses
    );
}

// oracle: the real upper bound of 'tabstop' is 9999 (`:set ts=9999` accepted, `:set ts=10000` →
// "E474: Invalid argument: ts=10000"); the engine's old ceiling of 1e6 let 10000 slip through.
#[test]
fn set_tabstop_ceiling_is_9999() {
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=9999");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E474")),
        "`:set ts=9999` is legal: {:?}",
        f.host.statuses
    );
    let mut f = Fixture::new("a\n");
    ex(&mut f, "set ts=10000");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument: ts=10000",
        "`:set ts=10000` exceeds the ceiling (oracle E474): {:?}",
        f.host.statuses
    );
}

// ======================================================== :marks filtering

// oracle (redir): `:marks z` (z is a legal but unset name) →
//   E283: No marks matching "z" (no header)
// Old engine behavior: unconditionally printed the header first, then listed no lines, silently.
#[test]
fn marks_unknown_filter_reports_e283() {
    let mut f = Fixture::new("aa\n");
    ex(&mut f, "marks z");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "E283: No marks matching \"z\""),
        "`:marks z` should report E283: {:?}",
        f.host.statuses
    );
    assert!(
        !f.host
            .statuses
            .iter()
            .any(|s| s == "mark line  col file/text"),
        "no header should be printed on E283: {:?}",
        f.host.statuses
    );
}

// oracle: `:marks az` (a exists, z does not) → header + the a line, no E283.
#[test]
fn marks_partial_filter_lists_existing_without_error() {
    let mut f = Fixture::new("aa\nbb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "marks az");
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s == "mark line  col file/text"),
        "`:marks az` should print the header: {:?}",
        f.host.statuses
    );
    assert!(
        f.host
            .statuses
            .iter()
            .any(|s| s.starts_with(" a") && s.contains("aa")),
        "`:marks az` should list a: {:?}",
        f.host.statuses
    );
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E283")),
        "`:marks az` has hits, must not report E283: {:?}",
        f.host.statuses
    );
}

// ======================================================== :delmarks

// oracle: `:2m/c` (a **search address** as target) → line 2 moves after the /c hit line:
//   [a,c,b,d,e], no error. The old engine treated `/c` as an invalid address and reported E16.
#[test]
fn move_to_search_address() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/c");
    assert_eq!(f.text(), "a\nc\nb\nd\ne\n", "`:2m/c` should move after /c");
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E16")),
        "a search address is legal: {:?}",
        f.host.statuses
    );
    // `:2t/c` copies after the /c hit
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2t/c");
    assert_eq!(f.text(), "a\nb\nc\nb\nd\ne\n", "`:2t/c` should copy after /c");
}

// oracle: the error messages for `:m` destination addresses each differ (the old engine always gave E16):
//   `:2m'a` (unset) → E20: Mark not set; `:2m'@` → E78: Unknown mark;
//   `:2m/` (no previous search) → E35: No previous regular expression;
//   `:2m/zzz` (no hit) → E486: Pattern not found: zzz.
#[test]
fn move_destination_error_messages() {
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m'a");
    assert_eq!(last_status(&f), "E20: Mark not set");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m'@");
    assert_eq!(last_status(&f), "E78: Unknown mark");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/");
    assert_eq!(last_status(&f), "E35: No previous regular expression");
    let mut f = Fixture::new("a\nb\nc\nd\ne\n");
    ex(&mut f, "2m/zzz");
    assert_eq!(last_status(&f), "E486: Pattern not found: zzz");
}

// Old engine behavior: treated `!` as "clear a-z", then deleted x anyway, with no error.
#[test]
fn delmarks_bang_with_argument_is_e474() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks! x");
    assert_eq!(
        last_status(&f),
        "E474: Invalid argument",
        "`:delmarks! x` should report E474: {:?}",
        f.host.statuses
    );
    // a was not deleted (the whole command failed)
    f.feed([":", "'", "a", "<CR>"]);
    assert_eq!(f.line(), 0, "after the failure mark a is still in place");
}

// oracle: `:delmarks !` (the ! after a space is not forceit, it is an invalid character) →
//   E475: Invalid argument: !
// Old engine behavior: the token "!" triggered clearing a-z.
#[test]
fn delmarks_spaced_bang_is_e475() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks !");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: !",
        "`:delmarks !` should report E475: {:?}",
        f.host.statuses
    );
}

// oracle: `:delmarks @` → E475: Invalid argument: @ (the old engine silently ignored it).
// `:delmarks a @` → deletes a first, then E475 at @ (message = the remaining arguments from that byte on).
#[test]
fn delmarks_invalid_char_is_e475_with_tail() {
    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks @");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: @",
        "`:delmarks @` should report E475: {:?}",
        f.host.statuses
    );

    let mut f = Fixture::new("a\nb\n");
    f.feed(["m", "a"]);
    ex(&mut f, "delmarks a @");
    assert_eq!(
        last_status(&f),
        "E475: Invalid argument: @",
        "`:delmarks a @` should report E475 (tail = @): {:?}",
        f.host.statuses
    );
}

// oracle: `:delmarks ab` is processed character by character → deleting both a and b (the old
// engine split on whitespace and then treated "ab" as a trailing-character error).
#[test]
fn delmarks_concatenated_names_delete_each() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed(["m", "a", "j", "m", "b", "j", "m", "c"]);
    ex(&mut f, "delmarks ab");
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "a should be deleted (jump reports E20): {:?}",
        f.host.statuses
    );
    f.feed([":", "'", "c", "<CR>"]);
    assert_eq!(f.line(), 2, "c was not deleted and is still jumpable to");
}

// oracle: `"` starts a comment and terminates the arguments → `:delmarks a " note` deletes only a
// (not mistakenly c/o/m/e/n/t), and `:delmarks "` → E471.
#[test]
fn delmarks_double_quote_starts_comment() {
    let mut f = Fixture::new("a\nb\nc\n");
    f.feed(["m", "a", "j", "j", "m", "c"]);
    ex(&mut f, "delmarks a \" note");
    f.feed([":", "'", "a", "<CR>"]);
    assert!(
        f.host.statuses.iter().any(|s| s.contains("E20")),
        "a should be deleted: {:?}",
        f.host.statuses
    );
    f.feed([":", "'", "c", "<CR>"]);
    assert_eq!(f.line(), 2, "c must not be deleted by the comment text");

    let mut f = Fixture::new("a\nb\n");
    ex(&mut f, "delmarks \"");
    assert_eq!(
        last_status(&f),
        "E471: Argument required",
        "`:delmarks \"` has empty arguments after the comment → E471: {:?}",
        f.host.statuses
    );
}
