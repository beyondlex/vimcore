//! Scratch probes for the audit round — deleted before delivery.
mod common;

use common::Fixture;

fn try_keys(vp: (usize, usize), keys: &[&str]) -> String {
    let mut f = Fixture::at("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n", 4, 0);
    f.host.viewport = vp;
    f.feed(keys);
    format!("cursor={} line={}", f.cursor(), f.line())
}

#[test]
fn scratch_isolate_panic() {
    for vp in [(0usize, 0usize), (usize::MAX, usize::MAX), (0, usize::MAX), (usize::MAX, 0), (9, 0)] {
        for keys in [&["<C-f>"][..], &["<C-b>"], &["<C-d>"], &["<C-u>"], &["<C-e>"], &["<C-y>"], &["z","z"], &["z","t"], &["z","b"], &["G"], &["g","g"]] {
            let r = std::panic::catch_unwind(|| try_keys(vp, keys));
            match r {
                Ok(s) => println!("vp={vp:?} keys={keys:?}: {s}"),
                Err(_) => println!("vp={vp:?} keys={keys:?}: PANIC"),
            }
        }
    }
}

#[test]
fn scratch_tilde_wide_cursor() {
    // ~ on a caseless wide char: does the cursor advance (vim) or stay?
    let mut f = Fixture::at("中文\n", 0, 0);
    f.feed(&["~"]);
    println!("~ on 中: cursor={} (vim: 2, on 文)", f.cursor());
    let mut f = Fixture::at("中文\n", 0, 0);
    f.feed(&["~", "~"]);
    println!("~~ : cursor={} text={:?}", f.cursor(), f.text());
    // ~ on combining-mark cluster
    let mut f = Fixture::at("e\u{0301}bc\n", 0, 0);
    f.feed(&["~"]);
    println!("~ on e+mark: cursor={} text={:?}", f.cursor(), f.text());
}

#[test]
fn scratch_offset_col1_check() {
    // /foo/1 with the match NOT at column 0: vim lands in COLUMN 1 of the line below
    let mut f = Fixture::at("xx foo bar\nzzzzz\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "/", "1", "<CR>"]);
    println!("/1 (match at col 3): cursor={} (vim: line1 col0 = 11)", f.cursor());
    let mut f = Fixture::at("xx foo bar\nzzzzz\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "/", "-", "<CR>"]);
    println!("/- : cursor={}", f.cursor());
}

#[test]
fn scratch_empty_offset_debug() {
    // understand //offset: what pattern got stored?
    let mut f = Fixture::at("foo\nl1\nl2\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "<CR>"]);
    println!("after /foo: cursor={} hl={:?}", f.cursor(), f.host.highlights);
    f.feed(&["/", "<CR>"]);
    println!("after /<CR>: cursor={} hl={:?} statuses={:?}", f.cursor(), f.host.highlights, f.host.statuses);
    let mut f = Fixture::at("foo\nl1\nl2\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "<CR>", "/", "1", "<CR>"]);
    println!("after /1 (typed offset): cursor={} hl={:?} statuses={:?}", f.cursor(), f.host.highlights, f.host.statuses);
    let mut f = Fixture::at("foo\nl1\nl2\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "<CR>", "/", "e", "<CR>"]);
    println!("after /e (typed): cursor={} statuses={:?}", f.cursor(), f.host.statuses);
}

#[test]
fn scratch_dip_empty_vimshape() {
    // vim: dip on a one-empty-line buffer errors? engine silent no-op
    let mut f = Fixture::new("\n");
    f.feed(&["d", "i", "p"]);
    println!("dip on single empty line: text={:?} bells={} statuses={:?}", f.text(), f.host.bells, f.host.statuses);
    // dap on single empty line
    let mut f = Fixture::new("\n");
    f.feed(&["d", "a", "p"]);
    println!("dap on single empty line: text={:?} bells={}", f.text(), f.host.bells);
}

#[test]
fn scratch_star_search_misc2() {
    // 2/pat via prompt count
    let mut f = Fixture::at("foo\nfoo\nfoo\n", 0, 0);
    f.feed(&["2", "/", "f", "o", "o", "<CR>"]);
    println!("2/foo: cursor={} (expect 8)", f.cursor());
    // history <Up>
    let mut f = Fixture::at("foo\nbar\n", 0, 0);
    f.feed(&["/", "f", "o", "o", "<CR>", "/", "<Up>", "<CR>"]);
    println!("/<Up><CR>: cursor={} (expect 0)", f.cursor());
    // * count
    let mut f = Fixture::at("foo\nfoo\nfoo\nfoo\n", 0, 0);
    f.feed(&["3", "*"]);
    println!("3*: cursor={} (expect 8)", f.cursor());
    // gD/gd pending operator shapes don't hang
    let mut f = Fixture::at("foo\n", 0, 0);
    f.feed(&["d", "g", "d"]);
    println!("dgd: text={:?} bells={}", f.text(), f.host.bells);
}
