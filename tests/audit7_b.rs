//! audit7_b: 第六轮独立审计·智能体 B 域的对拍 fuzz runner（motion/算子/
//! 可视 × 随机键序 vs `/usr/bin/vim` 实测）。`#[ignore]`，发现新 bug 用，
//! 默认不跑；抓到的形状转成定点断言另立文件（如 `audit8_a.rs`）。
//!
//! 采样纪律两侧对齐：vim 侧全程 typeahead（`:set binary` → `:w!` →
//! `writefile(line2byte+col-1)` → `:qa!`，startup 命令不参与，8s 看门狗
//! 兜住阻塞），引擎侧把选项钉成 vim 出厂值（`ts=8 sw=8 noet`）。键序
//! 一律不进插入模式打字（进 insert 就 `<Esc>` 收尾），避免把「宿主文本
//! 通道」的字符丢掉造成假阳。

mod common;

use common::Fixture;

// ---------------------------------------------------------------------------
// 对拍 fuzz（发现新 bug 用，默认不跑）
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

thread_local! {
    static SEED: RefCell<u64> = const { RefCell::new(0x1234_5678) };
}

fn rng() -> u64 {
    SEED.with(|s| {
        let mut x = *s.borrow();
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *s.borrow_mut() = x;
        x
    })
}

fn pick<T: Clone>(v: &[T]) -> T {
    v[(rng() as usize) % v.len()].clone()
}

fn chance(p: u32) -> bool {
    (rng() % 100) < p as u64
}

// a "motion" is a token sequence
fn gen_motion() -> Vec<String> {
    let pick: String = pick(&[
        "w", "b", "e", "ge", "W", "B", "E", "h", "l", "j", "k", "0", "^", "$", "G", "gg", "{", "}",
        "(", ")", "%", "iw", "aw", "is", "as", "ip", "ap", "iW", "aW",
    ])
    .to_string();
    match pick.as_str() {
        "f" | "F" | "t" | "T" => {
            let c = (b'a' + (rng() as u8 % 26)) as char;
            vec![pick, c.to_string()]
        }
        "iw" | "aw" | "is" | "as" | "ip" | "ap" | "iW" | "aW" | "it" | "at" | "ib" | "ab" => {
            vec![pick]
        }
        _ => {
            // sometimes a count before the motion
            if chance(20) {
                let n = (rng() % 9 + 1) as u8;
                vec![n.to_string(), pick]
            } else {
                vec![pick]
            }
        }
    }
}

fn gen_command() -> Vec<String> {
    let op: String = pick(&[
        "d", "y", "c", "gU", "gu", "g~", "x", "s", "C", "S", "R", "J", "~", "p", "P", "u", "o",
        "O", "A", "I",
    ])
    .to_string();
    match op.as_str() {
        "d" | "y" | "gU" | "gu" | "g~" => {
            let mut v: Vec<String> = Vec::new();
            if op.len() == 1 {
                v.push(op);
            } else {
                v.push("g".to_string());
                v.push(op[1..].to_string());
            }
            v.extend(gen_motion());
            v
        }
        "c" | "s" => {
            let mut v = if op == "c" {
                let mut v = gen_motion();
                v.insert(0, "c".to_string());
                v
            } else {
                vec!["s".to_string()]
            };
            v.push("<Esc>".to_string());
            v
        }
        "x" | "C" | "S" | "R" | "J" | "~" | "p" | "P" | "u" | "o" | "O" | "A" | "I" | "i" | "a" => {
            // insert-entering ops must be followed by <Esc> so vim doesn't
            // block waiting for input
            let mut v: Vec<String> = if chance(25) {
                vec![(rng() % 9 + 1).to_string()]
            } else {
                vec![]
            };
            v.push(op);
            v.push("<Esc>".to_string());
            v
        }
        _ => vec![op],
    }
}

fn gen_visual() -> Vec<String> {
    let vk: String = pick(&["v", "V", "<C-v>"]).to_string();
    let mut v: Vec<String> = vec![vk];
    v.extend(gen_motion());
    // op
    let op: String = pick(&["d", "y", "x", "c", "gU", "gu"]).to_string();
    if op == "c" {
        v.push("c".to_string());
        v.push("<Esc>".to_string());
    } else if op.len() == 2 {
        v.push("g".to_string());
        v.push(op[1..].to_string());
    } else {
        v.push(op);
    }
    v
}

fn gen_seq() -> Vec<String> {
    let mut v = Vec::new();
    let n = 1 + (rng() % 4);
    for _ in 0..n {
        let r = rng() % 10;
        if r < 5 {
            v.extend(gen_command());
        } else if r < 8 {
            // a plain motion
            v.extend(gen_motion());
        } else {
            v.extend(gen_visual());
        }
    }
    v
}

fn vim_keys(tokens: &[String]) -> String {
    let mut s = String::new();
    for t in tokens {
        s.push_str(match t.as_str() {
            "<Esc>" => "\\<Esc>",
            "<C-a>" => "\\<C-a>",
            "<C-x>" => "\\<C-x>",
            "<C-r>" => "\\<C-r>",
            "<C-v>" => "\\<C-v>",
            "<C-w>" => "\\<C-w>",
            "<C-u>" => "\\<C-u>",
            "<C-t>" => "\\<C-t>",
            "<C-d>" => "\\<C-d>",
            "<C-y>" => "\\<C-y>",
            "<C-e>" => "\\<C-e>",
            "<C-o>" => "\\<C-o>",
            "<BS>" => "\\<BS>",
            "<Space>" => " ",
            "<CR>" => "\\<CR>",
            other => other,
        });
    }
    s
}

fn oracle(initial: &str, tokens: &[String]) -> Option<(String, usize)> {
    use std::io::Write;
    let id = {
        let mut h = DefaultHasher::new();
        initial.hash(&mut h);
        tokens.hash(&mut h);
        h.finish()
    };
    let vin = format!("/tmp/ora_{id:x}.in");
    let vout = format!("/tmp/ora_{id:x}.out");
    let vcur = format!("/tmp/ora_{id:x}.cur");
    let vkeys = format!("/tmp/ora_{id:x}.keys");
    {
        let mut f = std::fs::File::create(&vin).ok()?;
        f.write_all(initial.as_bytes()).ok()?;
    }
    let keys = format!(
        ":set binary\r{}:w! {}\r:call writefile([string(line2byte(line(\".\"))+col(\".\")-1)],\"{}\")\r:qa!\r",
        vim_keys(tokens), vout, vcur
    );
    {
        let mut f = std::fs::File::create(&vkeys).ok()?;
        f.write_all(keys.as_bytes()).ok()?;
    }
    let mut child = std::process::Command::new("/usr/bin/vim")
        .args(["-Nu", "NONE", "-N", "-i", "NONE", "-n", "-s", &vkeys, &vin])
        .spawn()
        .ok()?;
    let pid = child.id();
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(8));
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status();
    });
    let status = child.wait().ok()?;
    let _ = watchdog.join();
    if !status.success() {
        return None;
    }
    let text = std::fs::read_to_string(&vout).unwrap_or_default();
    let cur = std::fs::read_to_string(&vcur)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()
        .map(|c| c.saturating_sub(1));
    let _ = std::fs::remove_file(&vin);
    let _ = std::fs::remove_file(&vkeys);
    let _ = std::fs::remove_file(&vout);
    let _ = std::fs::remove_file(&vcur);
    cur.map(|c| (text, c))
}

fn engine_run(initial: &str, tokens: &[String]) -> (String, usize) {
    let mut f = Fixture::new(initial);
    // align options with vim defaults to reduce false positives
    f.feed([
        ":",
        "s", "e", "t", " ", "t", "s", "=", "8", " ", "s", "w", "=", "8", " ", "n", "o", "e", "t",
        "<CR>",
    ]);
    f.feed(tokens.iter().map(|s| s.as_str()));
    (f.text(), f.cursor())
}

#[test]
#[ignore = "对拍 fuzz：发现新 bug"]
fn diff_fuzz() {
    let samples: Vec<&str> = vec![
        "abc def ghi\njkl mno\n",
        "abc def\n\nghi jkl\n",
        "a b c d e\n",
        "hello world\nfoo bar baz\n",
        "one\ntwo\nthree\nfour\n",
        "  indented line\nno indent\n",
        "tab\tsplit\nhere\n",
        "x\ny\nz\n",
        "the quick brown fox\njumps over\n",
        "",
        "a",
        "中文测试 hello\nworld 世界\n",
        "foo(bar)baz(qux)\n",
        "aaa\nbbb\nccc\n",
        "word1 word2 word3\n",
    ];
    let mut mismatches = 0;
    let n = 200;
    for i in 0..n {
        SEED.with(|s| *s.borrow_mut() = 0x1234_5678 + i as u64 * 2654435761);
        let initial = pick(&samples);
        let seq = gen_seq();
        let Some((otext, ocur)) = oracle(initial, &seq) else {
            continue;
        };
        let (etext, ecur) = engine_run(initial, &seq);
        if etext != otext {
            mismatches += 1;
            eprintln!(
                "TEXT MISMATCH #{i} init={initial:?} seq={seq:?}\n  engine={etext:?}\n  vim    ={otext:?}"
            );
        } else if ecur != ocur {
            // cursor-only diff; report sparsely
            if mismatches < 30 {
                eprintln!(
                    "CUR MISMATCH #{i} init={initial:?} seq={seq:?}\n  engine={ecur} vim={ocur}"
                );
            }
        }
        if mismatches >= 60 {
            break;
        }
    }
    eprintln!("total text mismatches: {mismatches}");
}
