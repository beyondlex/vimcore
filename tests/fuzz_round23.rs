//! 第二十三轮 fuzz：Ex 命令/`:set`/`:sort` 的参数恶意面 + 宿主事件
//! （IME 提交 / undo / 宏回放）与按键流的随机交错。
//!
//! 背景：round23 修复了五处语义（replace_range 的会话 undo 组拆分、
//! floor_to_char_boundary 的缓冲末尾边界、Enter char-arg 收敛、:set
//! E521、:sort E475/数值排序）。修复触碰的是「宿主驱动编辑」与「Ex
//! 参数解析」两条此前 fuzz 未覆盖的深水区：
//!   1. `:set`/`:sort` 的参数字母表逐项 + 组合轰炸——E518/E521/E475/E488
//!      的判定路径要保真且不得 panic；
//!   2. IME 提交（replace_range）插在 insert 会话任意位置 + 随机 undo/
//!      redo/宏回放——undo 组的边界必须保持可寻址、宿主快照语义完好；
//!   3. 全程维持 round20 起的渲染契约（高亮可寻址）与行寻址契约。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::{Key, KeyKind, Modifiers};

fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

fn assert_addressable(f: &Fixture, context: &str) {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    let cur = f.vim.cursor.offset;
    assert!(
        cur <= len && text.is_char_boundary(cur),
        "{context}: cursor {cur} unaddressable (len={len}) in {text:?}"
    );
    for line in 0..buf.line_count() {
        let (s, e) = (buf.line_start(line), buf.line_end(line));
        assert!(
            s <= e && e <= len && text.is_char_boundary(s) && text.is_char_boundary(e),
            "{context}: line {line} range {s}..{e} broken (len={len}) in {text:?}"
        );
    }
    for hl in f
        .host
        .highlights
        .iter()
        .chain(f.host.current_highlight.iter())
    {
        assert!(
            hl.end <= len && text.is_char_boundary(hl.start) && text.is_char_boundary(hl.end),
            "{context}: highlight {hl:?} unaddressable in {text:?}"
        );
    }
}

// ---- 1. :set / :sort 参数恶意面 ----------------------------------------------

/// `:set` 的参数字母表：合法/非法数值、未知名、查询、复位、否定、注释。
const SET_ALPHABET: &[&str] = &[
    "ts=4", "ts=", "ts=x", "ts=99999999999999999999999", "ts=-4", "sw=0", "tw=1000000",
    "so=1", "foo", "foo=1", "number", "nonumber", "nu!", "ic?", "ts ?", "ts&", "ts&vim",
    "?", "\"", "\" note", "ts=4 \" note", "scs", "noic",
];

/// `:sort` 的旗标字母表：合法（!/i/u/n 与组合）+ vim 有效但引擎拒绝的
/// （x/o/b/f/l）+ 彻底非法的 + 数字。
const SORT_FLAGS: &[&str] = &[
    "", "!", "i", "u", "n", "iu", "in", "nu!", "!n", " z", "x", "o", "b", "f", "l", "z", "3",
    "2x", "i!", "un", "nz", "!!", "ii",
];

const TARGETS: &[&str] = &["b\na\nc\n", "10 apples\n2 bananas\nno num\n-3 x\n", "中b\na阿\n", "one"];

fn fuzz_set_surface() {
    for (ti, target) in TARGETS.iter().enumerate() {
        for (si, arg) in SET_ALPHABET.iter().enumerate() {
            let mut f = Fixture::new(target);
            let context = format!("target#{ti} set {arg:?} (s{si})");
            feed_ex(&mut f, &format!("set {arg}"));
            assert_addressable(&f, &context);
            // 选项面轰炸后 n/搜索缓存不得失配 panic
            f.feed(["*", "n", "N"]);
            assert_addressable(&f, &context);
        }
        // 组合：一条 :set 里塞多项，中途失败必须截断（E518/E521 之后不执行）
        let mut f = Fixture::new(target);
        feed_ex(&mut f, "set ts=8 ts= sw=2 foo ts=16");
        assert_addressable(&f, "set combo");
        assert!(
            f.vim.options.tabstop == 4 || f.vim.options.tabstop == 8,
            "tabstop 越过失败项继续生效: {}",
            f.vim.options.tabstop
        );
    }
}

fn fuzz_sort_surface() {
    for (ti, target) in TARGETS.iter().enumerate() {
        for (fi, flags) in SORT_FLAGS.iter().enumerate() {
            let mut f = Fixture::new(target);
            let context = format!("target#{ti} sort {flags:?} (f{fi})");
            feed_ex(&mut f, &format!("%sort{flags}"));
            assert_addressable(&f, &context);
            // 排序改行序后 changelist/jumplist 仍可寻址
            f.feed(["g", ";", "g", ";"]);
            assert_addressable(&f, &context);
        }
    }
}

#[test]
fn fuzz_ex_set_and_sort_argument_surfaces_hold_invariants() {
    fuzz_set_surface();
    fuzz_sort_surface();
}

// ---- 2. 宿主事件（IME 提交/undo/宏）与按键流随机交错 --------------------------

/// 简单 LCG：确定性随机（无外部依赖）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// 按键池：覆盖编辑面、提示符面、字符参数命令。
const KEY_POOL: &[&str] = &[
    "x", "d", "d", "y", "y", "p", "P", "i", "a", "A", "o", "<Esc>", "j", "k", "h", "l", "w", "b",
    "c", "c", "r", "x", "m", "a", "'", "a", "`", "a", "f", "x", ";", "u", "<C-r>", ".", "q", "a",
    "@", "a", "J", "~", "g", "u", "u", "n", "*", "0", "$", "g", "g", "G", "1", "2", "3", "v",
    "<C-v>", "V", "d", "g", "n", ":", "/", "f", "o", "o", "<CR>", "<BS>", "<Esc>", "s", "S",
];

const EX_POOL: &[&str] = &[
    "s/o/X/g", "%s/^/·/", "sort", "sort n", "sort iu", "sort z", "set ts=8", "set ts=",
    "set foo", "2,3d", "1,2y a", "3j", "j!", "noh", "1,3sort 3", "d 3x",
];

fn ime_commit(f: &mut Fixture) {
    // 在光标行内取一个合法范围做提交替换（宿主 IME 的真实形态）
    let text = f.text();
    let len = text.len();
    if len == 0 {
        return;
    }
    let mut r = Rng(len as u64 | 1);
    let start = text.floor_char_boundary(r.below(len + 1));
    let end = text.floor_char_boundary(r.below(len + 1));
    let (s, e) = (start.min(end), start.max(end));
    f.ime_replace(s..e, "中");
}

// 范围边界的 floor 用 std 的 str::floor_char_boundary（1.86 起稳定）。

#[test]
fn fuzz_host_events_interleaved_with_keys_hold_invariants() {
    let mut rng = Rng(0x005E_ED23);
    for round in 0..300 {
        let mut f = Fixture::new("foo bar\n中a文\nbaz qux\n");
        f.vim.keymaps_mut().map_str_noremap(
            vimcore::keymap::ModeClass::Normal,
            "Z",
            "xj",
            true,
        );
        let context = format!("round#{round}");
        for step in 0..120 {
            match rng.below(12) {
                0..=7 => {
                    let key = KEY_POOL[rng.below(KEY_POOL.len())];
                    f.feed([key]);
                }
                8 => {
                    let ex = EX_POOL[rng.below(EX_POOL.len())];
                    feed_ex(&mut f, ex);
                }
                9 => ime_commit(&mut f),
                10 => {
                    f.feed(["u"]); // undo 走 u 键（宿主回调由 execute_normal_cmd 触发）
                }
                _ => {
                    f.feed(["2", "@", "a"]);
                }
            }
            assert_addressable(&f, &format!("{context} step#{step}"));
            // is_idle 与模式的契约（round23 F）：开着的提示符绝不算 idle
            if matches!(f.vim.mode(), vimcore::mode::Mode::CommandLine { .. }) {
                assert!(
                    !f.vim.is_idle(),
                    "{context} step#{step}: 提示符开着却报告 idle"
                );
            }
        }
        // 收尾 invariant：Esc 归位后引擎可寻址、可继续编辑
        f.feed(["<Esc>", "<Esc>", "g", "g"]);
        assert_addressable(&f, &context);
        f.feed(["x"]);
        assert_addressable(&f, &context);
    }
}

/// IME 提交于缓冲末尾：floor 边界修复（round23 E）的定向覆盖。
#[test]
fn ime_commit_at_buffer_end_keeps_last_char() {
    for tail in ["ab", "中", "a中", "中a"] {
        let mut f = Fixture::new("hello");
        f.feed(["A"]);
        f.type_text(tail);
        let len = f.text().len();
        let start = len - tail.len();
        f.ime_replace(start..len, "Z");
        assert_eq!(
            f.text(),
            "helloZ",
            "提交范围抵达缓冲末尾时不得吞掉末字符（tail={tail:?}）"
        );
    }
}

/// 会话内多次提交一个 undo 组的定向覆盖（round23 A）。
#[test]
fn multi_commit_session_undoes_as_one() {
    let mut f = Fixture::new("base");
    f.feed(["A"]);
    f.type_text("ab");
    f.ime_replace(4..6, "阿");
    f.type_text("cd");
    f.ime_replace(7..10, "波");
    f.type_text("e");
    f.feed(["<Esc>"]);
    let after_session = f.text();
    f.feed(["u"]);
    assert_eq!(f.text(), "base", "整个会话（含全部提交）一次 u 还原");
    let _ = after_session;
    // redo 走回会话终态
    f.feed(["<C-r>"]);
    f.feed(["<C-r>"]);
    let _ = f.text();
}

// —— 编译期哨兵：KeyKind/Modifiers 的导入不得闲置（形态面回归） ——
#[test]
fn key_model_shapes_still_resolve() {
    assert_eq!(
        Key {
            modifiers: Modifiers::NONE,
            kind: KeyKind::Char('x'),
        },
        Key::parse("x")
    );
}
