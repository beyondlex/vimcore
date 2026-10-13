# 中文 → 英文 迁移计划

覆盖 4 个仓库：`vimcore`（底座）、`alacer`、`gpui_vim`、`crossterm_vim`。
本文档是执行手册，不是提案——所有命令已在 vimcore 的完整副本上演练验证过。

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

| 仓库 | 提交总数 | 含中文 | 分支 | tag |
|---|---|---|---|---|
| vimcore | 231 | 217 | main | v0.2.0 |
| gpui_vim | 84 | 33 | **2 个**（已清理） | 1 个 |
| crossterm_vim | 39 | 8 | main | 0 |
| alacer | 34 | **0**（已全英文） | main | 0 |

**关键前提**：三个下游仓库全部通过 crates.io 依赖 vimcore
（`vimcore = "0.2"` / `"0.1"`），无 git 依赖、无 `path = "../vimcore"`。
因此 vimcore 历史重写导致 SHA 全变，**不会破坏任何下游构建**。

---

## 3. 前置检查（阻塞项，必须先清）

| 仓库 | 状态 | 处理 |
|---|---|---|
| alacer | 7 个未推送提交 + 脏工作树 | **待处理**：先提交或 stash |
| crossterm_vim | ✅ 已清理 | 修复已提交 `c51ccd4`（96 测试全绿） |
| gpui_vim | ✅ 已清理 | 分支 5 → 2，见 §3.1 |
| vimcore | 干净 | OK（`I18N_PLAN.md` 未跟踪，不影响） |

> 历史重写**不会**自动处理未提交的工作树改动。带着脏工作树跑 filter-branch
> 会让改动悬空。

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

---

## 5. Phase B：提交历史重写

### 5.0 执行顺序（重要）

**先 Phase A，后 Phase B。** 工作树翻译产生的英文提交会一并被这次重写覆盖，
只需重写一次。反过来做要 force push 两次。

### 5.1 备份

```bash
cd /Users/lex/code/github/vimcore
git bundle create ~/vimcore-pre-i18n.bundle --all
```

仓库很小（`.git` 共 68M，vimcore 仅 17M），无空间压力。
验证备份可用：`git bundle verify ~/vimcore-pre-i18n.bundle`

### 5.2 导出待翻译的提交消息

```bash
git log --all --reverse --format='%H%x09%s' > /tmp/vimcore-msgs.tsv
```

逐条翻译产出 `SHA → 英文 message` 映射，存为 `/tmp/msg_map.json`：
`{"<40位SHA>": "英文标题", ...}`

> 只翻译含中文的 218 条；其余原样返回。含 body 的提交另行处理。

### 5.3 执行重写

`msg_filter.py`（读 `$GIT_COMMIT` 查表，未命中原样输出）：

```python
import sys, os, json
m = json.load(open('/tmp/msg_map.json'))
sha = os.environ.get('GIT_COMMIT', '')
orig = sys.stdin.buffer.read()
new = m.get(sha)
sys.stdout.buffer.write(new.encode('utf-8') + b'\n' if new is not None else orig)
```

```bash
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f \
  --msg-filter 'python3 /tmp/msg_filter.py' \
  --tag-name-filter cat \
  -- --branches --tags
```

231 个提交耗时约 **60 秒**。

### 5.4 三个必对的参数（演练实证，错一个就出事）

| 参数 | 作用 | 漏掉的后果 |
|---|---|---|
| `-- --branches --tags` | 限定重写范围 | 用 `-- --all` 会把 `refs/remotes/origin/*` 一起重写，旧历史残留在 origin ref 上，形成新旧两条历史分叉 |
| `--tag-name-filter cat` | 让 tag 跟随新提交 | git 报 `WARNING: You said to rewrite tagged commits, but not the corresponding tag`，v0.2.0 仍指向旧提交 |
| `-f` 或 `FILTER_BRANCH_SQUELCH_WARNING=1` | 跳过警告 | 卡在 5 秒倒计时，脚本化时挂死 |

### 5.5 验证（逐条执行，全过才继续）

```bash
# 1. 内容零改动（决定性验证）
git log main --format='%T' | sort > /tmp/after.txt
# 与 §5.1 备份前采集的基线比对，必须逐一一致

# 2. 文件级 blob 比对
git ls-tree -r HEAD --format='%(objectname) %(path)' | sort

# 3. 提交数 / 作者 / 时间戳不变
git rev-list --count main
git log -5 --format='%an | %ad' --date=iso

# 4. tag 已迁移
git log -1 --format='%h %s' v0.2.0
```

**验证陷阱**：不要用 `git log --all --format='%T'` 做前后比对。
重写后 `--all` 会同时遍历新旧两条历史，tree 数量翻倍，产生「内容被改动」的假警报。
必须限定到具体分支（如 `main`）。

### 5.6 推送

```bash
git push --force-with-lease origin main
git push --force --tags
```

`--force-with-lease` 比 `--force` 安全：远端若有意料外的新提交会拒绝推送。

**回滚窗口**：在 `git push` 之前，`refs/remotes/origin/main` 仍指向旧历史，
`git reset --hard origin/main` 即可完整还原。推送之后就只能靠 §5.1 的 bundle。

### 5.7 其余三个仓库

同一流程。alacer 历史已全英文（0/32），**跳过 Phase B**，只做 Phase A 的 188 行。

---

## 6. 风险与已知限制

| 风险 | 说明 | 缓解 |
|---|---|---|
| L4 测试数据被误译 | 测试仍绿但覆盖率失效 | §1.1 自查模式 + §4.4 计数验收 |
| crates.io 已发布版本改不了 | vimcore 0.1.0/0.2.0、gpui-vim-pre 0.1.1、gpui-vim-x 0.2.0 已上架，包内中文无法撤回 | **历史重写不解决此项**。要覆盖只能发新版本 |
| gpui_vim 多分支分叉 | 5 个分支，漏一个就保留旧历史 | §3 先确认范围，§5.3 用 `--branches` 全覆盖 |
| force push 不可逆 | 推送后无本地回滚点 | §5.1 bundle 备份 + 推送前 `git log` 复核 |
| 文档机翻失真 | `NOTES.md` 是 vim 语义审计笔记 | §4.3 强制逐篇精译 |
| ~~归档删除破坏引用~~ | ~~`BUG_AUDIT*.md` 被 6 处引用~~ | ✅ **已处置**（§4.0.1）：27 处引用全部改写并验证，测试 1162 全绿；git 历史 + `.workbuddy/archive/` 双备份可回溯 |
| 引用数估算失准 | 初估 6 处，实扫 27 处（差 4 倍） | **规矩：任何「改引用」类操作，先 `rg -c` 实扫出准确清单再动手** |

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
```
