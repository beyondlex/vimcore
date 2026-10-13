# vimcore development reference strategy (how we work, aligned to vim 9.1 semantics)

The spec of this engine is vim itself. **Oracle hierarchy**: the actual
behavior of `vim -Nu NONE -N` is the final judge; the vim source is the
"rule interpreter", used to upgrade a single empirical data point into a
general understanding; this file tells you where to read, how to read,
and what not to do.

## Local vim source (read-only reference)

- Path: `~/code/github/vim` (a clone of GitHub vim/vim).
- The working tree is checked out at **v9.1.1752**, matching
  `/usr/bin/vim` (9.1 patches 1-1752). To see newer evolution run
  `git checkout master` (9.2+), but semantic rulings are based on the
  9.1 side.
- **Clean-room discipline**: read the source to understand the rules,
  then implement in the engine's own expression. Line-by-line
  translation is forbidden — vim is under the Vim License/Charityware
  while this crate is MIT/Apache dual-licensed, so translating would
  create a derivative work and contaminate the license. Test porting
  follows the same rule: port the **behavioral assertions**, annotate
  `Source: vim src/testdir/test_xxx.vim`, never port the code.

## Source map (domain -> functions to read first)

| Domain | Location |
|---|---|
| Ex addresses/ranges (tokenizing `:5 + 2`, clamping bare addresses) | `src/ex_docmd.c` `get_address()` |
| Search execution and acceptance rules (skipping / stopping at the cursor match) | `src/search.c` `do_search()` / `searchit()` |
| Text objects (the cross-line whitespace shape of aw/iw) | `src/search.c` `current_search()` |
| Operators and exclusive-linewise (the d2w vs 2cw divide) | `src/ops.c` `do_pending_operator()` (the famous adjustment-comment block) |
| Join/case/indent | `src/ops.c` `do_join()` / `do_tilde` etc. |
| Put and registers (linewise/blockwise, the unnamed-register asymmetry of `"A` append) | `src/register.c` |
| Insert-mode control keys (C-t/d/y/e/w/u, C-r, 0<C-d>) | `src/edit.c` main switch of `insert()` (direct dispatch for `case Ctrl_W` etc.; backspace semantics live in the mode parameter of `ins_bs(c, BACKSPACE_WORD/LINE)`) |
| The `:s` replacement side (`\u`/`&`/`~` special chars) | `src/ex_cmds.c` `do_sub()` and the regsub logic |
| `:set` | `src/option.c` / `optionstr.c` |
| marks/changelist/jumplist | `src/mark.c` |
| Command-line editing and history (the `<Up>` prefix match) | `src/cmdhist.c` |
| Macro record/replay (`q"`, `@`, visual `@`) | `src/register.c` `do_execreg()` |

How to read: grep the function name first, then follow the calls to the
decision conditions; special cases are often written directly in the
comments (the exclusive-linewise comment in `do_pending_operator` is
required reading). Note that the source logic is heavily conditioned on
`'cpoptions'`/`'selection'`/`'virtualedit'` — the engine implements only
a subset of options and has deliberately diverged on defaults
(ignorecase, ts=4, etc.); before copying a branch, confirm the engine's
stance on that option (see the known-divergence ledger in NOTES.md).

## Porting testdir behavior tests (the highest-value audit material)

- Location: `~/code/github/vim/src/testdir/test_*.vim` (318 files).
  The most relevant: `test_textobjects.vim`, `test_registers.vim`,
  `test_search.vim`, `test_put.vim`, `test_visual.vim`,
  `test_blockedit.vim`, `test_normal.vim`, `test_cmdline.vim`,
  `test_marks.vim`, `test_join.vim`, `test_indent.vim`.
- Usage: pick files by the domain currently under audit, convert the
  assertions relevant to the engine's subset into probes under `tests/`
  (most that fail are options/features the engine has not implemented —
  record them in the divergence ledger rather than forcing them
  through). When a behavior discovered this way is written into a
  regression test, annotate the source file.
- This is the systematic replacement for adversarial black-box
  trial-and-error: of the 60 bugs in the current pass (round 32),
  roughly over half could have been caught earlier by
  `test_textobjects` + `test_registers` + `test_put`.

## Oracle discipline (acceptance criteria for fixes)

- Semantic fixes must be evidenced by `/usr/bin/vim` (9.1). Channel
  selection: `-Nu NONE -N -i NONE -s <keys>` typeahead is the default
  channel; **arrow keys, `:` under visual, and bell-class behaviors must
  go through expect PTY** (lessons from many NOTES rounds: `-es`
  feedkeys is unreliable for macros/arrow keys).
- Known sampling traps (every one hit is on record in NOTES):
  `writefile()` writes newlines as NUL (to sample registers use
  `getreg` + substituted display, or `:w!` then read the file); a
  leftover swap file from a same-named probe swallows keys and
  invalidates the whole batch (use a unique file name per case);
  nesting a List inside `writefile`'s List argument raises E730.
- To construct an "empty register" only `"_x` (blackhole) is valid:
  `x` fills the register, so a subsequent `p` is a real put (a test
  setup lesson fixed in round-32).
- Byte-level assertions (buffer text) are judged by the od of the file
  after vim `:w` writes it back — the engine's noeol state gets a
  terminator appended on the vim side by `'fixendofline'`.
- For defaults the engine deliberately diverges on (ignorecase=on,
  ts=4, tw=78, etc.; see NOTES divergence #2/#3), probes must first pin
  the oracle premises with `:set` before asserting.

## Fix workflow conventions

1. First run the target-domain probe to confirm the failure shape, then
   consult the source map in this file to read the rules, bring in
   oracle evidence when needed, and only then change the
   implementation.
2. Each fix lands one probe/regression case (audit2_* plus the existing
   parity/review suites); "deleting a test along the way" is forbidden.
3. A newly discovered deliberate vim special case (e.g. the unnamed
   asymmetry of `"Add`/`"Ayy`) that conflicts with intuition must be
   implemented as vim does and recorded in the NOTES.md divergence
   ledger or the falsification list.
4. When fixing A breaks B, first suspect that the semantic model shared
   by both is wrong, rather than patching the old pin; when a test
   conflicts with the oracle, the oracle wins and the test is corrected
   (with the empirical evidence noted). Old NOTES conclusions may also
   be re-examined and overturned (re-test before changing).
