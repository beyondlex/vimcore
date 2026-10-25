//! The host-command Ex registry: registration, dispatch through the real
//! `:` pipeline, precedence over built-ins, ranges, completion.

mod common;

use common::{edit, Fixture};
use std::cell::RefCell;
use std::rc::Rc;
use vimcore::ex::{ExCompletion, ExHandler, ExInvocation};
use vimcore::state::VimState;

/// A recording handler: captures every invocation, optionally mutates the
/// buffer (like a real product command), returns the scripted outcome.
struct Recorder {
    calls: RefCell<Vec<ExInvocation>>,
    /// What to do on call: (text to insert at cursor, success).
    action: Option<(String, bool)>,
}

impl Recorder {
    fn handler(self: &Rc<Self>) -> ExHandler {
        let me = Rc::clone(self);
        Rc::new(move |vim: &mut VimState, ctx, inv| {
            me.calls.borrow_mut().push(inv.clone());
            match &me.action {
                Some((text, ok)) => {
                    let offset = vim.cursor.offset;
                    ctx.buf.insert_text(offset, text);
                    ctx.host.changed();
                    *ok
                }
                None => true,
            }
        })
    }
}

fn register_recorder(vim: &mut VimState, name: &str, aliases: &[&str]) -> Rc<Recorder> {
    let rec = Rc::new(Recorder {
        calls: RefCell::new(Vec::new()),
        action: None,
    });
    vim.register_ex(name, aliases, None, rec.handler());
    rec
}

/// The full pipeline: `:w-custom<CR>` reaches the registered handler with
/// the argument and no range.
#[test]
fn host_command_runs_through_the_cmdline_pipeline() {
    let mut f = Fixture::new("one\ntwo\n");
    let rec = register_recorder(&mut f.vim, "w-custom", &[]);
    f.feed([
        ":", "w", "-", "c", "u", "s", "t", "o", "m", " ", "a", "r", "g", "\r",
    ]);
    let calls = rec.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "w-custom");
    assert_eq!(calls[0].arg, "arg");
    assert_eq!(calls[0].range, None);
    assert!(!calls[0].bang);
}

/// Aliases dispatch as the registered NAME; `!` arrives as bang.
#[test]
fn alias_dispatches_and_bang_is_stripped() {
    let mut f = Fixture::new("one\n");
    let rec = register_recorder(&mut f.vim, "hunk-take", &["ht"]);
    f.feed([":", "h", "t", "!", "\r"]);
    let calls = rec.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "hunk-take", "the alias resolves to the name");
    assert!(calls[0].bang);
    assert_eq!(calls[0].arg, "");
}

/// `:2,3sort` hands the resolved 0-based inclusive range to the handler.
#[test]
fn range_is_parsed_by_the_engine_and_handed_over() {
    let mut f = Fixture::new("1\n2\n3\n4\n");
    let rec = register_recorder(&mut f.vim, "sort", &[]);
    f.feed([":", "2", ",", "3", "s", "o", "r", "t", "\r"]);
    let calls = rec.calls.borrow();
    assert_eq!(calls[0].range, Some((1, 2)));
    assert_eq!(calls[0].addr_count, 2);
}

/// A registration claiming `w` takes the spelling away from the built-in
/// `:w` (host.save stays uncalled) — but `:write` still reaches the
/// built-in, because `w` is not a boundary match inside `write`.
#[test]
fn host_registration_wins_over_the_builtin_but_not_its_longer_spellings() {
    let mut f = Fixture::new("one\n");
    register_recorder(&mut f.vim, "w", &[]);
    f.feed([":", "w", "\r"]);
    assert_eq!(f.host.saved, 0, "the registration owns the `w` spelling");
    f.feed([":", "w", "r", "i", "t", "e", "\r"]);
    assert_eq!(f.host.saved, 1, ":write stays with the built-in table");
}

/// Glued arguments: a registration named `s` sees `s/a/b/` with the
/// separator kept (`/a/b/`), like the built-in `:s` syntax family.
#[test]
fn glued_argument_keeps_its_separator() {
    let mut f = Fixture::new("one\n");
    let rec = register_recorder(&mut f.vim, "s-glue", &[]);
    // spell it with a glued arg through the full pipeline: `:s-glue/x/y/`
    f.feed([
        ":", "s", "-", "g", "l", "u", "e", "/", "x", "/", "y", "/", "\r",
    ]);
    let calls = rec.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arg, "/x/y/");
    assert!(!calls[0].bang);
}

/// A failing handler (false) bells and aborts a surrounding macro replay —
/// the same contract as a built-in error.
#[test]
fn failing_handler_bells_like_a_builtin_error() {
    let mut f = Fixture::new("one\n");
    let rec = Rc::new(Recorder {
        calls: RefCell::new(Vec::new()),
        action: Some(("X".to_string(), false)),
    });
    f.vim.register_ex("boom", &[], None, rec.handler());
    f.feed([":", "b", "o", "o", "m", "\r"]);
    assert_eq!(f.text(), "Xone\n", "the handler's edit landed");
    assert_eq!(f.host.bells, 1, "the false return bells");
}

/// The `|` separator runs a host head and a built-in tail in one line.
#[test]
fn bar_separator_runs_host_then_builtin() {
    let mut f = Fixture::new("one\n");
    let rec = register_recorder(&mut f.vim, "hostcmd", &[]);
    f.feed([":", "h", "o", "s", "t", "c", "m", "d", "|", "w", "\r"]);
    assert_eq!(rec.calls.borrow().len(), 1);
    assert_eq!(f.host.saved, 1, "the tail after | still ran");
}

/// Unknown commands keep vim's E492.
#[test]
fn unknown_command_still_e492() {
    let f = edit(
        "one\n",
        0,
        0,
        &[":", "n", "o", "s", "u", "c", "h", "c", "m", "d", "\r"],
    );
    assert!(
        f.host.statuses.last().unwrap().starts_with("E492"),
        "got {:?}",
        f.host.statuses.last()
    );
}

/// Completion: word-prefix lists names and aliases; an exact command match
/// routes to the entry's argument completion.
#[test]
fn completions_list_names_then_route_to_argument_completion() {
    let mut vim = VimState::new();
    let rec = register_recorder(&mut vim, "hunk-take", &["ht"]);
    let _ = rec;
    vim.register_ex(
        "config",
        &[],
        Some(Rc::new(|arg: &str| {
            ["editor", "theme"]
                .iter()
                .map(|s| s.to_string())
                .filter(|s| s.starts_with(arg))
                .collect::<Vec<_>>()
        }) as ExCompletion),
        Rc::new(|_, _, _| true),
    );
    let names = vim.ex_completions("h");
    assert!(names.contains(&"hunk-take".to_string()));
    assert!(names.contains(&"ht".to_string()));
    assert!(!names.contains(&"config".to_string()), "prefix filters");
    // exact match + space routes to argument completion
    assert_eq!(
        vim.ex_completions("config th"),
        vec!["theme".to_string()],
        "argument completion sees the partial arg"
    );
    assert_eq!(vim.ex_completions("config "), vec!["editor", "theme"]);
}

/// `@:` replays the last executed line — a host command recorded through
/// the shared Enter path replays like a built-in.
#[test]
fn at_colon_replays_a_host_command() {
    let mut f = Fixture::new("one\n");
    let rec = register_recorder(&mut f.vim, "again", &[]);
    f.feed([":", "a", "g", "a", "i", "n", "\r"]);
    f.feed(["@", ":"]);
    assert_eq!(
        rec.calls.borrow().len(),
        2,
        "the host command replayed via @:"
    );
}

/// Re-registering a name replaces the handler (host-layer rebinding).
#[test]
fn reregistration_replaces() {
    let mut f = Fixture::new("one\n");
    let first = register_recorder(&mut f.vim, "dup", &[]);
    let second = register_recorder(&mut f.vim, "dup", &[]);
    f.feed([":", "d", "u", "p", "\r"]);
    assert_eq!(first.calls.borrow().len(), 0);
    assert_eq!(second.calls.borrow().len(), 1);
}

/// Host commands run with 'modifiable' off unless they check it themselves
/// — product commands like `:w` are not buffer edits. The handler decides.
#[test]
fn host_commands_are_not_auto_blocked_by_modifiable() {
    let mut f = Fixture::new("one\n");
    f.vim.set_modifiable(false);
    let rec = register_recorder(&mut f.vim, "nop", &[]);
    f.feed([":", "n", "o", "p", "\r"]);
    assert_eq!(rec.calls.borrow().len(), 1, "the handler was consulted");
}
