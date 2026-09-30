# 审查记录（2026-09-29）

四轮代码检视的结论：修复的 bug（语义类全部用 vim 9.1 探针实证，引擎
不变量类用 fuzz 抓取）、与 vim 的已知分歧、悬而未决的可疑点、性能与
体验备注。以现实代码逻辑为准；README 与 `src/lib.rs` 的分层图是宿主
无关措辞。

## 〇⁺⁺⁺⁺、第八轮检视增补（2026-09-30，回归测试在 `tests/parity_round8.rs`、
fuzz 在 `tests/fuzz_round8.rs`）

本轮通读全部 `src/`，读码列可疑点 → vim 9.1 探针实证 → 修复；随后把
fuzz 键表扩到本轮修复路径并注入宿主点击，64 种子×150 轮×200 步离线
轰炸抓到三个引擎不变量违规。两条读码怀疑被探针**证伪**未改动：
`:j` 在 EOF 的空操作 vim 也会记 changelist + undo 条目（引擎的
`bump()` 反而一致）；`dip` 单空行后 vim 的 `"` 寄存器确实被写入空行。

### 新修复（1-4 语义类均先跑 vim 9.1 探针；5-7 为 fuzz 抓取）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`gq` charwise 起点复制光标前文本** | 探针：光标在 'bb' 上 `gq}`，vim 从 column 0 整行重排；引擎替换区间从 `span.start`（行中）开始而重排文本含整行 → "aaaa " 出现两份。`format_lines` 现把起点锚到首行行首（charwise visual `v$gq` 同款） |
| 2 | **`.` 重放被无变更 Ex 命令污染** | 探针：`:2` `x` `gg` `.`，vim 只重放 `x`（"one"→"ne"）；引擎把 `:2<CR>` 留在录制里，`.` 变成「跳 L2 再删」。`cmdline_key` 的 enter 分支在 `execute_ex` 后按 `edit_generation` 判定：无变更（`:5`/`:noh`/`:reg`/E492/…）即丢弃录制；变更类（`:s`/`:d`/`:j`/`:sort`）在 execute_ex 内已 commit |
| 3 | **`Vp` 行级粘贴破坏存活的缩进行** | 探针：`"XY`（linewise）+ `V` 选 'abc' + `p`，存活的 ' ghi' 完好。引擎 `PutReplace` 的 linewise 分支在 delete_span 停放的光标（first_non_blank，可能行中）插入 → `" XY"` 且缩进丢失。现在行级选区从 `span.start`（行首）插入；charwise 选区 + linewise 寄存器的组合仍悬置（见第七轮） |
| 4 | **`:s` 非法模式顶掉 `last_substitute`** | 编译失败的 `:s/(/x/` 曾在 `build()` 之前记入 `last_substitute`，`&` 会重放一条从未成功过的命令。改为 build 成功后记录 |
| 5 | **`gv` 恢复 mid-char 锚点**（fuzz） | `last_visual` 有两份：`marks.last_visual`（`'<`/`'>` 读）与引擎侧 `VimState.last_visual`（`gv` 读，带 kind）。第七轮 #11 的 refloor 只盖了 marks 副本——等长替换重画字节网格后引擎副本留在字符中间，`gv` 恢复出非法选区。`refloor_stored_offsets`/`sanitize_stored_offsets` 补 floor 引擎副本，`RestoreVisual` 读路径再防御一层。**随后重构消除根因**：`Marks::last_visual` 改为携带 kind 的单一事实源，`gv`/`'<`/`'>`/`:marks` 全部读它；三个 edit 漏斗里手写的引擎副本调整循环整体删除（`marks.adjust_*` 天然覆盖），双副本失步这一类 bug 不复存在 |
| 6 | **块插入会话三条跨行漏洞**（fuzz） | 第七轮只锁了纵向移动/回车；行首 BS 并线、`<Del>` 删 `\n`、含 `\n` 的 IME 文本同样打飞复制偏移（fuzz 复现：复制在缓冲末尾之外 insert）。三者现在响铃拒绝。连带修复：`record_typed_text` 在 `insert_text_at_cursor` **拒绝文本之前**就把内容推进 `block.text`——被拒绝的换行仍参与退出复制；改由 insert 落盘成功后追加 |
| 7 | **块会话中宿主点击甩走光标**（fuzz） | 点击注入把光标移到另一行后，复制偏移假设（所有键入落在打字行）崩坏。`set_cursor_offset`/`set_visual_range` 在块会话期间忽略移动——真实 gpui/crossterm 宿主打字中点击同样会触发 |

### 新功能

- **块选 `u`/`U`/`~`**（`gu`/`gU`/`g~` 同款）：逐行大小写翻转，光标落
  块起点（vim 9.1 探针：`<C-v>jllU` → ABC/DEF，光标 line 1 col 1）。
  旧行为响铃。多字节行按显示列解析块区间，替换走 `edit_replace`。
- **块选 `O`**：行端交换、两侧各自保持列——选区矩形在屏幕上不变，
  光标落到 anchor 行的光标列（vim 9.1 探针：anchor(2,2) cursor(3,3)
  → `O` 后 cursor(2,3)）。char/line 模式的 `O` 响铃（vim 仅块选支持）。
  既有 `o` = 对角角交换不变。
- **`:marks` 列出 `.` 与 `^`**：最后变更位与最后插入退出位（vim 列出）。
- **`:bN`**：`:bprev` 的 vim 别名。

### 悬而未决（本轮记录、未改动）

- `:bfirst`/`:blast` 未支持——`VimHost::cycle_buffer` 只有 forward 参数；
  未知命令走 E492 可见。
- visual **charwise** 选区 + linewise 寄存器的 `p` 精确语义仍悬置
  （第七轮遗留；本轮对齐的是 V/V 行级选区，探针明确）。
- `parse_range` 对**越界正行号**（`:5,10y` 于 3 行缓冲）饱和到边界，
  vim 报 E16——第七轮记录的负行号分歧的同族，无害。

### 体验备注（本轮）

- 块插入会话的跨行封锁清单至此完整：纵向移动、回车、行首 BS、
  `<Del>` 换行、含 `\n` 文本、宿主点击/拖拽——全部响铃或忽略。
- `:reg` 渲染去掉了一次冗余的全文 `replace` 计算（每寄存器两次 → 一次）。

### 性能备注（本轮复核）

- bench_probe 全量复跑：`w` 2.1µs/键（11KB）、hlsearch 重扫 0.25ms/次、
  `:%s` 600KB 5.0ms、`n` 连跳 3.7µs/键（ropey）——与第七轮同量级。
- 1MB 单行 `w` 109µs/键的大头是朴素 String 宿主 `offset_to_line` 的
  O(n) 扫描（每次键 3-4 次调用），ropey 宿主 3.7µs——引擎侧无新热点；
  `x` 232µs/键仍由宿主 String memmove 主导。
- 新增的 last_visual 双副本 refloor 是每次删除/替换 O(存储偏移数)，
  在 bench 量级不可见。

## 〇⁺⁺⁺、第七轮检视增补（2026-09-30，回归测试在 `tests/parity_round7.rs`、
fuzz 在 `tests/fuzz_round7.rs`）

本轮通读全部 `src/`，读码列可疑点 → vim 9.1 探针实证 → 修复。另有
两条读码怀疑被**证伪**未改动：`Options::describe("notabstop")` 正确
返回 None（`no` 前缀不误吞数值选项）；`:sort` 单行范围与 `:sort u`
单行范围在 vim 下都是无操作，引擎原有 `!unique` 守卫之外的行为一致。

### 新修复（语义类均先跑 vim 9.1 探针；fuzz 类标注抓取方式）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **visual 算子后 `:'<,'>` 读到过期范围** | 探针复现：`Vjd` 后 `:'<,'>d` 把缓冲删空。`marks.active_visual`（live 选区）在 `finish_visual_op` 落账后不清除，而 `parse_range` 的 `'<` 解析**优先**读它——visual 算子之后所有 `'<` 范围命令都命中过期偏移。修复：`finish_visual_op` 清除 live 选区 |
| 2 | **visual 文本对象扩展不更新 live 范围** | 探针复现：跨行 `vi(` 后 `:s/ba/X/` 只替换了塌缩行。visual 模式下只有 motion 扩展会更新 `active_visual`，对象扩展（`viw`/`vi(`）漏了。修复：对象扩展后同步 |
| 3 | **visual `c` / 块 `I`/`A`/`c` 后 `gv` 失效** | vim 探针：`viwcX<Esc>` 后 `gv` 重选编辑前字节范围（`'<` 保持 (1,1)、`'>` 保持 (1,5)），块 I 后恢复原块。引擎 insert 会话绕过 `finish_visual_op`，`last_visual` 永不更新 → gv 落空。新增 `pending_visual_marks` 暂存：三处 visual→insert 入口在改动前记录选区，`exit_insert` 会话结束后写回（floor 到当前文本） |
| 4 | **`hlsearch=false` 时取消 `/` 提示符高亮泄漏** | hlsearch 关闭 + incsearch 打开：预览高亮在 Esc 后被 `cancel_cmdline` 用 `last_matches`（上一模式的匹配）顶替，留下永久高亮。修复：取消时尊重 hlsearch |
| 5 | **`g<Esc>` 误响铃、`3"<Esc>` count 存活** | normal_key 的 Esc 检查排在 trie walk 之后：`g<Esc>` 走 trie-miss 重试路径响铃；register-pending 分支吞掉 Esc 而 count 存活，随后的 `dd` 变成删 3 行。修复：Esc 检查提前到全部 pending 状态之前 |
| 6 | **`:sort u` 单行范围建幻影 undo 组** | 单行没有可去重的连续重复，旧 `!unique` 守卫漏掉 `u` 形态——无变化的 replace 宣告 undo 组，空耗一个 `u`。修复：单行一律 no-op |
| 7 | **visual `:` 执行后 `'<`/`'>` 被重写**（消除分歧 #14） | `:s` 把光标放到行首非空白，按「锚点..执行后光标」重写让选区塌缩、破坏 `gv` 二次语义。修复：`cmdline_visual` 记录 prompt 时光标，`close_visual_after_cmdline` 按提示符时范围落账（vim：保留执行的 range） |
| 8 | **`gn` 多字节匹配光标 mid-char**（fuzz 抓取） | `cursor = range.end - 1` 字节算术在匹配以多字节字符结尾时（如 `中`）落进字符内部，宿主 `offset_to_line`/`slice` 直接 panic。`state.rs` 与 `motions.rs` 两处同款，改 `prev_char_offset` 取末字符起点 |
| 9 | **`db` 行首 usize 下溢 panic**（fuzz 抓取） | 列 1 规则只建模了向下跨行，`db` 向上跨行时 `target_line - 1` 下溢。vim 探针：`db` 在行首 = linewise 删除**上一行**（['abc','def'] → ['def']）。补齐镜像分支（覆盖 target_line..=start_line-1） |
| 10 | **块插入会话光标偏移陈旧**（fuzz 抓取） | `exit_insert` 复制行在光标上方插入后，光标字节偏移未跟随平移——逻辑位置落在 `text.len()` 字节之前（多字节文本上 mid-char）。修复：插入位在光标上方时同步平移光标。连带：块会话中 insert 回车能把光标移到另一行走飞复制假设——与纵向移动同款封锁（响铃） |
| 11 | **存储偏移可寻址性缺口**（fuzz 抓取） | 两形态逃过相对调整：删除触及缓冲末尾时偏移越过新长度（删空缓冲后 `last_visual` 越界）；等长替换（`gJ` 的 `\n`→空格）重画字节网格把内部 mark 留在多字节字符中间。`edit_delete`/`edit_replace` 新增 `refloor_stored_offsets`——全部存储偏移（marks/changes/jumps/last_visual）floor+clamp 回当前文本 |

### 新功能

- **`:d {register}`**：`:[range]d[elete] [x] [count]` 的寄存器参数落地
  （`:1,2d a` 存入 `"a`，vim 同款），参数解析与 `:y` 共用
  `parse_reg_count`；不带寄存器时仍走编号环/`"-` 路径。已知分歧 #7
  就此消除。

### 新增已知分歧（接续前表编号）

29. 块插入会话结束后光标停在**打字行**的插入位（逻辑位随复制平移
    修正）；vim 停在**顶行**插入列（探针 `x<C-v>jjI#<Esc>` → [1,1]）。
    行为差异不影响文本，暂不追。

### 悬而未决（本轮记录、未改动）

- **`:s` 空匹配**（如 `s/x*/-/`）引擎策略是「零宽匹配不计数不替换，
  全空则 E486」，vim 会逐位展开（`s/x*/-/` 改写整行）。推进规则未探
  针实证，维持第六轮悬置。
- `parse_range` 对负行号（`:-5`）饱和到 0，vim 报 E16。无害分歧。
- `set_cursor_offset` 在 visual 模式把 anchor 搬到点击点（选区塌缩成
  零宽但停留 visual）——需消费方确认意图，前轮悬置维持。
- Tab 显示宽度按 1 记账（tabstop 感知显示列牵动全链），前轮悬置维持。
- `set`/`mark` 等参数校验宽松：`m<Space>` 静默忽略（vim E355）、
  `q:` 记录到寄存器 `:` 而非开命令窗口。无害，暂缓。

### 体验备注（本轮新增）

- `Esc` 现在是 normal 模式最高优先级的「全部取消」键：任何 pending
  （count/寄存器前缀/算子/多键序列）+ 高亮一并清掉，与 vim 一致。
- `:reg`/`:marks`/`:set` 三类只读反馈通道（第六轮引入）继续有效；
  本轮补齐的 `:d a` 让删除也可进命名寄存器，`:reg` 可见。

### 性能备注（本轮复核）

- bench_probe 全量复跑：`n` 连跳 3.6µs/键、hlsearch 重扫 0.30ms/次
  （900KB/万匹配，优于第六轮记录的 0.75ms）、`:%s` 600KB 5.6ms。
  100x `x` 删除 232µs/键的大头是宿主 String 缓冲的 O(n) memmove
  （bench 的 `B` 宿主 `delete_range` 即 `String::replace_range`），
  引擎侧无新热点。refloor_stored_offsets 新增的每次编辑 O(存储偏移
  数) 开销在此量级下不可见。

## 〇⁺⁺、第六轮检视增补（2026-09-30，回归测试在 `tests/parity_round6.rs`）

本轮通读全部 `src/`（约 8k 行），先读码列可疑点、再 vim 9.1 探针逐例
实证。两条读码怀疑被探针**证伪**（行为本就正确，未改动）：`5s` 在字符
不足的行上 vim 同样只删到行尾再进 insert（钳制一致，非「整条取消」）；
`:sort u` 无 `i` 时不做大小写折叠去重（`[foo,FOO,bar]` → 全保留，字节
序），引擎的原 dedup 语义即对——只有 `i` 参与时才需折叠。

### 新修复（语义类均先跑 vim 9.1 探针）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **visual `<Del>` 只删光标处一个字符** | vim 探针 `viw<Del>` 删整个选区（`hello world` → `hello `）。`navigation_key` 在命令表之前拦截 `Named("delete")`，visual 模式下抢走了命令表里 `<Del>`→`Operator(Delete)` 那一行。修复：visual 模式该键放行给命令表 |
| 2 | **`2d3w` 删 23 个词**（count 跨算子串接） | vim 探针 6 词（2×3 相乘）。算子键按下时前缀 count 停在 `count` 里，motion 的数字继续 `×10+d` 拼上去。`op_count` 字段原本是死状态（无人写入）——现在算子 arm 里 `op_count = count.take()`，`take_total_count` 相乘。连带 `2gUU` 等前缀 count 到行级双写全部经此通路 |
| 3 | **`3S` 只清一行** | vim 探针 `3S` 清 lines 2-4（['l1','X','l5']）。`SubstituteLine` 忽略 count。修复：取 count、行区间展开后走既有的 linewise Change |
| 4 | **末行无尾换行时 linewise `p` 光标落原行** | vim 探针 `yy p` on "abc" 落第 2 行。`p` 的「末行后新开一行」分支插在 `buf.len()`，插入位指向新加的分隔 `\n`（仍是旧行），`offset_to_line(insert_at)` 差一行。修复：该分支光标位 +1 |
| 5 | **`:1,2d 3` 删 L1-L4**（count 扩了范围） | vim 探针删 L2-L4：`{count}` 从范围**末行**起算（help: "starting with the LAST line in [range]"）；`:1,2y 3` 同理拈 L2-L4。`:d`/`:y` 都改为 count>1 时 `first = range.last` |
| 6 | **`:sort iu` 不折叠大小写去重** | vim 探针 `[foo,FOO,bar]` → `[bar,foo]`（稳定 i 排序后原序在前的变体存活，与 Rust 稳定排序 + `dedup_by(lower)` 恰好一致；`u` 无 `i` 仍大小写敏感） |
| 7 | **`:j` 光标落在接缝** | vim 探针：`:1,2j` on `['    aaaa','bbbb']` 光标 col 5（合并行首非空白），裸 `:j` col 1；普通模式 `J` 才落接缝（对照探针 col 9）。ex_join 现落 `first_non_blank(first)` |
| 8 | **Replace 模式 BS 在 offset 0 吞栈项**（读码发现，非探针轮） | 位置守卫先行：`at > 0` 才 pop。旧行为在缓冲起点 BS 白白消耗一个覆盖栈项，后续 BS 恢复到错位字符（测试：`R ab Home BS End BS` → `ay` 而非 `ax`） |

### 新功能

- **`:set name?`**：按 vim 渲染报告当前值（`ignorecase` / `noignorecase`
  / `tabstop=4`），`no` 前缀拼写可查询（`noic?`）；查询不改值。
  裸 **`:set`** 逐行列出全部选项（旧实现只响铃）。
  `Options::describe` / `describe_all` 是新增只读查询面。
- **`:reg[isters]`**：非空寄存器逐行列出（unnamed 优先、命名按序；`^J`
  表示换行，超长省略）。**`:marks`**：命名 mark 加 `<` `'` 特殊 mark
  （`mark  line  col  text`）。`Registers::items` 新增。
  两者都走既有 `status_message` 通道，宿主零改动可见。

### 新增已知分歧（接续前表编号）

26. `:ju`（join 的缩写歧义）报 E492 而非 join（`:j` 与 `:join` 可用）。
27. `:sort` 的 `u` 去重代表元与 vim 在「多个变体 + 非稳定排序」的边角
    上可能不同——稳定排序下已对齐（见修复 #6），仅极端输入存疑。
28. Replace 模式 BS 的恢复栈按「逐字符 LIFO」建模：会话中做过纵向移动
    后，BS 会把上一行的恢复项用到当前行（vim 的恢复历史含行移动）。需
    位置感知栈才能对齐，暂缓。

### 悬而未决（本轮记录、未改动）

- **块寄存器行数与选区行数不齐**的 `p` 语义：vim 探针在 `-es` 下块选
  失真（四轮用过的方法本轮复现失败），`rows.get(i).or(last)` 与 vim 的
  「循环重复寄存器行」孰对孰错未实证，暂不动。
- **`:s` 空匹配**（如 `s/x*/-/`）逐位置展开的行为与 vim 的推进规则是
  否一致未探针；Rust regex `replace_all` 对空匹配每位置各展开一次，
  vim 有自己的跳步规则。罕见，暂缓。
- `parse_range` 对负行号（`:-5`）饱和到 0，vim 报 E16。无害分歧。

## 〇⁺、第五轮检视增补（2026-09-30，回归测试在 `tests/parity_round5.rs`）

本轮以「探针先行」推进：所有语义改动先用 vim 9.1 无头脚本（`-es` +
`normal!`，注意该模式下光标初始落在最后一行，探针必须显式 `gg` 锚定）
实证，再动引擎。两条第四轮结论被探针复核后**推翻**：
`visual y` 后 vim 的光标**原地不动**（留在可移动端，旧行为本来就对，
第四轮记录中无此问题但本轮一度误改后回滚）；`:set number` 的 ex_set
解析其实正确（`number` 不以 `no` 开头，首轮读码误判，探针证伪）。

### 新修复（语义类均先跑 vim 9.1 探针）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **visual 缩进算子忽略 count**：`Vj3>` 只缩一档 | vim 探针 sw=4 expandtab：`Vj3>` → 12 空格、`Vj2>` → 8。count 只对缩进算子生效（`Vj3d` 仍删一次选区）。实现上只解析一次行区间、直接循环 `shift_line`——复用 `ops::apply` 会复用旧字节 span，前面的缩进插入后 span 漂移，尾部行漏缩（实测 l1 缩 3 次 l2 缩 1 次） |
| 2 | **visual 未映射键不清 count**：`V 3 & j` 跳三行 | vim 取消整个 pending。两处 Miss 分支（cmd_seq 续走分支 + 单键分支）都补了 `reset_pending` |
| 3 | **no-op 命令污染 changelist**：空行 `x` 后第二次 `g;` 落在幽灵变更点 | vim 探针 `jx gg x g;g;` 两次都停在真实变更行。`x`/`X`/`s`/`S`/`p`/`P`/`J`/`gJ`/`~`/`<Del>`/`r`/visual `r`/linewise 算子全部改走 `bump_if_edited`（按 `edit_generation` 判真编辑） |
| 4 | **映射在展开队列中部完成时被当字面量**：`:imap a <Esc>` + `:imap q ax` 敲 `q`，`a` 不触发 | `Trie::get` 对「前缀已是叶子、后面还挂键」整队 Miss。新增 `longest_terminal_leaf_prefix`：完整序列恰为叶子才命中并消费前缀；带孩子的命中仍走既有 builtin-vs-mapping 消解（该取舍有专门测试防回归）。自指映射由 MAX_MAP_DEPTH 兜底 |
| 5 | **`/<CR>` 无前次模式静默**、**`:s//x/` 无前次模式哑铃** | vim 探针 `/<CR>` → v:errmsg = E35。两处都补 `E35: No previous regular expression` + bell |
| 6 | **`:1y a 2` 忽略 count** | vim 探针：寄存器后的数字参数是 count（两行进 `"a`）。`:y` 参数解析改为「首参数字=count；字母=寄存器+可选第二 count」 |
| 7 | **TCK 参考实现违反自身 `char_at` 契约**：裸切片在非边界偏移 panic | 引擎的 `floor_to_char_boundary` 刻意探测非边界；宿主照抄参考实现会从合法路径吃 panic（`"中文"` offset 1 实测复现）。补 `is_char_boundary` 守卫，并从测试模块提升为公共 `tck::TckStrBuf`（宿主可直接复用） |
| 8 | **visual `p` 选区贴行尾时插到行尾字符之前**（下游 crossterm-vim 集成测试发现，非探针轮） | `PutReplace` 的插入位走了 `cursor.offset`，而 `delete_span` 经 `clamp_cursor` 停放光标——clamp 是「停放」语义，offset 落在行尾时回拉到末字符。span.start 恰在行尾时（`v$`/`viw`）粘贴提前一字节：`"hello world"` 上 `yiw w viwp` → `"hellohello "`。插入位改取删除区真实起点 `span.start`，clamp 只留给最终光标停放；回归测试 `visual_put_selection_at_line_end_inserts_at_span_start`（`v$`/`viw`/单行整词三形）。引擎自家测试未拦住的原因：既有 visual put 用例的 span.start 都在行首，`end > line_start` 守卫使 clamp 恒等 |

### 新功能

- **`:{range}sor[t][!] [i] [u]`**：行排序。`!` 反转、`i` 忽略大小写、
  `u` 排序后去重；范围末尾的换行保真。单行范围是无操作（vim 同）。
  旧路径把 `:sort` 喂进替换解析器后只哑铃。`n`/`x`/`o` 等旗标忽略。
- **`:{range}j[oin][!]`**：行连接。单行范围连接下一行（裸 `:j` 即此
  语义），`!` 逐字连接（≈ `gJ`）。复用 `ops::join_lines` 的分隔符逻辑。
- **`&` 与裸 `:s`**：重复上一条替换命令（`Cmdline::last_substitute`
  在解析成功时记录，E486 后重放同一命令并报同一错误）；无前次时报
  vim 的 E33。
- **删除死代码 `CmdKind::mutates()`**：惰性 undo 组改造后无人调用，
  文档声称的用途已不存在（gpui_vim / crossterm_vim 下游未引用）。

### 新增已知分歧（接续前表编号）

23. visual 缩进的 count 只认**前置**形式（`3>`）；`>3` 在 vim 里同样
    忽略，故一致——但 `Vj>3` 里滞留的 count 会成为下一个 motion 的
    count（vim 相同，非分歧，备忘）。
24. `:sort` 不支持 `n`（数值）/`x`/`o`/`b` 旗标与 `{pattern}` 截断。
25. `:j` 的 count 形式（`:j 3`）未实现（用 `:j` 于 `:,+1j` 代替）。

### 悬而未决（本轮记录、未改动）

- **`set_cursor_offset` 在 visual 模式把 anchor 搬到点击点**：选区塌缩
  成零宽但停留在 Visual 模式——既不像 vim 的「点击退出 visual」也不像
  「拖拽扩展」。需要消费方（gpui-vim 的鼠标路径）确认意图后再定语义。
- **visual `p` 用 linewise 寄存器**的精确语义（选区被行级文本替换的
  边界情况）vim 探针结果不明确，双方行为都存疑，暂不动。
- **Tab 的显示宽度按 1 记账**（`char_display_width` 对控制字符返回 1）：
  j/k 列保持、`|`、块选在含 Tab 行上与终端渲染有偏差。真要修需要
  tabstop 感知的显示列，牵动 `display_column` 全链，本轮不做。
- **`all_matches` 的 10_000 匹配截断**：超出后 `n` 只在截断集合内环绕。
  巨型文件 + 宽匹配的场景需要宿主用 `set_hlsearch_live_update(false)`
  自管（既有机制），截断值暂不调。

## 〇、第四轮检视增补（同日，回归测试在 `tests/parity_round4.rs`）

探针方式升级：vim 9.1 以 `-es` 脚本逐例实证（注意 `-es` 下 setline 不进
undo 树、搜索提示符行为失真——undo 类探针用「数 undo 次数」、搜索类
用缓冲结果对照）。

### 新修复（语义类均先跑 vim 9.1 探针）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`dw` 在纯空白行误删行**：`dw` on `"  \nbar"` 把换行一起删了 | vim 探针 `dw@0` → `['', 'bar']`（保换行）。linewise 提升只对**真空行**成立；纯空白行走「单次 w 跨行钳制」= span 到行尾。连带修正：落在缓冲区尾 phantom 位置的 w 按 exclusive-linewise 连末行换行一起删（vim `d2w@blank` → `['AAAA']`） |
| 2 | **`^`/`gg`/`G` 在纯空白行落在 `\n` 上** | vim 落在最后一个空格（col=3 on `"   "`）。旧 `first_non_blank` 返回行尾，光标停在 `\n` 后 `diw`/`dw` 会吞换行并线 |
| 3 | **`I` 在空白行插错位置** | vim `"   "` + IZ → `"   Z"`（行尾）；`i` 在 `^` 处才是最后一个空格前（`"  Z "`） |
| 4 | **幻影 undo 组**：`x`/`X` 空行、`J` 至 EOF、`i<Esc>`、`p` 空寄存器、`r<Esc>` 各消耗一次 `u` | vim 探针（数 undo 次数法）：这些 no-op 都不建 undo 条目。修复为惰性组：`open_undo_group` 只记 id，三个 `edit_*` 漏斗在首次真实编辑前才调 `begin_undo_group`（光标仍为编辑前位置） |
| 5 | **空 span 污染寄存器**：空缓冲/行尾的 `dw` 把空串写进 `"-` | vim 探针：`ci(` on `()` 不动 unnamed。`delete_span`/`yank_span` 空 charwise span 直接返回 |
| 6 | **`<C-a>` 破坏 hex/octal/binary**：`0x1f`+1 → `2x1f` | vim nrformats 默认 bin,octal,hex：`007`+2=`011`、`077`+3=`0102`、`0x1f`+2=`0x21`、`0XAB`+1=`0XAC`、`0b101`+2=`0b111`、`0x10`-1=`0x0f`（宽度补零）、`010`-1=`007`。前缀大小写保留；十进制不补零（`0099`+2=`101`，vim 同）；负号直接附着并入（`ab-99` → `ab-98`，vim 同） |
| 7 | **visual `p` 空寄存器删了选区却不贴** | vim E353 且选区原封不动。现在响铃并保留选区；普通 `p` 空寄存器也响铃（旧为静默） |
| 8 | **块插入会话偏移漂移**（fuzz 抓到宿主 slice panic）：会话中翻页键把打字点移到另一行，复制偏移落入字符中间 | 双层修复：insert 模式封锁块会话中的纵向移动（vim 允许并扩展块语义，本引擎不建模，响铃拒绝）；`BlockInsert` 改记打字行行号+行长，按行长精确增量平移，BS 追溯收缩复制文本 |
| 9 | **光标落 phantom 位置**：`w` 在单词行落在 len（past 末字符） | vim 探针 `wx` on "abc" 删的是 c。`clamp_cursor` 统一为「行有内容就落在最后一字符」；宿主点击/拖拽、undo 恢复、g;/C-o/'' 全部入口改走它。配套：`apply` 末尾的 clamp 只对块光标生效（cw/cc 进 insert 后行尾偏移合法，`viwc` 的 `.` 回放依赖） |

### 新功能

- **`gn`/`gN`**：选中搜索匹配（光标所在匹配，否则下一个/前一个）。
  普通键进 visual 并选中；算子下（`dgn`/`cgn`/`ygn`）span 恰为匹配本身，
  不拖入光标到匹配间的缝隙——`cgn` + `.` 逐个替换的工作流可用
  （vim 探针：三个 `b` 逐一变 `X`）。count 按 vim 的 `2gn` ≙ `2n` 再选中。
- **`:d [count]`**：与 `:y` 对齐（`:2d 3` 删 2-4 行），旧实现忽略。
- **`<C-a>`/`<C-x>` 进制支持**（见修复 #6）。

### 新增已知分歧（接续前表编号）

19. `<C-a>` 大数饱和于 i64::MAX（vim 探针 26 个 9 → u64::MAX 回绕）。
20. `gn` 无模式时报 E35 + bell（vim 打开搜索提示符）。
21. 块插入会话中纵向移动被拒绝（vim 允许并扩展块语义）。
22. visual 模式 `/` 不扩展选区（vim 会）；insert `<C-o>{cmd}` 未实现。

## 一、第三轮检视增补（同日稍早，回归测试在 `tests/parity_round3.rs`）

### 新修复（语义类均先跑 vim 9.1 探针）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`cw`/`cW` 跨行吞词**：单字符词 `b` 上的 `cw` 删掉 `b\nc`；`cw` 在行尾词上吞掉下一行 | 旧实现把 motion 重写成 `ce`，绕过了 `dw` 的全部跨行特判。正确规则是「**`dw` 的 span 去掉尾部空白**」——`cw` 单字符词、`c2w`、`cW`、标点、制表符、`cw` 行尾词六个探针全部只有该规则能同时对齐（`ce` 本身确实跨行：`ce` 在 `a|b|c` 的 b 上会改 `b\nc`，所以不能靠它） |
| 2 | `gugu`/`gUgU`/`g~g~`/`gqgq` 中途响铃 | 双写经「弃首键重喂」路径完成，`g` 的 trie miss 误响铃；仅当重喂键补全双写时静默（`dgx` 等真失败仍响铃，vim 同） |
| 3 | `''`/`` `` ``/`'.` 无处可跳 | 新增 `marks.last_jump`（`record_jump` 维护）与 `resolve` 的隐式名分支；`g;`/`'.` 对齐 vim（insert 会话见 #4） |
| 4 | **changelist / jumplist 不随编辑平移**：`g;`/`C-o` 落在过期偏移 | `edit_*` 三个包装器现按 marks 同款规则调整两个列表；宿主 undo/redo 后则走 `sanitize_stored_offsets` 全量 floor（相对调整无从谈起） |
| 5 | 纯 `i` 会话不进 changelist（`g;`/`'.` 漏掉打字） | vim 探针：`g;` = 首个输入字节的精确位、`'.` = 该行行首非空白。`insert_text_at_cursor` 记录首输入位，`exit_insert` 落账；`o`+Esc 也算变更 |
| 6 | 空 linewise 寄存器 `p`/`P` 无操作 | vim 探针：插一个空行。只在「单空行缓冲 `yy`」这类真空文本时触发 |
| 7 | 搜索失败静默响铃 | `n`/`N`/`*`/`/⏎` 现报 `E486: Pattern not found` / `E35: No previous regular expression` |
| 8 | 空提示符上退格会关闭 `/`/`:` | vim 探针（`mode()` 仍为 `c`）：保持提示符打开 |
| 9 | `99999999.`/巨型 count 宏要排队上亿按键（实测跑 5 分钟） | 回放队列是预构造的，count 现按管线护栏 10 万步预算钳制；超出预算的轮次直接不排 |
| 10 | `99999999p` = 寄存器 × count 字节的分配（OOM） | 粘贴字节封顶 16MB（`registers::clamped_repeat` / `ops::clamped_repeat_count`），normal `p` 与 visual `p` 都走 |
| 11 | 可视算子删除自身选区后 `last_visual` 越界（fuzz 抓到 `0..17` 存于空缓冲） | `finish_visual_op`/`exit_visual` 改用 `clamped_visual_bounds`（floor+min）；缓冲末尾的 `unwrap_or(hi+1)` 回退改 `hi` |
| 12 | fuzz 的多字符键项（`dd`/`iw`/`<C-a>`）被 `Key::parse` 并成单个 Named 垃圾键，实为 no-op | fuzz 改走 `parse_key_sequence` 实化；新增 mark/last_visual 边界不变量；轮数 200→500 |

### 新功能

- **visual `r{char}`**：字符级按覆盖字符数填充、行级按各行字符数（`中文ab`
  → `----`，按字符不按显示宽）、块级按每行覆盖段；光标落选区起点（探针）。
- **`:{range}y[ank] [x] [count]`**：行级 yank 进寄存器（数字参数是行数），
  与 `:d` 对称。
- **计数重复插入**：`3ifoo<Esc>` → `foofoofoo`、`3ofoo<Esc>` 开三行、
  `2a!` 翻倍。光标停在最后一组末字符（探针 col 一致）、单一 undo 组、
  `.` 连 count 一起重放（count 键被录制，重放走同一条 EnterInsert 路径）。
  保守语义：光标仍在输入末尾且文本单行才复制；c/s 的 count 属于 motion；
  Replace/块选不参与。

### 新增已知分歧（接续前表编号）

16. 计数重复插入只覆盖「纯打字到 Esc」；会话内用过方向键/回车的重放语义
    未对齐 vim（不复制）。
17. `:s` 的 `c`/`n`/`e` 等标志被静默忽略（`i`/`I`/`g` 有效）。
18. 块选 `$` 扩展列（各行到自身行尾的 ragged block）未实现。

## 二、已修复的 bug（前三轮）

每条都有对应回归测试；语义类修复先在 vim 9.1 上跑探针取实证，再改引擎。

| # | 问题 | 根因 | 实证方式 |
|---|------|------|----------|
| 1 | `dw`/`cw` 跨入缩进行时合并行 | w 算子跨行规则只覆盖「光标在行尾词上」；落点不在第 1 列时 column-1 规则不触发，span 吞掉换行+缩进 | vim 探针 8 例（含 d2w 清空本行而非删行、d3w 合并、空行 dw 无操作/删行两个分支） |
| 2 | `2N`/`3N` 朝正方向走 | 后向计数用 `start_index + (count-1)`，方向反了 | vim 探针：第 5 个匹配上 1N→4、2N→3、3N→2 |
| 3 | `:s` 光标落在过期偏移上 | 记录的是替换前匹配字节偏移，前行替换改变行长度后失效（多字节下落进字符中间） | vim 探针：光标=最后被替换行+首非空白列，与行内匹配位置无关 |
| 4 | `gv` 恢复前向选区塌缩成单字符 | `exit_visual` 把光标停到选区起点后又重读选区 | 引擎测试（前向 v3l → gv 只剩 1 字符） |
| 5 | `<c-a>` 等小写修饰键解析成 `Named("c-a")` | 角括号内修饰前缀只认大写 `C-` | vim 接受 `<c-a>` 映射；rc 文件两种拼写都常见 |
| 6 | `*` 在无词行上沿用旧模式静默重跳 | `search_word_under_cursor` 失败无信号，`jump_to_match` 继续用上次模式 | 引擎测试（光标不动、高亮不变） |
| 7 | 可视区间排他端 `c+1` 字节算术 | 多字节光标字符内落点 → `'<`/`'>`、`:'<,'>` 范围在严格宿主上 panic | 随机按键 fuzz（ZWJ/中文字符） |
| 8 | 行级 span 用 `end-1` 取末行 | 末行无换行且以多字节字符结尾（缓冲 `"中"`）时 `end-1` 落进字符中 | fuzz + harness panic |
| 9 | 等长替换把 mark/变更表/跳转表偏移留在字符中间 | `adjust_replace` 的 preserve_inner 按相对字节保留；变更表/跳转表从不随编辑调整 | fuzz；所有外部偏移入口统一 `floor_to_char_boundary` |
| 10 | 巨型 count panic | `count*10+d` usize 溢出；`take_total_count` 乘积溢出；`:5+<huge>` 加法溢出 | fuzz/边界测试；饱和运算 + 1e9 封顶 |
| 11 | 块选 `A` 短行不补空格 | 直接行尾追加 | vim 探针：块列 3-5、行 `ab` → `ab   X` |
| 12 | `o`/`O` 把 tab 缩进换空格 | 用 `" ".repeat(indent)` 重建缩进 | 引擎测试；改为按字节切片复制原缩进 |
| 13 | `S` 在无换行多字节末行上 panic | 同 #8（`apply` 的 Change/Indent/Format 分支） | fuzz |
| 14 | 合法大计数宏被静默丢弃 | 管线护栏 500 步，`10000@a`（五键宏）= 5 万步 | 上限提到 100k，失控宏仍有界截停 |
| 15 | 未闭合 `<` 的按键序列吞掉尾部 | `parse_key_sequence` 无 '>' 时把余下字符全并进一个 Named 键 | 回退为逐字符字面量 |

## 三、用 vim 9.1 实证后确认**无需改**的行为

- `>>`/`<<` 对空行不动（vim 同样跳过空行）。
- `G`/`gg`/`H`/`M`/`L` 落首个非空白列（含 visual 模式），不保持列。
- `<Space>` 就是 `l`：独占 motion，行尾不动、不跨行。本轮把它注册进
  命令表（旧实现响铃）。
- 块选 `I` 短行：插到行尾、不补空格（与 vim 一致；此前注释即正确）。
- `cw` 在空白上：普通 w 跨度（多个空格一起换），非单字符。

## 四、新增功能（前三轮）

- **块寄存器普通模式 `p`/`P`**：首行落目标列（`p` = 光标列+1，短行补
  空格），其余行成为下方新行并补空格到同列，不下挤合并；count 横向重复。
  旧实现把块文本按字符流拍平。光标落首行插入点（vim 9.1 探针对齐）。
- **`r<CR>`**：以换行替换当前字符（vim 拆行语义），旧实现响铃。
- **`<Space>`** = `l`（见上）。

## 五、已知分歧（有意为之或暂不处理，接入方需知）

1. **正则方言**：搜索/`:s` 用 Rust `regex`（RE2 语义），替换用 `$1` 而非
   vim 的 `\1`；不支持 `\%V`、lookbehind 等值断言按 RE2 规则。
2. **`ignorecase` 默认 true**（vim 默认 false），`smartcase` true。
   对中文/代码检索的默认体验更好，但与 vim 出厂值不同。
3. **`textwidth` 默认 78**（vim 默认 0 = 不自动换行）；insert 不自动换行，
   `gq` 按 textwidth 重排。
4. **`scrolloff` 选项已存储但引擎不读**：视口归宿主，引擎只在
   `VimHost::scroll_to_line` 报告目标行。宿主可自行实现 scrolloff。
5. **`"+` 剪贴板寄存器的类型靠猜测**：含 `\n` 即按 linewise 粘贴。宿主
   剪贴板没有 charwise/linewise 元数据，多行 charwise 文本会被按行粘贴。
   若宿主能提供元数据，可在 `get_for_paste` 扩展。
6. **`:` 范围内 `;` 与 `,` 等价**（vim 的 `;` 会把光标依次落在中间地址）。
7. ~~`:d` 的命名寄存器参数被忽略~~（**第七轮已修复**：`:1,2d a`
   存入 `"a`）。
8. **句子 motion 只认 `.!?` 单字符**，无 `...`、换行跟随等规则
   （`Motion::SentenceNext` 注释已声明）。
9. **tag 对象（`it`/`at`）**：属性值里的 `<`/`>` 会干扰解析；光标恰好
   停在开标签 `<` 上不选中（代码注释已声明）。
10. **`\\"` 转义引号**：`quote_positions` 只看前一字符是否 `\`，`\\` 后的
    真引号会被误判为转义（罕见；需要一个小状态机，暂缓）。
11. **insert 模式 `<C-w>`/`<C-u>` 在行首不删除换行**（vim 会并行）。
12. **`:g`、`:sort`、`:normal`、函数/autocmd** 不支持，走 E492 或 rc
    `ignored` 收集（IdeaVim 同款取舍）。
13. **`""yy` 落 `"0`**：`""` 即匿名寄存器，yank 必写 `"0`，行为与 vim
    一致；但 `""dd` 不进数字环（vim 也如此）。
14. ~~visual `:` 执行后 `'<`/`'>` 被按「锚点..当前光标」重写~~
    （**第七轮已修复**：按提示符时的选区范围落账，`gv` 二次语义恢复）。
15. **`o`/`A` 进入 insert 后的块复制不含多行文本**：块插入复制的文本
    含 `\n` 时每行都会粘进（无宿主可见的崩坏，但语义未对齐 vim）。

## 六、性能备注

- **hlsearch 逐编辑全量重扫**是默认行为（`hlsearch_live_update`），
  每次编辑 O(全文 scan)。大文件宿主应
  `set_hlsearch_live_update(false)` + 定时 `refresh_highlights`
  （bench_probe 里 900KB/1 万匹配的重扫约 0.75ms）。
- `all_matches` 一次性 `slice(0..len)` 拷全文再扫：实测比逐行走 trait
  快 ~20000 倍（代码内注释保留了测量结论），代价是每次扫描一次全文
  memcpy——对 ropey 宿主可通过 `refresh_highlights` 节流摊薄。
- `n`/`N` 在无编辑时走缓存匹配表（`matches_generation`），O(1)/键。
- 随机按键 fuzz（200 轮×160 键）已转正常驻测试
  `fuzz_random_key_sequences_hold_invariants`，覆盖不变量：
  不 panic、光标恒在字符边界、行数 ≥ 1。

## 七、体验备注

- `*` 无词时现在响铃且不动光标（旧实现会静默用旧模式跳一次，是「反直觉
  的体验点」而非崩溃）。
- `r` 后按方向键：非可打印参数被吞并响铃，等待下一个键（vim 同款）。
- `q` 未开始时响铃；`@@` 无上次寄存器时响铃；都有对应测试。
- 未知 Ex 命令报 `E492` 文本 + bell，`:action` 未命中按 strict/lenient
  分流（宿主决定是否提示）。
- `:set` 三种只读形态（第六轮）：`name?` 查询单项、裸 `:set` 列出全部、
  `:reg[isters]`/`:marks` 列表——此前 `:set` 无参数只响铃，寄存器/mark
  状态对用户完全不可见。反馈统一走 `status_message`，宿主零改动。
