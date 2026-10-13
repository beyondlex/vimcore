# 中文 → 英文 迁移计划

覆盖 4 个仓库：`vimcore`（底座）、`alacer`、`gpui_vim`、`crossterm_vim`。
执行手册，不是提案。

**已演练验证**：§5.3 的 filter-branch 全流程（在 vimcore 完整副本上跑了 231
个提交，60s，tree hash 逐一一致）；§4.0.1 归档；§8.2 已发布包统计。

**未演练、首次执行时要小心**：
- §5.2 的 `%B` 导出（演练时用的是 `%s`，后来发现会丢正文，见 §5.2 警示）
- §5.5 第 6 步「正文没丢」验收（针对上面那个坑新加的）

> 执行前先读 §6 风险表，尤其是**加粗的三条**：正文丢失、L4 数据误删、
> 重复提交分叉。这三条都是「做了之后不可逆」或「静默损坏」。

---

## 1. 判定规则：什么改，什么不改

| 层 | 内容 | 规模 | 处理 |
|---|---|---|---|
| L1 | 代码注释 `//` `///` `//!` | 6,700 行 | **翻译**。不参与编译 |
| L2 | 项目文档 `.md` | 4,837 行 | **翻译**。需精译，见 §4.0 去留核查 + §4.3 纪律 |
| L3a | 断言消息 `assert_eq!(.., "光标应归零")` | ~950 处 | **翻译**。不影响断言逻辑 |
| L3b | 错误消息 / panic 文本 | ~35 处 | **翻译** |
| L3c | UI 文本（demo 提示语、`println!`） | ~10 处 | **翻译**（已确认） |
| **L4** | **CJK 测试数据** | **~350 处** | **禁止改动**，见 §1.1 |
| L5 | 提交历史 | 261 条 | **翻译**，见 §5 |

### 1.1 L4 是雷区，必须守住

`vimcore` 是 vim 引擎，CJK 宽字符/多字节边界是核心测试资产：

```rust
let b = buf("中文");                    // crossterm_vim/buffer.rs
let mut b = buf("三四五");               // gpui_vim/buffer.rs
let pool = ["x", "中\n", "\n", "ab\ncd"]; // vimcore/buffer.rs
buf.insert_text(buf.len(), "中");        // vimcore/tck.rs
```

**翻译后测试依然绿**——因为断言的是 `len_utf8() == 3` 这类关系，不是字符本身。
覆盖率静默失效，没人会发现。这是本次迁移唯一会造成实质损害的风险点。

识别模式（改完用它自查）：

```bash
rg -n '[\x{4e00}-\x{9fff}]' --glob '*.rs' | rg 'buf\(|insert_text|char_at|assert_eq!|"中|"文|"三'
```

---

## 2. 现状数据（2026-10-14 复核）

### 工作树含中文（2026-10-14 复核）

| 仓库 | 代码 | 文档 | 合计 |
|---|---|---|---|
| **vimcore** | 5,473 行 / 83 文件 | 2,341 行 / 6 文件 | **7,814** |
| gpui_vim | 255 行 / 7 文件 | 1,156 行 / 10 文件 | 1,411 |
| crossterm_vim | 604 行 / 12 文件 | 460 行 / 6 文件 | 1,064 |
| alacer | 181 行 / 8 文件 | 1 行 / 1 文件 | 182 |
| **四仓合计** | | | **11,471** |

vimcore 拆分：`src/` 276 · `tests/` 5,179 · 文档 2,341 · `Cargo.toml` 3

> vimcore 文档列已从 3,220 降到 2,341：`BUG_AUDIT*.md` 四个文件归档删除
> （§4.0.1，−884），NOTES.md 新增 6 行归档取回说明（+6）。

### 提交历史含中文

| 仓库 | 提交总数 | 标题含中文 | **正文含中文** | 分支 | tag |
|---|---|---|---|---|---|
| vimcore | 236 | 217 | **143** | main | v0.2.0 |
| gpui_vim | 84 | 33 | **25** | **2 个**（已清理） | 1 个 |
| crossterm_vim | 39 | 8 | **9** | main | 0 |
| alacer | 35 | 0 | **1**（刻意保留） | main | 0 |

> ⚠️ **「正文含中文」这一列原先没统计。** Phase B 若只映射标题（`%s`），
> 这 143+25+9 条正文会被 `--msg-filter` 整体删除。见 §5.2 与 §6。

**关键前提**：三个下游仓库全部通过 crates.io 依赖 vimcore
（`vimcore = "0.2"` / `"0.1"`），无 git 依赖、无 `path = "../vimcore"`。
因此 vimcore 历史重写导致 SHA 全变，**不会破坏任何下游构建**。

---

## 3. 前置检查（阻塞项，必须先清）

全部清完（2026-10-14 复核）：

| 仓库 | 工作树 | 分支 | 状态 |
|---|---|---|---|
| alacer | 干净 | `main` | ✅ 已同步 `origin/main` |
| crossterm_vim | 干净 | `main`（+ 备份 ref） | ✅ 已同步，见 §8.5 ③ |
| gpui_vim | 干净 | `main` `fork-gpui-pre` | ✅ 分支 5 → 2，见 §3.1 |
| vimcore | 干净 | `main` | ✅ 已推送 `1ad63f7..6ae45c4`（6 个提交：归档 2 + 发布准备 2 + 计划 2） |

四个仓库当前**全部与远端同步、工作树干净**，这是 Phase A 的起点。

> 历史重写**不会**自动处理未提交的工作树改动。带着脏工作树跑 filter-branch
> 会让改动悬空——所以每批翻译**必须先提交**再进入 Phase B。

### 3.1 gpui_vim 分支清理结果（2026-10-14）

用 `git branch --merged main` 判定（权威，相对 main 而非 HEAD）：

| 分支 | 独有提交 | 结论 | 处理 |
|---|---|---|---|
| `vimcore-0.2-migration` | 0（与 main 同提交 `54f98e9`） | 已合入 | ✅ 已删 |
| `fork-gpui-x` | 0（main 的祖先） | 已合入 | ✅ 已删 |
| `try/gpui-path-perf` | 0（main 的祖先） | 已合入 | ✅ 已删 |
| `fork-gpui-pre` | **2**（`eb70707` `4bfbfa0`） | **独立产品线，保留** | ⬅️ 不动 |

`fork-gpui-pre` 是 gpui-pre 宿主线（`gpui-vim-pre` crate），已发 crates.io
0.1.1 且被 tk 的 Bench 消费，与 main 的 `gpui-vim-x` 是两条产品线，**不能删**。

**删除时踩到的两个坑**（下次照做）：

1. `git branch -d` 的校验基准是**当前 HEAD**，不是 main。主 worktree 当时在
   `fork-gpui-pre` 上，导致已合并的分支被判为「not fully merged」而拒绝删除。
   → 要么临时切到 main，要么在已 checkout main 的 worktree 里执行。
2. 一个分支同一时刻只能被一个 worktree checkout。`vimcore-0.2-migration` 和
   `try/gpui-path-perf` 分别被 `gpui_vim_x`、`gpui_vim-gpui-perf` 占用。
   → 先 `git switch --detach` 解开占用（保留目录），再删分支。

清理后：`gpui_vim_x` 在 main 上，`gpui_vim-gpui-perf` 处于 detached HEAD
（`4a22ed6`，内容原样保留）。远端同名分支尚未删除，需另行确认。

---

## 4. Phase A：工作树翻译

顺序：**先 vimcore**（已确认），跑通后复制到其余三个。

### 4.0 文档去留：先核查「过期文档」假设（2026-10-14）

原计划以为 vimcore 有一批过期文档可删。**核查后该假设不成立**——
根目录 10 个文档全部被代码或文档交叉引用，没有真正的临时/废弃文件。

| 文档 | 行数 | 交叉引用 | 定性 |
|---|---|---|---|
| `NOTES.md` | 1,905 | 核心知识库 | **保留并翻译** |
| ~~`BUG_AUDIT*.md` ×4~~ | ~~884~~ | 曾 27 处（CHANGELOG 2 · NOTES 9 · src 3 · tests 13） | ✅ **已归档删除**（`8460018`） |
| `PLAN_READONLY.md` | 274 | `src/{lib,buffer,host,tables}.rs` · `tests/readonly_pipeline.rs` · NOTES · README · CHANGELOG | **设计文档，保留并翻译** |
| `AGENTS.md` | 64 | 工作约定 | 保留并翻译 |
| `README.md` | 52 | 门面 | 保留并翻译 |
| `CHANGELOG.md` | 38 | 版本历史 | 保留并翻译 |
| `Cargo.toml` | 3 | 注释 | 保留并翻译 |

**`PLAN_READONLY.md` 名字叫 PLAN，实为设计文档**——它被 4 个源码文件引用
（`PLAN_READONLY P1-A` / `P1-B` 等设计决策点），并记录「两处有意偏离」。
M1–M6 已完成，但设计决策是活的，**不能当过期计划删掉**。

### 4.0.1 `BUG_AUDIT*.md` 归档：已执行（2026-10-14，`8460018`）

**决策：归档（省 884 行翻译，占 vimcore 工作树中文量的 10%）。**

四轮独立审计的原始档案（1,082 行），全部发现早已修复；NOTES.md 已有每轮
摘要章节（轮次/发现数/修复数/要点），BUG_AUDIT 只是「逐项 oracle 证据」附件。

**执行记录**：

| 项 | 数值 |
|---|---|
| 删除文件 | 4 个（1,082 行） |
| 改写引用 | **27 处 / 11 文件** |
| 变更规模 | 15 files changed, 32 insertions(+), 1108 deletions(-) |
| 验证 | 编译通过；测试 **1162 passed / 0 failed / 8 ignored** |

**引用改写规则**（初估仅 6 处，实扫为 27 处——**这个教训：估算引用数必须
用 `rg -c` 实扫，不能靠印象**）：把指向已删文件的引用重定向到 NOTES.md
的轮次章节：

| 原文 | 新文 |
|---|---|
| `BUG_AUDIT.md` | 第三十一轮审计 |
| `BUG_AUDIT2.md` | 第三十二轮审计 |
| `BUG_AUDIT3.md` | 第三十三轮审计 |
| `BUG_AUDIT4.md` | 第三十四轮审计 |
| `BUG_AUDIT3 挂账 N` | 第三十三轮审计挂账 N |
| `BUG_AUDIT L3` | 第一轮审计 L3 |
| `BUG_AUDIT*.md`（CHANGELOG 2 处） | 合并进 `NOTES.md` 表述 |

轮次对应关系由各轮 NOTES.md 标题确认（31/32/33/34 轮 ↔ 审计 1/2/3/4）。

**证据未丢失**：git 历史永久保留，可取回
`git show b52851d:BUG_AUDIT4.md`（`b52851d` = 第四轮提交，其余各轮取对应
轮次提交）。NOTES.md 开头已写入这条取回指针。另在工作树外留了一份物理备份
`.workbuddy/archive/`（4 文件 71KB，`.workbuddy` 被全局 gitignore）。

### 4.1 批次划分（vimcore）

| 批次 | 内容 | 行数 | 说明 |
|---|---|---|---|
| A1 | `src/` 8 个文件 | 275 | 生产代码，量小价值高，先做 |
| A2 | 小文档 | 154 | `AGENTS.md` `README.md` `CHANGELOG.md` |
| A3 | `PLAN_READONLY.md` | 274 | 设计文档，术语密集，精译 |
| A4 | `NOTES.md` | 1,905 | 单文件最大，独立批次，精译 |
| A5 | `tests/` 74 个文件 | 5,179 | 体量最大，按文件分批推进 |
| ~~A6~~ | ~~`BUG_AUDIT*.md`~~ | ~~884~~ | ✅ **已随 §4.0.1 归档删除，不再需要翻译** |

> A1–A5 合计 **7,814 行**（实测，= 归档前 8,692 − 878）。

每批独立提交，message 用英文（`docs: translate ... comments to English`）。
**Phase A 全程普通提交，随时可 `git revert`。**

### 4.1.0 逐批文件清单（可直接照做）

数字是各文件当前的含中文行数。**按此顺序推进即可，无需再自行扫描。**

**A1 — `src/` 8 个文件 / 276 行**（生产代码，先跑通流程）

```
 139  src/tck.rs
  66  src/cmdline.rs
  41  src/state.rs
  16  src/ops.rs
  10  src/motions.rs
   2  src/tables.rs
   1  src/insert_mode.rs
   1  src/objects.rs
```

**A5 — `tests/` 74 个文件 / 5,179 行**，已按体量切成 5 组，每组约 1,000 行
（一组一次提交，规模可控、可 revert）：

<details>
<summary>A5.1 — 4 文件 / 1,093 行</summary>

```
 486  tests/review_regressions.rs
 213  tests/audit6_a.rs
 205  tests/audit5_c.rs
 189  tests/audit5_a.rs
```
</details>

<details>
<summary>A5.2 — 7 文件 / 1,092 行</summary>

```
 180  tests/audit3_c.rs
 177  tests/audit6_c.rs
 169  tests/audit5_b.rs
 158  tests/audit3_b.rs
 141  tests/audit3_d.rs
 135  tests/audit3_a.rs
 132  tests/parity_round14.rs
```
</details>

<details>
<summary>A5.3 — 11 文件 / 1,060 行</summary>

```
 125  tests/audit_probe.rs
 107  tests/parity_round9.rs
 105  tests/parity_round8.rs
 100  tests/audit2_e.rs
  97  tests/audit2_c.rs
  93  tests/parity_round13.rs
  90  tests/audit2_d.rs
  89  tests/readonly_pipeline.rs
  86  tests/probe_round30.rs
  85  tests/audit2_b.rs
  83  tests/parity_round7.rs
```
</details>

<details>
<summary>A5.4 — 17 文件 / 1,043 行</summary>

```
  82  tests/audit2_a.rs        61  tests/engine.rs         48  tests/fuzz_round19.rs
  82  tests/audit2_f.rs        61  tests/parity_round36.rs 47  tests/fuzz_round21.rs
  75  tests/audit8_a.rs        60  tests/audit9_a.rs       47  tests/probe_round29.rs
  73  tests/parity_round2.rs   59  tests/audit9_b.rs       54  tests/parity_round15.rs
  70  tests/probe_round35.rs   59  tests/probe_round29c.rs 50  tests/fuzz_round10.rs
  66  tests/parity_round17.rs  49  tests/fuzz_round13.rs
```
</details>

<details>
<summary>A5.5 — 35 文件 / 891 行（都是小文件，可一次做完）</summary>

```
  46  tests/parity_round12.rs   31  tests/fuzz_round36.rs    17  tests/fuzz_round8.rs
  45  tests/fuzz_round14.rs     30  tests/ex_filter.rs       16  tests/parity_round3.rs
  45  tests/fuzz_round24.rs     27  tests/fuzz_round11.rs    14  tests/fuzz_round33.rs
  44  tests/probe_round30b.rs   27  tests/fuzz_round20.rs    10  tests/fuzz_round7.rs
  40  tests/fuzz_round17.rs     25  tests/audit7_b.rs         9  tests/parity_round5.rs
  40  tests/fuzz_round23.rs     25  tests/probe_round29d.rs   6  tests/rc_mapping.rs
  38  tests/fuzz_round25.rs     18  tests/audit6_b.rs         4  tests/parity_round6.rs
  37  tests/fuzz_round16.rs     18  tests/fuzz_round9.rs      3  tests/audit2_scratch.rs
  37  tests/fuzz_round26.rs     35  tests/parity_round16.rs   3  tests/common/mod.rs
  36  tests/fuzz_round15.rs     34  tests/probe_round29b.rs   2  tests/builtin_commands.rs
  31  tests/audit6_main.rs      34  tests/probe_round34.rs    2  tests/scratch_b.rs
  31  tests/fuzz_round12.rs     31  tests/fuzz_round35.rs
```
</details>

> **A5 组内的 CJK 测试数据（L4）一个字都不能动**。`tests/` 里凡是
> `buf("中文")`、`"中\n"`、`char_at`、`split_at_col` 这类字面量，只翻注释和
> 断言消息，字面量保持原样。用 §1.1 的自查模式逐组核对。

### 4.1.1 其余三个仓库的范围（2026-10-14 核查）

| 仓库 | 注释（可译） | CJK 数据（禁改） | 断言/错误消息 | 文档 | 提交历史 |
|---|---|---|---|---|---|
| alacer | 156 | 13 | 13 | 1 | **0 / 35** |
| gpui_vim | 206 | 11 | 24 | 1,156 | 33 / 84 |
| crossterm_vim | 497 | 13 | 58 | 460 | 8 / 39 |

**alacer 的四个要点**：

1. **历史全英文，Phase B 整个跳过**——不用跑 filter-branch，不用 force push。
2. **没有用户可见的中文 UI 文本**（0 处）。alacer 是 TUI 客户端，但界面
   本来就是英文的，所以不存在「翻译会改变产品行为」的问题。
3. **CJK 数据集中在 `message_editor` 的 IME 输入法测试**——比 vimcore 的更
   业务化，一个字都不能动：
   ```rust
   ime.commit("你");                 // message_editor.rs:1630
   let input = "ab\n你cd";           // message_editor.rs:1608
   assert_eq!("hi你好".len(), ..);   // message_editor.rs:1656
   split_at_col("你好", 1)           // message_editor.rs:926
   ```
4. **一条断言消息要保留原样**：`message_editor/vim.rs:570` 的
   `"l steps one whole character over 在, not one byte"`——它本身是英文，
   只是引用了中文字形。翻译它反而破坏语义。

`vim_ro.rs` 的 106 行是 `//!` 模块文档，描述 vimcore 只读管线接入 diff view
的设计（`PLAN_READONLY §9.2`、三个适配器 + 一个入口），术语密集，需精译。

### 4.2 每批的验证

```bash
cargo test                    # 必须全绿
cargo clippy --all-targets    # 无新增警告
```

### 4.3 翻译纪律

- **保留 vim 术语原文**：operator、motion、text object、register、`'cpoptions'`、
  `'selection'`、exclusive-linewise、blackhole 等——这些是规格词汇，不是普通英文
- **保留代码标识符原文**：函数名、类型名、选项名一律不动
- **保留来源标注**：`来源: vim src/testdir/test_xxx.vim` 这类引用格式不动
- **保留 `AGENTS.md` 的 clean-room 纪律说明语义**（该文件本身是工作约定，改完要能读）
- 文档（L2）逐篇精译，不用批量正则——`NOTES.md` 记的是 vim 语义审计结论和
  oracle 实证记录，机翻失真会误导后续开发

### 4.4 Phase A 验收

```bash
# 翻译前基线（归档后实测）：vimcore 7,814 行含中文
rg -c '[\x{4e00}-\x{9fff}]' --glob '!target' | wc -l
# 翻译后预期：剩余 ~350 行，且全部是 L4 CJK 测试数据
```

**若降到 0，说明误删了测试数据，必须回溯。**

> 注意：这 ~350 行是 L4 测试数据，最后仍应保留中文；**NOTES.md 的归档取回
> 说明（6 行）也含中文，属预期残留**，验收时别误判为漏翻。

### 4.5 每批的操作流程（照此循环）

对批次里的每个文件：

**1. 读全文**，不要只跳到含中文的行。注释常与代码交错（尤其是
`src/*.rs` 里解释 vim 特例的块注释），断章取义会译错语义。

**2. 只改中文部分**——注释、doc 注释、断言消息翻成英文；L4 字面量、
代码标识符、vim 术语按 §4.3 保持原样。

**3. 写回后立刻验证**：

```bash
cargo test                      # 必须全绿
cargo clippy --all-targets      # 无新增警告
```

**4. 一个文件一次提交**（A5 的小组可以一组一次）：

```bash
git add <file>
git commit -m "docs: translate <file> comments to English"
```

提交信息**用英文**——这些提交后面会被 Phase B 一起重写，但当下用英文
能减少 Phase B 的待翻译量，也让 `git log` 现在就是干净的。

**5. 自查 L4 没被动**（每批必做）：

```bash
git diff HEAD~1 | rg '^-.*[\x{4e00}-\x{9fff}]'
```

看被删掉的中文行——**若出现 `buf("中文")` / `"中\n"` / `char_at` /
`split_at_col` / `ime.commit("你")` 这类行，说明误删了测试数据，立刻还原**。

**循环顺序**：`A1 → A2 → A3 → A4 → A5.1 → A5.2 → A5.3 → A5.4 → A5.5`
（文件清单见 §4.1.0）。每批之间跑一次全量 `cargo test`。

---

## 5. Phase B：提交历史重写

### 5.0 执行顺序（重要）

**先 Phase A，后 Phase B。** 工作树翻译产生的英文提交会一并被这次重写覆盖，
只需重写一次。反过来做要 force push 两次。

### 5.1 备份 + 采集验证基线

```bash
cd /Users/lex/code/github/vimcore

# 1) bundle 备份（推送前的回滚点）
git bundle create ~/vimcore-pre-i18n.bundle --all
git bundle verify ~/vimcore-pre-i18n.bundle

# 2) 采集验证基线 —— §5.5 的步骤 1 依赖它，漏了就没法验证内容零改动
git log main --format='%T' | sort > /tmp/before_trees.txt
git ls-tree -r HEAD --format='%(objectname) %(path)' | sort > /tmp/before_files.txt
git rev-list --count main > /tmp/before_count.txt
git log -5 --format='%an | %ad' --date=iso > /tmp/before_meta.txt
```

仓库很小（`.git` 共 68M，vimcore 仅 17M），无空间压力。

> ⚠️ **第 2 步不能省。** §5.5 的「内容零改动」验证要拿 `/tmp/before_trees.txt`
> 比对，没有基线就只能事后靠 bundle 反推，验证失去意义。

### 5.2 导出待翻译的提交消息

> ⚠️ **必须导出完整 message（`%B`），不是标题（`%s`）。**
> `--msg-filter` 替换的是**整个 commit message**。若只映射标题，
> 143 条正文的中文会被整体删除——这不是「没翻译」，是**数据丢失**，
> 且推送后不可逆。

实测（2026-10-14）：vimcore 236 条提交中——

| | 条数 |
|---|---|
| 标题含中文 | **217** |
| **正文含中文** | **143** |
| 含中文（标题或正文） | 约 250 |

```bash
# 导出：用 \x02 作记录分隔符（message 里可能有换行）
git log --branches --tags --reverse --format='%H%x09%B%x02' > /tmp/vimcore-msgs.txt
```

逐条翻译，产出 `/tmp/msg_map.json`：

```json
{
  "<40位SHA>": "English subject\n\nEnglish body (if any)",
  "...": "..."
}
```

规则：

- **值是完整的英文 message**（标题 + 空行 + 正文），不是只有标题；
  无正文的就只写标题一行。
- 只翻译含中文的条目；其余**不要**进映射表（未命中即原样输出）。
- 正文里的 vim 命令序列、文件路径、代码标识符保持原样。
- 分批翻译，别一次吞 250 条——每批翻完抽样核对 3–5 条再继续。

### 5.3 执行重写

`/tmp/msg_filter.py`（读 `$GIT_COMMIT` 查表；**未命中则原样输出整个 message**）：

```python
import sys, os, json

m = json.load(open('/tmp/msg_map.json'))
sha = os.environ.get('GIT_COMMIT', '')
orig = sys.stdin.buffer.read()
new = m.get(sha)
if new is not None:
    sys.stdout.buffer.write(new.encode('utf-8') + b'\n')
else:
    sys.stdout.buffer.write(orig)   # 未命中：原样，连正文一起保留
```

执行（**Python 用受管绝对路径**，别用裸 `python3`——系统版可能缺依赖或版本不符）：

```bash
PY=/Users/lex/.workbuddy/binaries/python/versions/3.13.12/bin/python3

FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f \
  --msg-filter "$PY /tmp/msg_filter.py" \
  --tag-name-filter cat \
  -- --branches --tags
```

236 个提交耗时约 **60 秒**（演练实测 231 条约 60s）。

> **执行前先小样本验证**：先只把 5 条放进 `msg_map.json` 跑一遍，确认
> ① 这 5 条被翻译 ② 其余原样 ③ 正文没丢。全量再上。
> 演练就是这么做的（5 条样本 → 全量 231 条）。

### 5.4 三个必对的参数（演练实证，错一个就出事）

| 参数 | 作用 | 漏掉的后果 |
|---|---|---|
| `-- --branches --tags` | 限定重写范围 | 用 `-- --all` 会把 `refs/remotes/origin/*` 一起重写，旧历史残留在 origin ref 上，形成新旧两条历史分叉 |
| `--tag-name-filter cat` | 让 tag 跟随新提交 | git 报 `WARNING: You said to rewrite tagged commits, but not the corresponding tag`，v0.2.0 仍指向旧提交 |
| `-f` 或 `FILTER_BRANCH_SQUELCH_WARNING=1` | 跳过警告 | 卡在 5 秒倒计时，脚本化时挂死 |

### 5.5 验证（逐条执行，全过才继续）

逐条跑，**全过才继续**。基线文件来自 §5.1 第 2 步：

```bash
# 1. 内容零改动（决定性验证）——每个历史版本的 tree 逐一一致
git log main --format='%T' | sort > /tmp/after_trees.txt
diff /tmp/before_trees.txt /tmp/after_trees.txt && echo "✅ tree 全一致"

# 2. 文件级 blob 比对 —— HEAD 树逐文件一致
git ls-tree -r HEAD --format='%(objectname) %(path)' | sort > /tmp/after_files.txt
diff /tmp/before_files.txt /tmp/after_files.txt && echo "✅ blob 全一致"

# 3. 提交数 / 作者 / 时间戳不变
git rev-list --count main > /tmp/after_count.txt
diff /tmp/before_count.txt /tmp/after_count.txt && echo "✅ 提交数不变"
git log -5 --format='%an | %ad' --date=iso > /tmp/after_meta.txt
diff /tmp/before_meta.txt /tmp/after_meta.txt && echo "✅ 作者与时间戳不变"

# 4. tag 已迁移到新历史
git log -1 --format='%h %s' v0.2.0

# 5. 翻译生效：重写后不应再有中文 message
git log main --format='%B' | rg -c '[\x{4e00}-\x{9fff}]'   # 应为 0

# 6. 正文没丢（§5.2 那个坑的验收）
git log main --format='%b' | rg -c '\S'   # 有正文的提交数，应与重写前相当
```

**验证陷阱 1**：不要用 `git log --all --format='%T'` 做前后比对。
重写后 `--all` 会同时遍历新旧两条历史，tree 数量翻倍，产生「内容被改动」的假警报。
必须限定到具体分支（如 `main`）。

**验证陷阱 2**：第 5 步若为 0 还要看第 6 步。只映射标题会让中文消失（第 5 步也
是 0），但正文同时也被删了——**两个 0 可能一个是成功、一个是数据丢失**，
必须用第 6 步区分。

### 5.6 推送

```bash
git push --force-with-lease origin main
git push --force --tags
```

`--force-with-lease` 比 `--force` 安全：远端若有意料外的新提交会拒绝推送。

**回滚窗口**：在 `git push` 之前，`refs/remotes/origin/main` 仍指向旧历史，
`git reset --hard origin/main` 即可完整还原。推送之后就只能靠 §5.1 的 bundle。

### 5.7 其余三个仓库

| 仓库 | 提交 | 标题含中文 | **正文含中文** | Phase B |
|---|---|---|---|---|
| `vimcore` | 236 | 217 | **143** | ✅ 全量重写 |
| `gpui_vim` | 84 | 33 | **25** | ✅ 重写（`main` + `fork-gpui-pre` 两个分支） |
| `crossterm_vim` | 39 | 8 | **9** | ✅ 重写 |
| `alacer` | 35 | 0 | **1** | ⬇️ 见下，实际跳过 |

**alacer 可以跳过 Phase B**，但理由要说准：它不是「零中文」，而是那 1 条
（`c5ca0e4`）正文里的中文是**引用 panic 消息的字形**：

```
"start byte index 2 is not a char boundary; it is inside '在'"
```

翻译它反而破坏语义，与 §4.1.1 要点 4（alacer 断言消息保留原样）同理。
**这 1 条刻意保留，其余 34 条本来就是英文。**

> 各仓库执行时按 §5.1–5.6 同一流程走，改路径与分支即可。
> gpui_vim 已清理到 **2 个分支**（`main`、`fork-gpui-pre`），`--branches`
> 会覆盖两者；`fork-gpui-pre` 是 gpui-pre 产品线，**必须一起重写**，
> 否则它保留旧历史形成分叉。

---

## 6. 风险与已知限制

| 风险 | 说明 | 缓解 |
|---|---|---|
| L4 测试数据被误译 | 测试仍绿但覆盖率失效 | §1.1 自查模式 + §4.4 计数验收 |
| crates.io 已发布版本改不了 | 实测 `vimcore-0.2.1.crate` 含 **8,658 行中文 / 93 文件**（含 4 个已归档的 BUG_AUDIT） | **历史重写不解决此项**。要覆盖只能发新版本，见 **§8** |
| gpui_vim 多分支分叉 | 已清理到 **2 个**（`main`、`fork-gpui-pre`）；后者是 gpui-pre 产品线，漏了就保留旧历史 | §5.3 用 `--branches` 全覆盖；§5.7 已列清单 |
| force push 不可逆 | 推送后无本地回滚点 | §5.1 bundle 备份 + 推送前 `git log` 复核 |
| 文档机翻失真 | `NOTES.md` 是 vim 语义审计笔记 | §4.3 强制逐篇精译 |
| ~~归档删除破坏引用~~ | ~~`BUG_AUDIT*.md` 被 6 处引用~~ | ✅ **已处置**（§4.0.1）：27 处引用全部改写并验证，测试 1162 全绿；git 历史 + `.workbuddy/archive/` 双备份可回溯 |
| 引用数估算失准 | 初估 6 处，实扫 27 处（差 4 倍） | **规矩：任何「改引用」类操作，先 `rg -c` 实扫出准确清单再动手** |
| **Phase B 正文丢失** | `--msg-filter` 替换整个 message。只映射标题 = 删掉 143+25+9 条正文，**推送后不可逆** | §5.2 强制导出 `%B`（完整 message）；§5.5 第 6 步专门验收「正文没丢」 |
| 重复提交造成分叉 | 本地脏工作树 ≠ 新工作，可能是已在别处提交过的同一改动 | **规矩：提交前先 `git fetch`，确认 origin 上没有同一改动的另一个提交**。见 §8.5 ③ 的实例 |

### 关于「外界还没使用」

这句话需要打个折。Cargo.toml 注释显示 gpui-vim-pre 0.1.1 已被 tk 的 Bench 消费、
gpui-vim-x 0.2.0 服务 pandagit。这些是公开包，且 crates.io 不可变。
**历史重写让本地仓库干净，但不改变已发布产物里的中文。**
评估收益时这一项要扣除。

---

## 7. 命令速查

```bash
# 扫描含中文的文件
rg -l '[\x{4e00}-\x{9fff}]' --glob '!target'

# 统计含中文行数
rg -c '[\x{4e00}-\x{9fff}]' --glob '!target' | awk -F: '{s+=$2} END {print s}'

# 统计含中文的提交
git log --all --format='%H%x09%s' | rg '[\x{4e00}-\x{9fff}]' | wc -l

# 备份
git bundle create ~/vimcore-pre-i18n.bundle --all

# 回滚（推送前）
git reset --hard origin/main

# 清理重写残留
git for-each-ref --format='%(refname)' refs/original/ | xargs -n 1 git update-ref -d
git reflog expire --expire=now --all && git gc --prune=now --aggressive

# --- 分支清理（gpui_vim 已用）---
# 权威判定：相对 main 而非 HEAD 检查是否已合并
git branch --merged main
git branch --no-merged main
# 解开 worktree 占用后再删（一个分支只能被一个 worktree checkout）
git -C <worktree> switch --detach
git branch -d <branch>          # -d 会校验，基准是当前 HEAD
git push origin --delete <branch>   # 远端删除，慎用

# --- 发布相关 ---
cargo package --list            # 看哪些文件会被打进包（归档后验证）
cargo publish --dry-run         # 发布前必跑
curl -s https://crates.io/api/v1/crates/<crate> -H 'User-Agent: x' | jq '.crate.max_stable_version'
```

---

## 8. 发布事项（2026-10-14 补充，原计划缺此节）

原计划只在 §6 风险表提了一句「crates.io 已发布版本改不了」，**没有发布流程**。
这是缺口——迁移的价值有一半取决于发布出去的新版本是否干净。

### 8.1 当前发布状态

| crate | 本地版本 | crates.io 最新 | 状态 |
|---|---|---|---|
| `vimcore` | 0.2.1 | **0.2.1**（10-12） | 版本一致；包内含 884 行中文审计档案 |
| `crossterm-vim` | **0.1.2** | **0.1.2**（10-09） | ✅ 已同步；修复**已随 0.1.2 发布**（见 §8.5 ③） |
| `gpui-vim-pre` | 0.1.1 | 0.1.1 | 一致（tk 的 Bench 在消费） |
| `gpui-vim-x` | — | 0.2.0 | 服务 pandagit；本次未核本地 |
| `alacer` | 0.1.0 | **未发布**（0 版） | 无需发布 |

### 8.2 已发布包里到底有多少中文（实测）

下载 `vimcore-0.2.1.crate` 解包统计：

```
合计 8,658 行中文 / 93 文件
   1,905  NOTES.md
     409  BUG_AUDIT2.md   ← 已归档，但 0.2.1 包里还在
     240  PLAN_READONLY.md
     199  BUG_AUDIT3.md   ← 同上
     165  BUG_AUDIT4.md   ← 同上
     ...  tests/ 86 个文件
```

**这是「历史重写不解决已发布包」这条风险的量化版本**：crates.io 不可变，
0.2.1 会永远带着这 8,658 行中文。要让公开产物变干净，**唯一办法是发新版本**。

### 8.3 要不要发布？分三档

| crate | 结论 | 理由 |
|---|---|---|
| **`vimcore`** | **建议发 0.2.2**，但**等 Phase A 之后** | 删文档不构成 semver 强制发布；但 0.2.1 包里带 884 行中文档案，只有新版能覆盖 |
| **`crossterm-vim`** | **不需要发**（结论已更正） | 唯一的实质修复已随 **0.1.2（10-09）** 发布，本地已同步到 `cb86295` |
| `alacer` | 不需要 | 从未发布 |
| `gpui-vim-x` | 待核 | 需先确认 main 分支相对 0.2.0 有无新改动 |

### 8.4 顺序：翻译完再发布（关键）

**发布必须排在 Phase A 之后。** 现在发 0.2.2，包里依然是全中文；
翻译完再发，公开产物才真正变干净。

```
Phase A 翻译 → 推送 GitHub → Phase B 历史重写 → force push
                                              → 发布新版本（此时源码已英文化）
```

Phase B 重写的是 message 不是 tree，不影响发布；但**新版本要在重写后的
提交上发**，否则 git 历史与 crate 内容对不上。

### 8.5 发布前必须处理的三件事

✅ **1. `I18N_PLAN.md` 不再进包**（已办，`b15567f`）

`Cargo.toml` 加了 `exclude = ["I18N_PLAN.md"]`。用真实 `cargo package`
验证过（不只是 `--list`）：打包文件 108 → **107**，产物里已无该文件。

> 顺带一个可量化的进度指标——**本地新打的包含中文 7,823 行 / 89 文件**
> （已发布 0.2.1 是 8,658 行 / 93 文件）。差额主要是归档掉的 4 个
> BUG_AUDIT。Phase A 翻译完，这个数会再降到 ~350。

✅ **2. `CHANGELOG.md` 补上 0.2.1 条目**（已办，`b15567f`）

0.2.1 实际只发了一个东西：`VimState::refresh_highlights_ro`（`aff0f60`），
已按 Added / Changed 两节补上，位置在 0.2.0 之上（新版在上）。

✅ **3. `crossterm-vim` 版本漂移——查清了，是重复提交事故**（已处置）

原判断「本地 0.1.1 落后已发布的 0.1.2，修复未发布」**是错的**。真相：

| | `cb86295`（origin/main，10-09） | `c51ccd4`（本地，10-14，已回退） |
|---|---|---|
| 代码改动 | `starts[line] + col - c.len_utf8()` | **一字不差，完全相同** |
| 测试 | 新增独立测试，全偏移扫描 + 钉住 11→8 | 已有测试里加 2 行断言（被前者覆盖） |
| 版本 | 0.1.1 → **0.1.2** | 无 |
| 提交信息 | 中文，写明「发 0.1.2 前的红测试根因」 | 英文（迁移后惯例） |

**修复早在 10-09 就随 0.1.2 发布了。** 昨天看到 crossterm_vim 的「脏工作树」
是 10-09 提交后**残留的同一份改动**（提交信息里提到的红测试根因），被误当成
新发现提交了，造成 `ahead 1 / behind 1` 分叉。

**处置**：`backup/dup-prev-char-offset-fix` 留 ref → `git reset --hard
origin/main` → 分叉消除，版本回到 0.1.2，`cargo test` 97 全绿，
四仓库全部与远端同步（2026-10-14 推送复核）。

> 📌 **Phase B 注意**：crossterm_vim 现在多一个本地分支
> `backup/dup-prev-char-offset-fix`（指向已回退的 `c51ccd4`，仅本地、未推送）。
> §5.7 的 `--branches` 会把它一起重写（内容不变，SHA 变），无实质影响；
> 若想让重写历史更干净，可在 Phase B 前 `git branch -D` 掉它——
> `c51ccd4` 的代码改动已确认与 `cb86295` 完全一致，无独立价值。

> ⚠️ **教训（写进流程）**：提交前必须 `git fetch` 并确认 origin 上**没有
> 同一改动的另一个提交**。本地脏工作树 ≠ 未提交的新工作——它可能是已经
> 在别处提交并推送过的同一份改动。判断依据是 diff 内容，不是「工作树脏不脏」。

### 8.6 已发布 vs 本地构建（对照）

| | 含中文行数 | 文件数 | 说明 |
|---|---|---|---|
| crates.io **0.2.1**（已发布） | **8,658** | 93 | 不可变，永久保留 4 个 BUG_AUDIT |
| 本地新构建（未发布） | **7,823** | 89 | 扣掉 BUG_AUDIT + exclude I18N_PLAN |
| Phase A 完成后（预期） | **~350** | — | 只剩 L4 CJK 测试数据 + NOTES 归档说明 |
