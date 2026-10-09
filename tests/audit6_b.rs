//! audit6_b：第五轮独立审计·智能体 B 域探针
//! （块可视 C-v 与块编辑 / Normal motion / 文本对象 / 算子 / 可视 motion 形状）。
//!
//! 每个 `#[test]` 对应一个 oracle（本机 /usr/bin/vim = 9.1 patches 1-1752）
//! 证实的 bug，编号 E-B-n；断言写 oracle 期望值，修复前应当失败。
//!
//! 批量对拍 runner（`batch_probe`，#[ignore]）：读 /tmp/oracle6/cases.tsv，
//! 逐例跑引擎、把结果写 /tmp/oracle6/eng_out.tsv，供 python 侧与 vim
//! typeahead/PTY 结果 diff。TSV 列：
//!   name \t hex(buf) \t line \t col \t keys(空格分词,Key::parse 记号)
//!   \t pre_ex(:开头 ex，先喂) \t set_vim(oracle 侧 :set 前提)
//! 输出列：
//!   name \t line \t col \t hex(text) \t mode \t bells \t regpayload
//! regpayload = hex(unnamed)+'/'+kind(v,V,\x16+width)；空寄存器 = '-'。
//!
//! oracle 通道记录在各 bug 注释里：typeahead（python 写缓冲+按键 →
//! vim -Nu NONE -N -i NONE -n -s keys buf）为主；方向键/响铃/块可视怪键
//! 走 expect PTY（/tmp/oracle6/oracle.py 的 ptty()）。

mod common;

use common::Fixture;
use vimcore::buffer::VimBuffer;

fn hex_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i + 1 < b.len() {
        let hi = (b[i] as char).to_digit(16).unwrap() as u8;
        let lo = (b[i + 1] as char).to_digit(16).unwrap() as u8;
        out.push(hi << 4 | lo);
        i += 2;
    }
    String::from_utf8(out).unwrap()
}

fn hex_encode(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn reg_payload(f: &Fixture) -> String {
    match f.vim.registers.get('"') {
        None => "-".to_owned(),
        Some(r) => {
            let kind = match r.kind {
                vimcore::registers::RegisterKind::Charwise => "v".to_owned(),
                vimcore::registers::RegisterKind::Linewise => "V".to_owned(),
                vimcore::registers::RegisterKind::Blockwise => {
                    // vim getregtype 的 \x16 后随宽度；引擎寄存器不带宽度，
                    // 用最宽行近似（比较时 python 侧同式计算）
                    let w = r.text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0);
                    format!("\u{16}{w}")
                }
            };
            format!("{}/{}", hex_encode(&r.text), kind)
        }
    }
}

fn run_case(pre_ex: &str, keys: &str, buf: &str, line: usize, col: usize) -> Fixture {
    let mut f = Fixture::at(buf, line, col);
    if !pre_ex.is_empty() {
        let mut ks: Vec<String> = vec![":".to_owned()];
        ks.extend(pre_ex.chars().map(|c| c.to_string()));
        ks.push("<CR>".to_owned());
        f.feed(ks);
    }
    // `<t:XY>` tokens go through the IME text path (record_typed_text +
    // insert_text_at_cursor) — the engine's input contract delivers
    // printables as text events, only commands as keys. `+` inside the
    // token becomes a space. Tokens are whitespace-delimited so `<C-v>`
    // never trips the typed-segment check.
    for tok in keys.split_whitespace() {
        if tok.starts_with("<t:") && tok.ends_with('>') {
            f.type_text(&tok[3..tok.len() - 1].replace('+', " "));
        } else {
            f.feed([tok]);
        }
    }
    f
}

#[test]
#[ignore = "批量对拍 runner：读 /tmp/oracle6/cases.tsv 写 eng_out.tsv"]
fn batch_probe() {
    let data = std::fs::read_to_string("/tmp/oracle6/cases.tsv").unwrap();
    let mut out = String::new();
    for line in data.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let (name, hbuf, hline, hcol, keys, pre, _set) =
            (c[0], c[1], c[2], c[3], c[4], c[5], c.get(6).unwrap_or(&""));
        eprintln!("RUN {name}");
        let buf = hex_decode(hbuf);
        let mut f = run_case(pre, keys, &buf, hline.parse().unwrap(), hcol.parse().unwrap());
        // flush progress so an engine panic still names its case
        let l = f.buf.offset_to_line(f.vim.cursor.offset);
        let colb = f.vim.cursor.offset - f.buf.line_range(l).start + 1;
        let text = f.text();
        out.push_str(&format!(
            "{name}\t{}\t{colb}\t{}\t{:?}\t{}\t{}\n",
            l + 1,
            hex_encode(&text),
            f.vim.mode(),
            f.host.bells,
            reg_payload(&f),
        ));
    }
    std::fs::write("/tmp/oracle6/eng_out.tsv", out).unwrap();
}

#[test]
#[ignore = "临时诊断：逐键模式轨迹"]
fn trace_block_x() {
    let mut f = Fixture::new("long line here\nabc\ndef\n");
    for k in ["<C-v>", "j", "l", "l", "X"] {
        f.feed([k]);
        eprintln!(
            "after {k:?}: mode={:?} cursor={} bells={} text={:?}",
            f.vim.mode(),
            f.cursor(),
            f.host.bells,
            f.text()
        );
    }
}

