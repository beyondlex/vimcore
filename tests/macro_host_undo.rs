//! The MACRO_HOST_UNDO capability gate: `u`/`<C-r>` inside a macro replay
//! reaches the host only when the host declared replay-safe undo.

mod common;

use common::BufferView;
use vimcore::host::{HostCapabilities, VimHost};
use vimcore::key::Key;
use vimcore::state::{Ctx, VimState};

/// A host with a WORKING undo stack whose replay capability is togglable —
/// the two axes the gate must keep apart (an undeclared host with working
/// undo must still not be asked).
struct GateHost {
    text: std::rc::Rc<std::cell::RefCell<String>>,
    declares_undo: bool,
    undo_calls: usize,
    undo_stack: Vec<(String, usize)>,
    open_group: Option<u64>,
    bells: usize,
}

impl VimHost for GateHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 9)
    }
    fn scroll_to_line(&mut self, _line: usize) {}
    fn clipboard_write(&mut self, _text: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(
        &mut self,
        _matches: &[std::ops::Range<usize>],
        _current: Option<std::ops::Range<usize>>,
    ) {
    }
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn begin_undo_group(&mut self, id: u64, cursor_offset: usize) {
        if self.open_group != Some(id) {
            self.undo_stack
                .push((self.text.borrow().clone(), cursor_offset));
            self.open_group = Some(id);
        }
    }
    fn undo(&mut self) -> Option<usize> {
        self.undo_calls += 1;
        let (text, cursor) = self.undo_stack.pop()?;
        *self.text.borrow_mut() = text;
        Some(cursor)
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            macro_host_undo: self.declares_undo,
        }
    }
}

fn run_macro_with_undo(declares_undo: bool) -> GateHost {
    let shared = std::rc::Rc::new(std::cell::RefCell::new("ab\n".to_string()));
    let mut buf = BufferView(shared.clone());
    let mut host = GateHost {
        text: shared.clone(),
        declares_undo,
        undo_calls: 0,
        undo_stack: Vec::new(),
        open_group: None,
        bells: 0,
    };
    let mut vim = VimState::new();
    let keys = ["q", "a", "x", "u", "q", "@", "a"];
    for k in keys {
        let mut ctx = Ctx {
            buf: &mut buf,
            host: &mut host,
        };
        vim.handle_key(&mut ctx, Key::parse(k));
    }
    host
}

/// Without the declaration the engine answers the replayed `u` itself
/// (bell, no host call) — a host whose undo answers from view state the
/// replay never reaches must not be asked mid-macro. The ONE host call is
/// the recording phase's direct `u` (a plain keypress, ungated).
#[test]
fn undeclared_host_is_not_asked_to_undo_during_replay() {
    let host = run_macro_with_undo(false);
    assert_eq!(host.undo_calls, 1, "only the recording-phase u asks the host");
    assert!(host.bells >= 1, "the replayed u bells");
    assert_eq!(
        host.text.borrow().as_str(),
        "b\n",
        "the replay's x stays applied (its u never reached the host)"
    );
}

/// A declared host keeps macro-undo: the replay's `x` applies and its `u`
/// asks the host, restoring the text (one call for the recording-phase u,
/// one for the replayed u).
#[test]
fn declared_host_gets_macro_undo() {
    let host = run_macro_with_undo(true);
    assert_eq!(host.undo_calls, 2, "recording u + replayed u both ask");
    assert_eq!(
        host.text.borrow().as_str(),
        "ab\n",
        "the replayed undo restored the pre-x text"
    );
}

/// Outside a replay the gate does not apply: a plain `u` asks the host
/// regardless of the declaration (that contract predates the flag).
#[test]
fn plain_undo_ignores_the_gate() {
    let shared = std::rc::Rc::new(std::cell::RefCell::new("ab\n".to_string()));
    let mut buf = BufferView(shared.clone());
    let mut host = GateHost {
        text: shared.clone(),
        declares_undo: false,
        undo_calls: 0,
        undo_stack: Vec::new(),
        open_group: None,
        bells: 0,
    };
    let mut vim = VimState::new();
    for k in ["x", "u"] {
        let mut ctx = Ctx {
            buf: &mut buf,
            host: &mut host,
        };
        vim.handle_key(&mut ctx, Key::parse(k));
    }
    assert_eq!(host.undo_calls, 1, "a direct u still reaches the host");
    assert_eq!(host.text.borrow().as_str(), "ab\n");
}
