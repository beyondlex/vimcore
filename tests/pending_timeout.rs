//! M5: the engine-side timeout adjudication for held mapping prefixes.
//!
//! `has_pending_prefix` is the host's timer-arming probe; `pending_timeout`
//! settles the held keys AS TYPED (vim's `'timeoutlen'` flush). The tests
//! here pin the four semantic edges the design called out: recording happens
//! exactly once at settle time, a held digit settles into the count, the
//! settle path is mode-specific (insert-mode printables come back declined
//! for the host to place), and the settle bypass rides the existing
//! `no_remap_left` mechanism.

use std::cell::RefCell;
use std::rc::Rc;

use vimcore::buffer::{VimBuffer, VimBufferMut};
use vimcore::host::{RoHost, VimHost, VimHostRo};
use vimcore::key::Key;
use vimcore::mode::Mode;
use vimcore::state::{Ctx, CtxRo, KeyResult, VimState};

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

#[derive(Default)]
struct H {
    actions: Rc<RefCell<Vec<String>>>,
    bells: Rc<RefCell<usize>>,
}
impl VimHost for H {
    fn viewport(&self) -> (usize, usize) {
        (0, 30)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(
        &mut self,
        _: &[std::ops::Range<usize>],
        _: Option<std::ops::Range<usize>>,
    ) {
    }
    fn bell(&mut self) {
        *self.bells.borrow_mut() += 1;
    }
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn dispatch_host_action_hinted(&mut self, id: &str, _strict: bool) {
        self.actions.borrow_mut().push(id.to_string());
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

struct F {
    vim: VimState,
    buf: B,
    host: H,
}

impl F {
    fn new(config: &str) -> Self {
        Self::with_text(config, "hello\nworld\n")
    }
    fn with_text(config: &str, text: &str) -> Self {
        let mut vim = VimState::new();
        if !config.is_empty() {
            vim.apply_config(&vimcore::config::parse(config));
        }
        F {
            vim,
            buf: B(text.to_string()),
            host: H::default(),
        }
    }
    /// Feed parsed keys one per `handle_key` call (one keystroke = one event).
    fn feed(&mut self, keys: &[&str]) -> KeyResult {
        let mut last = KeyResult::Consumed;
        for k in keys {
            let key = Key::parse(k);
            let mut ctx = Ctx {
                buf: &mut self.buf,
                host: &mut self.host,
            };
            last = self.vim.handle_key(&mut ctx, key);
        }
        last
    }
    /// Run the timeout settle (the host's timer contract, after a hold).
    fn timeout(&mut self) -> KeyResult {
        let mut ctx = Ctx {
            buf: &mut self.buf,
            host: &mut self.host,
        };
        self.vim.pending_timeout(&mut ctx)
    }
    /// Feed one key, then run the timeout settle.
    fn feed_then_timeout(&mut self, k: &str) -> KeyResult {
        self.feed(&[k]);
        self.timeout()
    }
    /// The host half of a declined printable: place the text at the cursor.
    fn place_text(&mut self, t: &str) {
        let mut ctx = Ctx {
            buf: &mut self.buf,
            host: &mut self.host,
        };
        self.vim.insert_text_at_cursor(&mut ctx, t);
    }
}

#[test]
fn mapping_completes_when_the_second_key_arrives_in_time() {
    let mut f = F::new("nmap <Space>d x");
    f.feed(&["<Space>"]);
    assert!(
        f.vim.has_pending_prefix(),
        "space is a mapping prefix: the engine holds it"
    );
    assert_eq!(f.vim.pending_prefix_keys(), vec![Key::char(' ')]);

    // The second key arrives within the timeout (no pending_timeout call):
    // the mapping fires and `x` deletes the first char.
    f.feed(&["d"]);
    assert!(!f.vim.has_pending_prefix());
    assert_eq!(f.buf.0, "ello\nworld\n");
    assert!(f.host.actions.borrow().is_empty(), "no :action involved");
}

#[test]
fn timeout_settles_the_leader_as_typed() {
    let mut f = F::new("nmap <Space>d x");
    // The timer fires with nothing following: the held space executes its
    // own meaning (move right) and the mapping never fires.
    let r = f.feed_then_timeout("<Space>");
    assert_eq!(r, KeyResult::Consumed);
    assert!(!f.vim.has_pending_prefix(), "the settle drained the queue");
    assert_eq!(
        f.vim.cursor.offset, 1,
        "the settled leader ran its builtin move-right"
    );
    assert_eq!(f.buf.0, "hello\nworld\n", "the mapping did not fire");
}

#[test]
fn timeout_then_continuation_cannot_reach_the_mapping() {
    let mut f = F::new("nmap gs :action TestA<CR>");
    f.feed(&["g"]);
    let r = f.timeout();
    assert_eq!(r, KeyResult::Consumed);
    assert!(!f.vim.has_pending_prefix());
    // `g` settled into the builtin prefix (not discarded): `s` now belongs
    // to the builtin path, which has no `gs` → bell, and the mapping's
    // action never fires.
    f.feed(&["s"]);
    assert_eq!(*f.host.bells.borrow(), 1, "gs is no builtin: bell");
    assert!(
        f.host.actions.borrow().is_empty(),
        "a timed-out mapping prefix must not complete on the next key"
    );
}

#[test]
fn no_mapping_means_no_hold() {
    // Without a mapping starting with `g`, the key falls straight through to
    // the builtin trie — the probe must report nothing pending.
    let mut f = F::new("");
    f.feed(&["g"]);
    assert!(!f.vim.has_pending_prefix());
}

#[test]
fn insert_mode_timeout_declines_the_held_printable_to_the_host() {
    let mut f = F::new("inoremap jk <Esc>");
    f.feed(&["i"]);
    assert!(matches!(f.vim.mode, Mode::Insert));
    f.feed(&["j"]);
    assert!(f.vim.has_pending_prefix(), "j of jk is held");
    assert_eq!(f.buf.0, "hello\nworld\n", "nothing inserted while held");

    let r = f.timeout();
    // Same contract as handle_key: Unknown = the engine declined the
    // printable; the host places the parked char.
    assert_eq!(r, KeyResult::Unknown);
    let parked = f.vim.take_pending_unknown_chars();
    assert_eq!(parked, vec!['j']);
    f.place_text("j");
    assert_eq!(f.buf.0, "jhello\nworld\n");
    assert!(matches!(f.vim.mode, Mode::Insert), "still inserting");
}

#[test]
fn insert_mode_jk_still_completes_in_time() {
    let mut f = F::new("inoremap jk <Esc>");
    f.feed(&["i", "j"]);
    assert!(f.vim.has_pending_prefix());
    f.feed(&["k"]);
    assert!(
        matches!(f.vim.mode, Mode::Normal),
        "jk fired: insert exited"
    );
    assert_eq!(f.buf.0, "hello\nworld\n");
}

#[test]
fn macro_capture_records_the_settled_key_exactly_once() {
    // The held key is NOT recorded while waiting (the wait happens before
    // the pipeline's record site); the settle records it once. The macro
    // must hold "gj", not "ggj" or "j".
    let mut f = F::new("nmap gs :action TestA<CR>");
    f.feed(&["q", "a"]);
    f.feed(&["g"]);
    f.timeout();
    f.feed(&["j"]); // gj builtin: one line down
    f.feed(&["q"]);
    let reg = f.vim.registers.lock().unwrap().get('a').cloned();
    let text = reg.expect("qa recorded register a").text;
    assert_eq!(text, "gj", "the settled g is captured once, in place");
}

#[test]
fn held_digit_settles_into_the_count() {
    // A mapping starting with a digit holds the digit; on timeout it is
    // interpreted AS TYPED — a count start — not discarded. `j` then moves
    // 3 lines.
    let mut f = F::with_text("nmap 3x :action TestA<CR>", "a\nb\nc\nd\ne\n");
    f.feed(&["3"]);
    assert!(f.vim.has_pending_prefix());
    f.feed_then_timeout("j");
    assert!(
        f.host.actions.borrow().is_empty(),
        "3x never completed: no action"
    );
    let third_line_start = f.buf.0.lines().take(3).map(|s| s.len() + 1).sum::<usize>();
    assert_eq!(
        f.vim.cursor.offset, third_line_start,
        "3j moved three lines down"
    );
}

#[test]
fn modified_key_while_pending_replays_the_leader_then_is_declined() {
    // vim: `<Space>` then an unrelated ctrl key — the leader executes as
    // typed, and the ctrl key (no builtin, no mapping) comes back Unknown
    // so the host can pass it to its own bindings. `<C-g>` is deliberately
    // NOT one of the engine's builtin ctrl chords (unlike C-f page-down,
    // which consumes).
    let mut f = F::new("nmap <Space>d x");
    f.feed(&["<Space>"]);
    let r = f.feed(&["<C-g>"]);
    assert_eq!(r, KeyResult::Unknown);
    assert_eq!(
        f.vim.cursor.offset, 1,
        "the held leader replayed its move-right"
    );
    assert!(f.host.actions.borrow().is_empty());
}

#[test]
fn probe_and_settle_are_noops_while_a_prompt_is_open() {
    let mut f = F::new("nmap <Space>d x");
    f.feed(&[":"]);
    assert!(
        !f.vim.has_pending_prefix(),
        "an open prompt owns the input: nothing to settle"
    );
    let r = f.timeout();
    assert_eq!(r, KeyResult::Consumed);
    // The prompt is still open with its buffer untouched (pending_timeout's
    // guard must not have disturbed it).
    assert!(matches!(f.vim.mode, Mode::CommandLine { prompt: ':' }));
}

#[test]
fn drop_discards_the_held_keys_without_replay() {
    // The host's cancel path (session end / pane switch): the held leader is
    // dropped, NOT executed — no phantom motion — and the engine is clean
    // for the next keystroke.
    let mut f = F::new("nmap <Space>d x");
    f.feed(&["<Space>"]);
    assert!(f.vim.has_pending_prefix());
    f.vim.drop_pending_prefix();
    assert!(!f.vim.has_pending_prefix());
    assert_eq!(f.vim.cursor.offset, 0, "nothing replayed");
    assert_eq!(f.buf.0, "hello\nworld\n");
    // the next key is processed normally (the budgets are clean)
    f.feed(&["x"]);
    assert_eq!(f.buf.0, "ello\nworld\n");
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
