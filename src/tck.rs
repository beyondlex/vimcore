//! Host contract tests (TCK)——INTEGRATION.md 坑位清单的可执行形态。
//!
//! 每个函数是一条宿主必须满足的契约。宿主在自己的 buffer / host 实现上
//! 调用它们（通常在 `#[cfg(test)]` 里 `.unwrap()`），把「接入质量靠读文档」
//! 变成「接入质量有测试下限」。全部纯 headless：不依赖 gpui、不开窗口。
//!
//! 典型用法（宿主仓库）：
//!
//! ```ignore
//! #[test]
//! fn my_buffer_meets_engine_contract() {
//!     let mut buf = MyBuffer::default();
//!     vimcore::tck::buffer_edit_contract(&mut buf).unwrap();
//!     let mut host = MyHost::default();
//!     vimcore::tck::engine_smoke_contract(&mut buf, &mut host).unwrap();
//! }
//! ```
//!
//! 刻意不覆盖的部分：undo/redo（宿主语义各异——PandaGit 走应用级统一撤销
//! 栈，`undo()` 合法地返回 None）、剪贴板、视口滚动。这些由宿主自己的测试
//! 与 [`TckHost`] 的存在边界共同划出。

use std::ops::Range;

use crate::buffer::{VimBuffer, VimBufferMut};
use crate::host::{VimHost, VimHostRo};
use crate::key::Key;
use crate::state::{Ctx, CtxRo, VimState};

/// 只读契约：`VimBuffer` 的行语义与字符边界。
///
/// 对宿主缓冲的**当前内容**逐条校验；建议用含 CJK、多字节字符与空行的
/// 多行内容运行。校验项：
///
/// - `line_count() >= 1`（空缓冲也有恰好 1 行）；
/// - `line_range(i)` 含终止 `\n`（非末行），末行延伸到缓冲区尾——引擎的
///   linewise 操作（dd/yy/V-line）依赖这一点，缺 `\n` 会在原位留空行；
/// - `slice(range).len() == range.len()`；
/// - `offset_to_line(line_start(i)) == i`；
/// - `char_at` 在每个字符边界返回 `Some`、非边界/越界返回 `None`；
/// - `prev_char_offset` 从 `len` 逐步回链到 0，步数恰等于字符数；
/// - **幻影行容忍**：引擎的普通光标命令（缓冲区末尾的 `w`/`}` 等）会以
///   `line == line_count` 调 `line_range`/`line_end`，并以任意字节（含
///   多字节字符中间）调 `offset_to_line`——实现必须像本参考一样返回
///   `len..len` / 行尾、并把偏移落到最近字符边界，越界 panic 的宿主
///   会在最普通的按键上崩溃。
pub fn buffer_read_contract(buf: &(impl VimBuffer + ?Sized)) -> Result<(), String> {
    let len = buf.len();
    let lines = buf.line_count();
    if lines == 0 {
        return Err("line_count() == 0：空缓冲也必须有恰好 1 行".into());
    }
    for line in 0..lines {
        let range = buf.line_range(line);
        if range.start > range.end || range.end > len {
            return Err(format!(
                "line {line}: line_range {range:?} 越界（len={len}）"
            ));
        }
        let text = buf.slice(range.clone());
        if text.len() != range.len() {
            return Err(format!(
                "line {line}: slice(range).len() = {} ≠ range.len() = {}",
                text.len(),
                range.len()
            ));
        }
        if line + 1 < lines {
            if !text.ends_with('\n') {
                return Err(format!(
                    "line {line}: 非末行的 line_range 必须包含终止 \\n（引擎的 dd/yy 依赖它）"
                ));
            }
        } else if range.end != len {
            return Err(format!(
                "line {line}: 末行 range 必须延伸到缓冲区尾（end={} ≠ len={len}）",
                range.end
            ));
        }
        if buf.offset_to_line(range.start) != line {
            return Err(format!(
                "line {line}: offset_to_line(行首={}) = {}",
                range.start,
                buf.offset_to_line(range.start)
            ));
        }
    }
    // 幻影行与越界/中间字符偏移必须被容忍（见上方契约说明）：普通命令
    // `w` 在缓冲末尾就会以 line == line_count 读 line_range
    if buf.line_range(lines) != (len..len) && len > 0 {
        return Err(format!(
            "line_range(line_count) 必须容忍幻影行（返回 len..len），got {:?}",
            buf.line_range(lines)
        ));
    }
    let full = buf.slice(0..len);
    if full.len() != len {
        return Err(format!(
            "slice(0..len).len() = {} ≠ len = {len}",
            full.len()
        ));
    }
    let mut o = 0;
    while o < len {
        let Some(c) = buf.char_at(o) else {
            return Err(format!("char_at({o}) = None：字符边界上必须返回 Some"));
        };
        // 多字节字符：中点偏移必须与边界同答——引擎的存储偏移兜底
        // （refloor/floor_to_char_boundary）会把陈旧偏移落到「最近的边界」，
        // 但落点之前的中间查询以任意字节发生，mid-char panic 的宿主会在
        // 普通编辑后立即崩溃（round13 #26 的教训，反向断言在此钉住）
        if c.len_utf8() > 1 {
            for mid in (o + 1)..(o + c.len_utf8()) {
                if buf.char_at(mid).is_some() {
                    return Err(format!(
                        "char_at({mid}) = Some：多字节字符中间必须返回 None"
                    ));
                }
                let (a, b) = (buf.offset_to_line(mid), buf.offset_to_line(o));
                if a != b {
                    return Err(format!(
                        "offset_to_line({mid}) = {a} ≠ offset_to_line(边界 {o}) = {b}：mid-char 偏移必须落到所属行"
                    ));
                }
                if let Some(prev) = buf.prev_char_offset(mid) {
                    // 引擎自己用 char_at+字节递减做 floor，从不要求宿主
                    // 对 mid-char 偏移回退成功——只要求返回值若是 Some
                    // 必须指向一个字符边界
                    if buf.char_at(prev).is_none() {
                        return Err(format!(
                            "prev_char_offset({mid}) = Some({prev})：必须指向字符边界"
                        ));
                    }
                }
            }
        }
        o += c.len_utf8();
    }
    if len > 0 {
        if buf.char_at(len).is_some() {
            return Err("char_at(len) 必须是 None".into());
        }
        // offset_to_line(len)：钳到末行（tests 宿主）或返回幻影行号
        // （本参考宿主）都合法——引擎对两种都容忍（line_range 容忍
        // line == line_count），但绝不能 panic
        let at_len = buf.offset_to_line(len);
        if at_len + 1 < lines {
            return Err(format!(
                "offset_to_line(len) = {at_len} 比末行还小（末行 = {}）",
                lines - 1
            ));
        }
        if buf.prev_char_offset(len + 42).is_some() {
            return Err("prev_char_offset(len+42) 必须是 None（越界偏移拒绝）".into());
        }
        if buf.prev_char_offset(0).is_some() {
            return Err("prev_char_offset(0) 必须是 None".into());
        }
        let chars = full.chars().count();
        let mut o = len;
        let mut steps = 0;
        while let Some(prev) = buf.prev_char_offset(o) {
            steps += 1;
            o = prev;
            if steps > chars {
                return Err("prev_char_offset 链不收敛（步数超过字符数）".into());
            }
        }
        if o != 0 || steps != chars {
            return Err(format!(
                "prev_char_offset 链终点 {o}（应为 0）或步数 {steps} ≠ 字符数 {chars}"
            ));
        }
    }
    Ok(())
}

/// 可变契约：`VimBufferMut` 两种编辑原语的 UTF-8 安全性。
///
/// 要求传入**空缓冲**；每步编辑之后只读契约必须仍然成立。序列覆盖：
/// 末尾多字节插入、行首插入拆行、删除换行、清空。
pub fn buffer_edit_contract(buf: &mut impl VimBufferMut) -> Result<(), String> {
    if !buf.is_empty() {
        return Err("buffer_edit_contract 要求空缓冲".into());
    }
    buf.insert_text(0, "ab");
    assert_content(buf, "ab")?;
    buf.insert_text(buf.len(), "中"); // 末尾多字节插入
    assert_content(buf, "ab中")?;
    buf.insert_text(0, "\n"); // 行首插入拆行
    assert_content(buf, "\nab中")?;
    if buf.line_count() != 2 {
        return Err(format!("line_count() = {} ≠ 2", buf.line_count()));
    }
    if buf.offset_to_line(1) != 1 {
        return Err(format!(
            "offset_to_line(1) = {} ≠ 1（\\n 之后的字节属于第 1 行）",
            buf.offset_to_line(1)
        ));
    }
    buf.delete_range(0..1); // 只删换行
    assert_content(buf, "ab中")?;
    buf.delete_range(0..buf.len()); // 清空
    if !buf.is_empty() || buf.line_count() != 1 {
        return Err(format!(
            "清空后 len={} line_count={}（应为 0 与 1）",
            buf.len(),
            buf.line_count()
        ));
    }
    Ok(())
}

/// 引擎冒烟契约：用引擎按键管线在宿主 buffer 上跑一段确定性编辑，每步
/// 校验全文与只读契约。
///
/// 要求传入**空缓冲**（宿主的 host 语义各异时可用 [`TckHost`]）。序列：
/// `ggx`（删除）、`$`（行尾光标）、`Vjd`（可视行删除）、`p`（linewise
/// 粘贴）、`gg$A` + 文本 + Esc（行尾追加）。刻意不含 `u`/`C-r`——见模块头。
pub fn engine_smoke_contract(
    buf: &mut impl VimBufferMut,
    host: &mut dyn VimHost,
) -> Result<(), String> {
    if !buf.is_empty() {
        return Err("engine_smoke_contract 要求空缓冲".into());
    }
    buf.insert_text(0, "alpha\nbeta\ngamma\n");
    let mut vim = VimState::new();

    feed(&mut vim, buf, host, "gg");
    assert_content(buf, "alpha\nbeta\ngamma\n")?;
    feed(&mut vim, buf, host, "x");
    assert_content(buf, "lpha\nbeta\ngamma\n")?;
    feed(&mut vim, buf, host, "$");
    let want = buf.line_end(0).saturating_sub(1);
    if vim.cursor_offset() != want {
        return Err(format!(
            "$ 后 cursor_offset() = {} ≠ 行尾字符 {want}",
            vim.cursor_offset()
        ));
    }
    feed(&mut vim, buf, host, "Vjd");
    assert_content(buf, "gamma\n")?;
    feed(&mut vim, buf, host, "p");
    assert_content(buf, "gamma\nlpha\nbeta\n")?;
    feed(&mut vim, buf, host, "gg$A");
    type_text(&mut vim, buf, host, "x");
    {
        let mut ctx = Ctx { buf, host };
        vim.handle_key(&mut ctx, Key::escape());
    }
    assert_content(buf, "gammax\nlpha\nbeta\n")?;
    Ok(())
}

/// String-backed 参考缓冲实现：`line_range` 含终止 `\n`、`char_at` 在非
/// 字符边界返回 `None`（永不 panic）。宿主可以直接复用，也可以照抄实现——
/// 引擎的 `floor_to_char_boundary` 会刻意探测非边界偏移，`char_at` 若用裸
/// 切片会从合法的引擎调用路径上 panic（第五轮检视抓到的宿主复制陷阱）。
#[derive(Default, Clone)]
pub struct TckStrBuf(pub String);

impl TckStrBuf {
    pub fn from_text(text: &str) -> Self {
        TckStrBuf(text.to_owned())
    }
}

impl VimBuffer for TckStrBuf {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn line_count(&self) -> usize {
        if self.0.is_empty() {
            1
        } else if self.0.ends_with('\n') {
            self.0.split('\n').count() - 1
        } else {
            self.0.split('\n').count()
        }
    }
    fn char_at(&self, offset: usize) -> Option<char> {
        if !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[offset..].chars().next()
    }
    fn prev_char_offset(&self, offset: usize) -> Option<usize> {
        if offset == 0 || offset > self.0.len() || !self.0.is_char_boundary(offset) {
            return None;
        }
        self.0[..offset]
            .chars()
            .next_back()
            .map(|c| offset - c.len_utf8())
    }
    fn line_range(&self, line: usize) -> Range<usize> {
        // single pass: walk the pieces once, counting as we go — the old
        // shape called line_count() up front (a full split/count) and again
        // for `total`, so every line_start/line_end/line_content via the
        // trait defaults cost three O(text) scans per call
        let mut start = 0;
        for (i, part) in self.0.split('\n').enumerate() {
            if i == line {
                // 终止 \n 存在就包含进 range；末行延伸到缓冲区尾
                let end = if start + part.len() < self.0.len() {
                    start + part.len() + 1
                } else {
                    self.0.len()
                };
                return start..end;
            }
            start += part.len() + 1;
        }
        // line >= line_count: the phantom line past the end
        self.0.len()..self.0.len()
    }
    fn offset_to_line(&self, offset: usize) -> usize {
        // offsets arrive from OUTSIDE (stored marks, host clicks) and can be
        // past the end or mid-character. Round13 #26 "fixed" this with
        // `offset.min(ceil_char_boundary(offset))` — which is the identity
        // for a mid-char offset (ceil ≥ offset) and still sliced mid-char.
        // Floor to the character START like the engine's own
        // floor_to_char_boundary does.
        let offset = offset.min(self.0.len());
        let offset = if self.0.is_char_boundary(offset) {
            offset
        } else {
            self.0.floor_char_boundary(offset)
        };
        self.0[..offset].split('\n').count() - 1
    }
    fn slice(&self, range: Range<usize>) -> String {
        self.0[range].to_owned()
    }
}

impl VimBufferMut for TckStrBuf {
    fn insert_text(&mut self, offset: usize, text: &str) -> bool {
        self.0.insert_str(offset, text);
        true
    }
    fn delete_range(&mut self, range: Range<usize>) -> bool {
        self.0.replace_range(range, "");
        true
    }
}

/// 只读契约：`'modifiable'` 关闭时编辑类命令必须按 vim 的 E21 拒绝。
///
/// 用自带的记录型宿主（`&mut impl VimBufferMut` 只提供缓冲）在真实按键
/// 管线上跑：每个编辑类键位喂进去后，断言
///
/// - **缓冲逐字节不变**（拒绝不能留下半个改动）；
/// - **状态通道收到 E21**（不是静默 no-op，也不是 panic）；
/// - **后续 motion / 搜索 / 标记 / yank 仍然正常**（拒绝不污染状态机：
///   算子不残留、计数不残留、光标不动）。
///
/// 语义取自 vim 9.1 实证：`:set nomodifiable` 下 `x` `dw` `dd` `i` `o` `O`
/// `C` `S` `~` `p` `J` `r` `u` `>` `:s` `:d` `:j` `:m` `:pu` 全部报 E21；
/// `y` `yy` `m{a}` `q` `:q` 不报。
/// 探针用文本：契约的光标/寄存器断言（`gg`→0、`w`→6、`yiw`→"beta"）
/// 全部按它写死。
pub const READONLY_PROBE_TEXT: &str = "alpha\nbeta\ngamma\n";

///
/// 要求缓冲内容恰为 [`READONLY_PROBE_TEXT`]（断言的光标/寄存器值写死在
/// 探针里；只读宿主拿不出"空缓冲 + 写入"这两步）。
pub fn readonly_contract(buf: &mut impl VimBufferMut) -> Result<(), String> {
    let mut host = TckHostRo::default();
    let mut vim = VimState::new();
    vim.set_modifiable(false);
    let before = buf.slice(0..buf.len());
    if before != READONLY_PROBE_TEXT {
        return Err(format!(
            "readonly_contract 要求缓冲内容恰为 {READONLY_PROBE_TEXT:?}（got {before:?}）"
        ));
    }
    buffer_read_contract(buf)?;
    // The A-step shape: a plain `Ctx` over the caller's `VimBufferMut`, with
    // `'modifiable'` already off.
    readonly_probe(&mut vim, &before, &mut host, |vim, host, keys| {
        let mut ctx = Ctx { buf, host };
        for k in keys {
            vim.handle_key(&mut ctx, Key::parse(k));
        }
        buf.slice(0..buf.len())
    })
}

/// [`readonly_contract`] 的只读管线版本：缓冲以 `&dyn VimBuffer`
/// 传入，**写原语在类型层面不可达**，宿主只实现 [`VimHostRo`]。
///
/// 这是 diff view / 日志查看器类宿主的接入形态
/// （`VimState::handle_key_ro` + `CtxRo`），也是本契约最强的一形态：
/// 缓冲不被改动不是断言出来的，而是构造上不可能。
///
/// 要求缓冲内容恰为 [`READONLY_PROBE_TEXT`]（只读管线拿不到 `&mut`，写不进去）。
pub fn readonly_pipeline_contract(buf: &dyn VimBuffer) -> Result<(), String> {
    let mut host = TckHostRo::default();
    let mut vim = VimState::new();
    let before = buf.slice(0..buf.len());
    if before != READONLY_PROBE_TEXT {
        return Err(format!(
            "readonly_pipeline_contract 要求缓冲内容恰为 {READONLY_PROBE_TEXT:?}（got {before:?}）"
        ));
    }
    buffer_read_contract(buf)?;
    // No `set_modifiable(false)` and no `&mut`: `handle_key_ro` forces
    // `'modifiable'` off for the call and the buffer reference is shared, so
    // a mutation is impossible BY CONSTRUCTION rather than by assertion.
    readonly_probe(&mut vim, &before, &mut host, |vim, host, keys| {
        let mut ctx = CtxRo { buf, host };
        for k in keys {
            vim.handle_key_ro(&mut ctx, Key::parse(k));
        }
        buf.slice(0..buf.len())
    })
}

/// 编辑类键位表：只读模式下每一条都必须被 E21 拒绝且不改动缓冲。
/// 全部有 vim 9.1 实证（`:set nomodifiable` 下逐条采样）。
///
/// **入口即拒 vs 首字才拒**：`i`/`I`/`a`/`A`/`o`/`O`/`gi`/`R` 在 vim 里就是
/// 入口即 E21（模式停留 Normal），引擎一致。而 `gI`、visual `I`/`A`
/// （`v`/`V`）与 block `I`/`A`（`<C-v>`）在 vim 里是**静默进入插入**、
/// 首个落字才 E21——引擎对这三族选择在入口拒绝（更严，安全方向），
/// 偏差记录在此：拒绝更早 + visual 选区得以保留（只读窗格体验更好），
/// 且 `CtxRo` 下写原语类型层面不可达，不存在"放进插入再写坏"的路径。
const EDIT_KEYS: &[&[&str]] = &[
    &["x"],
    &["X"],
    &["d", "w"],
    &["d", "d"],
    &["d", "i", "w"],
    &["c", "w"],
    &["s"],
    &["S"],
    &["C"],
    &["D"],
    &["o"],
    &["O"],
    &["i"],
    &["a"],
    &["I"],
    &["A"],
    &["R"],
    &["~"],
    &["r", "Z"],
    &["J"],
    &["g", "J"],
    &["p"],
    &["P"],
    &["u"],
    &["<C-r>"],
    &["."],
    &["<C-a>"],
    &["<C-x>"],
    &[">", ">"],
    &["<", "<"],
    &["g", "u", "u"],
    &["g", "U", "U"],
    // 插入族角落：vim 静默进插入、首字 E21；引擎入口即拒（见上注）
    &["g", "I"],
    &["g", "I", "x"],
    &["v", "I"],
    &["v", "A"],
    &["V", "I"],
    &["V", "A"],
    &["<C-v>", "j", "l", "I"],
    &["<C-v>", "j", "l", "A"],
    // Ex 命令（逐字符喂，Enter 收尾）
    &[":", "s", "/", "a", "/", "b", "/", "<CR>"],
    &[":", "%", "s", "/", "a", "/", "b", "/", "g", "<CR>"],
    &[":", "d", "<CR>"],
    &[":", "%", "d", "<CR>"],
    &[":", "j", "<CR>"],
    &[":", "1", "m", "2", "<CR>"],
    &[":", "p", "u", "<CR>"],
    // 可视模式下的编辑
    &["v", "l", "d"],
    &["V", "d"],
    &["v", "l", "~"],
];

/// 查看类键位表：只读模式下每一条都必须照常工作，**不得**出现 E21。
/// `y`/`yy`/`m{a}`/`q`/`:q` 在 vim 里都不报 E21（同批实证）。
const VIEW_KEYS: &[&[&str]] = &[
    &["g", "g"],
    &["w"],
    &["y", "i", "w"],
    &["y", "y"],
    &["m", "b"],
    &["V", "j", "y"],
    &["/", "g", "a", "m", "m", "a", "<CR>"],
    &["G"],
    &["<C-o>"],
    &["z", "z"],
];

fn readonly_probe(
    vim: &mut VimState,
    before: &str,
    host: &mut TckHostRo,
    // `run` feeds the keys and hands back the buffer text afterwards — one
    // closure so the two contracts can share the whole body while differing
    // in how they reach the buffer (`Ctx` vs `CtxRo`).
    mut run: impl FnMut(&mut VimState, &mut TckHostRo, &[&str]) -> String,
) -> Result<(), String> {
    for keys in EDIT_KEYS {
        run(vim, host, &["<Esc>"]);
        host.statuses.clear();
        let after = run(vim, host, keys);
        if after != before {
            return Err(format!(
                "{keys:?}: 缓冲被改动了（{before:?} → {after:?}）——只读模式下的编辑必须整条拒绝"
            ));
        }
        if !host.statuses.iter().any(|s| s.contains("E21")) {
            return Err(format!(
                "{keys:?}: 没收到 E21（收到 {:?}）——拒绝必须是响铃 + 状态消息，不是静默 no-op",
                host.statuses
            ));
        }
    }
    // 拒绝不污染状态机：查看类键位照常工作（motion / 搜索 / 标记 / yank）
    host.statuses.clear();
    run(vim, host, &["<Esc>"]);
    run(vim, host, &["g", "g"]);
    if vim.cursor.offset != 0 {
        return Err(format!(
            "拒绝之后 gg 的光标 = {}（应为 0）——状态机被 E21 污染了",
            vim.cursor.offset
        ));
    }
    run(vim, host, &["w"]);
    if vim.cursor.offset != 6 {
        return Err(format!(
            "拒绝之后 w 的光标 = {}（应为 6，\"beta\" 行首）",
            vim.cursor.offset
        ));
    }
    // yank 仍然可用：写寄存器，并（`"+` 前缀时）写剪贴板
    run(vim, host, &["y", "i", "w"]);
    let yanked = vim
        .registers
        .get('"')
        .map(|r| r.text.clone())
        .unwrap_or_default();
    if yanked != "beta" {
        return Err(format!(
            "只读模式下 yiw 抽出的是 {yanked:?}（应为 \"beta\"）"
        ));
    }
    host.clipboard = None;
    run(vim, host, &["\"", "+", "y", "i", "w"]);
    let clipped = vim.registers.get('+').map(|r| r.text.clone()).unwrap_or_default();
    if clipped != "beta" {
        return Err(format!(
            "只读模式下 \"+yiw 抽出的是 {clipped:?}（应为 \"beta\"）"
        ));
    }
    if host.clipboard.as_deref() != Some("beta") {
        return Err(format!(
            "只读模式下的 \"+y 没有写剪贴板（{:?}）——查看类宿主也要能复制",
            host.clipboard
        ));
    }
    for keys in VIEW_KEYS {
        host.statuses.clear();
        run(vim, host, keys);
        if host.statuses.iter().any(|s| s.contains("E21")) {
            return Err(format!(
                "{keys:?}: 查看类键位被误报 E21（{:?}）",
                host.statuses
            ));
        }
    }
    let after = run(vim, host, &[]);
    if after != before {
        return Err(format!("查看类键位改动了缓冲（{before:?} → {after:?}）"));
    }
    Ok(())
}

/// 无副作用宿主：只想让引擎驱动 buffer 时使用。宿主自身的 undo、剪贴板、
/// 高亮等语义不属于 TCK 范围，由宿主自己的测试覆盖。
#[derive(Default)]
pub struct TckHost;

impl VimHost for TckHost {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, _: usize) {}
    fn clipboard_write(&mut self, _: &str) {}
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, _: &[Range<usize>], _: Option<Range<usize>>) {}
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
}

/// 记录型只读宿主：`VimHostRo` 的最小实现 + 状态/响铃/剪贴板记录，供
/// [`readonly_contract`] 与 [`readonly_pipeline_contract`] 断言「拒绝被上报、
/// 而不是静默吞掉」。它同时实现 [`VimHost`]（编辑侧钩子全空），所以两条
/// 管线（A 步兼容层与 B 步 `handle_key_ro`）共用一套探针。
#[derive(Default)]
pub struct TckHostRo {
    pub statuses: Vec<String>,
    pub bells: usize,
    pub clipboard: Option<String>,
    pub highlights: Vec<Range<usize>>,
    pub current_highlight: Option<Range<usize>>,
    pub scrolled_to: Vec<usize>,
    pub close_requested: bool,
    pub close_forced: bool,
}

impl VimHostRo for TckHostRo {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, line: usize) {
        self.scrolled_to.push(line);
    }
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current_highlight = current;
    }
    fn changed(&mut self) {}
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
        self.close_forced = false;
    }
    fn request_close_forced(&mut self, forced: bool) {
        self.close_requested = true;
        self.close_forced = forced;
    }
}

impl VimHost for TckHostRo {
    fn viewport(&self) -> (usize, usize) {
        (0, 24)
    }
    fn scroll_to_line(&mut self, line: usize) {
        self.scrolled_to.push(line);
    }
    fn clipboard_write(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }
    fn clipboard_read(&self) -> Option<String> {
        None
    }
    fn set_search_highlights(&mut self, matches: &[Range<usize>], current: Option<Range<usize>>) {
        self.highlights = matches.to_vec();
        self.current_highlight = current;
    }
    fn begin_undo_group(&mut self, _: u64, _: usize) {}
    fn undo(&mut self) -> Option<usize> {
        None
    }
    fn redo(&mut self) -> Option<usize> {
        None
    }
    fn changed(&mut self) {}
    fn bell(&mut self) {
        self.bells += 1;
    }
    fn status_message(&mut self, message: &str) {
        self.statuses.push(message.to_owned());
    }
    fn request_close(&mut self) {
        self.close_requested = true;
        self.close_forced = false;
    }
    fn request_close_forced(&mut self, forced: bool) {
        self.close_requested = true;
        self.close_forced = forced;
    }
}

fn feed(vim: &mut VimState, buf: &mut dyn VimBufferMut, host: &mut dyn VimHost, keys: &str) {
    let mut ctx = Ctx { buf, host };
    for c in keys.chars() {
        vim.handle_key(&mut ctx, Key::char(c));
    }
}

/// insert 模式的打字路径：打印字符在 insert 模式刻意返回 `Unknown`
///（IME 契约），文本经 `record_typed_text` + `insert_text_at_cursor` 直落
/// buffer——与集成层 `place_text` 同款。
fn type_text(vim: &mut VimState, buf: &mut dyn VimBufferMut, host: &mut dyn VimHost, text: &str) {
    let mut ctx = Ctx { buf, host };
    vim.record_typed_text(text);
    vim.insert_text_at_cursor(&mut ctx, text);
}

fn assert_content(buf: &impl VimBuffer, want: &str) -> Result<(), String> {
    buffer_read_contract(buf)?;
    let got = buf.slice(0..buf.len());
    if got != want {
        return Err(format!("内容 {got:?} ≠ 期望 {want:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 反例：非末行 range 不含 \n——常见实现错误，TCK 必须抓住。
    struct NoNewlineBuf(pub String);

    impl VimBuffer for NoNewlineBuf {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn line_count(&self) -> usize {
            TckStrBuf(self.0.clone()).line_count()
        }
        fn char_at(&self, offset: usize) -> Option<char> {
            TckStrBuf(self.0.clone()).char_at(offset)
        }
        fn prev_char_offset(&self, offset: usize) -> Option<usize> {
            TckStrBuf(self.0.clone()).prev_char_offset(offset)
        }
        fn line_range(&self, line: usize) -> Range<usize> {
            let mut start = 0;
            for (i, part) in self.0.split('\n').enumerate() {
                if i == line {
                    return start..start + part.len(); // 错误：丢了 \n
                }
                start += part.len() + 1;
            }
            self.0.len()..self.0.len()
        }
        fn offset_to_line(&self, offset: usize) -> usize {
            TckStrBuf(self.0.clone()).offset_to_line(offset)
        }
        fn slice(&self, range: Range<usize>) -> String {
            self.0[range].to_owned()
        }
    }

    #[test]
    fn read_contract_accepts_reference_impl() {
        let buf = TckStrBuf("alpha\nbeta 中文\n\n尾行\n".into());
        buffer_read_contract(&buf).unwrap();
        let buf = TckStrBuf("中文 mix emoji 👨‍👩‍👧".into());
        buffer_read_contract(&buf).unwrap();
        let buf = TckStrBuf(String::new());
        buffer_read_contract(&buf).unwrap();
    }

    #[test]
    fn read_contract_catches_missing_newline_in_range() {
        let buf = NoNewlineBuf("alpha\nbeta".into());
        assert!(buffer_read_contract(&buf).is_err());
    }

    #[test]
    fn edit_contract_passes_reference_impl() {
        let mut buf = TckStrBuf::default();
        buffer_edit_contract(&mut buf).unwrap();
    }

    #[test]
    fn engine_smoke_passes_reference_impl() {
        let mut buf = TckStrBuf::default();
        let mut host = TckHost;
        engine_smoke_contract(&mut buf, &mut host).unwrap();
        assert_eq!(buf.0, "gammax\nlpha\nbeta\n");
    }

    /// 只读宿主的最小形态：`impl VimBuffer` + **零方法**的 `VimBufferMut`。
    /// 契约必须在这种"什么都写不了"的实现上也通过。
    struct RoBuf(&'static str);

    impl VimBuffer for RoBuf {
        fn len(&self) -> usize {
            TckStrBuf(self.0.to_owned()).len()
        }
        fn line_count(&self) -> usize {
            TckStrBuf(self.0.to_owned()).line_count()
        }
        fn char_at(&self, offset: usize) -> Option<char> {
            TckStrBuf(self.0.to_owned()).char_at(offset)
        }
        fn prev_char_offset(&self, offset: usize) -> Option<usize> {
            TckStrBuf(self.0.to_owned()).prev_char_offset(offset)
        }
        fn line_range(&self, line: usize) -> Range<usize> {
            TckStrBuf(self.0.to_owned()).line_range(line)
        }
        fn offset_to_line(&self, offset: usize) -> usize {
            TckStrBuf(self.0.to_owned()).offset_to_line(offset)
        }
        fn slice(&self, range: Range<usize>) -> String {
            TckStrBuf(self.0.to_owned()).slice(range)
        }
    }

    impl VimBufferMut for RoBuf {}

    #[test]
    fn readonly_contract_passes_a_write_refusing_buffer() {
        // an EMPTY `impl VimBufferMut` — the whole point of P1-A
        let mut buf = RoBuf(READONLY_PROBE_TEXT);
        readonly_contract(&mut buf).unwrap();
        assert_eq!(buf.slice(0..buf.len()), READONLY_PROBE_TEXT);
    }

    #[test]
    fn readonly_pipeline_contract_passes_the_reference_impl() {
        let buf = TckStrBuf::from_text(READONLY_PROBE_TEXT);
        readonly_pipeline_contract(&buf).unwrap();
    }

    /// A 步的安全网：宿主忘了 `set_modifiable(false)`，只有写原语在拒绝。
    /// 这时改动仍然一件事都没发生，而且必须上报 E21（不是静默 no-op）。
    #[test]
    fn write_primitive_refusal_reports_e21_even_with_modifiable_on() {
        let mut buf = RoBuf(READONLY_PROBE_TEXT);
        let mut host = TckHostRo::default();
        let mut vim = VimState::new(); // 'modifiable' stays ON
        {
            let mut ctx = Ctx {
                buf: &mut buf,
                host: &mut host,
            };
            vim.handle_key(&mut ctx, Key::parse("x"));
        }
        assert_eq!(buf.slice(0..buf.len()), READONLY_PROBE_TEXT);
        assert!(
            host.statuses.iter().any(|s| s.contains("E21")),
            "写原语拒绝时必须上报 E21，收到 {:?}",
            host.statuses
        );
    }
}
