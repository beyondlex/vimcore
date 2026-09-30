# 审查记录（2026-09-29）

四轮代码检视的结论：修复的 bug（语义类全部用 vim 9.1 探针实证，引擎
不变量类用 fuzz 抓取）、与 vim 的已知分歧、悬而未决的可疑点、性能与
体验备注。以现实代码逻辑为准；README 与 `src/lib.rs` 的分层图是宿主
无关措辞。

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
7. **`:d` 的命名寄存器参数被忽略**（`:1,2d a` 不存入 `"a`）。
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
14. **visual `:` 执行后 `'<`/`'>` 被按「锚点..当前光标」重写**，与 vim
    「保留执行前范围」不同。影响 `gv` 二次语义，待定是否修。
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
