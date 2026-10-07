//! The data-driven command table.
//!
//! Every built-in command is one row: a key sequence, the phase it applies
//! in, and what it does. Tries per phase are built once in `VimState::new`.
//! Adding a command = one row + (if new) one handler match arm.

use crate::key::Key;
use crate::keymap::{ModeClass, Trie};
use crate::motions::Motion;
use crate::objects::TextObject;
use crate::ops::Operator;
use crate::state::InsertKind;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Normal,
    /// Operator-pending (between an operator and its motion/object).
    Pending,
    Visual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalCmd {
    DeleteCharForward,  // x
    DeleteCharBackward, // X
    SubstituteChar,     // s
    SubstituteLine,     // S
    ChangeToEnd,        // C
    DeleteToEnd,        // D
    YankLine,           // Y
    ReplaceChar,        // r{char}
    ToggleChar,         // ~
    PutAfter,           // p
    PutBefore,          // P
    Join,               // J
    JoinLiteral,        // gJ
    Undo,               // u
    Redo,               // <C-r>
    MarkSet,            // m{char}
    RecordMacro,        // q{reg}
    PlayMacro,          // @{reg} / @@
    JumpMark {
        linewise: bool,
    }, // '{char} / `{char
    LinewiseOp(Operator), // guu / gUU / g~~ / gugu ...
    ScrollCenter,       // zz
    ScrollTop,          // zt
    ScrollBottom,       // zb
    RestoreVisual,      // gv
    RepeatChange,       // .
    JumpBackward,       // C-o
    JumpForward,        // C-i
    OlderChange,        // g;
    NewerChange,        // g,
    /// gn / gN: select the next match (enter visual; reshape an existing
    /// selection). With an operator (`dgn`) the span is the match itself.
    SelectMatch {
        backward: bool,
    },
    IncrementNumber, // C-a
    DecrementNumber, // C-x
    WriteQuit,       // ZZ
    QuitNoSave,      // ZQ
    ScrollLines {
        down: bool,
    }, // C-e / C-y: scroll the view one line
    /// `&`: repeat the last `:s` on the current line (the stored command
    /// line re-runs through `execute_ex`).
    RepeatSubstitute,
    /// `g&`: repeat the last :s over the WHOLE file, keeping flags
    RepeatSubstituteGlobal,
    /// `gp`: paste after, cursor just past the new text
    PutAfterGp,
    /// `gP`: paste before, cursor just past the new text
    PutBeforeGp,
    /// `ga`: character code info
    CharInfo,
    /// `]p`/`[p`/`]P`/`[P`: linewise paste with the indent adjusted to the
    /// current line (`:h ]p`, audit M1)
    PasteIndent { below: bool },
    /// `g8`: UTF-8 byte info
    ByteInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualCmd {
    Exit, // same-kind v / V / Esc handled elsewhere
    ToggleKind {
        to: char,
    }, // v / V
    SwapEnds, // o
    /// `O` (block mode only): cursor to the same column on the block's other
    /// row-end (vim: "same column, but cursor at the other end").
    SwapEndsKeepCol, // O
    PutReplace, // p / P replace selection
    /// `g C-a`: SEQUENTIAL increment — each selected line's number grows by
    /// one more than the previous (audit K1)
    SequentialIncrement,
    Join {
        literal: bool,
    }, // J / gJ
    /// `r{char}`: replace the whole selection with `char` (char-arg).
    ReplaceChar,
    /// zz / zt / zb inside visual mode: vim scrolls the view AND keeps the
    /// selection (round11 悬置闭环). The engine only reports the anchor;
    /// hosts own the viewport.
    Scroll(crate::host::ScrollAnchor),
    /// `Y`/`D`/`X`/`C`/`S`: vim's LINEWISE visual spellings (`:h v_Y` —
    /// v_D/v_X delete the covered LINES, v_Y yanks them linewise, v_C/v_S
    /// linewise-change them; 9.1 probes). The row keys still read as plain
    /// operators in the table below; this variant routes them to the
    /// covered-lines span instead of the charwise selection.
    LinewiseOp(Operator),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdKind {
    Motion(Motion),
    Object(TextObject),
    Operator(Operator),
    EnterInsert(InsertKind),
    EnterVisual(crate::mode::VisualKind),
    Normal(NormalCmd),
    Visual(VisualCmd),
}

impl CmdKind {
    /// Commands that consume the next key as an argument.
    pub fn takes_char(self) -> bool {
        matches!(
            self,
            CmdKind::Motion(Motion::FindChar { .. })
                | CmdKind::Motion(Motion::MarkJump { .. })
                | CmdKind::Normal(NormalCmd::ReplaceChar)
                | CmdKind::Normal(NormalCmd::MarkSet)
                | CmdKind::Normal(NormalCmd::JumpMark { .. })
                | CmdKind::Normal(NormalCmd::RecordMacro)
                | CmdKind::Normal(NormalCmd::PlayMacro)
                | CmdKind::Visual(VisualCmd::ReplaceChar)
        )
    }
}

struct RowBuilder {
    rows: Vec<(Vec<Key>, Phase, CmdKind)>,
}

impl RowBuilder {
    fn new() -> Self {
        RowBuilder { rows: Vec::new() }
    }

    fn normal(&mut self, keys: &[&str], kind: CmdKind) {
        self.rows.push((parse(keys), Phase::Normal, kind));
    }

    fn pending(&mut self, keys: &[&str], kind: CmdKind) {
        self.rows.push((parse(keys), Phase::Pending, kind));
    }

    fn visual(&mut self, keys: &[&str], kind: CmdKind) {
        self.rows.push((parse(keys), Phase::Visual, kind));
    }

    /// Register a motion in all three phases.
    fn motion_all(&mut self, keys: &[&str], motion: Motion) {
        self.normal(keys, CmdKind::Motion(motion));
        self.pending(keys, CmdKind::Motion(motion));
        self.visual(keys, CmdKind::Motion(motion));
    }

    /// Register a text object in pending + visual phases.
    fn object(&mut self, keys: &[&str], object: TextObject) {
        self.pending(keys, CmdKind::Object(object));
        self.visual(keys, CmdKind::Object(object));
    }
}

fn parse(keys: &[&str]) -> Vec<Key> {
    keys.iter().map(|k| Key::parse(k)).collect()
}

fn build_rows() -> Vec<(Vec<Key>, Phase, CmdKind)> {
    let mut b = RowBuilder::new();

    // ---- motions (all phases) ------------------------------------------
    b.motion_all(&["h"], Motion::Left);
    // <BS> is h's twin in vim notation (normal moves left, operator-pending
    // makes `d<BS>` delete left, visual shrinks the selection). Insert and
    // the cmdline consume backspace before the tries ever see it.
    b.motion_all(&["<BS>"], Motion::Left);
    b.motion_all(&["l"], Motion::Right);
    // <Space> is vim's alias for `l` (exclusive; stops at the line end —
    // verified against vim 9.1). handle_key canonicalizes Named("space")
    // keystrokes to Char(' ') so both delivery paths hit this row.
    b.motion_all(&[" "], Motion::Right);
    b.motion_all(&["j"], Motion::Down);
    b.motion_all(&["k"], Motion::Up);
    b.motion_all(&["g", "j"], Motion::Down); // no soft wrap in v1
    b.motion_all(&["g", "k"], Motion::Up);
    b.motion_all(&["0"], Motion::LineStart);
    b.motion_all(&["^"], Motion::FirstNonBlank);
    b.motion_all(&["$"], Motion::LineEnd);
    b.motion_all(&["g", "_"], Motion::LastLineNonBlank);
    b.motion_all(&["_"], Motion::Underscore);
    b.motion_all(&["w"], Motion::WordStart { big: false });
    b.motion_all(&["W"], Motion::WordStart { big: true });
    b.motion_all(&["b"], Motion::WordBack { big: false });
    b.motion_all(&["B"], Motion::WordBack { big: true });
    b.motion_all(&["e"], Motion::WordEnd { big: false });
    b.motion_all(&["E"], Motion::WordEnd { big: true });
    b.motion_all(&["g", "e"], Motion::WordEndBack { big: false });
    b.motion_all(&["g", "E"], Motion::WordEndBack { big: true });
    b.motion_all(
        &["f"],
        Motion::FindChar {
            forward: true,
            till: false,
        },
    );
    b.motion_all(
        &["F"],
        Motion::FindChar {
            forward: false,
            till: false,
        },
    );
    b.motion_all(
        &["t"],
        Motion::FindChar {
            forward: true,
            till: true,
        },
    );
    b.motion_all(
        &["T"],
        Motion::FindChar {
            forward: false,
            till: true,
        },
    );
    b.motion_all(&[";"], Motion::RepeatFind { reverse: false });
    b.motion_all(&[","], Motion::RepeatFind { reverse: true });
    b.motion_all(&["%"], Motion::MatchBracket);
    b.motion_all(&["g", "g"], Motion::GoToLine { first: true });
    b.motion_all(&["G"], Motion::GoToLine { first: false });
    b.motion_all(&["}"], Motion::ParaNext);
    b.motion_all(&["{"], Motion::ParaPrev);
    b.motion_all(&[")"], Motion::SentenceNext);
    b.motion_all(&["("], Motion::SentencePrev);
    // `forward` on SearchNext is the REPEAT polarity: `n` repeats the last
    // search in its own direction, `N` mirrors it (`?` + `n` goes up).
    b.motion_all(&["n"], Motion::SearchNext { forward: true });
    b.motion_all(&["N"], Motion::SearchNext { forward: false });
    b.motion_all(&["*"], Motion::StarSearch { forward: true, substring: false });
    b.motion_all(&["#"], Motion::StarSearch { forward: false, substring: false });
    // g* / g#: the word as a SUBSTRING pattern (`:h g*`, audit F2 — the
    // old keys degraded to whole-word `*`/`#` plus a stray bell)
    b.motion_all(&["g", "*"], Motion::StarSearch { forward: true, substring: true });
    b.motion_all(&["g", "#"], Motion::StarSearch { forward: false, substring: true });
    // gd / gD: declaration search (audit F1 — unbound, `g`+`d` re-armed
    // the delete operator through the trie-miss retry)
    b.motion_all(&["g", "d"], Motion::SearchDeclaration { whole_file: false });
    b.motion_all(&["g", "D"], Motion::SearchDeclaration { whole_file: true });
    b.motion_all(&["|"], Motion::Column);
    b.motion_all(&["H"], Motion::ScreenTop);
    b.motion_all(&["M"], Motion::ScreenMiddle);
    b.motion_all(&["L"], Motion::ScreenBottom);
    b.motion_all(&["<C-d>"], Motion::ScrollHalfDown);
    b.motion_all(&["<C-u>"], Motion::ScrollHalfUp);
    b.motion_all(&["<C-f>"], Motion::PageDown);
    b.motion_all(&["<C-b>"], Motion::PageUp);
    b.motion_all(&["<CR>"], Motion::LineDownFirstNonBlank);
    b.motion_all(&["+"], Motion::LineDownFirstNonBlank);
    b.motion_all(&["-"], Motion::LineUpFirstNonBlank);
    b.pending(&["'"], CmdKind::Motion(Motion::MarkJump { linewise: true }));
    b.pending(
        &["`"],
        CmdKind::Motion(Motion::MarkJump { linewise: false }),
    );
    b.normal(
        &["'"],
        CmdKind::Normal(NormalCmd::JumpMark { linewise: true }),
    );
    b.normal(
        &["`"],
        CmdKind::Normal(NormalCmd::JumpMark { linewise: false }),
    );
    // visual `'a` / `` `a ``: vim MOVES the cursor to the mark and the
    // selection follows (the old trie had no visual rows — the key belled
    // and dropped the pending state). The CharArgCmd::JumpMark arm goes
    // through goto_motion, which extends the visual selection.
    b.visual(
        &["'"],
        CmdKind::Normal(NormalCmd::JumpMark { linewise: true }),
    );
    b.visual(
        &["`"],
        CmdKind::Normal(NormalCmd::JumpMark { linewise: false }),
    );

    // ---- operators -------------------------------------------------------
    b.normal(&["d"], CmdKind::Operator(Operator::Delete));
    b.normal(&["c"], CmdKind::Operator(Operator::Change));
    b.normal(&["y"], CmdKind::Operator(Operator::Yank));
    b.normal(&[">"], CmdKind::Operator(Operator::IndentRight));
    b.normal(&["<"], CmdKind::Operator(Operator::IndentLeft));
    b.normal(&["g", "u"], CmdKind::Operator(Operator::Lowercase));
    b.normal(&["g", "U"], CmdKind::Operator(Operator::Uppercase));
    b.normal(&["g", "~"], CmdKind::Operator(Operator::ToggleCase));
    b.normal(&["g", "q"], CmdKind::Operator(Operator::Format));
    b.normal(&["g", "w"], CmdKind::Operator(Operator::Format));
    // NOTE: there are no `guu`/`gUU`/`g~~`/`gqq` (or `gugu`-style) rows
    // here on purpose: `gu`/`gU`/`g~`/`gq` already RESOLVE at the second
    // key (operator armed, cmd_seq cleared), so a third key can never
    // accumulate in this trie. The linewise doubling forms are handled by
    // the operator-pending doubling path in state.rs (`gUU` etc. work —
    // engine.rs + fuzz_round10 pin them).

    // ---- text objects (pending + visual) ---------------------------------
    b.object(
        &["i", "w"],
        TextObject::Word {
            inner: true,
            big: false,
        },
    );
    b.object(
        &["a", "w"],
        TextObject::Word {
            inner: false,
            big: false,
        },
    );
    b.object(
        &["i", "W"],
        TextObject::Word {
            inner: true,
            big: true,
        },
    );
    b.object(
        &["a", "W"],
        TextObject::Word {
            inner: false,
            big: true,
        },
    );
    b.object(&["i", "s"], TextObject::Sentence { inner: true });
    b.object(&["a", "s"], TextObject::Sentence { inner: false });
    b.object(&["i", "p"], TextObject::Paragraph { inner: true });
    b.object(&["a", "p"], TextObject::Paragraph { inner: false });
    for (q, key) in [('"', "\""), ('\'', "'"), ('`', "`")] {
        b.object(
            &["i", key],
            TextObject::Quote {
                inner: true,
                quote: q,
            },
        );
        b.object(
            &["a", key],
            TextObject::Quote {
                inner: false,
                quote: q,
            },
        );
    }
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')] {
        let open_key: &'static str = match open {
            '(' => "(",
            '[' => "[",
            '{' => "{",
            '<' => "<",
            _ => unreachable!(),
        };
        let close_key: &'static str = match close {
            ')' => ")",
            ']' => "]",
            '}' => "}",
            '>' => ">",
            _ => unreachable!(),
        };
        b.object(
            &["i", open_key],
            TextObject::Block {
                inner: true,
                open,
                close,
            },
        );
        b.object(
            &["i", close_key],
            TextObject::Block {
                inner: true,
                open,
                close,
            },
        );
        b.object(
            &["a", open_key],
            TextObject::Block {
                inner: false,
                open,
                close,
            },
        );
        b.object(
            &["a", close_key],
            TextObject::Block {
                inner: false,
                open,
                close,
            },
        );
    }
    b.object(
        &["i", "b"],
        TextObject::Block {
            inner: true,
            open: '(',
            close: ')',
        },
    );
    b.object(
        &["a", "b"],
        TextObject::Block {
            inner: false,
            open: '(',
            close: ')',
        },
    );
    b.object(
        &["i", "B"],
        TextObject::Block {
            inner: true,
            open: '{',
            close: '}',
        },
    );
    b.object(
        &["a", "B"],
        TextObject::Block {
            inner: false,
            open: '{',
            close: '}',
        },
    );
    b.object(&["i", "t"], TextObject::Tag { inner: true });
    b.object(&["a", "t"], TextObject::Tag { inner: false });

    // ---- normal-mode actions ---------------------------------------------
    b.normal(&["x"], CmdKind::Normal(NormalCmd::DeleteCharForward));
    // NOTE: no `<Del>` row — the navigation-key interceptor (state.rs) takes
    // "delete" before the trie walk, and visual mode maps it to `d` there
    // too; a trie row would be unreachable.
    b.normal(&["X"], CmdKind::Normal(NormalCmd::DeleteCharBackward));
    b.normal(&["s"], CmdKind::Normal(NormalCmd::SubstituteChar));
    b.normal(&["S"], CmdKind::Normal(NormalCmd::SubstituteLine));
    b.normal(&["C"], CmdKind::Normal(NormalCmd::ChangeToEnd));
    b.normal(&["D"], CmdKind::Normal(NormalCmd::DeleteToEnd));
    b.normal(&["Y"], CmdKind::Normal(NormalCmd::YankLine));
    b.normal(&["r"], CmdKind::Normal(NormalCmd::ReplaceChar));
    b.normal(&["~"], CmdKind::Normal(NormalCmd::ToggleChar));
    b.normal(&["p"], CmdKind::Normal(NormalCmd::PutAfter));
    b.normal(&["P"], CmdKind::Normal(NormalCmd::PutBefore));
    // gp/gP: same pastes, cursor left just AFTER the new text (`:h gp`,
    // audit G6 — the old miss fell back to plain `p`'s cursor)
    b.normal(&["g", "p"], CmdKind::Normal(NormalCmd::PutAfterGp));
    b.normal(&["g", "P"], CmdKind::Normal(NormalCmd::PutBeforeGp));
    b.normal(&["J"], CmdKind::Normal(NormalCmd::Join));
    b.normal(&["g", "J"], CmdKind::Normal(NormalCmd::JoinLiteral));
    b.normal(&["u"], CmdKind::Normal(NormalCmd::Undo));
    b.normal(&["<C-r>"], CmdKind::Normal(NormalCmd::Redo));
    b.normal(&["m"], CmdKind::Normal(NormalCmd::MarkSet));
    b.normal(&["."], CmdKind::Normal(NormalCmd::RepeatChange));
    b.normal(&["q"], CmdKind::Normal(NormalCmd::RecordMacro));
    b.normal(&["<C-o>"], CmdKind::Normal(NormalCmd::JumpBackward));
    b.normal(&["<C-i>"], CmdKind::Normal(NormalCmd::JumpForward));
    // <Tab> and <C-i> are the SAME key on a terminal: vim jumps forward on
    // both (the crossterm key layer delivers plain "tab" for the Tab key)
    b.normal(&["<Tab>"], CmdKind::Normal(NormalCmd::JumpForward));
    // free scrolling: C-e / C-y move the VIEW one line, cursor follows only
    // when it would leave the viewport
    b.normal(
        &["<C-e>"],
        CmdKind::Normal(NormalCmd::ScrollLines { down: true }),
    );
    b.normal(
        &["<C-y>"],
        CmdKind::Normal(NormalCmd::ScrollLines { down: false }),
    );
    b.normal(&["g", ";"], CmdKind::Normal(NormalCmd::OlderChange));
    b.normal(&["g", ","], CmdKind::Normal(NormalCmd::NewerChange));
    // gn / gN: select the next match in the search direction (or the current
    // one when the cursor is inside it). Plain keys enter visual; the same
    // motion under an operator (`dgn`) changes just the match.
    b.normal(
        &["g", "n"],
        CmdKind::Normal(NormalCmd::SelectMatch { backward: false }),
    );
    b.normal(
        &["g", "N"],
        CmdKind::Normal(NormalCmd::SelectMatch { backward: true }),
    );
    b.pending(
        &["g", "n"],
        CmdKind::Motion(Motion::SelectMatch { backward: false }),
    );
    b.pending(
        &["g", "N"],
        CmdKind::Motion(Motion::SelectMatch { backward: true }),
    );
    b.visual(
        &["g", "n"],
        CmdKind::Normal(NormalCmd::SelectMatch { backward: false }),
    );
    b.visual(
        &["g", "N"],
        CmdKind::Normal(NormalCmd::SelectMatch { backward: true }),
    );
    b.normal(&["<C-a>"], CmdKind::Normal(NormalCmd::IncrementNumber));
    b.normal(&["<C-x>"], CmdKind::Normal(NormalCmd::DecrementNumber));
    // &: repeat the last :s on the current line (vim)
    b.normal(&["&"], CmdKind::Normal(NormalCmd::RepeatSubstitute));
    // `g&`: the whole-file repeat — same flags, every line (audit E10)
    b.normal(
        &["g", "&"],
        CmdKind::Normal(NormalCmd::RepeatSubstituteGlobal),
    );
    b.normal(&["@"], CmdKind::Normal(NormalCmd::PlayMacro));
    b.normal(&["z", "z"], CmdKind::Normal(NormalCmd::ScrollCenter));
    b.normal(&["z", "t"], CmdKind::Normal(NormalCmd::ScrollTop));
    b.normal(&["z", "b"], CmdKind::Normal(NormalCmd::ScrollBottom));
    // visual 模式同款:滚动 + 保留选区(round11 悬置闭环,vim 行为)
    b.visual(
        &["z", "z"],
        CmdKind::Visual(VisualCmd::Scroll(crate::host::ScrollAnchor::Center)),
    );
    b.visual(
        &["z", "t"],
        CmdKind::Visual(VisualCmd::Scroll(crate::host::ScrollAnchor::Top)),
    );
    b.visual(
        &["z", "b"],
        CmdKind::Visual(VisualCmd::Scroll(crate::host::ScrollAnchor::Bottom)),
    );
    b.normal(&["g", "v"], CmdKind::Normal(NormalCmd::RestoreVisual));
    // ]] / [[ / ][ / [] / ]m / [m / ]M / [M — section & method motions
    // (`:h ]]`, audit A3; the family was entirely unbound)
    b.motion_all(&["]", "]"], Motion::Section { ch: '{', backward: false, method: false });
    b.motion_all(&["[", "["], Motion::Section { ch: '{', backward: true, method: false });
    b.motion_all(&["]", "["], Motion::Section { ch: '}', backward: false, method: false });
    b.motion_all(&["[", "]"], Motion::Section { ch: '}', backward: true, method: false });
    b.motion_all(&["]", "m"], Motion::Section { ch: '{', backward: false, method: true });
    b.motion_all(&["[", "m"], Motion::Section { ch: '{', backward: true, method: true });
    b.motion_all(&["]", "M"], Motion::Section { ch: '}', backward: false, method: true });
    b.motion_all(&["[", "M"], Motion::Section { ch: '}', backward: true, method: true });
    // ga / g8: the char-code / byte-hex info messages (`:h ga`, audit J2/J3)
    b.normal(&["g", "a"], CmdKind::Normal(NormalCmd::CharInfo));
    b.normal(&["g", "8"], CmdKind::Normal(NormalCmd::ByteInfo));
    // ]p / [p / ]P / [P: indent-adjusting paste (`:h ]p`, audit M1 — the
    // old keys were unbound and `]p` degraded to a plain `p`, keeping the
    // register's original indent)
    b.normal(
        &["]", "p"],
        CmdKind::Normal(NormalCmd::PasteIndent { below: true }),
    );
    b.normal(
        &["]", "P"],
        CmdKind::Normal(NormalCmd::PasteIndent { below: true }),
    );
    b.normal(
        &["[", "p"],
        CmdKind::Normal(NormalCmd::PasteIndent { below: false }),
    );
    b.normal(
        &["[", "P"],
        CmdKind::Normal(NormalCmd::PasteIndent { below: false }),
    );
    b.normal(&["Z", "Z"], CmdKind::Normal(NormalCmd::WriteQuit));
    b.normal(&["Z", "Q"], CmdKind::Normal(NormalCmd::QuitNoSave));

    // ---- entering insert ---------------------------------------------------
    b.normal(&["i"], CmdKind::EnterInsert(InsertKind::Insert));
    // <Insert> in normal mode enters insert, like `i` (vim: `:h <Insert>`)
    b.normal(&["<Insert>"], CmdKind::EnterInsert(InsertKind::Insert));
    b.normal(&["a"], CmdKind::EnterInsert(InsertKind::Append));
    b.normal(
        &["I"],
        CmdKind::EnterInsert(InsertKind::InsertFirstNonBlank),
    );
    b.normal(&["A"], CmdKind::EnterInsert(InsertKind::AppendLineEnd));
    b.normal(
        &["o"],
        CmdKind::EnterInsert(InsertKind::OpenLine { below: true }),
    );
    b.normal(
        &["O"],
        CmdKind::EnterInsert(InsertKind::OpenLine { below: false }),
    );
    b.normal(
        &["g", "I"],
        CmdKind::EnterInsert(InsertKind::InsertAtColumnZero),
    );
    b.normal(
        &["g", "i"],
        CmdKind::EnterInsert(InsertKind::LastInsertExit),
    );
    b.normal(&["R"], CmdKind::EnterInsert(InsertKind::Replace));

    // ---- entering visual ---------------------------------------------------
    b.normal(&["v"], CmdKind::EnterVisual(crate::mode::VisualKind::Char));
    b.normal(&["V"], CmdKind::EnterVisual(crate::mode::VisualKind::Line));
    b.normal(
        &["<C-v>"],
        CmdKind::EnterVisual(crate::mode::VisualKind::Block),
    );

    // ---- visual mode -------------------------------------------------------
    b.visual(&["v"], CmdKind::Visual(VisualCmd::ToggleKind { to: 'v' }));
    b.visual(&["V"], CmdKind::Visual(VisualCmd::ToggleKind { to: 'V' }));
    b.visual(
        &["<C-v>"],
        CmdKind::Visual(VisualCmd::ToggleKind { to: 'b' }),
    );
    b.visual(&["o"], CmdKind::Visual(VisualCmd::SwapEnds));
    b.visual(&["O"], CmdKind::Visual(VisualCmd::SwapEndsKeepCol));
    // `@{reg}` in visual mode: the macro's first visual-capable command acts
    // ON the selection (`:h v_@` — audit D9; the key used to bell and the
    // selection stayed untouched). The replay stays in Visual until a
    // replayed key resolves the selection, exactly like vim.
    b.visual(&["@"], CmdKind::Normal(NormalCmd::PlayMacro));
    b.visual(&["d"], CmdKind::Operator(Operator::Delete));
    b.visual(&["x"], CmdKind::Operator(Operator::Delete));
    // visual `D`: blockwise — block column to each row's end; char/line —
    // span start to that line's end (`:h v_D`, audit G2)
    b.visual(&["D"], CmdKind::Operator(Operator::DeleteToEnd));
    b.visual(&["<Del>"], CmdKind::Operator(Operator::Delete));
    b.visual(&["y"], CmdKind::Operator(Operator::Yank));
    // Y/D/X/C/S are vim's LINEWISE visual commands (see VisualCmd::LinewiseOp;
    // 9.1 probes: `vlY` yanks the line, `vlD` deletes the line whole, `v_x`
    // stays charwise and `v_s` stays charwise change)
    b.visual(
        &["Y"],
        CmdKind::Visual(VisualCmd::LinewiseOp(Operator::Yank)),
    );
    b.visual(&["c"], CmdKind::Operator(Operator::Change));
    b.visual(&["s"], CmdKind::Operator(Operator::Change));
    b.visual(
        &["C"],
        CmdKind::Visual(VisualCmd::LinewiseOp(Operator::Change)),
    );
    b.visual(
        &["S"],
        CmdKind::Visual(VisualCmd::LinewiseOp(Operator::Change)),
    );
    // visual `D`: blockwise — block column to each row's end; linewise —
    // delete the highlighted lines; charwise — span start to line end
    // (`:h v_D`; the old LinewiseOp(Delete) hit block mode too and wiped
    // the whole buffer — audit G2)
    b.visual(&["D"], CmdKind::Operator(Operator::DeleteToEnd));
    b.visual(
        &["X"],
        CmdKind::Visual(VisualCmd::LinewiseOp(Operator::Delete)),
    );
    b.visual(&[">"], CmdKind::Operator(Operator::IndentRight));
    b.visual(&["<"], CmdKind::Operator(Operator::IndentLeft));
    b.visual(&["u"], CmdKind::Operator(Operator::Lowercase));
    b.visual(&["U"], CmdKind::Operator(Operator::Uppercase));
    b.visual(&["~"], CmdKind::Operator(Operator::ToggleCase));
    b.visual(&["g", "u"], CmdKind::Operator(Operator::Lowercase));
    b.visual(&["g", "U"], CmdKind::Operator(Operator::Uppercase));
    b.visual(&["g", "~"], CmdKind::Operator(Operator::ToggleCase));
    b.visual(&["g", "q"], CmdKind::Operator(Operator::Format));
    b.visual(&["p"], CmdKind::Visual(VisualCmd::PutReplace));
    b.visual(&["P"], CmdKind::Visual(VisualCmd::PutReplace));
    b.visual(&["r"], CmdKind::Visual(VisualCmd::ReplaceChar));
    b.visual(&["J"], CmdKind::Visual(VisualCmd::Join { literal: false }));
    // `g C-a`: sequential increment over the selection's lines
    // (`:h v_g CTRL-A`, audit K1)
    b.visual(
        &["g", "<C-a>"],
        CmdKind::Visual(VisualCmd::SequentialIncrement),
    );
    b.visual(
        &["g", "J"],
        CmdKind::Visual(VisualCmd::Join { literal: true }),
    );

    b.rows
}

/// Per-phase command tries plus derived lookup helpers.
pub struct CommandTables {
    tries: HashMap<Phase, Trie<CmdKind>>,
}

impl CommandTables {
    pub fn build() -> Self {
        let mut tries: HashMap<Phase, Trie<CmdKind>> = HashMap::new();
        for (keys, phase, kind) in build_rows() {
            tries.entry(phase).or_default().insert(&keys, kind);
        }
        // make sure every phase has a (possibly empty) trie
        for phase in [Phase::Normal, Phase::Pending, Phase::Visual] {
            tries.entry(phase).or_default();
        }
        CommandTables { tries }
    }

    pub fn trie(&self, phase: Phase) -> &Trie<CmdKind> {
        &self.tries[&phase]
    }

    /// Find the longest terminal command in `keys` (for trie-miss recovery).
    pub fn longest_terminal(&self, phase: Phase, keys: &[Key]) -> Option<(usize, CmdKind)> {
        let mut best = None;
        for len in 1..=keys.len() {
            if let Walk::Hit(kind) = self.trie(phase).get(&keys[..len]) {
                best = Some((len, *kind));
            }
        }
        best
    }
}

use crate::keymap::Walk;

/// Which mapping class applies to a phase/mode.
pub fn mapping_class_for(mode: crate::mode::Mode) -> ModeClass {
    match mode {
        crate::mode::Mode::Visual { .. } => ModeClass::Visual,
        crate::mode::Mode::Insert | crate::mode::Mode::Replace => ModeClass::Insert,
        _ => ModeClass::Normal,
    }
}
