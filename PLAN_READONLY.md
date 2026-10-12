# PLAN: 只读管线（read-only pipeline）与宿主 motion 拦截点

> 起草：2026-10-12。动机来自第一个真实消费方——alacer 的 diff view
> （只读 diff 浏览器，双窗格 + unified 合成行投影 + 折叠）。
> 本文所有"现状"摘录均核对过当前 main 的源码。

## 1. 动机

vimcore 的编辑能力（insert session、undo 组、ops）与它的 mode machine /
motions / text objects / search / marks / 滚动是绑在一起消费的：按键管线的
唯一入口要求一个**可变**缓冲区，导致纯只读宿主（diff/日志/预览类查看器）接不进来——
要么伪造 `VimBufferMut` 的写原语，要么放弃整台引擎。

alacer 目前在 diff view 用了一个几百行的手写只读 vim
（`diff_view/vim_motion.rs`），只覆盖 motions 的一小角（h/l/w/e、f/F/t/T、
引号/括号 text object、滚动、计数），且与 vimcore 的语义一致性靠手工对齐。
只读管线落地后，它应整体迁移到 vimcore。

## 2. 现状与硬约束

```rust
// state.rs
pub struct Ctx<'a> {
    pub buf: &'a mut dyn VimBufferMut,   // ← 硬约束：只读宿主拿不出这个
    pub host: &'a mut dyn VimHost,
}
pub fn handle_key(&mut self, ctx: &mut Ctx, key: Key) -> KeyResult; // Consumed | Unknown

// buffer.rs
pub trait VimBuffer { len / line_count / char_at / prev_char_offset
                      / line_range / offset_to_line / slice … }      // 只读，够用
pub trait VimBufferMut: VimBuffer {
    fn insert_text(&mut self, offset: usize, text: &str);           // 必须实现
    fn delete_range(&mut self, range: Range<usize>);                // 必须实现
}

// host.rs —— 现有 VimHost 的钩子按"编辑会话"与"纯查看"分两组：
//   查看侧：viewport / scroll_to_line(_anchored) / scroll_lines /
//           clipboard_write / set_search_highlights / changed / bell /
//           status_message / request_close(_forced) / buffer_name
//   编辑侧：begin_undo_group / undo / redo / save /
//           cycle_buffer / first_buffer / last_buffer / clipboard_read
//           （paste 是编辑；:w 与 buffer 轮换对只读宿主无意义）
```

写原语的调用点集中在 ops.rs（operator 执行）与 insert_mode.rs，
mode machine / motions / objects / search / marks 本身不写缓冲区——
这是"只读管线"可行的结构基础。

## 3. 目标 / 非目标

**目标**
1. 只读宿主只需 `impl VimBuffer` + 一个精简 host trait，即可获得完整的
   normal/visual motion、text object、搜索、标记、滚动、寄存器-yank 能力；
2. 编辑类按键在只读模式下按 vim 的 E21 语义拒绝（响铃 + status_message，
   不 panic、不 silent no-op）；
3. 宿主可接管个别 motion（diff 的 `]c`/`[c` chunk 跳转、折叠感知的行移动）；
4. 现有 API 零破坏，全部增量。

**非目标**
- 不在本计划内做"行索引模型"重构（引擎仍以字节偏移为光标模型，折叠由
  宿主经拦截点处理）；
- 不改 gpui 集成层以外的前端假设（ratatui/crossterm 宿主同样受益）。

## 4. 设计

### P1 — 只读管线

**A 步（兼容层，先行，无 breaking change）**

- `VimBufferMut` 的两个写原语改为**可选**：
  ```rust
  fn insert_text(&mut self, offset: usize, text: &str) -> bool { false }
  fn delete_range(&mut self, range: Range<usize>) -> bool { false }
  ```
  返回 `false` = 拒绝（默认实现即拒绝）。编辑型宿主照旧实现并返回 `true`；
  ops.rs 的 apply 路径在原语返回 `false` 时走"未修改"分支（保持
  cursor/register 状态一致，相当于 vim 的 E21：改动未发生）。
- `VimState` 增加 `modifiable: bool`（默认 `true`；仿 vim 的 `'modifiable'`）。
  为 `false` 时，编辑类命令（operator 执行、i/a/I/o/O 进 insert、`~`/`x`
  等直接编辑、`:s`、`:!` 过滤、`.` 重复编辑命令、paste）在**命令执行点**
  提前拒绝：`host.bell()` + `host.status_message("E21: Cannot make changes,
  'modifiable' is off")`，按键记为 `Consumed`（与 vim 一致——被拒绝也是
  "引擎处理过"，不能漏给宿主二次处理）。
- 只读宿主的接入成本变成：`impl VimBuffer` + `impl VimBufferMut {}`（零方法，
  全走默认拒绝）+ `vim.set_modifiable(false)`。

**B 步（类型层拆分，0.2 的 API 演进）**

- 新增 `VimHostRo`：现有 `VimHost` 减去编辑侧钩子（§2 分组），
  现 `VimHost: VimHostRo`（supertrait），编辑宿主零迁移。
- 新增 `CtxRo<'a> { buf: &'a dyn VimBuffer, host: &'a mut dyn VimHostRo }`
  与 `VimState::handle_key_ro(&mut self, ctx: &mut CtxRo, key: Key) -> KeyResult`。
- 实现路径（不复制 6.4k 行的 state.rs）：内部把 `Ctx` 的 `buf` 换成
  `BufferRef<'a>`（`enum { Ro(&dyn VimBuffer), Rw(&mut dyn VimBufferMut) }`，
  文本读取方法转发两者、写原语仅 Rw 分支有实现），`handle_key` 构造 Rw 视图、
  `handle_key_ro` 构造 Ro 视图，主管线共用。`modifiable=false` 的拒绝检查
  与 Ro 视图的写原语不可达互为双保险。
- marks/jumplist 在只读模式下天然安全：缓冲区不变，偏移永不失效。

**语义对齐清单（P1 验收的一部分）**

| 类别 | 只读模式行为 |
|---|---|
| motions（h/l/w/e/f/F/t/T/%/G/gg/{count}） | 正常 |
| text objects（visual 内 iw/a"/i( 等） | 正常 |
| operator `y`（yank） | 正常，写寄存器 + `clipboard_write` |
| operator `d/c/x/s/p/…`、`~` 等直接编辑 | E21 拒绝，状态不漂移 |
| `v`/`V`/`gv`、`/`?n/N`、搜索高亮发布 | 正常 |
| `m{a}` 标记、`` `{a} `` 跳转 | 正常 |
| 滚动（zz/zt/zb/C-d/C-u/C-e/C-y） | 正常 |
| insert 系（i/a/I/o/O、R） | E21 拒绝（不进 insert） |
| `:s`、`:%!`、`.`（编辑重复）、undo/redo | E21 / 静默拒绝（vim 同款：undo 无历史时是 bell） |
| `:q`/`:q!`/`ZZ` | 走 `request_close(_forced)`（查看器同样合法） |

### P2 — 宿主 motion 拦截点

解决"折叠/合成行投影"类宿主的需求（alacer diff view：`j/k` 必须走
**可见行**、`]c`/`[c` 跳 chunk；`z a` 折叠切换本来就在宿主侧）：

```rust
// VimHostRo（或单独 trait，见"决策点"）
/// 宿主先答：Some = 已处理该 motion（引擎只消费 count 与模式状态副作用）；
/// None = 交回引擎内建实现。
fn intercept_motion(
    &mut self, vim: &VimState, motion: Motion, count: usize,
) -> Option<MotionResult>;
```

- 插入点：`Motion::target()` 的最外层。宿主拦截器里可以自由实现
  "跳过隐藏行"、"跳到下一个 diff chunk"，并返回与引擎同型的
  `MotionResult`（含 Exclusive/Inclusive/Linewise 与 stuck 语义），
  operator-pending / visual 扩展全部免费获得。
- 这比"引擎内建可见行模型"（给 `VimBuffer` 加 `is_line_visible()` 并改
  Up/Down 语义）侵入小得多，且不把折叠概念泄漏进引擎。后者列为
  远期可选项，不进本计划。

**决策点（实现前拍板）**
1. `intercept_motion` 放 `VimHostRo` 还是独立 opt-in trait `MotionHook`？
   倾向独立 trait：不污染普通宿主，未实现即零开销。
2. 拦截点收到的 `Motion` 是枚举原值；宿主自造 motion（`]c`）如何表达？
   倾向：宿主在拦截器里按"上/下一个目标行"直接返回 `MotionResult`，
   不需要枚举扩展；枚举保持内建 motion 集合。

### P3 — gpui-vim 集成层 + alacer 迁移验证

- gpui-vim 增加 `VimEditorRo`（只读变体：按键拦截 + 模式徽章，无
  text-input 桥、无 undo 对账）。
- alacer diff view 迁移（第一个真实消费方，充当验收）：
  - 删除 `vim_motion.rs` 中的 motions/find/text-object/滚动/计数实现，
    换成 vimcore RO；
  - 保留宿主侧：双窗格焦点与同步滚动、折叠、unified 行投影、
    `z a`、`]c`（经 `intercept_motion`）；
  - 键位表 `[diff]` 与 merge 的差异项重新审计（哪些键可以合并）。

## 5. 测试计划

- **tck 新契约探针**：`readonly_contract(buf, vim)`——`modifiable=false`
  下喂全部编辑类键位表，断言：缓冲区逐字节不变、返回 E21 状态消息、
  后续 motion/搜索仍正常（拒绝不污染状态机）。
- **motions 等价性**：同一批 tck 按键序列分别在 RW 与 RO 管线回放，
  cursor/高亮/寄存器逐一相等。
- **fuzz**：现有 round 套件加一个 RO 配置，host 用"写原语即 panic"的
  探针实现——任何一次到达写原语即红。 fuzz 的 `!` 安全阀（round20）在
  RO 模式下天然强断言。
- **拦截点**：宿主拦截器返回 stuck / jump 的单测；visual 模式下拦截
  motion 的选区扩展测试。

## 6. 兼容性

- A 步：`VimBufferMut` 写原语签名由 `()` 变 `bool` —— 对已实现的编辑宿主
  是源级破坏，但机械替换（加 `-> true`）。0.1.x 内做掉，随
  changelog 标注；或先加 `insert_text_checked`/`delete_range_checked`
  过渡、旧方法 `#[deprecated]`。实现时二选一。
- B 步：纯增量；`VimHost: VimHostRo` 保证编辑宿主零迁移。
- semver：A 步按 0.1.x 的破坏预算处理，B 步随 0.2。

## 7. 里程碑

| 里程碑 | 内容 | 规模 |
|---|---|---|
| M1 | A 步：可写原语默认拒绝 + `modifiable` + E21 + 语义对齐清单全绿 | 1 个 PR，中 |
| M2 | tck/fuzz RO 契约 + RW/RO 等价性回放 | 1 个 PR，中 |
| M3 | `intercept_motion`（独立 trait）+ 测试 | 1 个 PR，小 |
| M4 | B 步：`VimHostRo`/`CtxRo`/`handle_key_ro`（0.2） | 1–2 个 PR，大（动 Ctx 构造） |
| M5 | gpui-vim `VimEditorRo` + alacer diff view 迁移验收 | 跨仓 |

M1–M3 完成后 alacer 即可迁移（用 A 步兼容层 + 拦截点即可满足 diff view
全部需求）；M4 是 API 洁癖步，不阻塞消费方。

## 8. 实施状态（2026-10-12）

| 里程碑 | 状态 | 落地 |
|---|---|---|
| M1 | ✅ | `VimBufferMut` 写原语返回 `bool` 且默认拒绝；`VimState::modifiable`；E21 五处闸门；`:set [no]ma` |
| M2 | ✅ | `tck::readonly_contract` / `tck::readonly_pipeline_contract`；`tests/readonly_pipeline.rs`（RW/RO 等价回放 + panic-on-write fuzz） |
| M3 | ✅ | `MotionHook` + `VimHost(Ro)::motion_hook()`，插入点 `Motion::target` |
| M4 | ✅ | `VimHostRo` / `CtxRo` / `handle_key_ro`（`RoHost` + `RoBuffer` 适配器，主管线零改动） |
| M5 | ⏳ 跨仓 | gpui-vim `VimEditorRo` + alacer diff view 迁移（不在本仓） |

版本随 M1 的源级破坏（写原语签名）与 M4 的 0.2 API 演进提到 **0.2.0**。

实现相对本文的两处有意偏离（`VimHost` 不做 `VimHostRo` 的 supertrait、
缓冲区用适配器而非 `BufferRef` 枚举）已连同理由记入 `NOTES.md`。
