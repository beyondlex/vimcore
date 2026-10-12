# CHANGELOG

本文件只记录**面向宿主的 API 变化**。各轮语义修复的逐项明细在
`NOTES.md`（按轮次）与 `BUG_AUDIT*.md`（独立审计全量账），发布前以两者为准。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号与 `Cargo.toml` 一致。

## [0.2.0] - 2026-10-12

只读管线（read-only pipeline）与宿主 motion 拦截点——`PLAN_READONLY.md`
M1–M4。动机：diff / 日志 / 预览类纯查看宿主接不进来（`Ctx` 要求
`&mut dyn VimBufferMut`），此前只能伪造写原语或放弃整台引擎。

### Added

- **只读管线**：`VimHostRo`（`VimHost` 的查看侧钩子子集）、`CtxRo { buf:
  &dyn VimBuffer, host: &mut dyn VimHostRo }`、`VimState::handle_key_ro`。
  主管线与 `handle_key` 共用，不复制 state.rs。缓冲以共享引用进入，
  改动在构造上不可能。
- **`MotionHook`**（独立 opt-in trait）+ `VimHost::motion_hook()` 取用器
  （默认 `None`）：宿主可接管个别 motion（diff 的 `]c`/`[c`、折叠感知的
  `j/k`），插入点在 `Motion::target` 最外层；算子待决与可视扩展免费获得。
- **`'modifiable'`**：`VimState::set_modifiable` / `is_modifiable`，
  `:set [no|inv]ma[?|!|&]` 可用。为 `false` 时编辑类命令在执行点按 vim
  9.1 的 E21 语义拒绝（响铃 + `status_message`，按键记 `Consumed`）。
- **tck 新契约**：`tck::readonly_contract`（A 步）与
  `tck::readonly_pipeline_contract`（B 步）——41 条编辑键位断言 E21 +
  缓冲逐字节不变 + 拒绝后状态机健康；10 条查看键位不得误报。
- `RoBuffer` / `RoHost` 适配器（`&dyn VimBuffer` → `VimBufferMut`、
  `&dyn VimHostRo` → `VimHost`），宿主自己的类型无需改动即可接入。

### Changed（**源级破坏**）

- `VimBufferMut::insert_text` / `delete_range` / `replace_range` 的返回类型
  由 `()` 改为 **`bool`**，且**默认实现返回 `false`（拒绝）**。
  编辑型宿主的迁移是机械替换：两个方法各加 `-> true`。
  只读宿主的接入成本变成 `impl VimBufferMut for X {}`（零方法）。
- `Cargo.toml` 版本 0.1.30 → 0.2.0。

### 设计取舍（相对计划的偏离）

- `VimHost` **不做** `VimHostRo` 的 supertrait（会逼编辑宿主多写一个 impl
  块，违背零迁移；且默认值钩子会让查看侧静默拿到错视口）——改用 `RoHost`
  适配器。
- 缓冲区用 `RoBuffer` 适配器而非计划中的 `BufferRef` 枚举——枚举方案要改
  `ctx.buf` 的 589 处引用，适配器方案主管线零改动。

理由与 oracle 实证记录在 `NOTES.md` 的「只读管线（PLAN_READONLY P1/P2）」段。

## [0.1.30] - 2026-10-11 及以前

0.1.x（0.1.1 → 0.1.30）是与 vim 9.1 的逐轮语义对齐：三十余轮检视/独立审计，
累计 300+ 项语义修复，全部经 `/usr/bin/vim`（9.1 patches 1-1752）实证。
每轮的修复清单见 `NOTES.md` 对应轮次与 `BUG_AUDIT*.md`；测试按轮次落在
`tests/audit*`、`tests/parity_round*`、`tests/probe_round*`、`tests/fuzz_*`。

0.1.x 内无面向宿主的破坏性变更（宿主侧 API 稳定至 0.1.30）。
