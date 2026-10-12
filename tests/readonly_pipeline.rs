//! 只读管线（PLAN_READONLY P1 / P2）的行为测试。
//!
//! 覆盖三件事：
//!
//! 1. `handle_key_ro` + `CtxRo`：只读宿主（`impl VimBuffer` +
//!    `impl VimHostRo`，零写原语）拿到完整的 motion / text object /
//!    搜索 / 标记 / 滚动 / yank 能力；
//! 2. `'modifiable'` 关闭时编辑类命令按 vim 的 E21 拒绝，且写原语一次都
//!    没被调到（fuzz 用"写原语即 panic"的缓冲做硬断言）；
//! 3. `MotionHook` 拦截点：宿主接管的 motion 在普通移动、算子待决与可视
//!    扩展三条路径上同时生效。
//!
//! E21 的键位语义以 vim 9.1 实证为准（`:set nomodifiable` 下 `x` `dw` `dd`
//! `i` `o` `~` `p` `J` `u` `>` `:s` `:d` `:j` `:m` `:pu` 全部报 E21；
//! `y` `yy` `m{a}` `:q` 不报）。

mod common;

use std::ops::Range;

use common::BufferView;
use vimcore::buffer::{VimBuffer, VimBufferMut};
use vimcore::host::{MotionHook, VimHost, VimHostRo};
use vimcore::key::Key;
use vimcore::mode::Mode;
use vimcore::motions::{Motion, MotionKind, MotionResult};
use vimcore::state::{Ctx, CtxRo, VimState};

// ---- 只读宿主 ------------------------------------------------------------

/// 一个"折叠感知"的宿主：第 `hidden` 行被折叠，j/k 必须跨过它。这就是
/// alacer diff view 的 `]c`/`[c` 与可见行移动在引擎侧的接入形态。
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
            // 交回引擎：`j` 在末行的 stuck 语义由内建实现给出，宿主不必
            // 自己复刻一遍（返回 None 就是"这条我不接管"）。
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

// 同一类型也实现完整 `VimHost`，好让 RW/RO 等价性回放共用一套宿主语义。
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

// ---- 1. 只读管线：查看能力全在，编辑被 E21 拒绝 ----------------------------

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
    // 搜索
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    feed_ro(&mut vim, &buf, &mut host, &["/", "b", "e", "t", "a", "<CR>"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
    assert!(!host.highlights.is_empty(), "搜索高亮必须照常发布给宿主");
    // 标记（vim: `m{a}` 在 nomodifiable 下合法）
    feed_ro(&mut vim, &buf, &mut host, &["m", "a"]);
    feed_ro(&mut vim, &buf, &mut host, &["g", "g"]);
    feed_ro(&mut vim, &buf, &mut host, &["'", "a"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 1);
    // yank（vim: `y` 在 nomodifiable 下合法）— 写寄存器 + 剪贴板
    feed_ro(&mut vim, &buf, &mut host, &["\"", "+", "y", "y"]);
    assert_eq!(host.clipboard.as_deref(), Some("beta\n"));
    assert_eq!(
        vim.registers.get('+').map(|r| r.text.clone()).unwrap_or_default(),
        "beta\n"
    );
    // 滚动
    feed_ro(&mut vim, &buf, &mut host, &["z", "z"]);
    host.statuses.clear();
    feed_ro(&mut vim, &buf, &mut host, &["/", "g", "a", "m", "m", "a", "<CR>"]);
    assert!(
        !host.statuses.iter().any(|s| s.contains("E21")),
        "查看类键位不得报 E21：{:?}",
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
        assert_eq!(buf.slice(0..buf.len()), before, "{keys:?} 改动了缓冲");
        assert!(
            host.statuses.iter().any(|s| s.contains("E21")),
            "{keys:?} 没有上报 E21：{:?}",
            host.statuses
        );
        feed_ro(&mut vim, &buf, &mut host, &["<Esc>"]);
    }
    // 拒绝之后引擎仍然可用
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
    assert!(!vim.is_modifiable(), ":set noma 没有关掉 'modifiable'");
    host.statuses.clear();
    feed_rw(&mut vim, &mut buf, &mut host, &[":", "s", "e", "t", " ", "m", "a", "?", "<CR>"]);
    assert_eq!(host.statuses.last().map(|s| s.as_str()), Some("nomodifiable"));

    host.statuses.clear();
    feed_rw(&mut vim, &mut buf, &mut host, &["x"]);
    assert!(
        host.statuses.iter().any(|s| s.contains("E21")),
        "关闭后 x 必须报 E21：{:?}",
        host.statuses
    );
    assert_eq!(buf.slice(0..buf.len()), "alpha\n");

    // 再打开：编辑恢复
    feed_rw(&mut vim, &mut buf, &mut host, &[":", "s", "e", "t", " ", "m", "a", "<CR>"]);
    assert!(vim.is_modifiable());
    feed_rw(&mut vim, &mut buf, &mut host, &["x"]);
    assert_eq!(buf.slice(0..buf.len()), "lpha\n");
}

// ---- 2. RW / RO 等价性回放 ------------------------------------------------

/// 查看类键序（不含任何编辑）：两条管线必须给出逐项相同的状态。
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
        // RW：完整 VimHost，'modifiable' 开着（观测序列里没有编辑）
        let mut rw_buf = BufferView::from_text(text);
        let mut rw_host = RoHost::new();
        let mut rw = VimState::new();
        feed_rw(&mut rw, &mut rw_buf, &mut rw_host, seq);

        // RO：只读管线
        let ro_buf = BufferView::from_text(text);
        let mut ro_host = RoHost::new();
        let mut ro = VimState::new();
        feed_ro(&mut ro, &ro_buf, &mut ro_host, seq);

        assert_eq!(rw.cursor_offset(), ro.cursor_offset(), "{seq:?} 光标不同");
        assert_eq!(
            format!("{:?}", rw.mode()),
            format!("{:?}", ro.mode()),
            "{seq:?} 模式不同"
        );
        assert_eq!(
            format!("{:?}", rw.registers),
            format!("{:?}", ro.registers),
            "{seq:?} 寄存器不同"
        );
        assert_eq!(rw_host.highlights, ro_host.highlights, "{seq:?} 高亮不同");
        assert_eq!(rw_host.current, ro_host.current, "{seq:?} 当前高亮不同");
    }
}

// ---- 3. MotionHook 拦截点 -------------------------------------------------

#[test]
fn motion_hook_owns_vertical_motion() {
    let buf = BufferView::from_text("a\nb\nc\nd\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim = VimState::new();
    // 第 1 行被折叠：`j` 必须落在第 2 行，不是第 1 行
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(
        buf.offset_to_line(vim.cursor_offset()),
        2,
        "拦截点没有被调用（j 落到了内建实现的位置）"
    );
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 3);
    // 到底了：stuck（光标不动）
    feed_ro(&mut vim, &buf, &mut host, &["j"]);
    assert_eq!(buf.offset_to_line(vim.cursor_offset()), 3);
}

#[test]
fn motion_hook_is_consulted_for_operator_pending_and_visual() {
    // 算子待决：dj 的跨度必须走宿主的行投影（跨过折叠行）
    let mut buf = BufferView::from_text("a\nb\nc\nd\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim = VimState::new();
    // 内建 j 会落在第 1 行（`dj` → "c\nd\n"）；拦截点跨过折叠行落到第 2
    // 行，跨度覆盖到第 2 行末（`dj` → "d\n"）。
    feed_rw(&mut vim, &mut buf, &mut host, &["d", "j"]);
    assert_eq!(buf.slice(0..buf.len()), "d\n", "dj 的跨度没有用拦截点");

    // 可视扩展：vj 的选区同样走宿主的行投影
    let buf2 = BufferView::from_text("a\nb\nc\nd\n");
    let mut host2 = RoHost::new();
    host2.hook = Some(HiddenLineHook { hidden: vec![1] });
    let mut vim2 = VimState::new();
    feed_ro(&mut vim2, &buf2, &mut host2, &["v", "j"]);
    assert!(matches!(vim2.mode(), Mode::Visual { .. }), "v 之后应在可视模式");
    assert_eq!(
        buf2.offset_to_line(vim2.cursor_offset()),
        2,
        "可视扩展没有用拦截点"
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
    // 只接管 Up/Down，其余交回内建实现
    let buf = BufferView::from_text("alpha beta\n");
    let mut host = RoHost::new();
    host.hook = Some(HiddenLineHook { hidden: vec![] });
    let mut vim = VimState::new();
    feed_ro(&mut vim, &buf, &mut host, &["w"]);
    assert_eq!(vim.cursor_offset(), 6, "未被接管的 motion 必须走内建实现");
}

// ---- 4. fuzz：只读模式下写原语一次都不能被调到 -----------------------------

/// 写原语即 panic 的缓冲：任何一次到达 `insert_text`/`delete_range` 都红。
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
        panic!("只读模式下到达了 insert_text({offset}, {text:?})")
    }
    fn delete_range(&mut self, range: Range<usize>) -> bool {
        panic!("只读模式下到达了 delete_range({range:?})")
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

/// 不含 `:`（避免 ex 的 `!` 过滤分支真的去 spawn sh）与 `q`/`@` 之外的
/// 重放炸弹；编辑类键位全部在内，因为这正是要证明"被拦住"的那批。
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
        // 两条形态并跑，同一个键序喂给两台引擎：
        //   · A 步 —— `handle_key` + `set_modifiable(false)` + 写原语 panic
        //     （任何一次漏过 E21 闸门立刻红）；
        //   · B 步 —— `handle_key_ro`，缓冲以 `&dyn VimBuffer` 进入，
        //     `VimBufferMut` 根本不在调用路径上。
        // 两者还必须给出同一个光标：这是"只读管线不改语义"的强断言。
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
                    "seed {seed}: 光标 {} 越界（len {len}），键 {k:?}",
                    rw.cursor_offset()
                );
                assert_eq!(
                    rw.cursor_offset(),
                    ro.cursor_offset(),
                    "seed {seed}: 键 {k:?} 之后 RW/RO 光标不同（{} vs {}）",
                    rw.cursor_offset(),
                    ro.cursor_offset()
                );
            }
            // 两条管线必须收到完全相同的键流——漏给一边一个 Esc 就会让
            // 它们从下一轮起各走各的
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
        assert_eq!(rw_buf.slice(0..rw_buf.len()), text, "seed {seed}: 缓冲被改动了");
        assert_eq!(ro_buf.slice(0..ro_buf.len()), text);
    }
}
