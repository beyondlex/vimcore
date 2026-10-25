//! SharedGlobals (方案 B of the multi-panel split): registers and macros are
//! session-global across engine instances; everything else stays per
//! instance.

mod common;

use common::{edit, BufferView, Fixture};
use vimcore::state::{SharedGlobals, VimState};

/// Two independent fixtures whose engines share one globals session.
struct Panels {
    left: Fixture,
    right: Fixture,
}

fn shared_panels() -> Panels {
    let globals = SharedGlobals::new();
    let mk = |text: &str| {
        let mut f = Fixture::new(text);
        f.vim.share_globals(&globals);
        f
    };
    Panels {
        left: mk("alpha\nbeta\n"),
        right: mk("RIGHT\n"),
    }
}

fn feed(f: &mut Fixture, keys: &[&str]) {
    f.feed(keys.iter().copied());
}

/// yy in one panel, p in the other: the yank travels through the shared
/// register file — the cross-panel workflow the single-engine model used to
/// get for free.
#[test]
fn yank_in_one_panel_pastes_in_the_other() {
    let mut p = shared_panels();
    feed(&mut p.left, &["y", "y"]);
    feed(&mut p.right, &["p"]);
    assert_eq!(
        p.right.text(),
        "RIGHT\nalpha\n",
        "the shared unnamed register carries the linewise yank across panels"
    );
}

/// An explicit register is shared too ("ayy here, "ap there).
#[test]
fn named_register_is_shared() {
    let mut p = shared_panels();
    feed(&mut p.left, &["\"", "a", "y", "y"]);
    feed(&mut p.right, &["\"", "a", "p"]);
    assert_eq!(p.right.text(), "RIGHT\nalpha\n");
}

/// A macro recorded in one panel plays in the other — the macro table is
/// part of the session globals.
#[test]
fn macro_recorded_in_one_panel_plays_in_the_other() {
    let mut p = shared_panels();
    // record into a: one x
    feed(&mut p.left, &["q", "a", "x", "q"]);
    // play it twice in the right panel
    feed(&mut p.right, &["2", "@", "a"]);
    assert_eq!(
        p.right.text(),
        "GHT\n",
        "the recorded macro executed twice in the other panel"
    );
}

/// Undo stays per panel: editing the left panel never lets `u` in the right
/// panel reach the left's history.
#[test]
fn undo_stays_per_panel() {
    let mut p = shared_panels();
    feed(&mut p.left, &["x"]);
    feed(&mut p.right, &["x"]);
    feed(&mut p.right, &["u"]);
    assert_eq!(p.left.text(), "lpha\nbeta\n", "left's edit untouched");
    assert_eq!(p.right.text(), "RIGHT\n", "right undid its own edit only");
    // and the left can still undo its own edit
    feed(&mut p.left, &["u"]);
    assert_eq!(p.left.text(), "alpha\nbeta\n");
}

/// Unshared instances keep private globals: a yank does not leak into a
/// fresh engine (the default single-engine host sees no behavior change).
#[test]
fn unshared_instances_do_not_leak_registers() {
    let f = edit("alpha\n", 0, 0, &["y", "y"]);
    let mut other = Fixture::new("RIGHT\n");
    feed(&mut other, &["p"]);
    assert_eq!(other.text(), "RIGHT\n", "no leak without share_globals");
    // sharing later still works: `shared_globals()` hands out the session
    let globals = f.vim.shared_globals();
    other.vim.share_globals(&globals);
    feed(&mut other, &["p"]);
    assert_eq!(other.text(), "RIGHT\nalpha\n");
}

/// The last-insert register `".` is shared: vim keeps it session-global
/// (the insert text is a register like any other).
#[test]
fn last_insert_register_is_shared() {
    let mut p = shared_panels();
    feed(&mut p.left, &["i"]);
    p.left.type_text("ZZ");
    feed(&mut p.left, &["escape"]);
    // `".` never points the unnamed register at itself — the explicit
    // prefix is how a paste reads it
    feed(&mut p.right, &["\"", ".", "p"]);
    assert_eq!(
        p.right.text(),
        "RZZIGHT\n",
        "the last-insert register crossed panels"
    );
}

/// Two pagers plus an editor over one text: the read-only pipeline also
/// accepts shared globals (handle_key_ro shares the same fields).
#[test]
fn readonly_pipeline_shares_registers() {
    use vimcore::host::VimHostRo;
    use vimcore::state::CtxRo;
    let globals = SharedGlobals::new();

    let mut ro_vim = VimState::new();
    ro_vim.share_globals(&globals);
    let ro_buf = BufferView::from_text("alpha\nbeta\n");

    struct RoView;
    impl VimHostRo for RoView {
        fn viewport(&self) -> (usize, usize) {
            (0, 9)
        }
        fn scroll_to_line(&mut self, _line: usize) {}
        fn clipboard_write(&mut self, _text: &str) {}
        fn set_search_highlights(
            &mut self,
            _matches: &[std::ops::Range<usize>],
            _current: Option<std::ops::Range<usize>>,
        ) {
        }
    }
    let mut ro_host = RoView;

    // yank a word through the read-only pipeline
    for k in ["y", "i", "w"] {
        let key = vimcore::key::Key::parse(k);
        let mut ctx = CtxRo {
            buf: &ro_buf,
            host: &mut ro_host,
        };
        ro_vim.handle_key_ro(&mut ctx, key);
    }

    // an editable engine on the same session pastes it
    let mut w = Fixture::new("RIGHT\n");
    w.vim.share_globals(&globals);
    feed(&mut w, &["p"]);
    assert_eq!(
        w.text(),
        "RalphaIGHT\n",
        "the pager's charwise yank reached the editor through the shared registers"
    );
}
