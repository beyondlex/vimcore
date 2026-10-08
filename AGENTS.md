# vimcore 开发参考策略（对齐 vim 9.1 语义的工作方式）

本引擎的规格是 vim 本身。**oracle 层级**：`vim -Nu NONE -N` 的实际行为
是最终裁判；vim 源码是"规则解释器"，用来把单点实证升级为普遍理解；
本文件告诉你去哪读、怎么读、什么不能做。

## 本地 vim 源码（只读参考）

- 路径：`~/code/github/vim`（GitHub vim/vim 的克隆）。
- 工作树签出在 **v9.1.1752**，与 `/usr/bin/vim`（9.1 patches 1-1752）
  一致。要看更新的演进 `git checkout master`（9.2+），但语义判定以
  9.1 侧为准。
- **Clean-room 纪律**：读源码理解规则，用引擎自己的表达实现。禁止
  逐行翻译——vim 是 Vim License/Charityware，本 crate 是 MIT/Apache
  双许可，翻译会构成衍生作品污染许可。测试移植同理：搬**行为断言**，
  标注 `来源: vim src/testdir/test_xxx.vim`，不搬代码。

## 源码地图（域 → 先读的函数）

| 域 | 位置 |
|---|---|
| Ex 地址/范围（`:5 + 2` 记号化、裸地址钳制） | `src/ex_docmd.c` `get_address()` |
| 搜索执行与接受规则（跳过/停在光标匹配） | `src/search.c` `do_search()` / `searchit()` |
| 文本对象（aw/iw 的跨行空白形状） | `src/search.c` `current_search()` |
| 算子与 exclusive-linewise（d2w vs 2cw 分野） | `src/ops.c` `do_pending_operator()`（著名的调整注释块） |
| 连接/大小写/缩进 | `src/ops.c` `do_join()` / `do_tilde` 等 |
| 粘贴与寄存器（linewise/blockwise、`"A` 追加的 unnamed 非对称） | `src/register.c` |
| 插入模式控制键（C-t/d/y/e/w/u、C-r、0<C-d>） | `src/edit.c` `insert()` 主 switch（`case Ctrl_W` 等直接分发；退格语义在 `ins_bs(c, BACKSPACE_WORD/LINE)` 的模式参数里） |
| `:s` 替换侧（`\u`/`&`/`~` 特殊符） | `src/ex_cmds.c` `do_sub()` 及 regsub 逻辑 |
| `:set` | `src/option.c` / `optionstr.c` |
| marks/changelist/jumplist | `src/mark.c` |
| 命令行编辑与历史（`<Up>` 前缀匹配） | `src/cmdhist.c` |
| 宏录制/重放（`q"`、`@`、可视 `@`） | `src/register.c` `do_execreg()` |

读法：先 grep 函数名，再顺着调用读判定条件；注释里常直接写着特例
（`do_pending_operator` 的 exclusive-linewise 注释是必读）。注意源码
逻辑大量条件化在 `'cpoptions'`/`'selection'`/`'virtualedit'` 上——
引擎只实现选项子集且有意分歧了默认值（ignorecase、ts=4 等），照抄
分支前先确认该选项在引擎里的立场（见 NOTES.md 已知分歧账目）。

## 移植 testdir 行为测试（最高性价比的审计素材）

- 位置：`~/code/github/vim/src/testdir/test_*.vim`（318 个文件）。
  相关度最高的：`test_textobjects.vim`、`test_registers.vim`、
  `test_search.vim`、`test_put.vim`、`test_visual.vim`、
  `test_blockedit.vim`、`test_normal.vim`、`test_cmdline.vim`、
  `test_marks.vim`、`test_join.vim`、`test_indent.vim`。
- 用法：按当前审计的域挑文件，把其中与引擎子集相关的断言转成
  `tests/` 下的探针（跑不通的多数是引擎未实现的选项/特性，记入
  分歧账目而非硬造）。凡由此发现的行为写入回归测试时标注来源文件。
- 这是对抗性黑盒试错的系统性替代：本轮（第三十二轮）60 个 bug 里
  估计过半能被 `test_textobjects` + `test_registers` + `test_put`
  提前逮住。

## oracle 纪律（修复的验收标准）

- 语义类修复必须有 `/usr/bin/vim`（9.1）实证。通道选择：
  `-Nu NONE -N -i NONE -s <keys>` typeahead 是默认通道；**方向键、
  可视下的 `:`、响铃类必须走 expect PTY**（NOTES 多轮教训：`-es`
  feedkeys 对宏/方向键不可靠）。
- 已知的采样坑（踩过的都在 NOTES 有案）：`writefile()` 把换行写成
  NUL（采样寄存器要用 `getreg` + 替换显示，或 `:w!` 后读文件）；
  同名探针文件残留 swap 会吞按键导致整批结果作废（每例换唯一文件
  名）；`writefile` 的 List 参数嵌套 List 会 E730。
- 造「空寄存器」只能用 `"_x`（blackhole）：`x` 会填寄存器，随后的
  `p` 是真粘贴（round-32 修过的测试 setup 教训）。
- 字节级断言（缓冲文本）以 vim `:w` 写回后的 od 为准——引擎的 noeol
  状态在 vim 侧会被 `'fixendofline'` 补终结符。
- 引擎有意分歧的默认值（ignorecase=on、ts=4、tw=78 等，见 NOTES
  分歧 #2/#3）在探针里要先用 `:set` 钉成 oracle 前提再断言。

## 修复流程约定

1. 先跑目标域探针确认失败形态，再查本文件源码地图读规则，必要时
   oracle 实证，最后改实现。
2. 每个修复落一个探针/回归用例（audit2_* 与既有 parity/review
   套件），禁止"顺手删测试"。
3. 新发现的 vim 有意特例（如 `"Add`/`"Ayy` 的 unnamed 非对称）若
   与直觉冲突，照 vim 实现，并在 NOTES.md 分歧账目或证伪清单记录。
4. 修 A 坏 B 时优先怀疑两边共享的语义模型错了，而不是给旧钉子
   打补丁；测试与 oracle 冲突时以 oracle 为准并更正测试（附实证
   说明），NOTES 的旧结论同样允许被复核推翻（先复测再改）。
