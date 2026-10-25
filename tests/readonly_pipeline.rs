//! Behaviour tests for the read-only pipeline (PLAN_READONLY P1 / P2).
//!
//! Covers three things:
//!
//! 1. `handle_key_ro` + `CtxRo`: a read-only host (`impl VimBuffer` +
//!    `impl VimHostRo`, zero write primitives) gets the full motion / text
//!    object / search / mark / scroll / yank capability;
//! 2. with 'modifiable' off, editing commands are refused with vim's E21, and
//!    no write primitive is ever called (the fuzz case asserts this hard with a
//!    buffer whose write primitives panic);
//! 3. the `MotionHook` interception point: a host-owned motion takes effect on
//!    all three paths at once -- plain movement, pending operator and visual
//!    extension.
//!
//! The E21 key semantics follow the vim 9.1 evidence (under `:set nomodifiable`
//! all of `x` `dw` `dd` `i` `o` `~` `p` `J` `u` `>` `:s` `:d` `:j` `:m` `:pu`
//! report E21; `y` `yy` `m{a}` `:q` do not).

mod common;

use std::ops::Range;

use common::BufferView;
use vimcore::buffer::{VimBuffer, VimBufferMut};
use vimcore::host::{MotionHook, VimHost, VimHostRo};
use vimcore::key::Key;
use vimcore::mode::Mode;
use vimcore::motions::{Motion, MotionKind, MotionResult};
use vimcore::state::{Ctx, CtxRo, VimState};
use vimcore::tck::TckHostRo;

// ---- read-only host -------------------------------------------------------

/// A "fold-aware" host: the lines in `hidden` are folded away, so j/k must step
/// over them. This is exactly how the alacer diff view's `]c`/`[c` and
/// visible-line movement plug into the engine side.
struct HiddenLineHook {
    hidden: Vec<usize>,
}

impl MotionHook for HiddenLineHook {
    fn intercept_motion(
        &mut self,
        vim: &VimState,
        buf: &dyn VimBuffer,
        motion: Motion,
        count: usize,
    ) -> Option<MotionResult> {
        let down = match motion {
            Motion::Down => true,
            Motion::Up => false,
            _ => return None,
        };
        let from = buf.offset_to_line(vim.cursor.offset);
        let total = buf.line_count();
        let visible = |line: usize| !self.hidden.contains(&line);
        let mut target = from;
        let mut moved = false;
        for _ in 0..count {
            let next = if down {
                (target + 1).min(total.saturating_sub(1))
            } else {
                target.saturating_sub(1)
            };
            if next == target {
                break;
            }
            target = next;
            while target + 1 < total && !visible(target) && down {
                target += 1;
            }
            while target > 0 && !visible(target) && !down {
                target -= 1;
            }
            if (down && target > from) || (!down && target < from) {
                moved = true;
            }
        }
        if !moved {
            // hand it back to the engine: the stuck semantics of `j` on the last
            // line is already given by the built-in implementation, the host does
            // not have to replicate it (returning None means "I do not own this
            // one").
            return None;
        }
        Some(MotionResult {
            offset: buf.line_start(target),
            kind: MotionKind::Linewise,
            moved: true,
            till_target: None,
        })
    }
}

struct RoHost {
    statuses: Vec<String>,
    bells: usize,
    clipboard: Option<String>,
    highlights: Vec<Range<usize>>,
    current: Option<Range<usize>>,
    close_requested: bool,
    hook: Option<HiddenLineHook>,
}

impl RoHost {
    fn new() -> Self {
        RoHost {
            statuses: Vec::new(),
            bells: 0,
            clipboard: None,
            highlights: Vec::new(),
            current: None,
            close_requested: false,
            hook: None,
        }
    }
}

impl VimHostRo for RoHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, _line: usize) {}
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current = current;
    }
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
    }
    fn motion_hook(&mut self) -> Option<&mut dyn MotionHook> {
        self.hook.as_mut().map(|h| h as &mut dyn MotionHook)
    }
}

// The same type also implements the full `VimHost`, so the RW/RO equivalence
// replay can share one set of host semantics.
impl VimHost for RoHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, _line: usize) {}
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn clipboard_read(&self) -> Option<String> {
        self.clipboard.clone()
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current = current;
    }
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
    }
    fn motion_hook(&mut self) -> Option<&mut dyn MotionHook> {
        self.hook.as_mut().map(|h| h as &mut dyn MotionHook)
    }
}

fn feed_ro(vim: &mut VimState, buf: &BufferView, host: &mut RoHost, keys: &[&str]) {
    let buf: &dyn VimBuffer = buf;
    let mut ctx = CtxRo { buf, host };
    for k in keys {
        vim.handle_key_ro(&mut ctx, Key::parse(k));
    }
}

fn feed_rw(vim: &mut VimState, buf: &mut BufferView, host: &mut RoHost, keys: &[&str]) {
    let mut ctx = Ctx { buf, host };
    for k in keys {
        vim.handle_key(&mut ctx, Key::parse(k));
    }
}

// ---- 1. read-only pipeline: all viewing capability stays, edits get E21 ----

#[test]
fn ro_pipeline_keeps_motions_search_marks_and_yank() {
    let buf = BufferView::from_text("alpha\nbeta\ngamma\n");
    let mut host = RoHost::new();
    let mut vim = VimState::new();
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    assert_eq!(vim.cursor_offset(), 0);
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
    feed_ro(&mut vim, &buf, &mut host, &["G"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 2);
    // search
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    feed_ro(&mut vim, &buf, &mut host, &["/", "b", "e", "t", "a", "<CR>"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
    assert!(!host.highlights.is_empty(), "search highlights must still be published to the host");
    // marks (vim: `m{a}` is legal under nomodifiable)
    feed_ro(&mut vim, &buf, &mut host, &["m", "a"]);
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    feed_ro(&mut vim, &buf, &mut host, &["'", "a"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
    // yank (vim: `y` is legal under nomodifiable) -- writes the register + clipboard
    feed_ro(&mut vim, &buf, &mut host, &["\"", "+", "y", "y"]);
    assert_eq!(host.clipboard.as_deref(), Some("beta\n"));
    assert_eq!(
        vim.registers.lock().unwrap().get('+').cloned().map(|r| r.text.clone()).unwrap_or_default(),
        "beta\n"
    );
    // scrolling
    feed_ro(&mut vim, &buf, &mut host, &["z", "z"]);
    host.statuses.clear();
    feed_ro(&mut vim, &buf, &mut host, &["/", "g", "a", "m", "m", "a", "<CR>"]);
    assert!(
        !host.statuses.iter().any(|s| s.contains("E21")),
        "viewing keys must not report E21: {:?}",
        host.statuses
    );
}

#[test]
fn ro_pipeline_refuses_edits_with_e21_and_leaves_the_text_alone() {
    let buf = BufferView::from_text("alpha\nbeta\ngamma\n");
    let before = buf.slice(0..buf.len());
    let mut vim = VimState::new();
    for keys in [
        vec!["x"],
        vec!["d", "w"],
        vec!["d", "d"],
        vec!["i"],
        vec!["o"],
        vec!["~"],
        vec!["p"],
        vec!["J"],
        vec!["u"],
        vec![">", ">"],
        vec![":", "s", "/", "a", "/", "b", "/", "<CR>"],
        vec![":", "%", "d", "<CR>"],
        vec!["v", "l", "d"],
    ] {
        let mut host = RoHost::new();
        feed_ro(&mut vim, &buf, &mut host, &keys);
        assert_eq!(buf.slice(0..buf.len()), before, "{keys:?} changed the buffer");
        assert!(
            host.statuses.iter().any(|s| s.contains("E21")),
            "{keys:?} did not report E21: {:?}",
            host.statuses
        );
        feed_ro(&mut vim, &buf, &mut host, &["<Esc>"]);
    }
    // the engine is still usable after the refusals
    let mut host = RoHost::new();
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    assert_eq!(vim.cursor_offset(), 0);
}

#[test]
fn set_modifiable_is_a_real_option_spelling() {
    let mut buf = BufferView::from_text("alpha\n");
    let mut host = RoHost::new();
    let mut vim = VimState::new();
    assert!(vim.is_modifiable());

    feed_rw(&mut vim, &mut buf, &mut host, &[":", "s", "e", "t", " ", "n", "o", "m", "a", "<CR>"]);
    assert!(!vim.is_modifiable(), ":set noma did not turn 'modifiable' off");
    host.statuses.clear();
    feed_rw(&mut vim, &mut buf, &mut host, &[":", "s", "e", "t", " ", "m", "a", "?", "<CR>"]);
    assert_eq!(host.statuses.last().map(|s| s.as_str()), Some("nomodifiable"));

    host.statuses.clear();
    feed_rw(&mut vim, &mut buf, &mut host, &["x"]);
    assert!(
        host.statuses.iter().any(|s| s.contains("E21")),
        "x must report E21 after it is turned off: {:?}",
        host.statuses
    );
    assert_eq!(buf.slice(0..buf.len()), "alpha\n");

    // back on: editing resumes
    feed_rw(&mut vim, &mut buf, &mut host, &[":", "s", "e", "t", " ", "m", "a", "<CR>"]);
    assert!(vim.is_modifiable());
    feed_rw(&mut vim, &mut buf, &mut host, &["x"]);
    assert_eq!(buf.slice(0..buf.len()), "lpha\n");
}

// ---- 2. RW / RO equivalence replay ----------------------------------------

/// Viewing key sequences (no editing at all): the two pipelines must report
/// item-for-item identical state.
const VIEW_SEQUENCES: &[&[&str]] = &[
    &["g", "g"],
    &["w"],
    &["2", "w"],
    &["j", "j"],
    &["$"],
    &["0"],
    &["G"],
    &["g", "g", "}", "}"],
    &["/", "b", "e", "t", "a", "<CR>"],
    &["n"],
    &["*"],
    &["m", "x", "G", "'", "x"],
    &["v", "j", "l", "<Esc>"],
    &["V", "j", "<Esc>"],
    &["y", "y"],
    &["y", "i", "w"],
    &["z", "z"],
    &["z", "t"],
    &["<C-e>"],
    &["<C-y>"],
    &["%"],
    &["g", "v"],
];

#[test]
fn rw_and_ro_pipelines_agree_on_every_view_sequence() {
    for seq in VIEW_SEQUENCES {
        let text = "alpha beta\n(gamma)\n\ndelta\nepsilon\n";
        // RW: full VimHost, 'modifiable' on (no edit in the observed sequence)
        let mut rw_buf = BufferView::from_text(text);
        let mut rw_host = RoHost::new();
        let mut rw = VimState::new();
        feed_rw(&mut rw, &mut rw_buf, &mut rw_host, seq);

        // RO: the read-only pipeline
        let ro_buf = BufferView::from_text(text);
        let mut ro_host = RoHost::new();
        let mut ro = VimState::new();
        feed_ro(&mut ro, &ro_buf, &mut ro_host, seq);

        assert_eq!(rw.cursor_offset(), ro.cursor_offset(), "{seq:?} cursor differs");
        assert_eq!(
            format!("{:?}", rw.mode()),
            format!("{:?}", ro.mode()),
            "{seq:?} mode differs"
        );
        assert_eq!(
            format!("{:?}", rw.registers),
            format!("{:?}", ro.registers),
            "{seq:?} registers differ"
        );
        assert_eq!(rw_host.highlights, ro_host.highlights, "{seq:?} highlights differ");
        assert_eq!(rw_host.current, ro_host.current, "{seq:?} current highlight differs");
    }
}

// ---- 3. the MotionHook interception point ---------------------------------

#[test]
fn motion_hook_owns_vertical_motion() {
    let buf = BufferView::from_text("a\nb\nc\nd\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim = VimState::new();
    // line 1 is folded: `j` must land on line 2, not line 1
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(
        buf.offset_to_line(vim.cursor_offset()),
        2,
        "the interception point was not called (j landed on the built-in implementation's line)"
    );
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 3);
    // at the bottom: stuck (the cursor does not move)
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 3);
}

#[test]
fn motion_hook_is_consulted_for_operator_pending_and_visual() {
    // pending operator: the span of dj must go through the host's line projection
    // (stepping over the folded line)
    let mut buf = BufferView::from_text("a\nb\nc\nd\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim = VimState::new();
    // the built-in j would land on line 1 (`dj` -> "c\nd\n"); the interception
    // point steps over the folded line to line 2, so the span reaches the end of
    // line 2 (`dj` -> "d\n").
    feed_rw(&mut vim, &mut buf, &mut host, &["d", "j"]);
    assert_eq!(buf.slice(0..buf.len()), "d\n", "the span of dj did not use the interception point");

    // visual extension: the selection of vj also goes through the host's line projection
    let buf2 = BufferView::from_text("a\nb\nc\nd\n");
    let mut host2 = RoHost::new();
    host2.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim2 = VimState::new();
    feed_ro(&mut vim2, &buf2, &mut host2, &["v", "j"]);
    assert!(matches!(vim2.mode(), Mode::Visual { .. }), "should be in visual mode after v");
    assert_eq!(
        buf2.offset_to_line(vim2.cursor_offset()),
        2,
        "the visual extension did not use the interception point"
    );
}

#[test]
fn without_a_hook_the_builtin_motion_wins() {
    let buf = BufferView::from_text("a\nb\nc\nd\n");
    let mut host = RoHost::new(); // hook = None
    let mut vim = VimState::new();
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
}

#[test]
fn hook_declines_unknown_motions() {
    // only owns Up/Down; everything else goes back to the built-in implementation
    let buf = BufferView::from_text("alpha beta\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![] });
    let mut vim = VimState::new();
    feed_ro(&mut vim, &buf, &mut host, &["w"]);
    assert_eq!(vim.cursor_offset(), 6, "a motion that is not intercepted must use the built-in implementation");
}

// ---- 4. fuzz: in read-only mode a write primitive must never be reached ----

/// A buffer whose write primitives panic: any single arrival at
/// `insert_text`/`delete_range` goes red.
struct PanicBuf(String);

impl VimBuffer for PanicBuf {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn line_count(&self) -> usize {
        if self.0.is_empty() {
            1
        } else if self.0.ends_with('\n') {
            self.0.split('\n').count() - 1
        } else {
            self.0.split('\n').count()
        }
    }
    fn char_at(&self, offset: usize) -> Option<char> {
        if !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[offset..].chars().next()
    }
    fn prev_char_offset(&self, offset: usize) -> Option<usize> {
        if offset == 0 || offset > self.0.len() || !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[..offset]
            .chars()
            .next_back()
            .map(|c| offset - c.len_utf8())
    }
    fn line_range(&self, line: usize) -> Range<usize> {
        let mut start = 0;
        for (i, part) in self.0.split('\n').enumerate() {
            if i == line {
                let end = if start + part.len() < self.0.len() {
                    start + part.len() + 1
                } else {
                    self.0.len()
                };
                return start..end;
            }
            start += part.len() + 1;
        }
        self.0.len()..self.0.len()
    }
    fn offset_to_line(&self, offset: usize) -> usize {
        let offset = offset.min(self.0.len());
        let offset = if self.0.is_char_boundary(offset) {
            offset
        } else {
            self.0.floor_char_boundary(offset)
        };
        self.0[..offset].split('\n').count() - 1
    }
    fn slice(&self, range: Range<usize>) -> String {
        self.0[range].to_owned()
    }
}

impl VimBufferMut for PanicBuf {
    fn insert_text(&mut self, offset: usize, text: &str) -> bool {
        panic!("reached insert_text({offset}, {text:?}) while in read-only mode")
    }
    fn delete_range(&mut self, range: Range<usize>) -> bool {
        panic!("reached delete_range({range:?}) while in read-only mode")
    }
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// No `:` (so the ex `!` filter branch never really spawns sh) and no replay
/// bombs other than `q`/`@`; every editing key is in here, because that is
/// exactly the batch we want to prove "got stopped".
const FUZZ_KEYS: &[&str] = &[
    "x", "X", "d", "w", "b", "e", "0", "$", "j", "k", "h", "l", "gg", "G", "dd", "yy", "p", "P",
    "u", "<C-r>", "v", "V", "<C-v>", "o", "O", "i", "a", "A", "I", "R", "s", "S", "C", "D",
    "escape", "n", "N", "*", "%", ".", ">>", "<<", "gqj", "daw", "\"ay", "\"ap", "f", ";", ",",
    "gv", "~", "J", "<C-a>", "<C-x>", "guu", "gUU", "zz", "zt", "zb", "<C-e>", "<C-y>", "m", "z",
    "'", "`", "3", "2",
];

const FUZZ_BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\n",
    "中文测试\n多字节行\n",
    "foo bar baz\n\n\nqux\n   \ntail\n",
    "one",
    "",
    "x\n",
    "    indented\ntab\there\n",
    "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
];

#[test]
fn fuzz_readonly_never_reaches_a_write_primitive() {
    let seeds: u64 = 8;
    let rounds: usize = 20;
    let steps: usize = 60;
    for seed in 0..seeds {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let text = FUZZ_BUFFERS[seed as usize % FUZZ_BUFFERS.len()];
        // both shapes run side by side, the same key sequence fed to two engines:
        //   - step A -- `handle_key` + `set_modifiable(false)` + panicking write
        //     primitives (any single leak past the E21 gate goes red immediately);
        //   - step B -- `handle_key_ro`, the buffer entering as `&dyn VimBuffer`,
        //     so `VimBufferMut` is not on the call path at all.
        // Both must also give the same cursor: that is the strong assertion that
        // "the read-only pipeline does not change semantics".
        let mut rw_buf = PanicBuf(text.to_owned());
        let mut rw_host = RoHost::new();
        let mut rw = VimState::new();
        rw.set_modifiable(false);
        let ro_buf = PanicBuf(text.to_owned());
        let mut ro_host = RoHost::new();
        let mut ro = VimState::new();
        for _ in 0..rounds {
            for _ in 0..steps {
                let k = FUZZ_KEYS[(xorshift(&mut rng) as usize) % FUZZ_KEYS.len()];
                let key = Key::parse(k);
                {
                    let mut ctx = Ctx {
                        buf: &mut rw_buf,
                        host: &mut rw_host,
                    };
                    rw.handle_key(&mut ctx, key.clone());
                }
                {
                    let ro_ref: &dyn VimBuffer = &ro_buf;
                    let mut ctx = CtxRo {
                        buf: ro_ref,
                        host: &mut ro_host,
                    };
                    ro.handle_key_ro(&mut ctx, key.clone());
                }
                let len = rw_buf.len();
                assert!(
                    rw.cursor_offset() <= len,
                    "seed {seed}: cursor {} out of range (len {len}), key {k:?}",
                    rw.cursor_offset()
                );
                assert_eq!(
                    rw.cursor_offset(),
                    ro.cursor_offset(),
                    "seed {seed}: RW/RO cursors differ after key {k:?} ({} vs {})",
                    rw.cursor_offset(),
                    ro.cursor_offset()
                );
            }
            // the two pipelines must receive exactly the same key stream -- dropping
            // one Esc on either side makes them diverge from the next round onwards
            let esc = Key::parse("<Esc>");
            {
                let mut ctx = Ctx {
                    buf: &mut rw_buf,
                    host: &mut rw_host,
                };
                rw.handle_key(&mut ctx, esc.clone());
            }
            {
                let ro_ref: &dyn VimBuffer = &ro_buf;
                let mut ctx = CtxRo {
                    buf: ro_ref,
                    host: &mut ro_host,
                };
                ro.handle_key_ro(&mut ctx, esc);
            }
        }
        assert_eq!(rw_buf.slice(0..rw_buf.len()), text, "seed {seed}: the buffer was changed");
        assert_eq!(ro_buf.slice(0..ro_buf.len()), text);
    }
}

// ---- insert-family corners: the divergence of refusing at entry -----------

/// vim 9.1 **silently enters insert** for `gI` / visual `I`/`A` / block
/// `I`/`A`, and only reports E21 on the first character landed; the engine
/// chooses to refuse at the entry (the divergence recorded in the tck.rs
/// EDIT_KEYS comment). This test locks the two observable semantics of that
/// divergence: E21 is still reported + the mode never gets dragged into insert.
/// (Refusing earlier is the safer direction: under `CtxRo` write primitives are
/// unreachable at the type level, and keeping the visual selection is the
/// better experience for a read-only pane.)
#[test]
fn insert_corners_refuse_at_entry_without_entering_insert() {
    let text = "alpha beta\ngamma delta\n";
    let cases: &[(&[&str], Mode)] = &[
        (&["g", "I"], Mode::Normal),
        (&["v", "I"], Mode::Visual {
            kind: vimcore::mode::VisualKind::Char,
        }),
        (&["V", "A"], Mode::Visual {
            kind: vimcore::mode::VisualKind::Line,
        }),
        (&["<C-v>", "j", "l", "I"], Mode::Visual {
            kind: vimcore::mode::VisualKind::Block,
        }),
    ];
    for (keys, want_mode) in cases {
        let buf = BufferView::from_text(text);
        let mut host = TckHostRo::default();
        let mut vim = VimState::new();
        {
            let ctx_buf: &dyn VimBuffer = &buf;
            let mut ctx = CtxRo {
                buf: ctx_buf,
                host: &mut host,
            };
            for k in *keys {
                vim.handle_key_ro(&mut ctx, Key::parse(k));
            }
        }
        assert_eq!(vim.mode, *want_mode, "{keys:?}: the mode was dragged into insert");
        assert!(
            host.statuses.iter().any(|s| s.contains("E21")),
            "{keys:?}: did not receive E21 ({:?})",
            host.statuses
        );
        assert_eq!(buf.slice(0..buf.len()), text, "{keys:?}: the buffer was changed");
    }
}

/// `refresh_highlights_ro`: after the host swaps out the text underneath the
/// engine (a pane switch in a multi-view layout), the highlights are rescanned
/// and republished against the current buffer without feeding synthetic keys.
/// With no active pattern it is a quiet no-op (it must not wipe what the host
/// already has published).
#[test]
fn refresh_highlights_ro_republishes_for_the_new_buffer() {
    let a = BufferView::from_text("alpha\nbeta\ngamma\n");
    let b = BufferView::from_text("beta\nzzz\n");
    let mut host = RoHost::new();
    let mut vim = VimState::new();
    feed_ro(&mut vim, &a, &mut host, &["/", "b", "e", "t", "a", "<CR>"]);
    assert_eq!(host.highlights, vec![6..10], "the beta hit on buffer A");

    // swap the buffer (the equivalent of a host-side pane switch): the same
    // pattern is rescanned
    {
        let b: &dyn VimBuffer = &b;
        let mut ctx = CtxRo {
            buf: b,
            host: &mut host,
        };
        vim.refresh_highlights_ro(&mut ctx);
    }
    assert_eq!(
        host.highlights,
        vec![0..4],
        "the rescan must publish coordinates of the new buffer, not reuse the old offsets"
    );
    assert_eq!(a.slice(0..a.len()), "alpha\nbeta\ngamma\n", "refresh must not write the buffer");

    // no pattern (a fresh engine instance): no-op, does not clear host state
    let mut fresh = RoHost::new();
    // a one-element highlight vec (the lint reads `1..2` as a range-expansion)
    #[allow(clippy::single_range_in_vec_init)]
    let hl_seed = vec![1..2];
    fresh.highlights = hl_seed;
    let mut vim2 = VimState::new();
    {
        let b: &dyn VimBuffer = &b;
        let mut ctx = CtxRo {
            buf: b,
            host: &mut fresh,
        };
        vim2.refresh_highlights_ro(&mut ctx);
    }
    assert_eq!(
        fresh.highlights,
        vec![1..2],
        "refresh must be a no-op when there is no active pattern"
    );
}
