//! M5 groundwork: the `:action` bridge on the read-only pipeline.
//!
//! A vimrc-style `:map <Leader>h :action X<CR>` must complete the same way
//! on a read-only pane (a viewer) as on an editor — `RoHost` forwards the
//! host-action hook instead of dropping it.

use std::cell::RefCell;
use std::rc::Rc;

use vimcore::buffer::{VimBuffer, VimBufferMut};
use vimcore::host::VimHostRo;
use vimcore::key::Key;
use vimcore::state::{CtxRo, KeyResult, VimState};

struct B(String);
impl VimBuffer for B {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn line_count(&self) -> usize {
        self.0.lines().count()
    }
    fn char_at(&self, o: usize) -> Option<char> {
        self.0[o..].chars().next()
    }
    fn prev_char_offset(&self, o: usize) -> Option<usize> {
        Some(o.saturating_sub(1)).filter(|_| o > 0)
    }
    fn line_range(&self, l: usize) -> std::ops::Range<usize> {
        let start: usize = self.0.lines().take(l).map(|s| s.len() + 1).sum();
        let len = self.0.lines().nth(l).map(|s| s.len()).unwrap_or(0);
        start..start + len + 1
    }
    fn offset_to_line(&self, o: usize) -> usize {
        self.0[..o].matches('\n').count()
    }
    fn slice(&self, r: std::ops::Range<usize>) -> String {
        self.0[r].to_string()
    }
}
impl VimBufferMut for B {
    fn insert_text(&mut self, o: usize, t: &str) -> bool {
        self.0.insert_str(o, t);
        true
    }
    fn delete_range(&mut self, r: std::ops::Range<usize>) -> bool {
        let _ = self.0.drain(r);
        true
    }
}

/// A read-only host: pins the `:action` bridge on the RO half (the mapping
/// chord must complete the same way on a viewer).
#[derive(Default)]
struct RoH {
    actions: Rc<RefCell<Vec<String>>>,
}
impl VimHostRo for RoH {
    fn viewport(&self) -> (usize, usize) {
        (0, 30)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn set_search_highlights(
        &mut self,
        _: &[std::ops::Range<usize>],
        _: Option<std::ops::Range<usize>>,
    ) {
    }
    fn dispatch_host_action_hinted(&mut self, id: &str, _strict: bool) {
        self.actions.borrow_mut().push(id.to_string());
    }
}

#[test]
fn ro_pipeline_completes_an_action_mapping() {
    // The RO host's action bridge: a vimrc-style `:action` mapping completes
    // on a read-only pane through handle_key_ro + the forwarded hook.
    let mut vim = VimState::new();
    vim.apply_config(&vimcore::config::parse("map <Leader>h :action TestB<CR>"));
    let buf = B("hello\nworld\n".to_string());
    let mut host = RoH::default();
    {
        let ro: &dyn VimBuffer = &buf;
        let mut ctx = CtxRo {
            buf: ro,
            host: &mut host,
        };
        let leader = Key::parse("\\");
        assert_eq!(vim.handle_key_ro(&mut ctx, leader), KeyResult::Consumed);
        assert!(
            vim.has_pending_prefix(),
            "the leader is held on the RO pipeline too"
        );
        vim.handle_key_ro(&mut ctx, Key::parse("h"));
    }
    assert_eq!(
        *host.actions.borrow(),
        vec!["TestB".to_string()],
        "the RO host received the action"
    );
}
