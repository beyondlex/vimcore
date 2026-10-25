//! audit6_b: agent-domain-B probes of the fifth independent audit round
//! (blockwise visual C-v and block editing / Normal motions / text objects /
//! operators / visual motion shapes).
//!
//! Each `#[test]` corresponds to a bug confirmed by the oracle (local
//! /usr/bin/vim = 9.1 patches 1-1752), numbered E-B-n; assertions carry the
//! oracle-expected values and should fail before the fix.
//!
//! Batch differential runner (`batch_probe`, #[ignore]): reads
//! /tmp/oracle6/cases.tsv, runs each case through the engine, writes results to
//! /tmp/oracle6/eng_out.tsv for the python side to diff against vim
//! typeahead/PTY results. TSV columns:
//!   name \t hex(buf) \t line \t col \t keys(space-separated, Key::parse tokens)
//!   \t pre_ex(ex starting with :, fed first) \t set_vim(:set premises on the oracle side)
//! Output columns:
//!   name \t line \t col \t hex(text) \t mode \t bells \t regpayload
//! regpayload = hex(unnamed)+'/'+kind(v,V,\x16+width); empty register = '-'.
//!
//! The oracle channel used is recorded in each bug's comment: typeahead (python
//! writes the buffer + keystrokes → vim -Nu NONE -N -i NONE -n -s keys buf) is
//! primary; arrow keys/bell/blockwise-visual odd keys go through expect PTY
//! (ptty() in /tmp/oracle6/oracle.py).

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
    match f.vim.registers.lock().unwrap().get('"') {
        None => "-".to_owned(),
        Some(r) => {
            let kind = match r.kind {
                vimcore::registers::RegisterKind::Charwise => "v".to_owned(),
                vimcore::registers::RegisterKind::Linewise => "V".to_owned(),
                vimcore::registers::RegisterKind::Blockwise => {
                    // vim getregtype appends the width after \x16; the engine's registers carry no
                    // width, so approximate with the widest line (the python side computes the same
                    // way when comparing)
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
#[ignore = "batch differential runner: reads /tmp/oracle6/cases.tsv, writes eng_out.tsv"]
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
        let f = run_case(pre, keys, &buf, hline.parse().unwrap(), hcol.parse().unwrap());
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
#[ignore = "temporary diagnostic: per-key mode trace"]
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

