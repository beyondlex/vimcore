//! 第二十轮 fuzz：命令行面（`:` `/` `?` 提示符）的路径轰炸。
//!
//! 相对 round19 的增量：round16-19 的键表里 Ex 行是**整串命名键**
//! （`:3j\n` 这类固定拼写），本轮改成「进提示符 → 逐字符喂恶意字母表 →
//! Enter/Esc 收尾」的爆发式输入，专打 `parse_range`/`ex_substitute`/
//! `parse_reg_count`/history/C-w/C-u 这些只在 cmdline 内部走到的解析器。
//! 另加本轮专属不变量：**发布给宿主的高亮区间必须整体可寻址**（起止在
//! 字符边界且 ≤ 缓冲长）——搜索缓存失配/偏移调整出错时宿主 slice 会
//! panic，这是引擎对宿主的渲染契约。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;
use vimcore::key::Key;
use vimcore::state::Ctx;

fn fuzz_xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// 16 seeds × 50 rounds × 100 steps = 8 万步（与 round13-19 同量级）
const SEEDS: u64 = 16;
const ROUNDS: usize = 50;
const STEPS: usize = 100;

/// 提示符恶意字母表：范围语法/分隔符/寄存器名/正则元字符/多字节。
// 安全阀（与 round35 同款）：字母表**不含 `!`**——ex 的 `:{range}!{filter}`
// 分支会把后续字符交给 `sh -c` 真实执行（`>` 等元字符会在 cwd 落文件）。
// `!` 形态的确定性覆盖在 tests/ex_filter.rs。
const EX_ALPHABET: &[char] = &[
    '0', '1', '2', '3', '9', '$', '%', '.', '\'', '+', '-', ',', ';', ' ', '/', '?', '\\',
    '&', '|', '~', '{', '}', '(', ')', '[', ']', '<', '>', '#', '*', '^', '=', '"', 'a', 'b', 'd',
    'e', 'g', 'i', 'j', 'k', 'l', 'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    'J', 'S', 'D', '中', '文', '\u{1f600}',
];

/// 普通模式打散键（保持管线不总停在同一状态；不需要 Ex 完整拼写）。
const MODE_KEYS: &[&str] = &[
    "w", "b", "e", "0", "$", "j", "k", "h", "l", "gg", "G", "x", "dd", "yy", "p", "P", "u",
    "<C-r>", "v", "V", "o", "O", "i", "a", "A", "escape", "n", "N", "*", "%", ".", ">>", "gqj",
    "daw", "\"ay", "\"ap", "@a", "qa", "q", "f", "a", ";", ",", "gv", "~", "J", "<C-w>",
];

const BUFFERS: &[&str] = &[
    "a\nb\nc\n",
    "hello world\nsecond line\n",
    "中文测试\n多字节行\n",
    "foo bar baz\n\n\nqux\n   \ntail\n",
    "one",
    "",
    "x\n",
    "    indented\ntab\there\n",
    "aaa bbb aaa bbb\nccc\naaa\n",
    "😀 emoji\nline\n",
    "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
];

fn buffer_reset_when_idle(f: &mut Fixture, seed: u64) -> bool {
    if !f.vim.is_idle() || !matches!(f.vim.mode(), vimcore::mode::Mode::Normal) {
        return false;
    }
    let text = BUFFERS[(seed as usize) % BUFFERS.len()];
    *f.buf.0.borrow_mut() = text.to_owned();
    f.vim.cursor.offset = 0;
    f.vim.cursor.desired_col = None;
    // 宿主换缓冲必然丢弃旧视图——先让引擎对新文本重发布，之后抓到
    // 的越界高亮才是引擎自身的责任（而非 reset 前的陈账）
    let mut ctx = Ctx {
        buf: &mut f.buf,
        host: &mut f.host,
    };
    f.vim.refresh_highlights(&mut ctx);
    true
}

fn invariants_hold(f: &Fixture) -> Result<(), String> {
    let buf = &f.buf;
    let len = buf.len();
    let text = f.text();
    if buf.line_count() == 0 {
        return Err("line_count == 0".into());
    }
    let cur = f.vim.cursor.offset;
    if cur > len || !text.is_char_boundary(cur) {
        return Err(format!("cursor {cur} unaddressable (len={len})"));
    }
    for (name, off) in f.vim.marks.items() {
        let o = off.min(len);
        let o = (0..=o).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        if buf.offset_to_line(o) >= buf.line_count() {
            return Err(format!("mark {name} at {o} past last line"));
        }
    }
    // 渲染契约：宿主拿到的每个高亮区间都必须可直接 slice
    for hl in f.host.highlights.iter().chain(f.host.current_highlight.iter()) {
        if hl.end > len || !text.is_char_boundary(hl.start) || !text.is_char_boundary(hl.end) {
            return Err(format!(
                "highlight {hl:?} unaddressable (len={len}) in {text:?}"
            ));
        }
    }
    Ok(())
}

/// 一次命令行爆发：prompt + 若干恶意字符 + 收尾键。
fn cmdline_burst(f: &mut Fixture, rng: &mut u64, prompt: char, trace: &mut Vec<String>) {
    let mut keys: Vec<String> = vec![prompt.to_string()];
    let n = (fuzz_xorshift(rng) % 24) as usize + 1;
    for _ in 0..n {
        // 每步 1/6 概率塞一个 <...> 编辑键，其余取字母表字符
        if fuzz_xorshift(rng).is_multiple_of(6) {
            keys.push(
                ["<BS>", "<C-w>", "<C-u>", "<up>", "<down>", "<left>", "<right>", "<Esc>", "<Tab>"]
                    [(fuzz_xorshift(rng) % 9) as usize]
                    .to_owned(),
            );
        } else {
            keys.push(EX_ALPHABET[(fuzz_xorshift(rng) as usize) % EX_ALPHABET.len()].to_string());
        }
    }
    // 2/3 概率回车执行，否则 Esc 取消（两条尾路径的记账都要炸到）
    keys.push(if !fuzz_xorshift(rng).is_multiple_of(3) {
        "<CR>".to_owned()
    } else {
        "<Esc>".to_owned()
    });
    // 逐 token 喂：字母表单字符，<...> 为整体命名键（与 feed 的 Key::parse 域一致）
    trace.extend(keys.iter().cloned());
    f.feed(keys);
}

/// 逐字符喂一条 Ex 行（`:` + 字符 + Enter）。
fn feed_ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

#[test]
fn fuzz_round20_cmdline_face_holds_invariants() {
    for seed in 0..SEEDS {
        let mut rng = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        for round in 0..ROUNDS {
            let mut f = Fixture::new(BUFFERS[((seed as usize) + round) % BUFFERS.len()]);
            let mut trace: Vec<String> = Vec::new();
            for _ in 0..STEPS {
                if fuzz_xorshift(&mut rng).is_multiple_of(20) {
                    let r = fuzz_xorshift(&mut rng);
                    if buffer_reset_when_idle(&mut f, r) {
                        trace.push(format!("RESET {}", (r as usize) % BUFFERS.len()));
                    }
                }
                let roll = fuzz_xorshift(&mut rng) % 100;
                if roll < 34 {
                    trace.push("burst :".into());
                    cmdline_burst(&mut f, &mut rng, ':', &mut trace);
                } else if roll < 44 {
                    trace.push("burst /".into());
                    cmdline_burst(&mut f, &mut rng, '/', &mut trace);
                } else if roll < 50 {
                    trace.push("burst ?".into());
                    cmdline_burst(&mut f, &mut rng, '?', &mut trace);
                } else if roll < 60 {
                    let c = ['\n', '\x7f', '\x08', '\t'][(roll % 4) as usize];
                    trace.push(format!("raw {:?}", c));
                    // 直接字符喂（含 ctrl 原始形态）：IME 归一键
                    f.feed_raw(Key::char(c));
                } else {
                    let k = MODE_KEYS[(roll as usize * 7) % MODE_KEYS.len()];
                    trace.push(format!("feed {k}"));
                    f.feed([k]);
                }
                if let Err(e) = invariants_hold(&f) {
                    panic!("seed={seed} round={round}: {e}\ntrace: {trace:?}");
                }
            }
        }
    }
}

/// 定向：`:s` 尾随 count 的每个截断形态都不得越出 [range 末行, EOF]。
#[test]
fn fuzz_substitute_count_never_slides_start_upward() {
    for n in [1usize, 2, 3, 7, 8, 9, 10, 11, 99, 1_000_000] {
        for start in 0..10usize {
            let mut f = Fixture::new("a1\na2\na3\na4\na5\na6\na7\na8\na9\na10\n");
            feed_ex(&mut f, &format!("{}s/a/X/{}", start + 1, n));
            let changed = f.text().lines().filter(|l| l.starts_with('X')).count();
            // vim 模型：从 start 行起、min(n, 剩余) 行被改，且都 ≥ start
            let expect = (start..start + n).take_while(|&i| i < 10).count();
            assert_eq!(
                changed, expect,
                "n={n} start={start}: got {:?}",
                f.text()
            );
            if let Some((first_changed, _)) = f.text().lines().enumerate().find(|(_, l)| l.starts_with('X')) {
                assert_eq!(first_changed, start, "n={n} start={start} slid");
            }
        }
    }
}
