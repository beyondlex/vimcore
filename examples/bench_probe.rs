//! Performance probe: measure the real cost of the engine's hot paths
//! (the benchmark command in the README "Development" section).
//!
//! Two host shapes for contrast: `B` is a naive `String` host (its
//! `offset_to_line` is O(n) — `w` on a 1MB line exposes the host's
//! line-lookup cost), `R` is a ropey host (O(log n), isolating the
//! engine's own overhead). See the NOTES performance section for the
//! `refresh_highlights`/`hlsearch_live_update` trade-off.
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;
use vimcore::buffer::{VimBuffer, VimBufferMut};
use vimcore::host::VimHost;
use vimcore::key::Key;
use vimcore::state::{Ctx, VimState};

#[derive(Clone)]
struct B(Rc<RefCell<String>>);
impl VimBuffer for B {
    fn len(&self) -> usize {
        self.0.borrow().len()
    }
    fn line_count(&self) -> usize {
        self.0.borrow().split('\n').count()
    }
    fn char_at(&self, o: usize) -> Option<char> {
        self.0.borrow()[o..].chars().next()
    }
    fn prev_char_offset(&self, o: usize) -> Option<usize> {
        if o == 0 {
            return None;
        }
        self.0.borrow()[..o]
            .chars()
            .next_back()
            .map(|c| o - c.len_utf8())
    }
    fn line_range(&self, line: usize) -> Range<usize> {
        let t = self.0.borrow();
        let mut s = 0;
        for (i, p) in t.split('\n').enumerate() {
            if i == line {
                return s..s + p.len() + usize::from(i + 1 < t.split('\n').count());
            }
            s += p.len() + 1;
        }
        t.len()..t.len()
    }
    fn offset_to_line(&self, o: usize) -> usize {
        self.0.borrow()[..o.min(self.0.borrow().len())]
            .split('\n')
            .count()
            - 1
    }
    fn slice(&self, r: Range<usize>) -> String {
        self.0.borrow()[r].to_owned()
    }
}
impl VimBufferMut for B {
    fn insert_text(&mut self, o: usize, t: &str) -> bool {
        self.0.borrow_mut().insert_str(o, t);
        true
    }
    fn delete_range(&mut self, r: Range<usize>) -> bool {
        self.0.borrow_mut().replace_range(r, "");
        true
    }
}

/// ropey-backed buffer: the "good host" (O(log n) line lookups), mirroring
/// the demo's `RopeBuffer`
#[derive(Clone)]
struct R(Rc<RefCell<ropey::Rope>>);
impl VimBuffer for R {
    fn len(&self) -> usize {
        self.0.borrow().len_bytes()
    }
    fn line_count(&self) -> usize {
        self.0.borrow().len_lines()
    }
    fn char_at(&self, o: usize) -> Option<char> {
        let rope = self.0.borrow();
        let ci = rope.try_byte_to_char(o).ok()?;
        rope.get_char(ci)
    }
    fn prev_char_offset(&self, o: usize) -> Option<usize> {
        let rope = self.0.borrow();
        if o == 0 || o > rope.len_bytes() {
            return None;
        }
        let ci = rope.try_byte_to_char(o).ok()?;
        if ci == 0 {
            return None;
        }
        let prev = rope.get_char(ci - 1)?;
        Some(o - prev.len_utf8())
    }
    fn line_range(&self, line: usize) -> Range<usize> {
        let rope = self.0.borrow();
        if line >= rope.len_lines() {
            return rope.len_bytes()..rope.len_bytes();
        }
        let start = rope.line_to_byte(line);
        let end = if line + 1 < rope.len_lines() {
            rope.line_to_byte(line + 1)
        } else {
            rope.len_bytes()
        };
        start..end
    }
    fn offset_to_line(&self, o: usize) -> usize {
        let rope = self.0.borrow();
        rope.try_byte_to_line(o.min(rope.len_bytes())).unwrap_or(0)
    }
    fn slice(&self, r: Range<usize>) -> String {
        let rope = self.0.borrow();
        let (s, e) = (
            rope.try_byte_to_char(r.start).unwrap_or(0),
            rope.try_byte_to_char(r.end).unwrap_or(0),
        );
        rope.slice(s..e).to_string()
    }
}
impl VimBufferMut for R {
    fn insert_text(&mut self, o: usize, t: &str) -> bool {
        let mut rope = self.0.borrow_mut();
        let ci = rope.byte_to_char(o);
        rope.insert(ci, t);
        true
    }
    fn delete_range(&mut self, r: Range<usize>) -> bool {
        let mut rope = self.0.borrow_mut();
        let (s, e) = (rope.byte_to_char(r.start), rope.byte_to_char(r.end));
        rope.remove(s..e);
        true
    }
}
#[derive(Default)]
struct H {
    hl: Vec<Range<usize>>,
}
impl VimHost for H {
    fn viewport(&self) -> (usize, usize) {
        (0, 40)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, m: &[Range<usize>], _: Option<Range<usize>>) {
        self.hl = m.to_vec()
    }
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
    fn status_message(&mut self, _: &str) {}
}

fn feed<B: VimBufferMut>(vim: &mut VimState, buf: &mut B, h: &mut H, keys: &[Key]) {
    for k in keys {
        let mut ctx = Ctx { buf, host: h };
        vim.handle_key(&mut ctx, k.clone());
    }
}

fn main() {
    // ---- 1. plain motion: 10k keystrokes ----
    let body = "lorem ipsum dolor sit amet ".repeat(400); // ~11KB long line + a short line
    let mut buf = B(Rc::new(RefCell::new(format!("{body}\nsecond line here\n"))));
    let mut vim = VimState::new();
    let mut h = H::default();
    let keys: Vec<Key> = std::iter::repeat_n(Key::parse("w"), 2000).collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &keys);
    println!(
        "2000x 'w' motion (11KB buffer): {:>8.2?}  ({:.1} us/key)",
        t.elapsed(),
        t.elapsed().as_micros() as f64 / 2000.0
    );

    // ---- 2. `w` motion on a large buffer (1MB) ----
    let big = "word ".repeat(200_000); // one 1MB line
    let mut buf = B(Rc::new(RefCell::new(big)));
    let mut vim = VimState::new();
    let mut h = H::default();
    let keys: Vec<Key> = std::iter::repeat_n(Key::parse("w"), 200).collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &keys);
    println!(
        "200x 'w' motion (1MB line):     {:>8.2?}  ({:.1} us/key)",
        t.elapsed(),
        t.elapsed().as_micros() as f64 / 200.0
    );

    // ---- 3. search + persistent hlsearch: republish cost per edit ----
    let lines: String = "the quick brown fox jumps over the lazy dog\n".repeat(20_000); // ~900KB
    let mut buf = B(Rc::new(RefCell::new(lines)));
    let mut vim = VimState::new();
    let mut h = H::default();
    feed(
        &mut vim,
        &mut buf,
        &mut h,
        &[
            Key::parse("/"),
            Key::char('f'),
            Key::char('o'),
            Key::char('x'),
            Key::named("enter"),
        ],
    );
    let matches = h.hl.len();
    let t = Instant::now();
    // republish_search's core cost = the whole-file regex scan in all_matches;
    // bump is crate-internal, so measure 20 equivalent search scans directly
    for _ in 0..20 {
        let _ = vimcore::search::all_matches(&vim, &buf, "fox");
    }
    println!(
        "20x search re-scan (900KB, {} matches): {:>8.2?}  ({:.2} ms/scan)",
        matches,
        t.elapsed(),
        t.elapsed().as_millis() as f64 / 20.0
    );

    // ---- 4. :%s whole-file substitute ----
    let lines: String = "foo bar baz\n".repeat(50_000); // ~600KB
    let mut buf = B(Rc::new(RefCell::new(lines)));
    let mut vim = VimState::new();
    let mut h = H::default();
    let cmd: Vec<Key> = ":s/foo/bar/g"
        .chars()
        .map(Key::char)
        .chain(std::iter::once(Key::named("enter")))
        .collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &cmd);
    println!(":%s/foo/bar/g (600KB, 50k lines): {:>8.2?}", t.elapsed());

    // ---- 5. gq reflow of a 200-line paragraph ----
    let para =
        "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod\n".repeat(200);
    let mut buf = B(Rc::new(RefCell::new(para)));
    let mut vim = VimState::new();
    vim.options_mut().textwidth = 80;
    let mut h = H::default();
    let cmd: Vec<Key> = ["g", "q", "G"].iter().map(|k| Key::parse(k)).collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &cmd);
    println!("gqG reflow (200 lines):          {:>8.2?}", t.elapsed());

    // ---- 6. `x` delete (10k-line buffer) ----
    let mut buf = B(Rc::new(RefCell::new("line of text\n".repeat(10_000))));
    let mut vim = VimState::new();
    let mut h = H::default();
    let keys: Vec<Key> = std::iter::repeat_n(Key::parse("x"), 100).collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &keys);
    println!(
        "100x 'x' delete (10k lines):     {:>8.2?}  ({:.1} us/key)",
        t.elapsed(),
        t.elapsed().as_micros() as f64 / 100.0
    );

    // ---- 7. repeated `n` jumps (900KB, 10k matches, ropey buffer) ----
    // With hlsearch live, each `n` is a whole-file slice + regex rescan when the
    // match cache misses; the ropey buffer removes the naive line-lookup host
    // cost, leaving the engine's own overhead
    let lines: String = "the quick brown fox jumps over the lazy dog\n".repeat(20_000); // ~900KB
    let mut buf = R(Rc::new(RefCell::new(ropey::Rope::from(lines))));
    let mut vim = VimState::new();
    let mut h = H::default();
    feed(
        &mut vim,
        &mut buf,
        &mut h,
        &[
            Key::parse("/"),
            Key::char('f'),
            Key::char('o'),
            Key::char('x'),
            Key::named("enter"),
        ],
    );
    let keys: Vec<Key> = std::iter::repeat_n(Key::parse("n"), 200).collect();
    let t = Instant::now();
    feed(&mut vim, &mut buf, &mut h, &keys);
    println!(
        "200x 'n' jump (900KB ropey):    {:>8.2?}  ({:.1} us/key)",
        t.elapsed(),
        t.elapsed().as_micros() as f64 / 200.0
    );
}
