# vimcore 独立审计（第四轮）：新发现 Bug 清单（2026-10-09）

> **状态：63 项修复（A19 / B18 / C18 / 自有赛道 8），全量 `cargo test`
> 1034 例 0 失败（另 2 ignored：1 例基线原有，1 例本轮挂账）。**
> 回归探针：`tests/audit5_{a,b,c}.rs`（oracle 证实的红测试）与
> `tests/probe_round35.rs` + `tests/fuzz_round35.rs`（自有赛道）。
>
> **审计方式**：3 个并行分域智能体（A=Ex/搜索/替换，B=motion/对象/算子，
> C=寄存器/可视/undo/插入）+ 主会话自有 fuzz 赛道。本轮首次系统移植
> vim testdir 行为断言（`~/code/github/vim/src/testdir/`），每条候选
> 以本机 vim 9.1（typeahead 通道为主，铃声走 expect PTY）实证后定案，
> 并对照 NOTES.md 分歧账目与 BUG_AUDIT 1/2/3 全部 164 项去重。三个智能
> 体共提交 96 个候选形状，其中 12 个被复验证伪剔除（含本会话 3 个旧钉
> 翻正，见「旧钉更正」）。
>
> **严重度**：P0 崩溃 2（都在自有赛道）/ P1 行为错 11 / 其余 P2/P3。

---

## A. Ex 命令 / 命令行 / 搜索 / `:s` / `:set`（19 项，全修复）

来源：智能体 A。探针 `tests/audit5_a.rs`（19 例全绿）。每项 oracle 证据
见探针注释（o/q/u/p 系列采样）。

- [x] **A-1（P1）`2/foo/` 数字+搜索地址连写缺失** — vim get_address 把
  紧随数字的搜索地址并入同一地址链且**从前一地址起搜**；引擎整体 E16。
  范围解析器重写为多段折叠（数字段 × 搜索段 × mark 段），搜索锚定改为
  「前一地址」。
- [x] **A-2（P2）`|` 命令分隔符不识别，第一条命令也不执行** —
  `:1,2d|j` 整行 E488。新增 `split_command_bar`（`:s` 族按分隔符计数，
  三段关闭后 `|` 才分离），execute_ex 头部分发递归执行。
- [x] **A-3（P2）`:%s/\n//` 连接行惯用法失效** — vim 特化成 join
  （ex_cmds.c）；引擎逐行替换永不命中 → E486。加 `\n` 全模式特判
  （拼行 + "N fewer lines" 报文）。
- [x] **A-4（P2）`\/`/`\?`/`\&` 地址形态缺失（E492）** — 扫描字符表收
  `\`，base_line 折叠 `\X` 段（复用最近 pattern，`\?` 反向）。
- [x] **A-5（P2）`:s/foo/bar/0` 照常替换** — vim E939 拒绝。count=0
  分支报 E939 不执行。
- [x] **A-6（P2）`:s` 的 `r` 旗标 E488** — 接受 `r`（RE_SEARCH；引擎单
  pattern 槽与语义一致）。
- [x] **A-7（P2）`:dl`/`:dp`/`:dell` 族 E492** — 新增 delete_print_form
  识别（delete 前缀 + l/p/# 旗标），删除并 list 式打印。
- [x] **A-8（P2）`:sort n` 无数字行按 0 混入数字段** — 改两段式组合键
  （有无数字先行）；无数字行整段前置。
- [x] **A-9（P2）`:s a b` 空格分隔符真实替换** — vim E146 拒绝。分隔符
  守卫（空白 → E146）。
- [x] **A-10（P3）`:sort u` 缺 "N fewer lines"** — 去重后报删行数。
- [x] **A-11（P3）`:s/…/…/gg` 不翻转 g** — 按 g 出现次数取奇偶。
- [x] **A-12（P3）`:s` count 超 2147483646 不拒绝** — E1510（阈值
  ≥ i32::MAX，oracle 定谳）。
- [x] **A-13（P3）`:set ts=0` 静默接受** — E487（Argument must be
  positive）。`set_value` 改返回 Result 并分发具体 E 消息。
- [x] **A-14（P3）`:set ts=2000000` 静默钳制** — E474（Invalid
  argument），不再悄悄改值。
- [x] **A-15（P3）`:s\foo\bar\` 不报 E10** — 反斜杠分隔符守卫。
- [x] **A-16（P3）命令行 `<C-r>=` 不开表达式提示符** — 新增
  expr_pending/expr_buffer 子提示符（求值贴回；失败 E121/E15 且命令行
  原样）。探针第二断言按 oracle 修正（E121 本就引用变量名）。
- [x] **A-17（P3）命令行 `<C-w>` 删整个非空白段** — c_CTRL-W 只删字母
  数字词（`set ts=4<C-w>` → `set ts=` → E521）。
- [x] **A-18（P3）超大 `:y` count 报 E488** — parse_reg_count 溢出饱和
  为 usize::MAX 交给钳制（vim 静默截到缓冲末）。
- [x] **A-19（P3）裸 `:'` 报 E16** — vim 静默（无报文、光标不动）。折叠
  解析器对长度 1 的 `'` 落回光标行。

## B. Normal motion / 算子 / 文本对象（19 修复 + 1 挂账）

来源：智能体 B。探针 `tests/audit5_b.rs`（19 例全绿 + B-17 ignore）。

- [x] **B-2（P3）缓冲起点 `b`/`B` 静默** — WordBack 补 stuck 分支
  （WordEndBack 孪生；PTY 实测 1 BEL）。
- [x] **B-3（P1）`y2b` 跨空行多吞一行** — 算子 exclusive 规则 (a) 新增
  向上分支：span 终点（光标侧）在列 1 → 终点移到上一行行尾（空邻行只
  贡献自己的起点），换行存活。
- [x] **B-4~B-7（P1/P2）块对象不前向搜索** — `0di)` 一类全灭。补 vim
  「光标不在块内则向前找第 count 个未配对开括号」，count 前向消费；
  forward-found 时 object_span_count 不再二次爬层。
- [x] **B-8（P2）`a'` 空白取舍** — 尾随空白优先、无则取前导（引擎两种
  形状都只取引号串）。
- [x] **B-9（P2）算子待决 `v`/`V` 未绑定** — 新增 motion_force
  （`:h o_v`），`dvgo` 一类恢复。
- [x] **B-10（P2）`2yis`/`3yis` 句子计数** — 边界按「词尾 → 空隙尾」
  交替推进（oracle 矩阵定谳）；sentence_range 重写支持 count。
- [x] **B-11（P2）`das` 于句间空白取前一句** — 锚定下一句、范围含前导
  空隙、止于词尾（不带尾随空白）。
- [x] **B-12（P2）`dg_` 吞行尾空白** — vim `:h exclusive` 规则 3（inclusive
  + 起点在首非空白前 → 起点移到首非空白）落入 span_from_motion；纯空白
  行与零宽 span 有守卫。
- [x] **B-13（P3）`3yap` 超段不响铃** — 对象计数超界检测（done 计数，
  桥接换行不计对象、达词才计）→ None → 调用方响铃取消。
- [x] **B-14（P1）标签对象被属性引号内的 `>` 骗走** — 标签收集逐字符扫
  描（引号态机）；inner 端点复用收集到的开标签终点（裸 find 二次截断）。
- [x] **B-15（P2）跨行内层块并线** — inner 端点取次行行首/上行行尾，
  两侧换行存活。
- [x] **B-16（P3）`y3aw` 超词不响铃** — 同 B-13。
- [x] **B-18（P2）`3cc` 超末行照改** — 整体失败（双写算子臂 +
  SubstituteLine 臂都拦），不进插入。
- [x] **B-19（P2）首空行 `dap` 形状** — 全缓冲 linewise 删除经探针口径
  复核为伪 bug（vim [''] 与引擎 "" 同态），探针按内部文本口径转为回归
  钉；顺带确认 delete_span 无需特判。
- [x] **B-20（P1）`db` 自下行行首向上并线** — 同 B-3（规则 (a) 向上
  分支）；MarkJump（`` ` ``/`'`）按 vim end_adjusted 豁免。
- [x] **B-1 / C-19（证伪翻转）** — `}` 于空白行的落点、`dd u gv` 的选区
  恢复，复验均与引擎一致（智能体采样误读），探针改为钉住复验过的
  oracle。
- [ ] **B-17（挂账）testdir `1gg0da<1pjd2at` 全序列** — 爬层本体已验证
  正确（object_span_count(Tag,2) = div 外层）；分歧在 `at` 外层删除域的
  尾随换行/空行归属（vim 连行 4 的 " " 一起删）。探针 `#[ignore]` 保留
  oracle 矩阵，待 PTY 专项。

## C. 寄存器 / 可视 / undo / 插入（18 项，全修复）

来源：智能体 C。探针 `tests/audit5_c.rs`（19 例全绿）。

- [x] **C-1..C-4（P2）use_reg_one 缺失** — `%`、`` ` ``/`'`、`/`/`?`/
  `n`/`N`、`{`/`}` 的删除不写编号环。store_delete 增 use_reg_one 参数
  （环移位 + `"1` 与 `"-` 两笔独立写；命名寄存器形状不写 `"-`），运动层
  置位、delete_span 消费。
- [x] **C-5（P2）粘贴不落 `'[`/`']`** — put_ex 两臂落账（linewise `']`
  取最后粘贴字符的字素起点——裸 line_end-1 落进多字节尾巴，fuzz
  round-117 抓到）。
- [x] **C-6（P2）undo/redo 不落 `'[`/`']`** — history_step 双双指向
  变更首行行首。
- [x] **C-7（P2）插入会话不落 `'[`/`']`** — exit_insert 落账（'[
  = 首插入字符、'] = 插入后一格）；`]` 跳转经复验**无回退**（落在 mark
  本身，oracle [0,1,6,0]），两探针按 oracle 修正。
- [x] **C-8（P2）`gp` 行级光标差一行** — 落粘贴块之后一行
  （PUT_CURSEND lnum+1）；旧钉「落最后粘贴行」是末端钳制巧合，按 oracle
  翻正。
- [x] **C-9/C-10（P2）多行 charwise p/P 光标** — 落首个粘贴字符
  （b_op_start），不再停末字符。
- [x] **C-11/C-12（P1）v/V 的 `I`/`A` 未绑定** — 新增
  VisualCmd::InsertAtSelection（I 在选区起点行插入一次；A 落光标端
  ——行可视光标即末行行首，oracle "-bb——用 Insert kind 防止
  AppendLineEnd 重泊行尾）。
- [x] **C-13/C-14（P2）可视 `P` 交换寄存器** — nv_put_opt 的
  keep_registers：新 PutReplaceKeep/block_put_replace_keep，被替换文本进
  blackhole，粘贴源原样。
- [x] **C-15（P2）undo 后的新改动不作废 redo** — 宿主契约补齐：
  begin_undo_group 文档 + harness 在新组打开时清 redo_stack。
- [x] **C-16（P2）`c` 后 `']` 不随插入文本走** — 同 C-7 的 exit_insert
  落账覆盖（']=插入后一格）。
- [x] **C-17（P2）`o` 开行插入不落这对 mark** — 同 C-7。
- [x] **C-18（P2）`"=<CR>` 空表达式不重用上次** — `=` 提示符记忆
  last_expression，空输入重放求值。

## 自有赛道（fuzz round35 + probe_round35，8 项，全修复）

主会话独立赛道（Ex 随机组装 × 不寻常缓冲，48 seeds × 48 rounds × 90
steps ≈ 20 万步），抓 P0 崩溃与不变量违规：

- [x] **R35-1（P0）`vl<Esc>:marks` 于 CJK 缓冲 panic** — `:marks` 渲染
  `>` 行 `hi.saturating_sub(1)` 裸减字节落进多字节字符内部。mark_line
  防御取整 + harness offset_to_line 全函数化。
- [x] **R35-2（P2）`:marks` 的 col 公式** — vim 是 0 基字节列（TAB 后
  的 'd' 打 3，显示列是 8）；行内容原样列出（不 trim）、TAB/^M 转
  caret 记号。
- [x] **R35-3（P1）`:retab` 后光标陷进多字节字符** — 整段行重写不重映射
  光标。新增 snap/restore（行号 + 相对内容起点的显示列精确还原；范围后
  的行按字节增量平移）。
- [x] **R35-4（P1）`:ce`/`:ri`/`:le` 同病** — 同 R35-3 修复（`:ce 20` 后
  光标留在原字符，oracle [0,1,6,0]）。
- [x] **R35-5（P1）范围后的行光标不随字节增量平移** — 同族。
- [x] **R35-6（P2）`r<C-E>` 邻行取不到字符整体放弃** — vim 逐位跳过
  （nv_replace 的 NUL 臂只 ++col）；oracle `abcdef`+`abX` 2r<C-E> →
  `abXdef`。越列判定用覆盖区间（offset_for_display_column 的钳制是运动
  落点语义）。
- [x] **R35-7（P1）`r<C-E>` 替换更短字符后光标倒退进下一多字节字符** —
  陈旧 end 算术；改记新文本内最后处理位起点。
- [x] **R35-8（P3）全跳过形状不应响铃** — 无 E 报文，静默无操作。

## 旧钉更正（本轮 oracle 复验推翻前三轮结论，测试已附证据更新）

1. **`r<C-E>` 邻行不足**：旧钉「整条取消 + 响铃」→ 实为逐位跳过
   （`abc`+`XY` 3r<C-E> = `XYc`）。
2. **`:sort n` 无数字行**：旧钉「视为 0 混排」→ 实为整段前置
  （test_sort.vim 同文）。
3. **`}` 于空白行 / `dd u gv` 选区 / `']` 跳转回退 / dap 全删形状**：
   智能体初稿误报，复验与引擎一致（探针已改为钉真 oracle 并注明）。

## 挂账

1. **B-17**：`at` 外层删除域的尾随换行/空行归属（oracle 矩阵已存
   probe 注释）。
2. **ex_align 的 `:le {indent}`+多字节、`:` 系命令的 `l`/`p`/`#` 显示
   旗标效果面**（旗标接受为 no-op 是记录在案的分歧）。
3. fuzz round35 的键表多字符项经 parse_key_sequence 才是真键序（本轮已
   由 harness 保证）；`!` 过滤算子刻意不进键表（fuzz 不得触发 shell）。
