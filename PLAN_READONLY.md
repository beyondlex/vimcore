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
| M5 | ✅ 跨仓 | gpui-vim-x 0.2.0 已发布（pager 迁 RO 管线，40/40 绿）；alacer 已合流 registry 0.2.0 并完成 diff view 迁移（§9，三期全落地，773/773 绿） |
| M6 | ✅ 跨仓 | alacer merge view 只读窗格（Local/Remote）迁 RO 管线，Result 保持 RW；`is_write_key` 拦截表 + 写了再回滚机制删除（§10，778/778 绿） |

版本随 M1 的源级破坏（写原语签名）与 M4 的 0.2 API 演进提到 **0.2.0**。

实现相对本文的两处有意偏离（`VimHost` 不做 `VimHostRo` 的 supertrait、
缓冲区用适配器而非 `BufferRef` 枚举）已连同理由记入 `NOTES.md`。

## 9. M5 实施设计：alacer diff view 迁移（2026-10-12 定稿）

### 9.1 现状摸底（迁移前）

- `crates/alacer-ui/src/diff_view/vim_motion.rs` **1226 行**手写只读 vim：
  motions（w/b/e、f/F/t/T、quoted/bracket text object）+ `vim_key` 分发
  （h l ^ _ $ w b e f F t T i a n N / v V y Y g G z{zt,zb,a} 0-9 计数）。
- 宿主状态（`diff_view/mod.rs`）：`vim_col`/`vim_mode`/`vim_anchor`/
  `vim_count`/`pending_yank`/`pending_g`/`pending_z`/`pending_find`/
  `pending_text_obj`，坐标是 (row, grapheme-col)。
- 宿主侧特权：`/` 搜索（IME + 增量高亮，`find_input` 管线）、`n`/`N`
  （find_goto）、`z a`（toggle_fold_all）、`]c`/`[c`（keymap 动作
  NextChange/PrevChange）、j/k（keymap 动作）、C-o/C-i（宿主 jumplist，
  记录 (pane, row, col)——比 vimcore 的 offset-only jumplist 多窗格侧信息）、
  鼠标拖选（pane_mouse）。
- **参照实现在本仓**：`merge_view/vim.rs` 已是 vimcore 的在住 RW 集成
  （`SharedLines` + 前缀和缓存、`MergeVimHost: VimHost`、`offset_rc`/
  `rc_offset` 换算、`vim_frame_sync`/`vim_sync_pane`/`vim_selection`）。
  diff view 迁移照此施工，换成 RO 形态。
- 窗格 `pane.lines: Arc<Vec<String>>`（code_pane.rs:573）——缓冲区适配器
  克隆 Arc 即得零拷贝快照。

### 9.2 三个适配器（新文件 `diff_view/vim_ro.rs`）

1. **`DiffVimRoBuf(Arc<Vec<String>>)`**：只 impl `VimBuffer`（行访问 +
   `len`），**不 impl `VimBufferMut`**——写路径类型上不可达；`VimState` 设
   `set_modifiable(false)`，编辑类按键走 E21。
2. **`DiffVimRoHost`**：impl `VimHostRo`——`viewport`（读窗格可见行范围）、
   `scroll_to_line(_anchored)`、`clipboard_write`、`status_message`（→
   `flash_status`，E21 由此透传）、`set_search_highlights`、`columns`。
   **延迟施用模式**：引擎调用期间把这些效应收进 collector，`handle_key_ro`
   返回后再落到 DiffView（gpui 的 `cx.write_to_clipboard`/`cx.notify` 都在
   调用后发生），绕开 `&mut DiffView` 与 Ctx 的借用冲突。
3. **光标权威归引擎**：所有宿主侧光标变更（j/k、鼠标、]c/[c、n/N、gg/G、
   minimap 点击）一律写穿 `vim_set_cursor_rc(row, col)`；渲染只读引擎
   （`vim.cursor.offset` / `vim.mode` / `vim.visual_anchor`）。与 merge view
   同一模型。

### 9.3 键位划分：引擎拿什么、宿主留什么

| 键 | 归属 | 理由 |
|---|---|---|
| h l ^ _ $ w b e f F t T v V y Y 计数 gg/G/{N}G | **引擎** | vimcore 已全实现且 oracle 对齐 |
| d/c/s/S/C/o/O/i/a/I/A/R/~ x X p P J u C-r . 等 | **引擎**（→ E21） | `modifiable` off 下拒绝 + `status_message` 透传 |
| zz/zt/zb | 宿主（现状） | 与 `z a` 同 Chord；引擎的 z 前缀会吞掉 `z a` |
| `z a` | 宿主 | 折叠切换，引擎无此概念 |
| `/` `n` `N` `?` | 宿主 | 既有 find 管线（IME/增量高亮）远比引擎搜索丰富 |
| `i` `a` | **宿主拦截后转引擎 text-object** | 引擎的 i/a 是 insert/append（RO 下 E21），会杀死 vi"/va" |
| j k ]c [c C-o C-i 鼠标 | 宿主（keymap/既有通道） | 写穿引擎光标；C-o/C-i 走宿主 jumplist（含窗格侧） |

**与计划的两处偏离（连同理由）**：
- **`]c`/`[c` 不走 `MotionHook`**：它们是 keymap 动作而非引擎 motion，
  要让引擎处理就得为"跳到下一个变更"加引擎侧 motion 变体——只读场景
  `d]c` 无意义，不值当。MotionHook 保留给未来的 RW 消费方。
- **z 和弦整体留宿主**：见上表；引擎的 z 前缀与 `z a` 冲突。

### 9.4 删除与测试迁移

- **删**：`vim_word_forward/back/end`、`vim_find`、`quoted_span`、
  `block_span`、`vim_key` 的 motion 分发、五个 pending 字段
  （yank/g/z/find/text_obj——find/text_obj 若宿主保留 text-object 才留，
  见 9.3 的 i/a 行）。
- **留**：`vim_set_caret*`/`vim_move`（写穿用）、`yank_*`（]n/N 之外的
  宿主 yank 通路若保留则留，否则删）、`vim_scroll_cursor_anchored`、
  find 管线、`toggle_fold_all`、`vim_goto_line`（]c/[c 落点用）。
- **测试**：`tests/z_anchor.rs`、`tests/model.rs` 的 `vim_key` 调用改调
  新入口；motion 级语义归 vimcore（已 oracle 对齐 + tck），alacer 侧只测
  集成面：光标写穿、选区渲染、yank 剪贴板、E21 flash、宿主保留键。

### 9.5 分期

1. **并行接入**：新写 `vim_ro.rs`，`vim_key` 的 motion 分支改调引擎，
   宿主特权键原位拦截——行为不变、可对拍。
2. **删除**：手写 motion 代码 + mod.rs 字段 + render/mouse 改读引擎；
   测试改写。
3. **收尾**：`flash_status` 的 E21 文案、help 面板的 vim_keys 清单核对、
   `cargo test -p alacer-ui` 全绿后单独提交。

## 10. M6 实施设计：alacer merge view 只读窗格迁 RO 管线（2026-10-13 定稿并完成）

### 10.1 与 diff view（§9）的关键差异：混合，不是整体

merge view 是三窗格 RW 在住集成（Local / Result / Remote），**Result 是冲突
解决的手改出口，必须保持 RW**。所以 M6 的范围是：

| 窗格 | 路径 | 写入 |
|---|---|---|
| Result | `Ctx` + `MergeVimBuffer`（RW，不变） | 真编辑 |
| Local / Remote | `CtxRo` + `LineBufView` → `handle_key_ro` | 引擎内 E21 |

### 10.2 替换掉的旧机制（及其漏洞）

旧设计对只读窗格是「**拦截表 + 写了再回滚**」：`is_write_key` 表拦住单键写
操作；漏网的（如 `gI` 这类和弦）会经 RW 路径真写进 SharedLines，靠
`vim_flush` 发现缓冲 Arc 变了再回滚 + 闪 READONLY_MSG。新设计下写操作在
**构造上不可能**（`RoBuffer` 拒写、`RoHost` 惰化编辑侧钩子），拦截表和回滚
分支（和弦路径的安全网保留）一并删除。

### 10.3 接入形态

- `MergeVimHost` **直接** `impl VimHostRo`（收集器字段与 `VimHost` impl 共
  享，`vim_flush` 两条路径同一套效应落盘）——不需要第二个适配器结构体；
  `VimHostRo` 与 `VimHost` 的调用都经 trait object，无方法名歧义。
- `vim_feed_engine_ro`：馈送前仍 `vim_buffer.0.set(before)`（保持光标换算
  的 Arc 快路径与 clamp 路径一致），但缓冲以 `&dyn VimBuffer` 直接呈给
  `CtxRo`，RW 存储不被触碰。
- 路由点在 `vim_process_key` 原拦截表位置；**映射和弦键保留 RW 路径**
  （和弦是故意的：action 型映射要在只读窗格能完成，而 `RoHost` 惰化
  host-action 桥）；leader 中止重放也按窗格选路。

### 10.4 顺带核实的引擎行为（vim 9.1 oracle + 引擎探针）

| 按键族 | vim 9.1（`:set noma`） | 引擎 | 处置 |
|---|---|---|---|
| `i`/`I`/`a`/`A`/`o`/`O`/`gi`/`R` | E21 在入口 | 入口即拒（M1 已锁） | 一致 |
| `gI`、visual/block `I`/`A` | **静默进插入**、首字 E21 | 入口即拒 | 引擎更严（安全方向），tck `EDIT_KEYS` 补锁 + 注释记偏 |

`gI` 的 vim 行为（静默入口）是 vim 的怪癖；引擎拒绝更早且只读窗格拉住
visual 选区，体验更好。写原语在 `CtxRo` 下类型层面不可达，不存在"放进插入
再写坏"的路径。

### 10.5 结果

- alacer：`vim_ro_panes.rs` 5 个集成测试（E21 flash + 缓冲不变 + 光标不
  漂 / visual 编辑拒且选区保留 / 导航 + 搜索 + yank + `"+y` 剪贴板 /
  Result 可写路径不受影响 / leader 和弦在只读窗格仍完成）；全量
  `cargo test --workspace` 绿，clippy 干净。
- vimcore：`tck.rs` EDIT_KEYS 补 8 条插入族角落 + `readonly_pipeline.rs`
  锁偏离后模式语义；1161/1161 绿。

## 11. 引擎信号闭环：搜索高亮渲染 + 视觉响铃（2026-10-13 完成）

M1–M6 收口后剩下的两个已知缺口，本轮闭合：

### 11.1 引擎搜索高亮进渲染（diff view）

`*`/`#` 的移动一直即时生效，但 `set_search_highlights` 的发布在 diff view
只被收藏、每次喂键后清空（`vim_ro.rs` 旧 312–315 行），从未渲染。现在：

- `vim_apply_engine_search_hits`（diff view）把引擎字节范围转换成焦点窗格的
  `search_hits`——与 merge view `vim_publish_highlights` 同构（跨行命中夹到
  起始行），接的是 CodePane 既有的金色高亮画布 + 行高亮缓存，零新渲染机制；
- 引擎搜索**接管宿主 find 状态**（find_matches/find_current 重建、
  find_query 让位）：vim 语义里 `*` 即取代上一次搜索，`n`/`N` 因此 navigate
  新命中而不是陈旧的主机 `/` 结果；
- 只画焦点窗格（vim 的 `*` 只搜当前缓冲）；宿主 `/` 开搜时
  `clear_search_hits` 复位 `vim_hits_pane`，两套搜索互不叠加；
- **窗格切换重扫**：引擎缓冲随焦点换文本，旧发布是旧坐标。vimcore 0.2.1 新增
  `refresh_highlights_ro`（与 `handle_key_ro` 同一层 RoBuffer/RoHost 包装），
  宿主在切换后第一次喂键时按新缓冲重扫重发——顺带消除了「`n` 把光标拽回旧
  窗格」的 bug。残留小缺口：上次发布为空（pattern 无命中）再切换时不重扫，
  直到下一次搜索动作（`vim_hits_pane` 为 None 即无记录）。

### 11.2 bell 真转发 + 视觉响铃

引擎的 `bell()` 在 `VimHostRo` 有空默认实现，两个宿主都落入"被丢弃"：

- diff view 曾在 `vim_key` 里**合成**两条手写期消息（"No matching bracket
  found" / "No quoted string on this line"）——vim 9.1 真值是失败动作只响铃、
  **无消息文本**（实证见 §9 对拍）。合成逻辑连同 `pending_target`/`block_pair`
  一起删除；
- 两个宿主实现 `bell()` → 收集 → flush 触发**视觉响铃**：约 150ms 的整屏
  闪烁（前景色 12% 透明度叠层，终端 visualbell 的同构），代次机制防旧定时器
  清掉新响铃；
- E21 走 `status_message`（toast 文本）**加** bell（闪烁）——vim 的错误是
  消息 + beep，两者并存不互斥。

### 11.3 发布与验证

- vimcore **0.2.1** 已发布 crates.io（`refresh_highlights_ro`，新增 API 向后
  兼容）；alacer lock 跟进。
- alacer：782/782 绿（新增 5 个测试：`*` 高亮 + n/N 接管、窗格切换重扫、esc
  清高亮、merge RO 窗格 bell 无消息、E21 消息+响铃）；clippy 干净。
