//! Bug-hunt probes for the cmdline / history / register-at-prompt / insert
//! control-key domains. Each case was sampled against `/usr/bin/vim` (9.1.1752)
//! through an expect PTY (`:messages` redirected to a file, buffer written with
//! `:w!`), so the expectation is oracle evidence, not intuition.
//!
//! Status: every test here is a REPRODUCED DIVERGENCE, left in failing shape
//! (do not "fix" the assertion without re-running the oracle).

mod common;

use common::Fixture;

fn ex(f: &mut Fixture, line: &str) {
    let mut keys: Vec<String> = vec![":".to_owned()];
    keys.extend(line.chars().map(|c| c.to_string()));
    keys.push("<CR>".to_owned());
    f.feed(keys);
}

/// `c_CTRL-R <C-O> {reg}` at the command line inserts the register LITERALLY —
/// `src/ex_getln.c` `cmdline_insert_reg()`: `if (i == Ctrl_O) i = Ctrl_R;`, so
/// `<C-r><C-o>x` is the same key sequence as `<C-r><C-r>x` (and
/// `cmdline_paste_reg()` forces `literally = TRUE` for any valid yank register,
/// so plain `<C-r>x` agrees too).
///
/// oracle (`:"ayl` then `:echo <C-r><C-o>a<CR>`): `E121: Undefined variable: h`
/// — the register content ("h") went into the line, the `a` was consumed.
/// engine: the `<C-o>` drops the pending state and the following `a` is typed
/// as a plain character → the command line reads `a`.
/// Root cause: `src/cmdline.rs:761-779` (the pending-`<C-r>` arm has no
/// Ctrl_R/Ctrl_O literal handling; `src/insert_mode.rs:22-32` does have it).
#[test]
fn cmdline_ctrl_r_ctrl_o_inserts_the_regname_literally() {
    let mut f = Fixture::new("hello\nworld\n");
    f.feed(["\"", "a", "y", "l"]); // charwise register a = "h"
    f.feed([":", "<C-r>", "<C-o>", "a"]);
    assert_eq!(
        f.vim.cmdline.buffer, "h",
        "`:<C-r><C-o>a` must insert register a (oracle E121: Undefined variable: h), \
         engine cmdline = {:?}",
        f.vim.cmdline.buffer
    );
}

/// The `<Up>`/`<Down>` history filter is the text BEFORE the editing cursor, not
/// the whole line: `src/ex_getln.c` `cmdline_browse_history()` saves the line and
/// truncates it (`lookfor[ccline.cmdpos] = NUL; lookforlen = ccline.cmdpos;`),
/// and `c_CTRL-B`/`<Home>`/`<End>` are not in the exempt key list, so they reset
/// the remembered prefix too.
///
/// oracle (history has `set ts=4`; then `:z<C-b><Up><CR>`): NO message — the
/// empty prefix matched, `set ts=4` was recalled and executed.
/// engine: prefix = the whole buffer "z" → no match → bell, and `:z` runs →
/// `E492: Not an editor command: z`.
/// Root cause: `src/cmdline.rs:1097-1101` uses `self.cmdline.buffer.clone()`.
#[test]
fn history_prefix_must_be_the_text_before_the_cursor() {
    let mut f = Fixture::new("x\n");
    ex(&mut f, "set ts=4");
    f.feed([":", "z", "<C-b>", "<Up>", "<CR>"]);
    assert!(
        !f.host.statuses.iter().any(|s| s.contains("E492")),
        "`:z<C-b><Up>` must recall `set ts=4` (oracle: silent); engine statuses = {:?}",
        f.host.statuses
    );

    // same shape through the arrow key: <Left> keeps the (empty-prefix) browse
    let mut g = Fixture::new("x\n");
    ex(&mut g, "set ts=4");
    g.feed([":", "z", "<Left>", "<Up>", "<CR>"]);
    assert!(
        !g.host.statuses.iter().any(|s| s.contains("E492")),
        "`:z<Left><Up>` must recall `set ts=4`; engine statuses = {:?}",
        g.host.statuses
    );
}

/// Editing a recalled entry re-derives the browse prefix from the CURRENT line:
/// `src/ex_getln.c` clears `lookfor` for every key outside the browse/arrow set
/// (`VIM_CLEAR(lookfor); lookforlen = 0;`), so a `<BS>` makes the next `<Up>`
/// search with the edited line as the prefix — and when nothing matches it
/// leaves the line untouched (no bell either).
///
/// oracle (`:aaa<CR>:bbb<CR>:<Up><BS><Up><CR>`): `E492: aaa`, `E492: bbb`,
/// `E492: Not an editor command: bb` — the line stayed "bb".
/// engine: the stash prefix ("" from the pre-browse line) survives the edit, so
/// the second `<Up>` keeps walking and lands on "aaa".
/// Root cause: `src/cmdline.rs:896` only clears `history_pos` for typed
/// characters (not `<BS>`/`<Del>`) and `src/cmdline.rs:1099` prefers the stale
/// `stash` as the prefix.
#[test]
fn history_up_after_editing_walks_with_a_stale_prefix() {
    let mut f = Fixture::new("x\n");
    ex(&mut f, "aaa");
    ex(&mut f, "bbb");
    f.feed([":", "<Up>", "<BS>", "<Up>", "<CR>"]);
    let last = f.host.statuses.last().cloned().unwrap_or_default();
    assert_eq!(
        last,
        "E492: Not an editor command: bb",
        "after `<Up><BS>` the next `<Up>` must search with the edited line (oracle \
         executed `bb`); engine statuses = {:?}",
        f.host.statuses
    );
}

/// The terminator of a completed `i_CTRL-V` numeric code is unget()c and
/// processed as a normal key (`src/edit.c` `get_literal()`: `vungetc(c)`), so
/// control/edited keys after the digits still act: `i<C-v>6<C-h>` deletes the
/// 0x06 it just inserted, `i<C-v>6<CR>x` inserts 0x06, a NEWLINE and `x`.
///
/// oracle: `i<C-v>6<C-h><Esc>` on "start" → buffer unchanged `start\n`;
///         `i<C-v>6<CR>x<Esc>` on "start" → bytes `06 \n x s t a r t \n`.
/// engine: both leave `\u{6}start\n` — the terminator is swallowed
/// (`src/insert_mode.rs:439-448` `finish_quote_terminator` only knows Esc).
#[test]
fn ctrl_v_numeric_terminator_is_swallowed_instead_of_reprocessed() {
    let mut f = Fixture::new("start\n");
    f.feed(["i", "<C-v>", "6", "<C-h>", "<Esc>"]);
    assert_eq!(
        f.text(),
        "start\n",
        "`i<C-v>6<C-h><Esc>`: the unget<C-h> must delete the 0x06 (oracle keeps \
         `start`); engine = {:?}",
        f.text()
    );

    let mut g = Fixture::new("start\n");
    g.feed(["i", "<C-v>", "6", "<CR>", "x", "<Esc>"]);
    assert_eq!(
        g.text(),
        "\u{6}\nxstart\n",
        "`i<C-v>6<CR>x<Esc>`: the unget<CR> breaks the line (oracle bytes \
         06 \\n x start); engine = {:?}",
        g.text()
    );
}
