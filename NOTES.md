# 审查记录（2026-09-29）

多轮代码检视的结论：修复的 bug（语义类全部用 vim 9.1 探针实证，引擎
不变量类用 fuzz 抓取）、与 vim 的已知分歧、悬而未决的可疑点、性能与
体验备注。以现实代码逻辑为准；README 与 `src/lib.rs` 的分层图是宿主
无关措辞。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第二十四轮检视增补（2026-10-04，回归测试并入
`tests/review_regressions.rs`（+17 例）+ 新增 `tests/fuzz_round24.rs`
（字素簇语料全按键轰炸 + 可视/宏交错，携带一条新引擎不变量））

本轮主题：**字素簇（grapheme cluster）全链路**。fuzz_round24 首次把
「normal 光标永不落在宽度 0 的延续字符（组合字符/VS16/ZWJ）上」写成
可执行不变量——宽度 0 的字符是簇的延续，光标停那里 `x` 就会把簇拆开
（删掉基字符留下裸 mark，或反之）。围绕这条不变量共修复九处路径，
全部由「先写探针 → 最小化 → 读码定位」流程实证（无 vim 探针：vim 字节
级模型在复合字符上本就允许瞬态中间态，引擎取「簇原子性」这个更强的
自洽约束）。

### 语义修复（探针 + fuzz 最小化实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`prev_grapheme_offset` 回溯不看候选位上的字符** | 旧实现检查候选位*之前*的字符决定是否续行——尾随组合字符（`"e\u{0301}"` 行尾）直接返回 mark 自身的偏移。重写：候选位*上*的字符是宽度 0/ZWJ → 簇向左延伸继续回溯；起点字符的前邻是 ZWJ → 胶合（家族 emoji 镜像正向扫描） |
| 2 | **Esc 步退落在簇中间** | `exit_insert` 用 `prev_char_offset` 回退一格：输入 `a\u{0301}` 后 Esc，光标停在 mark 上，`x` 只删 mark 留下裸 `a`。改 `prev_grapheme_offset`（vim delcombine=off 默认：复合字符是删除单元） |
| 3 | **`$`/`j`/`\|`/`g_` 的行尾落点同病** | `clamp_cursor`/`offset_for_display_column`/`Motion::LineEnd` 全用 `prev_char_offset` 取「行尾最后一个字符」；`g_` 的非空白扫描把 mark 当落点。全部改簇起点 |
| 4 | **charwise `p` 把文本插进簇中间** | 「光标后插入」用 `next_char_offset` 跨一格：在 `#`+VS16 上 `p`，粘贴文本落在基字符与 VS16 *之间*（`#n\u{fe0f}`）——簇被劈开。改 `next_grapheme_offset`。可视粘贴落点/可视对象光标/`gn` 光标三处 `prev_char_offset` 同修 |
| 5 | **`w`/`b`/`e` 词扫描簇盲** | `class_at` 把组合字符归类为 Punct：`w` 在 `"b\u{0301}"` 上停在 mark 处；`e` 的 run 尾（`next_word_end`）落在 mark；`b` 把 mark 当上一个 run 的起点；`ge` 把 mark 报成 run 端。修复：`class_at` 里延续字符继承基字符类别（`"base+marks"` 是一个单元）；`e` 落点收回到簇起点（`pull_off_continuation`）；`b`/`w` 的空白跳过把 mark 并入所属空白簇；`ge` 向后扫描跳过延续字符 |
| 6 | **块删除孤儿化 mark** | `block_row_range` 的单元尾只含基字符：块 `d` 删 `a` 留下 U+0301，mark 转而复合到前一字符上。单元尾延伸过尾随延续字符 |
| 7 | **可视 `r` 按字符数填充** | `中文ab`→`----` 的填充数按 chars 数：簇（基+mark）算两个。新增 `buffer::grapheme_count` 按簇计数（vim 语义：复合字符是一个替换单元） |
| 8 | **`*` 在可视化空白行上搜出 mark** | 前扫「下一非空白」用 is_whitespace：mark 不是空白 → 搜出不可见字符并把光标停上去。前扫跳过延续字符 |
| 9 | **`^`/`gg` 停在行首 mark 上** | `first_non_blank` 的落点/空白行回退都按字符：行首 mark（`\u{0301}abc`）或缩进后的 mark 直接停靠。落点跳过宽度 0；空白行回退先剥离尾随延续 run 再取最后一个可见字符 |

### 配套设施

- `buffer::last_grapheme_start(&str)`：`prev_grapheme_offset` 的 `&str`
  镜像，供块插入会话的 `block.text` 字节账本按簇裁剪（BS 删整簇时
  `typed_end`/`text` 必须同步收缩，退出侧 delta==text.len() 复制不变量
  才能保持）。
- `tests/fuzz_round24.rs`：11 条簇语料（分解组合字符/VS16/ZWJ 家族/肤色
  修饰符/全角空白/行首 mark）× 100-120 步按键轰炸 + 可视/宏交错。唯一
  合法例外：基字符被删除后的独立 mark（自成簇且行内无基可贴，只允许
  行首形态）。delta-debugging 最小化器（临时脚本，未入库）把 96 步
  失败序列收缩到 4 键 `v i w y`，直指可视对象光标落点。

### 检视中证伪、无需改的（免下轮重查）

- **`dw`/`cw`/`diw`/`J`/`:s`（含簇模式与簇替换串）/`R`/`~`/块插入 CJK
  对齐/`gq` CJK 重排**——定向探针全部符合预期（簇作为整单元参与运算
  符与 Ex）。首轮探针里 `:s` 的诡异输出是探针自身漏喂 `:` 所致
  （`%s/ab/XY` 被普通模式吃掉：`s` 删字符进 insert，`<CR>` 换行），
  非引擎问题。
- **`Put` 的块寄存器/linewise 分支**、`replace_overwritten` 栈（Replace
  模式 BS 镜像自身单字符记录，簇语义自洽）。
- **性能零回归**：bench_probe 对照 round23 基线逐项持平（w 6.4µs/键、
  1MB 单行 w 163µs、hlsearch 重扫 0.35ms、`:%s` 6.6ms、`n` 3.7µs）。
  `class_at` 的基字符回溯只在含延续字符时触发，无 mark 文本走原快路径。

### 新增已知分歧（接全局序号）

66. **字素簇语义整体比 vim 字节级模型更强**：vim（无 `delcombine`）在
    复合字符上允许字节级瞬态中间态（`ta` 可把光标停在复合字符*内*）；
    引擎一律钳到簇起点。对用户可见的差异仅限「行尾组合字符/ZWJ 家族」
    这类罕见排布，且引擎行为（簇原子）更符合渲染直觉。
67. **独立 mark 行（基字符被删后只剩 `\u{0301}`）**：`w` 的空白跳过会
    越过它落到下一可见字符，`^` 的落点也跳过它——vim 字节级会停在
    mark 上。不可见字符的停靠无渲染意义，取可见语义。

### 性能备注

- 本轮唯一新增的热路径分支是 `class_at` 的延续字符判定（一次宽度表
  查询）与 `is_cont`（同行）；无 mark/ZWJ 的文本零额外开销。bench_probe
  全项与 round23 基线持平（见上）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第二十三轮检视增补（2026-10-04，回归测试并入
`tests/review_regressions.rs`（+7 例）+ 新增 `tests/fuzz_round23.rs`
（`:set`/`:sort` 参数恶意面 + 宿主事件（IME 提交/undo/宏回放）随机交错
+ `is_idle`×模式契约））

本轮流程：全量通读 `src/`（键形→提示符→cmdline→编辑路径→宿主事件面）
→ 两个探针批次的恶意输入定谳（`-es` 脚本 + Python PTY 字节级）→ 修复。
新工具形态：`pty.fork` + 裸按键喂入替代 `:normal!`/execute 探针（后两者
在 `-es` 下会被 Ex 解析污染，`m<CR>` 曾伪报 mode=c）。

### 语义修复（探针 + vim 9.1 实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **IME 提交拆散 insert 会话 undo 组（数据丢失级）** | `replace_range`（IME 组合提交路径）无条件 `end_edit()`——会话中途每次提交各拆一组，`i<Esc>u` 只撤销最后一次提交（vim 全会话一个 undo 块）。修复：仅在本调用自开 undo 组时关闭；REPL 式会话外替换仍自开自关不悬挂 |
| 2 | **`floor_to_char_boundary` 把缓冲末尾当 mid-char** | `char_at(len)` 返回 None 被当作「非边界」floor 到末字符起点——宿主 `replace_range(5..7)` 于 7 字节缓冲只编辑 5..6，吞掉末字符。修复：`offset == len` 恒为边界提前返回；parity_round5 的 floor 契约按新语义更新 |
| 3 | **Enter 被无条件映射为 `\n` 传给所有 char-arg 命令** | `f<CR>` 失败却覆写 `last_find`（下一个 `;` 变成找换行——vim 失败不更新）；`'<CR>` 产出带裸换行的 E20 文本。PTY 实证：vim 仅 `r<CR>`（拆行）与 `vllr<CR>`（逐字符拆行）消费 Enter。修复：仅 Replace/VisualReplace 映射，其余走失败路径（响铃 + `last_find` 保持） |
| 4 | **`:set ts=` 报 E518（Unknown option）** | vim 9.1 `-es` 实证：`E521: Number required after =: ts=`。已知名 + 非法值走 E521，E518 保留给未知选项名 |
| 5 | **`:sort z` 静默排序** | PTY 实证：vim 报 `E475: Invalid argument: z` 且缓冲不动。修复：旗标白名单 `!iun`，其余（含 vim 有效但未实现的 `x/o/b/f/l`）E475 拒绝——响亮的分歧好过静默错排 |
| 6 | **`:sort n`（数值排序）缺失** | 新实现：行内首个十进制数为键（前导 `-` 计符号）、无数值视为 0、stable（等值保原序）、`u` 按 sort 键去重（vim 的 uniq 共享排序比较器） |
| 7 | **`is_idle` 在开着的提示符期间返回 true** | 宿主以它为按键拦截闸门（VimEdit 本地 undo），提示符里的字面文本（`:ru` 的 `u`）会被劫持。修复：cmdline 模式恒 false；insert 仍算 idle（文本输入是宿主职责） |
| 8 | **通用 `'x` mark 分支缺 `'>` exclusive-end 修正** | exact `'>` 分支解析 `line(off-1)`，mangled token（`'<,'>'<,'>` 双预填）走通用分支落到下一行。顺带：E20 文本引用解析出的 mark 名而非原始 token 的垃圾尾巴 |

### 体验改进

- `:set`/`:sort` 的错误语义从「错类」回到 vim 的两个错误码（E521/E475），
  宿主状态栏的报错不再误导（值非法 ≠ 选项不存在）。
- `m<CR>` 等非法 mark/查找参数从静默忽略改为响铃（与 `r` 等失败参数同款）。
- 提示符 C-w 的尾部空白归一化改为 `trim_end()`（先前 stash 里的半成品，
  Tab 实际进不了提示符缓冲——注释按真实可达面重写）。

### 检视中证伪、无需改的（免下轮重查）

- **`3/pat` 的计数语义**：引擎 `N/pat ≡ Nn`（含 wrap：3 匹配缓冲上 `3/foo`
  回落第 1 个匹配）——vim 9.1 PTY 七形态全对齐，round14 的实现本来就对。
- **空缓冲/空行 `dd` 的寄存器**：引擎存空 linewise——vim 同（首轮探针的
  SENTINEL 未变是 0 行缓冲假象；`['']` 上 dd 后 getreg 为 `"\n"`，
  strtrans 渲染 `^@`）。寄存器轮转同样发生。
- **`v r<CR>`**：vim 把每个选中字符各换成一个换行（PTY：`vllr<CR>` 于
  "abcdef" → 三空行 + "def"）——引擎逐字符 repeat 一致。
- **空缓冲 `S`/`cc` 进 insert、`:d` 于空缓冲响铃**——均与 vim 一致。
- **`9999@x` 巨 count 宏**：管线守卫 1.3ms 内收敛，无挂起。

### 新增已知分歧（接全局序号）

64. **IME 提交（`replace_range`）不进 `.` 重放记录**：宿主驱动的 raw 替换
    没有按键/文本形态可录——重放只含打字的 Text 步骤（探针：会话落地
    "hello阿"，`.` 重放得 "helloab"）。vim 重放最终落地文本形态。
65. **`:sort x/o/b/f/l`（hex/octal/binary/float/locale）E475 拒绝**：vim
    实现这些旗标；引擎响亮拒绝而非静默按字典序错排。

### 性能备注

- bench_probe 复跑 + **基线提交（e96d0bb）同机对照**：`w` 6.4µs/键 vs
  基线 6.2µs、1MB 单行 `w` 167µs vs 163µs、hlsearch 重扫 0.30-0.35ms、
  `:%s` 6.1ms、`n` 3.6µs——与 round22 记录（2.3µs/103µs）的偏差在基线
  上同样出现，属机器状态漂移而非本轮回归（本轮唯一热路径邻近改动是
  `floor_to_char_boundary` 的提前返回，只减不增）。


## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第二十二轮检视增补（2026-10-04，回归测试并入
`tests/review_regressions.rs`（+8 例）+ 新增 `tests/fuzz_round21.rs`
（`:s` 替换串转义面 × 6 缓冲 × 9 模式的行寻址契约 + CJK search-motion
定向 + NUL 寻址 + `&` 重放单次性）

本轮流程：全量通读 `src/`（键形→提示符→cmdline→编辑路径）→ 恶意输入
探针定谳 → vim 9.1 `-es` 字节级实证 → 修复。新工具形态：`vim -es`
脚本模式替代 PTY 交互（`printf 脚本 | vim -N -es -u rc` + `writefile`
捕获 v:errmsg/光标/缓冲字节），对 Ex 语义面的实证比 PTY 稳定。

### 语义修复（探针 + vim 9.1 实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`<C-Space>` 映射劫持普通空格（数据破坏级）** | `parse_angle` 的 `space`/`lt`/`bar` 分支无条件 `Key::char(..)` 丢修饰键——`:nmap <C-Space> ix` 被存成普通空格，按空格即触发。vim `:nmap` 列表实证 `<C-Space>` 与 `<Space>` 是两个键。修复：修饰键随形 |
| 2 | **单字符 Named ctrl 键死键** | `Key::ctrl("w")`（gpui 形态）与 `Key::ctrl_char('w')`（parse 形态）不等价——insert 的 `<C-w>` 弦与全部 ctrl 命令表只认 Char 形，Named 形静默 Unknown。修复：`handle_key` 把单字符 Named 键折叠为 Char（修饰键保留）；顺带让 ctrl+Named("space") 能命中 `<C-Space>` 映射 |
| 3 | **`3/pat` 提示符大 count 溢出 panic** | 提示符 `search_count` 是唯一绕过 `take_total_count` 十亿封顶的 count 入口；20 个 9 后 `/pat<CR>` 在 `jump_to_match` 的 u64 加法上溢出——debug panic（release 静默错跳）。修复：步数先按匹配数取模，和式永不出界 |
| 4 | **`:s` 替换串 `\r`/`\n` 字面穿透** | vim 9.1 字节级探针：`\r` 是换行（`%s/,/,\r/g` 惯用法）、`\n` 是 NUL（`fX\x00Yo bar`）；引擎把两者当字面量写进缓冲。修复：`unescape_replacement` 补两条映射，`\\r` 保持字面 |
| 5 | **拆行替换的光标落点** | `\r` 落地后光标仍按**拆行前**行号停放。vim 三形态实证：`%s/a/\r/` 于 "aaa\nbbb" 落 "aa" 行、g 形态落第 4 行、`2s/a/\r/g` 落拆行区末行。修复：按替换槽内新行数下移 |
| 6 | **`&`/裸 `:s` 重放存储的范围** | state.rs 旧注释断言「ranges behave as typed」——vim 9.1 探针翻案：`:1s/a/B/` + `+` + `&` 作用于**第 2 行**而非存储的第 1 行。旧引擎 `&` 重打绝对行号，且裸 `:s` 在带范围存储后直接 E492（`strip_prefix('s')` 打不穿 "1,2"）。修复：重放前剥范围前缀（键入的范围仍然生效） |
| 7 | **`:sort` 尾随数字被静默接受** | vim 9.1：`:1,3sort 3` E488 且缓冲不动；引擎静默排序。修复：args 含数字即 E488 拒绝执行 |

### 体验改进

- `:set` 未知选项从哑铃改为 **E518: Unknown option: {name}**（vim 实证
  文案，其后项不再执行）。
- `:s` 非法 pattern 从哑铃改为发布编译错误文本（不冒充 vim 的 E486——
  传统引擎宽容不闭合的 `[`，RE2 方言只能如实报编译失败，分歧 #1 细化）。
- `.` 重放热路径上的 `DOT_TEXT_MARKER` 比较从 `KeyKind::Named(String 构造)`
  改 `matches!`——每键一次 String 分配移出管线。

### 注释/格式纠偏（防未来误修）

- `motions.rs` `GoToFilePercent` 旧注释称「percentage formula needs the
  original count」——该变体按构造只承载 typed-1，公式从不看 count。
- `cmdline.rs` `ex_join` 的错位缩进（历史遗留）；round21/回归新码对齐
  rustfmt。刻意**不做**全仓 fmt：20+ 轮手排风格，全仓 churn 淹没 blame。

### 检视中证伪、无需改的（免下轮重查）

- **f/t 参数过映射表**：`f` 等待参数时 `:nmap x l` 的映射照常展开——
  vim 同（`;` 被映射会破坏 `f;` 是 vim 社区常识），无需 no_remap。
- **`&` 既有回归测试**（`ampersand_repeat_drops_substitute_flags` 等）
  全部用无范围存储命令，范围剥离不触碰。
- **显式 `"1dd` 不轮转数字环**（`store('1')` 直写）——vim 同。
- **`increment_number_at_cursor` 的行内字节索引**：全部基于行切片相对
  偏移，多字节行安全（`0b2`+C-a 这类 radix 歧义与 vim 内部判定同深，
  已有注释钉住，未跟）。

### 新增已知分歧（接全局序号）

62. **`<S-Space>` 形映射仍死**：交付侧 shift+space 被规范化为普通空格
    （`<S-1>` 键盘布局相关家族同款）。`<C-Space>`/`<M-Space>` 本轮已修。
63. **非法正则的报错文案**：vim 传统引擎宽容更多语法（不闭合的 `[` 报
    E486 且存储 pattern）；引擎（RE2）报 `Invalid pattern: …` 且不存储
    （`&` 不重放坏命令）。

### 性能备注

- bench_probe 全量复跑：`w` 2.3µs/键（round21 记 5.9µs，含机器抖动）、
  1MB 单行 `w` 103µs、hlsearch 重扫 0.25ms、`:%s` 5.4ms、`n` 3.7µs——
  与 round18-21 基线持平或略优。本轮改动全在提示符/Ex 面，非每键热路径
  （唯一热路径改动是 DOT_TEXT_MARKER 的去分配）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第二十轮补记（2026-10-03，fuzz round20）

`tests/fuzz_round20.rs`：命令行面（`:` `/` `?`）从「整串命名键」换成
「进提示符 → 逐字符喂恶意字母表 → Enter/Esc 收尾」的爆发式输入，新增
渲染契约不变量——**发布给宿主的高亮区间必须整体可寻址**。抓到 E35
早退路径（`/ab<C-u><CR>`）不清 incsearch 预览的渲染违例并修复。本轮
（二十一）顺手清掉该文件的三处 clippy warning（manual_is_multiple_of）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第二十一轮检视增补（2026-10-04，回归测试并入
`tests/review_regressions.rs`（+15 例）；无新 fuzz 文件——本轮的异常面
由既有 round20 的 cmdline 轰炸与定向回归覆盖）

本轮流程：全量通读 `src/`（读码列候选 → PTY 探针定谳 → 修复），
第二轮对新面（search-motion × 宏/`.`/跨行/多字节/CJK）复核。方法论
收获一条：**探针先打自己的断言**——第二轮 4 个「失败」里 4 个都是
探针断言算错而引擎与 vim 一致（visual 选区含光标字符、反向 exclusive
保留光标字符、`/aa` 跳过光标自身匹配、append 打字落点），先把探针
钉对再谈翻案。

### 语义修复（PTY 字节级探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`d` + `/` 操作符悬空跨提示符（数据丢失级）** | 旧引擎 `d` 挂起后 `/` 照常开提示符，Enter 执行纯跳转而操作符**原封不动**——搜索后下一个 motion 键误触 `d<motion>`（探针：`j` → `dj` 删空整个缓冲）。修复即下述新功能：提示符现在就是 motion |
| 2 | **`:marks` 重复列出 `^`** | `exit_insert` 同时写 offsets 表（`set('^')`）与引擎跟踪的 `last_insert_exit`，列表追加分支只对 `.` 去重、对 `^` 无条件追加 |
| 3 | **CJK 行尾 `iw` 误判整行** | 文本对象探针 `line_end - 1` 是裸字节步：多字节尾字符落 mid-char → `class_at` 读 None → 空行分支触发。修复：探针 floor 到字符边界，且整个 run 扫描锚定在 clamp 后的位置（旧代码只 clamp 了分类探针、扫描仍从原始 offset 出发，会产生越行 range） |
| 4 | **`:1j` 末行 no-op 光标落 col 0** | 无接缝路径把光标留在 join 前设置的 `line_start(first)`，而成功路径与 `:5` 空命令都落首非空白——vim 9.1 探针（`:1j` 于 `"    abc"` 后 `x` 删 `a`）确认落首非空白。顺带修正 round19 的记录：当时写的「与 :5 同款」在缩进行上并不成立，本轮才真正对齐 |
| 5 | **`<S-a>` 型映射永不触发** | `handle_key` 对纯字符键丢弃 shift 标志，而 `parse_angle` 保留 `shift+Char('a')`——交付形态永远对不上。vim 9.1 探针：`:nmap <S-x> ihello` 的映射列表显示 `n  X  ihello`（vim 同样规范化）。修复：shift-only + ASCII 字母 → 大写字母；非字母（`<S-1>` 键盘布局相关）保持原样 |
| 6 | **`d/foo` 匹配仅在光标处静默 no-op** | wrap 回落自匹配时 span 为空——引擎无操作无反馈；vim 探针给 wrap 消息 + 响铃。修复：`target == origin` 视为 motion 失败，响铃 + 放弃操作符 |

### 新功能：search-as-motion（`d/pat`・`c?pat`・`gu/pat`・`v/pat`）

`/`/`?` 在操作符下或 visual 模式打开提示符时记入 `search_motion`，
Enter 按 vim 语义收账（全部 PTY 探针对齐）：

- **操作符下**：以 exclusive span `[cursor, match_start)` 补齐操作符
  （`d/bar<CR>` 于 `"foo bar\nbaz"` → `"bar\nbaz"`）；count 合并
  （`2d/foo` ≡ `d2/foo`）；miss 报 E486 并**放弃操作符**；Esc 静默
  取消（vim 的 clearop）；wrap 语义与 `n` 一致。
- **visual 下**：光标跳匹配起点、选区扩展（锚不动）、**保持 visual**
  （`v/bar<CR>` 高亮 `"foo "`，选区含光标字符——vim 探针
  `v/bar<CR>x` → `"ar\nbaz"`）。
- **免费得到的**：`.` 重放（键序含提示符全程被录，`d/foo<CR>` 后
  `.`/`2.` 正确重复）、宏录制回放（`qb d/bar<CR> q` 六步，`@b` 同形）、
  `y/pat` 光标不动（yank 不动光标，vim 同）、跨行 span、`gu`/`gU`/
  `g~`/`c` 全操作符族。

修复前 visual 的 `/` 是响铃死键——本轮顺带变为可用。

### 检视中证伪、无需改的（免下轮重查）

- **宏回放 search-motion**：首轮探针「`@b` 回放不删」是探针自身的
  fixture bug（宏存在一个 `VimState` 里，换 fixture 即丢）——同
  fixture 重置缓冲后回放逐字节正确。宏不跨 `VimState`，此前从未
  被钉过，本轮顺手写进回归。
- **反向 exclusive 的光标字符**：`$d?def<CR>` 于 `"abc def ghi"` →
  `"abc i\n"`——删 `[match_start, cursor)`，光标字符保留。引擎与
  vim 逐字节一致（首轮注释误读为「含光标字符」，已改）。
- **`v/pat` 后选区含光标字符**：`d` 删 `"foo b"` 留 `"ar"`——visual
  语义如此，vim 探针一致。
- **`/aa` 跳过光标自身匹配**、**`qa` 内 `v`（visual 可录制）**、
  **`R` 会话的 `.` 重复**、**`i<C-r>/`（搜索寄存器插入）**、
  **`:+0d`/`:;d`/`:1;3d` 等 7 种怪范围**——全部与 vim/文档一致。

### 注释纠偏（防未来误修）

- `objects.rs word_range` 的旧注释自称「never INTO a trailing multi-byte
  char」但 `line_end - 1` 恰恰会——注释与代码一起重写。
- `review_regressions` 中 round20 的 `angle_keys_accept_lowercase…` 钉住
  `<s-x>` 保留 shift 的死映射行为——按 vim 探针改正（规范化为 `X`）。

### 新增已知分歧（接全局序号）

59. **search-motion 的 wrap 提示消息**：vim 显示 "search hit BOTTOM,
    continuing at TOP"；引擎没有该消息通道（bell 语义已对齐——失败必
    响铃；wrap 成功两侧都不响）。
60. **`i<C-r>{reg}` 的 autoindent 展开**：引擎经 `insert_text_at_cursor`
    按行展开缩进；vim 是否字面插入未探（多行寄存器 + ai 场景，极窄）。
61. **`:2,2j` 等值 no-op 的光标**：引擎早期返回不动光标；vim 是否落到
    地址行未探（`:1j` 的落点已实证对齐，等值双地址形态未跟）。

### 性能备注

- bench_probe 全量复跑：`w` 5.9µs/键、1MB 单行 `w` 120µs、hlsearch
  重扫 0.25ms、`:%s` 5.5ms、`n` 3.8µs——与 round18/19 基线持平或略优
  （本轮改动全在提示符路径，非每键热路径）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十九轮检视增补（2026-10-03，回归测试并入
`tests/review_regressions.rs`（+8 例），round13 两条推理钉子按新探针
改正；fuzz 新增 `tests/fuzz_round19.rs`（11.5 万步 + D/C 形态真值表
定向轰炸））

本轮单线通读全部 `src/`（读码列候选 → 探针实证 → 修复）。方法论上
的最大收获是**探针通道的分型**：`-es` 通道对方向键（触发 cmdline 回溯）、
visual 模式（`v` 不生效，round8 已有记录）、多位 count（`99D` 整段吞
键）全部失真，而 **`script` 起的 PTY 通道全部可用**——本轮的 D/C 行
模型就靠它钉死。反向教训一条：**xxd 首组 `0a` 是「清空后的空行」不是
「删了行首字符」**——一次误读让 D 行首形态的结论来回翻转了三轮，最后
靠「同形态重复 3 次 + 换缓冲长度对照」才收敛。

### 语义修复（PTY 字节级探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`D`/`C` 在末行吞掉文件 eol** | `abc\ndef\n` 末行 `D` → vim 文件 `abc\nde\n`；引擎旧模型把 `line_range(last).end`（含尾 `\n`）收进 span——宿主的尾 `\n` 就是文件 eol，静默丢字节。count=1 于行首同类：清空该行但行保留（`"\nb\nc\n"`，旧模型在 sole-line 才侥幸一致）。count≥2 于行首的「覆盖行整行消失」倒是 vim 行为（`2D`@a/b/c → `"c\n"`、`99D` 两行缓冲行首 → 空缓冲），但旧模型对它也判错——**留了一个幻影空首行**。终版模型一句话：count≥2 且行首起 → `line_range(last).end`；其余一律 `line_end(last)`。C 不走整行分支（行首 `C` 清空行进入插入，探针 `"z\nb\nc\n"`；`2C` 行首 → `['new','cccc','dddd']`） |
| 2 | **`Nr<CR>` 产出 N 个换行** | `:h r` 明文 "5r<CR> replaces five characters with a single line break"（PTY 字节探针：`3r<CR>` 于 abcdef → 一个空行+def）。引擎旧实现逐字符 push。autoindent 细节按文档模型推演恰好与裸 `\n` 等价（缩进留在首行、新行未触打的 ai 缩进被 Esc 剥掉）。**连带纠正 round18 的「3r<CR> 三换行」探针误读**（当时大概只验证了引擎自身行为） |
| 3 | **round13 两条「PROBE」实为推理钉** | `probe_count_d_shapes` 钉的 `3D` 行首 → `"\ndddd\n"` 与 `99D` 尾 eol 一起删，均与 PTY 实测相反——注释写 PROBE 但从未跑过真探针（-es 通道对 gg+col0 的组合失真，当年大概率被静默吞掉）。按新探针改正并注明缘由（round8 O 探针误读同款教训：钉子固化误读三轮） |

### 新功能

- **`r<C-E>`/`r<C-Y>`**：替换字符取自下/上行同**显示列**（`:h r`，
  `10r<C-E>` 复制下方 10 字符；宽字符按显示列对齐，镜像 `i_CTRL-E`）。
  任一侧越界（邻行太短 / 光标行字符不足 / 无邻行）整条取消并响铃，
  与 `3rx` 只剩两字符的取消语义同款。visual `r<C-E>` 不支持（原路径
  响铃，维持）。

### 体验修复

- **未设 mark 的 `'z`/`` `z `` 报 E20 文本**（`E20: Mark 'z not set`，
  与 Ex 侧 `:'a` 同文案同通道），不再只有哑铃。
- **incsearch 的 current 高亮改为「光标起首个匹配」**（Enter 会跳的那个；
  旧实现恒标缓冲首匹配，光标在下方时预览误导）。
- **`:j` 于末行（无接缝）纯 no-op**：不再 bump——changelist / `.` mark
  不被空操作污染（与操作符族的无操作纪律对齐）；光标仍落到范围地址行
  （与 `:5` 空命令同款，vim 侧行为未探，挂账见下）。

### 注释纠偏（防未来误修）

- `complete_char_arg` 的非可打印参数分支：注释自称「吞掉继续等待」
  （round3/round7 体验备注也这么记），**代码从第一天起就是「取消+响铃」**。
  PTY 实证 vim 同款（`r<Up>` 后 `x` 走新命令：`llr<Up>x` → `abdef`）——
  行为正确、文档错了九轮，本轮改注释。这个分支没有任何测试钉过行为，
  fuzz round19 的 char-arg×取消键交错面补上。

### 检视中证伪、无需改的（免下轮重查）

- **`2qa…q` 的 count 泄漏假设**：读码怀疑 `2qa` 后 count 灌给录制内的
  第一条命令（vim 把 q 前的 count 丢掉）。实证：count 被
  `complete_char_arg` 尾部共享的 `end_command()` 正确清掉（`2qaxq` 删
  一个字符，与 vim 同），只是靠的是共享尾路径而非显式语义——行为对，
  无需改。
- **`f<CR>`**：以 `\n` 为目标在行内必失败 → 响铃，与 vim 一致。
- **`vy` 光标落点**：引擎留在可移动端——round5 已探针证伪过「vim 落
  选区起点」，本轮不翻案。

### fuzz 抓取的引擎行为注记（非 bug）

- **插入模式光标可停在 line_end 与缓冲幻影末位**（append 打字、
  `<Del>` 并线之后）：addressability 不变量（≤len、字符边界）满足，
  但「不越行尾」只对普通/可视模式成立——fuzz round19 的不变量检查按
  模式放宽后通过。宿主渲染按 `offset_to_line`（钳末行）处理即可。
- **fuzz 的缓冲重置收敛到契约内**：裸换文本只在引擎 idle 且 Normal 时
  合法（真实宿主经 undo/redo 路径换文本，引擎侧有 sanitize；会话中途
  裸换违反宿主契约，首轮即炸出 cursor 8>len 3 的假阳性）。

### 新增已知分歧（接全局序号）

55. **`D` count≥2 行首的寄存器形态**：整行消失走 charwise span（寄存器
    收到含 `\n` 的 charwise 文本）；vim 同形态的寄存器内容/种类未探针
    （PTY 下 `:reg` 面太窄）。极窄面，挂账。
56. **`r<CR>` 于缩进内部**：引擎把字符换成裸 `\n`（缩进留在首行）；
    vim 的「删字符后 i<CR><Esc>」在缩进中段的换行缩进截断规则未探针。
57. **`:j` no-op 的光标**：【第二十一轮已结】vim 9.1 探针确认落地址行
    首非空白（`:1j` 于 `"    abc"` 后 `x` 删 `a`），引擎已对齐；等值
    双地址形态（`:2,2j`）仍挂账（见 61）。
58. **visual `J` 带 count 的锚点**：`:h v_J` 说 count 从「最后高亮行」
    起数；引擎从选区首行起数。`-es`/PTY 均无法可靠驱动 visual count
    组合，探针未决，维持现状挂账。

### 存疑（读码候选，挂账）

- **`publish_incsearch` 的 `current`**：已改为「光标起首个匹配」；但
  Enter 前的预览是否该移动**光标**（vim 的 incsearch 跳跃预览）是更大
  的一块，维持只发高亮（宿主可自行实现）。
- **`:set` 部分成功部分失败**：`ic wrap` 中 `ic` 生效后 `wrap` 失败
  提前 return，跳过搜索规则重发布。影响面≈0，挂账。
- **句子 motion `(` 于句末标点后一位**：落前一句开头还是当前句开头，
  引擎模型与 vim 的差异已在前轮分歧 #8/#46 覆盖，未再深挖。

### 性能备注

- bench_probe 全量复跑：`w` 6.5µs/键、1MB 单行 `w` 164µs、hlsearch
  重扫 0.35ms、`:%s` 6.5ms、`n` 3.6µs——与 round18 基线持平（本轮改动
  均在 span 端点计算，非热路径）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十八轮检视增补（2026-10-03，回归测试并入
`tests/review_regressions.rs`（+6 例），config 单测内联 +1；fuzz
沿用第十七轮的 16 种子套件——本轮无新增不变量机制）

本轮流程：先清零 clippy（**此前仓库 clippy 并非全绿**——1 个 never_loop
deny 级 error 压着，4 个 warning 散在 lib/tests），再全量通读 + 一批
异常操作/异常数据探针（递归宏自录 `qa@aq`、`99999999@q` 预算、insert
`<C-r>` 空寄存器、空缓冲 `:%s/^/x/`、空缓冲 `dip`/`guu`、emoji 家庭
簇的 `x`/`~`、`viwr<CR>`、括号失衡 `( ( )` 的 `ci(`……全部通过），
最后在导航键路径抓出三个同源 bug。

### 语义修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **方向键泄漏进 `.` 重放记录** | `navigation_key` 绕过 `end_command`，`<Down>` 滞留未提交 recording，被下一条修改命令烤进 `last_change`——`<Down>x` 再 `.` 重放成「下移+删字符」。vim 的 redo 缓冲永不含纯 motion。修：motion 分支收尾走 `end_command()`（visual 的累计录制分支不受影响） |
| 2 | **`<Del>` 不可被 `.` 重复** | 同源：Del 分支从不提交 change record，`.` 重放的还是上上条命令；且滞留的 Del 键同样污染后续记录。修：同上，走 `end_command()` |
| 3 | **`d<Down>` 悬空失效** | 操作符待决时方向键直接移动光标（d 静默取消，下一键响铃）。vim `d<Down>` = `dj`。修：op 悬挂时路由 `execute_command(CmdKind::Motion)`——计数合并（`2d3<Down>` 等价 `d6<Down>`）、linewise span、失败 bell 全部复用既有管线 |
| 4 | **`let mapleader = "="` 静默丢弃** | `split('=').nth(1)` 取到前两个 `=` 之间的片段（` "`），trim 后为空，leader 保持默认反斜杠且无迹（`ignored` 也不计）。修：`split_once` 取第一个 `=` 之后的完整尾部 |

### 检视中证伪、无需改的（免下轮重查）

- **`ci"` 单引号行**（positions 奇数尾巴）：`chunks(2)` 丢奇尾 + 兜底
  取行内首对，探针 bell 正确——本轮顺手把兜底循环改写为
  `first_chunk::<2>()`（原形是 clippy never_loop：for 循环体必 break，
  永不迭代第二轮；行为等价，纯形状修复）。
- **`ci(` 于括号失衡 `( ( )`**：内层配对正确（探针）。
- **`ci"` bell 后的 `X`**：读码时误判为「无操作」，实际删除的是引号
  本身（光标在引号后一位，X 向前删）——探针证伪候选。
- **异常数据/操作面**：`3r<CR>`（三换行替换）、`viwr<CR>`、`m1`
  （数字标记静默忽略）、`yy` 后 `A<C-r>"`、`:1,2d 0`（count 0 = 无
  count）、空缓冲全家——探针全过，无新账。

### 体验备注

- `d<Down>`/`d<Up>`/`d<Home>` 等（操作符×导航键）第一次可用；
  `<Del>` 进入 `.` 体系。对手感影响最大的是方向键不再「粘」进下一条
  修改的重放——此前任何「方向键移动后修改再 `.`」的序列都会多动一次。
- rc 文件里 `let mapleader = "="`（及任何含 `=` 的值）不再无声失效。

### 性能备注

- bench_probe 复跑：`w`/`n`/`:%s`/`gq` 与既有基线持平；100x `x` 的
  233µs/key 是朴素 String 宿主的 O(n) 行查找 + memmove（NOTES 既有
  结论），非引擎回归。无新增热路径。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十七轮检视增补（2026-10-03，回归测试在
`tests/parity_round17.rs`（23 例）、fuzz 在 `tests/fuzz_round17.rs`
（11.5 万步 + 宿主事件注入 + 巨 count 文本对象预算断言））

本轮三路并行读码（state / ops·motions·objects / cmdline·insert·search）
加逐条探针复核。**两条读码候选被字节级探针证伪**，是本轮方法论上的
关键教训：①`g_` 于纯空白行——候选声称引擎的 Inclusive(line_start)
是 bug（"vim 只删到光标"），实际 vim 9.1 在 `   ` 上 `dg_` 自 c1/c2/c3
分别删 1/2/3 字符，**引擎原样正确**，候选的探针键位有误；改动半途
被探针矩阵拦下回滚，行为已钉进回归测试防再犯。②`2>>` 的「count =
sw 倍数」假说——实际 count 是**行数**（`2>>` = 位移两行），首轮
探针缓冲只有 1 行才看起来像 no-op。

### 语义修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **宏停止的 `q` 残留 `.` 录制** | `qaxq` 后按 `J`：停止分支早退不提交录制，残留的 `q` 烤进下一个修改的 `last_change`——`.` 重放 MacroRecord 把引擎**楔进永不闭合的录制态**（后续按键全被吞）。修：停止分支清 `recording`/`recording_mutated`/`register`。`"a` 寄存器前缀同路泄漏一并清 |
| 2 | **`@a` 期间抑制 redo 提交，`.` 重复宏前的修改** | vim 的 redo 模型：宏按键等同键入，`qaxq~0@a.` 后 `.` 重复宏内的 `x`（探针 ['def']）。引擎 `replaying` 一刀切抑制了 `@` 的录制/提交。新增 `replay_records`：`.` 自身重放保持抑制（不自我扩展），`@` 回放录制照常——宏内最后一次修改成为 `last_change`。回放路径的 Text 步骤（宏内 insert 会话）同步补录，否则 `.` 重放出空 insert |
| 3 | **`"+` 寄存器写入死槽** | `"+yy` 落进 named 槽而 `get_for_paste` 只读宿主剪贴板——yank 后 `"+p` 贴出**外部旧内容**，roundtrip 永远不成立。修：三个寄存器漏斗（delete_span/yank_span/块 yank）显式推 `clipboard_write`；读侧宿主剪贴板为空时回退 named 槽镜像（无剪贴板宿主保 roundtrip） |
| 4 | **文本对象前的 count 被静默丢弃** | `d2aw` 等价 `daw`（`objects::range` 无 count 参数）。修：**从 span.end 重复扫描对象**——这一个模型同时复现 vim 的全部 count 行为：`v2aw`="foo bar "、`v2i"` 两对引号、`v2i(` 攀到外层块（内层块 end 落在外层闭括号上，跨过它重扫）、`2iw`="foo "（词+尾随空白串）、`3iw`="foo bar"——连 vim 自身的 iw 计数怪癖都精确对齐 |
| 5 | **句子对象 `is`/`as` 空白归属整面偏差** | `is` 吞句尾空白（vim 9.1 `dis` 止于句末标点，`"Aaa. Bbb."` 留 `" Bbb."`）；`as` 缺两条规则：尾随空白在**段界（空行）封顶**、缓冲**末换行不算尾随空白**（`das` 于末句走前导回退，缓冲保持单行）。前导回退不跨行首（`["Aaa.","","Bbb."]` 光标 Bbb 只删句子——逐字节探针 ['Aaa.', '', '']） |
| 6 | **insert `<C-w>` 于行中空白每次剥 1 字节并错误并线** | 行首到光标全空白时 `prev_word_start` 跨行，落进 join 分支删 `at-1..at`（最后一个空格）。vim 9.1：一笔删掉行首到光标整段（自动缩进清除惯用法，`o` + `C-w` 清空行）；并线只发生在 col 1。修：全空白前缀整段删（行内不并线，块会话安全） |
| 7 | **`<<`/`>>` 字节剥离模型与 vim 列模型分歧** | 旧模型 tab=整 sw 单位、右移恒插裸 tab——ts≠sw、混合缩进、noet 全错（探针矩阵：noet ts=8 sw=4 下 `<<` 于 `\tx` → vim `    x`、`\t\tx` → `\t    x`；`>>` 于 col5 → `\t `）。修：缩进**显示列** ± sw，按 tabstop 重表达（凑整 tab + 余量空格；et 全空格）；真空行跳过、纯空白行参与（`<<` 清空、`>>` 追加——vim 同） |
| 8 | **Ex 裸 `+`/`-` 偏移报 E1247** | `:+d`/`:-d`/`:5+d`/`:.-d`——vim `:h :range`「数字省略 = 1」。round15 的 E1247 修复（数字溢出方向）误伤了空数字串。修：`end == 0` 即裸符号取 1，溢出路径保持 E1247 |
| 9 | **`:s`/`:d` 尾 count 缺失**（`:s`）/无数字边界（`:d`/`:y`/`:j`） | `:s/a/X/ 3` = 从范围末行起数 3 行（`:1,2s/a/X/2` = 行 2-3）；`boundary_cmd` 不认数字边界，`:d2`/`:j2` E492。`:d`/`:y` 的 count 语义已在（round 早前），`:s` 补齐同一规则 |
| 10 | **`:nohlse` 族前缀缺失** | vim 缩写规则覆盖 `nohlsearch` 全部无歧义前缀，旧表停在 `nohls` |

### 检视中证伪、无需改的（免下轮重查）

- **`g_` 纯空白行**：见篇首——Inclusive(line_start) 就是 vim 行为。
- **`N%`/`1%` 落点列**：`apply_motion_result` 统一改写首非空白，与 vim 一致。
- **`ip`/`ap` 以空白行为段界**：vim 文档明文例外条款，round15 的修法正确。
- **`ci"` 光标在闭引号上**：vim "search from start of line" 取包含对，引擎一致。
- **char-argument 等待键的用户映射**：`:nmap x ihello` 后 `rx` 替换字面
  `x`（探针），引擎不 remap char-arg 参数——行为一致，无需守卫。
- **句子对象于光标在句间空隙/空行上**：vim 行为退化（`dis` 于空行删掉
  空行、光标在句尾空白上只删空白），引擎不复刻，见分歧 #46。

### 引擎健壮性（读码 + 探针）

- **文本对象 count 的攀爬合并**：内层块（`i(`）重复扫描的探针点落在
  同层闭括号上会零进展；跨过闭括号重扫 + min/max 合并（起止双向），
  `v2i(`/`v2a(` 对齐 vim 的「攀层」语义。
- **`replay_records` 生命周期**：管线守卫溢出与排空两个出口同步复位，
  嵌套（宏内的 `.`）保持外层宏的录制活性。

### 新增已知分歧（接续前表编号）

46. **句子对象在句间空隙上的退化行为不复刻**：vim 于空行/句尾空白上
    的 `is`/`as` 有怪癖（空行 `dis` 删掉空行本身、光标在空白上 `dis`
    只删空白）。引擎对这些光标位置给出安全端点（no-op 或句级 span）。
47. **`dis` 于带缩进的首句多删前导空白**：vim 保留 1 个缩进空格
    （`"  Aaa. Bbb."` 的 `dis` → `" Bbb."`）；引擎剥全部前导空白 →
    `"Bbb."`。仅首句带缩进时可见。
48. **`:s` 完成消息格式**：引擎 `{n} substitutions`；vim
    `{n} substitutions on {m} lines`。纯文案。
49. **拒绝范围的命令忽略范围**：`:1,2set ts=8`/`:1,2reg` 照常执行；
    vim 报 E481 No range allowed（`:w` 族的丢弃已在 #45）。
50. **`:s` 的 `\` 分隔符**：vim 拒绝（E10 族）；引擎当普通分隔符走
    进转义折叠，正则编译失败哑铃。`"`、`|` 同族。
51. **Ex 字面地址溢出**：`:<21 位数字>d` vim 溢出回绕后删第 1 行
    （探针）；引擎 E16 无操作。引擎行为更安全，不追。
52. **`v2i"` 语义**：vim 的 2i" 等价一对完整引号串（含引号）；引擎
    重复扫描覆盖两对。引号对象 + count>1 的极窄面。
53. **`:@:` 后的 `.`**：vim 探针 `.` 无操作（redo 缓冲被 `@:` 复位？）；
    引擎 `.` 重放该 `:s`。极端面，待探针定论。
54. **`so3` 式 sort 直连数字**：vim E488；引擎把 `2` 当旗标静默忽略。
    sort 无 count 语义，无害漂移。

### 存疑（读码候选，探针未定论，挂账）

- **Unicode 空白分类**：`word.rs` 的 `char_class` 用 `char::is_whitespace`
  ——NBSP/U+2028 归空白；vim 的 motion 空白类只有 ASCII。触发面：
  `"a\u{a0}b"` 上的 `w`/`iw`。需要一组 NBSP 探针定 vim 行为再决。
- **`char_arg` 等待键的 `CmdKind` 双重检查**：`complete_char_arg` 的
  Find/JumpMark 两臂近乎复制（坏味道挂账，无行为差异）。
- **`replicate_count_insert` 的 clamp 失配**：span 走未 clamp 的
  copies、替换文本走 clamp 后的 copies——需要「单次 R 会话键入
  >8MB + count≥3」才可达，现实不可达；两条腿应共用同一 copies（挂账
  为一致性清理）。

### 体验备注

- `:+`/`:-` 现在像 vim 一样逐行移动地址（`:5` 单地址跳行的姊妹路径），
  失败时 E16 文本落状态栏而非哑铃。
- `"+p` 首次可用：gpui-vim / crossterm-vim 的宿主剪贴板 roundtrip
  不再依赖宿主自行桥接 `"*`。
- `<<`/`>>` 在 noet + tab 缩进的代码库里不再「越缩越小」——tabstop
  感知的重表达是本次对真实项目手感影响最大的一条。

### 性能备注

- `object_span_count` 的重复扫描每次至少推进一个字符（`next.end >
  range.end || next.start < range.start` 守卫），巨 count（10⁹）在
  span 触达缓冲末尾后即 break——fuzz 的预算断言钉住终止性。
- `shift_line` 每行 O(缩进宽度)（不含正文），旧实现同阶；重表达
  分配一次字符串，无可测回退。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十六轮检视增补（2026-10-02，回归测试在
`tests/parity_round16.rs`（17 例）、fuzz 在 `tests/fuzz_round16.rs`
（11.5 万步 + 每步 1/8 概率宿主事件注入 + 巨 count 空复制预算断言））

本轮单线读码（全部 `src/` 重读一遍）加探针。两条读码候选被探针**证伪**
免修：`ge` 于无词尾可寻时落词首（vim 同款落法，P1/P2/Q7——`prev_word_end`
从头扫起的行为恰好正确）；`~`/`x` 的计数跨行（vim 计数到行尾即停，
引擎的 limit 守卫已对，P6-P9）。fuzz 的**宿主事件注入**（拖选/点击/
IME 文本/IME 替换在任意模式投递）是本轮最大收获：首轮即抓出两个
读码扫不出的宿主路径不变量违规。

### 语义修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **空文本 linewise 计数重复整段缺失** | `3o<Esc>` 只开 1 行（`rep.text` 为空即放弃）；vim 9.1 开 count 行（探针 R1/R7）。复制单元改纯换行（空复制**不带缩进**——vim 对未触打的 ai 行剥缩进）；charwise 空会话（`3i/3A/3R`）维持 no-op（Q3-Q5）。导航守卫从 `insert_change_pos`（首打字位，空会话为 None）改挂 `InsertRepeat.anchor`（会话起点位），空会话同样受箭头漂移保护 |
| 2 | **autoindent 空行生命周期（vim `did_ai`）整面缺失** | vim 的「未触打的纯缩进行」规则引擎没有：`o<Esc>` 留缩进（vim 留纯空行，S1/S6）、`o<CR><Esc>` 两行都留（vim 都空，S3）、`o<BS>` 只删一字符（vim 一笔画掉整段，S5）、`cc<Esc>` 留缩进（vim 空，S8）。新增会话级 `insert_did_ai`：o/O 开行、linewise change 恢复缩进、`<CR>` 携 ai 拆行时置位（置位点统一在 `begin_insert` **之后**——会话起点清位）；任何非换行打字清除（S4：打满再删光仍保缩进）。三条生效路径：Esc 剥当前纯空白行（光标落行首）、CR 拆行前剥被拆行的陈旧缩进、BS 于行尾一笔删整段 |

### 语义修复的跟进（次序交互，回归抓出）

- 剥离原置于 replicate **之前**：光标被移到行首后 `anchor` 守卫把
  `3o<Esc>` 的复制整段跳过。现 replicate 先行（非空复制读打字行缩进
  ——探针 R2 的 `3ofoo` 三份都带缩进；anchor 校验需要光标未动），
  剥离后置并**同时覆盖打字行与光标行**——计数复制把光标带到末份复制
  上，打开行的缩进仍须剥（R1）。上方行剥离后按删除字节数平移光标。

### fuzz 抓取的引擎不变量 bug（宿主事件注入，2 个）

| # | 问题 | 根因 |
|---|------|------|
| 1 | **`replace_range` 非边界 range 透传 + 光标 delta 饱和** | IME 提交替换路径把宿主 range 原样写进 buffer——宿主从 UTF-16 坐标换算可交来 mid-char 字节偏移，裸写 panic 宿主。另：光标调整 `text.len().saturating_sub(range.len())` 把**缩短**替换的负 delta 钳成 0，光标留在越过新缓冲末尾的位置。修：range 先 floor 到字符边界（与 `set_cursor_offset` 同款防御）；delta 改带符号；出口 floor 收尾 |
| 2 | **`edit_insert` 漏调 `visual_anchor`/`cmdline_visual`** | 两个裸字段活在 marks 之外，错过 `adjust_insert` 的相对平移——visual 选区存活期间的宿主 IME 插入把锚点留在新插文本的多字节字符中间（delete/replace 漏斗经 `refloor_stored_offsets` 有收尾，insert 漏了）。补相对平移 + 同款 refloor |

### 引擎健壮性（宿主 API 守卫，非探针）

- **`set_visual_range` 于 insert 会话/提示符期间撕裂模式机**：宿主拖选
  把 `self.mode` 整个改写成 Visual，而 `insert_session`/cmdline 状态
  仍存活——Esc 走 `exit_visual` 但 undo 组与 `last_insert_exit` 永不
  闭合。现仅 Normal/Visual 接受拖选。
- **`set_cursor_offset` 于提示符期移动缓冲光标**：Ex 范围默认地址读
  这个偏移，vim 在 cmdline 期忽略点击——同款忽略。

### 体验修复

- **失败删除/接合族响铃**（vim 同款）：空行 `x`、行首 `X`、空行 `D`、
  EOF 的 `J`/`gJ` 旧实现全静默。`s` 的删除半失败响铃但照常进插入；
  `join_lines` 返回执行数——`3J` 于 EOF-1 对够不着的接缝响铃，
  `2J`（单接缝）成功不响（探针 R3 无铃，顺带钉住）。
- **showcmd 补 char-arg 命令字母**：`r`/`f`/`t`/`F`/`T`/`m`/`q`/`@`/
  `'`/`` ` `` 等待参数期间旧实现什么都不显示（vim 显示部分命令如
  `4r`）。新增 `CharArgCmd::pending_key`，参数已知时一并显示。

### 性能修复

| # | 问题 | 修复 |
|---|------|------|
| P1 | **`:sort` 每次比较重新分配排序键** | `sort_by_key(lower(s))` 对每比较做一次 `to_lowercase` 分配。改 decorate-sort-undecorate：键每行算一次，dedup 比较同一键列（语义与 round6 探针结论不变） |

### 死代码清理与注释纠偏

- `tests/fuzz_round15.rs` 未用的 `addressable` 辅助、`parity_round12`
  一处 `mut`（两处编译警告）。
- `last_change` 字段注释纠偏：「visual 变更不可重放 (v1)」自 round8
  起不实（`finish_visual_op` 落账后可 `.` 重放），按现实改写。
- `exit_insert` 块复制 delta 保持 isize 直至比较（usize 装箱让净删除
  会话的负 delta 回绕成巨正值，靠「不可能等于 text.len()」侥幸安全，
  现显式判负）；`insert_tab` 局部变量 `sw` 实为 tabstop，改名。

### 被探针证伪/关闭的读码候选（下轮免重查）

- **`ge`/`gE` 无词尾可寻时落词首**：读码推断应失败响铃；探针 P1/Q7
  实证 vim 同样落到 `'a`（`prev_word_end` 从光标前一位起扫、扫尽落 0
  的行为恰好与 vim 一致）。引擎无错，关闭。
- **`~`/`x` 计数跨行**：读码怀疑 `3~`/`2x` 该跨行；探针 P6-P9 实证
  vim 计数到行尾即停——引擎的 grapheme/limit 守卫本就正确，关闭。
- **`2J` 于 EOF-1**：一度想给「部分接合后」补铃；探针 R3 实证 `2J`
  本就只有一个接缝、无失败无铃。按 `requested = count.max(2)-1` 建模
  后自然正确。

### 新增已知分歧（接全局序号）

45. **带范围的 `:w`/`:q`/`:wq`/`:x` 丢弃范围**：`:2,5w` 照常全量
    save。vim 的 `:[range]w` 写部分文件；宿主 `save()` 钩子没有范围
    参数，接入方需知（静默接受 vs vim 的部分写/报错）。

### 悬而未决（更新）

- **insert `<C-r>` 特殊寄存器面**（`C-r .`/`C-r C-w`/`C-r %`…）：维持
  （round15 挂账）。
- **`display_column` 每键 O(行字节)**：维持 0.2（round15 挂账）。
- **config.rs 与 ex_set 的 set 解析两套实现**：维持（round14 挂账）。
- **tck 缺口**：scroll_to_line 最小滚动断言、IME 回程 smoke、空缓冲
  幻影行检查——均维持。
- **`:s` 的未知旗标静默忽略**（`e` 之外还有 `l`/`p`/`#` 等）：并入
  分歧 #42 的旗标子集面，不单独追。

### 体验备注（本轮）

- fuzz round16 的宿主事件注入把「宿主在错误时机调 API」变成常驻压
  力：拖选/点击/IME 四类事件在 insert/cmdline/visual/normal 全模式
  随机投递，两个宿主路径真 bug 全部由它首轮抓出——下轮注入面继续扩。
- 失败删除的响铃让「按键没生效」可感知（gpui-vim/crossterm-vim 的
  bell 通道此前对这族命令从不触发）。
- fuzz round16 与 round15 同预算（16 种子 × 60 轮 × 120 步），全量
  套件 522 例（round15 基线 503）。

### 性能备注（本轮复核）

- bench_probe 全量复跑，与 round14 记录逐项一致：`w` 6.5µs/键、
  1MB 单行 `w` 176µs、hlsearch 重扫 0.30ms、`:%s` 6.1ms、`n` 3.6µs/键
  ——`edit_insert` 新增的 refloor 尾扫（O(存储偏移数)，x 删除路径早
  就在付同款成本）无可测回归；ex_sort 的装饰排序在 bench 量测点之外
  （probe 无 sort 行），量级按构造判断为单调改善。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十五轮检视增补（2026-10-02，回归测试在
`tests/parity_round15.rs`（16 例）、fuzz 在 `tests/fuzz_round15.rs`（11.5
万步 + 巨 count 终止断言））

本轮三个独立读码 agent 分域扫 cmdline/config/options/keymap/key、
ops/motions/objects/word/search、insert/registers/marks/buffer/host/tck，
共产出约 25 条候选。**全部语义修复先经引擎探针实证再改**——其中三条被
证伪或改写：Replace 模式多字节 BS 的位置栈失配（实测正确，读码推断有
误）、`d{` 反向「三行全删」断言（vim 实测留 `bbb`，agent 把 `d}` 的结论
错套给了 `d{`）、`:s` 重放损坏的实际表现与预测不同（真实根因在首次执行
的 replacement 转义缺失，非重放切分）。两条早前修复被新探针**推翻**：
参考宿主 `offset_to_line` 的 mid-char 守卫（round13 #26）实为恒等函数、
`:s` 的转义分隔符修复（round14 #20）只覆盖了模式侧。

### 语义修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`:s` 的 replacement 转义整面缺失** | `:s/a/b\/c/` 得 `b\/c`（vim `b/c`）、`:s/a/b\\c/` 得 `b\\c`（vim `b\c`）。round14 #20 只修了模式侧切分；replacement 走 regex expand（反斜杠不特殊）。现 `unescape_replacement` 折叠 `\{sep}`/`\\`，未知转义保持原样 |
| 2 | **`&`/裸 `:s` 重放损坏 replacement** | 重建用朴素 `split(sep)`：`s/a/b\/c/` 的字段被切碎，重放结果与首次执行不一致（`:h :&` 要求相同替换串）。重放改经共享 `split_escaped_fields` |
| 3 | **越界地址偏移 `unwrap_or(0)` 静默删错行** | `:1+<21 位 9>d` parse 溢出变 +0 → 删第 1 行。vim 9.1 双向报 **E1247** 且不执行（负向同）。`with_offset` 改 `Result` |
| 4 | **`}`/`{` 把空白行当段落边界** | vim 只认**真空行**（探针：`aaa/␢␢␢/bbb` 上 `d}` 三行全删、`{` 落第 1 行）。`next_paragraph`/`prev_paragraph` 换 `is_empty_line`；`d{` 反向留 `bbb` 一并钉住（agent 原断言「三行全删」被探针推翻） |
| 5 | **空行上的 `J` 插多余空格** | `["","def"]` 上 `J` 得 `" def"`（vim `"def"`）。接缝任一侧为空免分隔；`["def",""]` 同 |
| 6 | **`ci"` 光标越行末引号响铃** | 末对闭引号作左回退后无右配对 → no-op。vim 回退到**行内第一对**（`say "hi" then "bye" end` 尾部改 `hi`）。悬空左引号（无右伴）同走该回退 |
| 7 | **`N%` 文件百分比差一行** | 引擎 floor `count*total/100`（0-based）；vim nv_percent 是 1-based **ceil** `(count*total+99)/100`。`2%`@200 行曾落第 5 行（vim 第 4）。`count>100` 按 vim 拒绝 |
| 8 | **键入 `1%` 被当裸 `%`（括号匹配）** | count 塌缩后 1 与「无 count」不可分——承 `1G`→`gg` 改写先例，新增 `Motion::GoToFilePercent`。200 行缓冲 `1%` 曾响铃原地（vim 第 2 行） |
| 9 | **`"_yy` 污染 unnamed 寄存器** | store_yank fall-through 无条件重指 `last`：`"_yy` 后 `p` 贴出 yank 文本（vim 9.1：贴不出任何东西）。`"_dd` 早正确（store 早退），不对称即判据 |
| 10 | **`3R{text}<Esc>` 丢弃 count** | Replace 被排除在 count-repeat 外。vim 重复整个输入并**按字符数覆盖**后续文本（`abcdefghij` 上 `3Rab` → `abababghij`）。退出路径新增 Replace 臂（字符遍历 span，行尾钳制即追加）；含换行会话保守跳过 |
| 11 | **redo-register 缺失** | `"1P` 后 `.` 重贴同一寄存器。vim 的 `.` 递增寄存器号走编号环（`:h redo-register`：`dd dd "1P .` 恢复两行）。`bump_redo_register` 识别 `[count]"{1-8}[count]p|P` 形态改写并回写 last_change |
| 12 | **`<C-h>` 规范 ctrl 形态被吞** | 提示符与插入模式只认裸 `\x08` 字节，gpui 风格 `Key::ctrl_char('h')` 静默无效果（注释自称「folds to Backspace」）。两处统一折叠为 BS；vim 绑定 C-h ≡ BS |
| 13 | **visual 模式无 `'a`/`` `a ``** | 单键 trie Miss：响铃 + 清 pending。vim 中光标跳标记、选区跟随。补 visual 行走 CharArgCmd::JumpMark → goto_motion |
| 14 | **`:s` 旗标 `e` 语义翻转**（记录不修） | 行内无匹配时 `:s/z/q/e` 引擎报 E486；vim 因 `e` 旗标静默。属旗标子集缺失但改变了报错行为，进分歧表 #42 |
| 15 | **config 不识别 `setl[ocal]`/`setglobal`** | rc 行 `setlocal ts=4` 静默落 ignored（运行时 `:setl` round14 已支持）。config 侧补齐四种拼写 |
| 16 | **search_rule 别名表臆造 `isc`、漏 `scs`** | `:set scs` 改变匹配规则但不触发高亮缓存失效。去臆造、补真实别名 |
| 17 | **`:brew` 前缀 E492** | vim 缩写规则全前缀可用（round14 #23 修复面的漏网）；`:br`/`:bre` 因 `:break` 歧义保持排除 |
| 18 | **块寄存器空末行被 put 吞掉** | yank 侧 `join("\n")` 无终止符，但空末行产出尾 `\n`；put 侧 `trim_end_matches('\n')` 连行界一起吃。改 plain split（宿主块 API 可达） |

### 引擎外：参考宿主真 bug（新契约抓出）

- **tck 参考宿主 `offset_to_line` 的 mid-char 守卫是恒等函数**：
  `offset.min(ceil_char_boundary(offset))` 对中点偏移恒等于原值（ceil ≥
  offset），照旧 `[..mid]` 切 panic——round13 #26 声称修掉的问题实际还在，
  因为契约从未测过 mid-char。改 floor 到字符起始。buffer_read_contract
  新增多字节中点断言（char_at/offset_to_line/prev_char_offset 的容忍
  面），`offset_to_line(len)` 放行「钳末行」与「幻影行号」两种实现
  （引擎对两种都容忍，tests 宿主与 tck 宿主各站一边）。

### fuzz 抓取的引擎不变量 bug

- 无新增（本轮 fuzz 的增量键表全部落在已修复路径上；巨 count 终止由
  专用测试断言预算）。`last_matches` 可过期不变量沿用 round13 约束
  （live update 关闭时允许陈旧，消费前重扫）。

### 性能修复

| # | 问题 | 修复 |
|---|------|------|
| P1 | **巨 count 在运动定点上空转** | `999999999e` 在缓冲末字符跑满 10⁹ 次全词扫描（姊妹实现 WordEndBack 有 `next == o break` 守卫，WordEnd 漏了）；`}`/`)`/`{`/`(`/`b`/`w` 同族在端点每步 O(行扫描)。全部补定点守卫，首个不前进的步即停（vim motion 循环同语义）。<150µs 返回 |
| P2 | **tck 参考宿主 `line_range` 每调用三次全文扫描** | 守卫 + total + 枚举各一遍 O(text)，trait 默认的 line_start/line_end/line_content 每动作都过它。并单趟（照抄参考实现的宿主每按键省 2 次 O(全文)） |
| P3 | **`tag_range` 裸 `<` 尾段二次方** | 无 `>` 的模板碎片上每个 `<` 重扫全文尾。find 返回 None 即 break（后面不可能再有闭合） |

### 死代码清理

- `key.rs` parse_angle 尾部逐字段重建自身等值 Key 的 ctrl 块（注释描述
  的是旧实现）；`keymap::Trie::is_empty`（全仓零调用，承 round14 判据）；
  `find_from` 末尾不可达的 till 分支（till 在上方必然 return）；search.rs
  两处 `vim.search.forward = forward`（set_pattern_inner 已赋同值）。
- `daw`/`caw` 逐字重复的换行合并条件块抽 `merges_following_line_break`；
  `:marks` 的 `items()` 双调用并单次收集；`:y` 的无谓解构。

### 注释纠偏（防未来误修）

- `word.rs` `next_word_end` 定点返回曾注释为「no-op/motion 失败」——
  实测 `ye`/`de` 在末词字符上**删/抽该字符恰是 vim 行为**（round12 的
  措辞差点诱导成本轮把它改成 stuck()）。改为「fixed point 语义 + 调用方
  守卫」表述。
- `marks.rs` `adjust_replace` 的 preserve_inner 注释「column-preserving」
  实为**字节**位置保持（等长替换时），与 refloor 的分工写明。
- `host.rs` `scroll_to_line` 契约补「必须最小滚动」：C-e/C-y 自由滚动
  依赖宿主的自律实现，重居中实现会静默禁用它（体验缺口，接 INTEGRATION
  面）。tck 契约对应断言仍缺（悬置）。

### 被探针证伪/关闭的读码候选（下轮免重查）

- **Replace 多字节 BS 位置栈失配**（agent 高置信）：`R中<BS>` 在
  `ax`/`xyz` 上实测恢复正确——等宽字节网格重画 + 偏移平移已自洽，关闭。
- **`d{` 反向三行全删**：vim 留 `bbb`（agent 把 `d}` 的探针结论错套）。
- **巨值负向偏移删行**：`with_offset` 的 saturating_sub 使 `:5-<巨值>d`
  落 0 以内前就因 base 越界报 E16——本轮统一 E1247 后两个方向都显式。

### 新增已知分歧（接全局序号）

42. **`:s` 的 `e` 旗标翻转报错行为**：vim 中 `e` 吞掉 E486；引擎静默
    丢弃旗标照常报 E486。与 `c` 旗标（#40）同属旗标子集，但 `e` 改变
    的是可观测报错而非执行语义。
43. **`3R` 含换行会话不重复**：`3Rab<CR>cd<Esc>` 的重复未实现（span
    与 seam 行模型都假设单行 typing）；vim 会连换行一起重复。保守跳过。
44. **块寄存器文本的 put 路径对 linewise/charwise 寄存器与可视块替换
    的组合仍用 `trim_end_matches`**（state.rs 3711 行）：多空行 linewise
    寄存器在该路径的精确 vim 语义未探针，维持旧行为。

### 悬而未决（更新）

- **insert `<C-r>` 特殊寄存器面**（`C-r .`/`C-r C-w`/`C-r %`…）：现静默
  插空、无 E353 反馈；`%` 已在 host.rs 挂账。涉及 last-insert 文本跟踪
  与 cmdline 侧光标词读取，留待专项。
- **`display_column` 每键 O(行字节)**：`j`/`k` 在 `desired_col == None`
  时每次从行首逐字符重算（1MB 单行场景退化）。修法需在纵向移动里传递
  上一列或按行缓存，动 `apply_motion_result` 面，留 0.2。
- **`:set sw=0`/`ts=0` 语义**：vim 的 `sw=0` 表示跟随 tabstop、`ts=0`
  应报 E487；引擎无下限。与 #41（so 负值）同批，留 0.2 breaking 窗口。
- **`:set ts=4 ?`（值形态 + 空格问号）**：引擎报错且丢赋值；vim 对混合
  形态的接受度未探针。
- **tck 缺口**：scroll_to_line 最小滚动无可执行断言；IME 回程路径
  （take_pending_unknown_chars）无 smoke 覆盖；空缓冲幻影行检查仍在
  （line_count==1 时跳过）。
- config.rs 与 ex_set 的 set 解析两套实现（round14 悬置维持——本轮
  config 侧又补了 setlocal/setglobal，分歧进一步收窄，抽共享函数仍待做）。

### 体验备注（本轮）

- `<C-h>` 在 gpui 风宿主（规范 ctrl 形态）曾是「按了没反应」的哑键，
  现两种投递形态一致折叠 BS。
- visual 模式 `'a`/`` `a `` 从「响铃 + 丢 pending」变为可用的选区跳转。
- 巨 count 误击（`999999999e`）从近似死机变为即时返回。
- 越界 Ex 地址的报错从「删错行/钳制执行」变为 vim 同款 E1247/E16 且
  不动缓冲——错误消息通道（status_message）让宿主状态栏可显示原因。

### 性能备注（本轮复核）

- bench_probe 未复跑（本轮改动不在 motion/search 热路径的量测点上：
  定点守卫只在端点生效、tck 参考宿主不在 bench 路径、tag_range 是
  文本对象慢路径）。round14 的结论（绝对值以同次运行为准）维持。
- fuzz round15 与 round14 同预算（16 种子 × 60 轮 × 120 步），全量
  套件 503 例（round14 基线 486）。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十四轮检视增补（2026-10-02，回归测试在
`tests/parity_round14.rs`（48 例）、fuzz 在 `tests/fuzz_round14.rs`）

本轮三个独立读码 agent 重扫 state.rs 全文、cmdline/config/options/keymap
与 ops/motions/objects/word/search/registers/marks/insert_mode，共产出约
40 条候选；全部语义类修复先在本机 vim 9.1 探针实证（三轮批量探针
脚本，结论见各条目括号）。两个早前轮次的断言被探针**推翻并改正**
（块 `O` 的 round8 探针误读、`:1,2d 3x` 的 round13 终点），一个 round12
悬置项被证伪关闭（`:s//` 空模式），till 家族挖出一个**前所未知的
真实偏差**（operator span 少一个字符——`dto` 旧断言得 "lo world"、
vim 实为 "o world"）。

### 语义修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **till 重复 `;`/`,` 原地假成功** | `t2;` 从光标重扫立即重命中同一目标、moved=true 偏移不动、无铃声。现跳过紧邻目标一次后按查找次数推进（`t3;;` 一次一跳、`t3` 后 `2;` 只前进一个——探针 A2/A4/A6/Q5） |
| 2 | **fresh `Nt{x}` 的 count 数查找尝试而非目标** | `2t3` 曾重命中同一目标只落第一个；vim 数**不同目标**（紧邻目标算第一个：探针 A1/A5/A9 全落第二个 3） |
| 3 | **till operator span 少一个字符** | 光标停靠位被直接用作 span 端点；vim 的 span 延伸到**目标位**（`dto` 得 "o world" 非 "lo world"、`d2t3` 删到第二个 3 前）。MotionResult 新增 `till_target`，span_from_motion 对齐 |
| 4 | **yank 污染 changelist/`.` 标记** | operator 完成路径无条件 `bump()`：`yw`/`yi(` 进 `:changes`、`g;` 落 yank 处。探针 Q2：vim 的 `:changes` 在 yiw 前后条目数不变。改 `bump_if_edited`（空跨度 operator 同步受益） |
| 5 | **`ci"` 的转义判定只看单个前置反斜杠** | `"a\"` 的收尾引号被误判转义、整行失去配对。按反斜杠游程长度 mod 2（探针 P20/Q8） |
| 6 | **`ap` 于末段（无尾随空行）留下段前空行** | `:h ap` 无尾随空白时含前导。探针 P19：[para1,"",para2] `3Gdap` → [para1] |
| 7 | **`"ax`/`"aX`/`"as`/`"<Del>` 丢弃寄存器前缀** | 只进 unnamed。探针 P10/P10b/P10c：vim 的 @a 收到删除文本。`delete_chars` 增寄存器参数 |
| 8 | **`""` 前缀冻结编号环** | `""dd` 只写 unnamed。探针 P18：vim `""` ≡ 无前缀（`""dd` 照常轮转）。归一化为 None |
| 9 | **块 visual `O` 方向弄反** | round8 测试固化的 probe10 是误读；三次独立探针一致：光标**保持所在行**换到块另一列角、anchor 镜像、矩形不变（anchor(0,0)+cursor(1,2) → cursor(1,0)；本几何探针 cur=3:2）。单行块是真实水平换角（(0,2)→(0,0)），曾 no-op |
| 10 | **char/line visual `O` 响铃** | 探针 Q3/Q4：vim 中等同 `o`（镜像端点、无铃声） |
| 11 | **映射劫持部分命令的续键** | `:nnoremap j gj` 后输入 `gj`：引擎让续键 `j` 进映射表、展开重放再次命中 → runaway 熔断、gj 完全失效；`:nnoremap j G` 则 G 跳末行。探针 P6：vim 的续键直接完成 builtin（cur=2:1）。mapping_step 增 cmd_seq 守卫——映射只在命令边界生效 |
| 12 | **no_remap 预算跨守卫泄漏** | 流水线守卫/maxmapdepth 清队列后残余预算让后续按键静默绕过映射。清队列时归零 |
| 13 | **MAX_MAP_DEPTH=100 低于 vim 默认** | vim maxmapdepth=1000；合法的百级展开链曾被误判 runaway。对齐 1000 |
| 14 | **`N:` 的 count 泄漏** | `3:<CR>` 后 `j` 跳 3 行、`3:` 光标不动。探针 P7：vim 把 count 变成范围预填 `.,.+2`（`3:d<CR>` 删 3 行）。现预填范围 |
| 15 | **`N/pat` 的 count 被丢弃** | 曾硬编码 count=1。探针 P8b：vim `3/foo` 跳第 3 个匹配。取消的搜索丢弃 count（探针 P9：`2/x<Esc>` 后 `x` 只删 1 字符）；visual `:` 同 |
| 16 | **`:d`/`:y`/`:j` 尾参垃圾静默执行** | `:1,2d 3x` 删范围、`:d a b` 只取 a。探针 P13/P54/B4：vim 报 E488 且不执行（round13 的注释写着正确行为但 `unwrap_or(0)` 没实现它）。`|` 分隔符同落 E488（`:h :bar`） |
| 17 | **`:d!`/`:y!` 被静默接受** | 探针 P16：vim 报 E477 No ! allowed。消息带完整原行（`E477: ...: 1,2y!`） |
| 18 | **越界地址被钳制而非报错** | `:1000000d`/`:2,99999d`/`:.+99d`/裸 `:99999999` 曾钳到末行照常执行。探针 P15/P50：vim 报 E16 且不动。地址 0/负数仍合法作用于首行（探针 B1/B3） |
| 19 | **`:y a3` 打包形式丢 count** | 寄存器 token 只取首字符。探针 P17：vim 的 `a3` = 寄存器 a + 3 行。共享 token 解析（寄存器+纯数字尾） |
| 20 | **`:s` 的 `\/` 转义分隔符不存在** | `:s/a\/b/x/` 解析切碎。探针 P26：vim 匹配字面 `a/b`。按未转义分隔符切分，反斜杠留在模式里 |
| 21 | **`:s` 的 `n` 标志被忽略并执行破坏性替换** | `:%s/foo//n` 曾真删。探针 P14：vim 报 "2 matches on 2 lines" 不动缓冲 |
| 22 | **`:set` 面缺失** | `:se` 缩写、`:setl[ocal]`（单缓冲= :set）、`:set ts&`/`ts&vim` 重置默认、`:set ts ?` 空格问号查询、裸 `:set ts` 查询——曾 bell 并中止整行（探针 P27/P29/B8/Q16） |
| 23 | **`:bn`/`:bp` 只认精确拼写** | `:bne` E492。vim 缩写规则全前缀可用 |
| 24 | **config：`let mapleader2` 劫持 leader** | strip_prefix 整行吞掉，无关变量静默改绑 `<Leader>`、不进 ignored。词边界检查 |
| 25 | **config：TAB 分隔的 set/source 进 ignored** | map 族早修过 TAB，set/source 漏了 |
| 26 | **config：rc 的 set 注释/查询产垃圾** | `set ts=4 " note` 产出 On("\"")/On("note") 虚增 ignored；`set ic?` 产出垃圾 token。注释截断、查询丢弃；新增 `Setting::Reset`（`set ts&`） |
| 27 | **`<LocalLeader>` 映射静默死掉** | 残留 Named("localleader") 标记键、键盘永远无法产生。现经 `let maplocalleader` 解析（缺省 `\`） |
| 28 | **Replace BS 是裸 LIFO** | BS 后 `<Right>` 再 BS 把属于更后位置的字符写到光标处（曾得 'ayy'）且栈永久错位。条目携带缓冲偏移：栈顶恰为退格位才恢复（追加字符按删除），失配整键 no-op——与探针矩阵 Q8a-e/P23a-d 吻合；换行分割失配（round9 悬置）顺势闭合 |
| 29 | **ai 下 count-repeat insert 丢 count** | `3ifoo<CR>bar<Esc>` 的退出不变式比对原文长度，缩进使缓冲增长超原文 → 静默放弃。探针 P24：vim 得三组 foo/bar。现以展开文本为复制单元 |
| 30 | **`*` 在非词字符上响铃 + 引用陈旧 pattern** | 探针 Q1：vim 搜该字符的**字面**（pattern = 转义单字符）。光标行全空白报 E348（旧行为报 E486 引用无关旧 pattern） |
| 31 | **changelist 去重分支不前移 change_pos** | 新编辑恰落最新条目时指针滞留，后续 `g,` 从错误位置走 |
| 32 | **u/`<C-r>` 不清 desired_col** | 陈旧纵向目标列带进 undo 后的 `j`（JumpBackward/OlderChange 同款处理） |
| 33 | **`replace_range` 悬挂 undo 组** | pub 宿主 API 在 insert 会话外调用时 open_undo 不闭合，下一次编辑并进同一 undo 组、一次 u 撤两步 |

### 死代码清理

- `word::is_non_blank`、`keymap::Trie::is_prefix`（全仓零调用）；
- `search::compile` 的永真 `Option` 分支（签名改返回 builder）、
  `SearchState.last_index` 死状态（只写不读）；
- tables.rs 不可达的 `guu`/`gUU`/`g~~`/`gqq` 及全部 4 键 doubling 行
  （`gu` 第二键即 Hit、cmd_seq 清空，更长的序列永远无法在 trie 累积——
  doubling 走 operator 挂起路径，fuzz_round10 一直覆盖的正是它）与
  normal `<Del>` 行（navigation 拦截器先于 trie，visual 也映射到 d）；
- `ops::span_from_object` 未用的 `_buf` 参数。

### 被探针证伪/关闭的悬置项

- **`:s//x/` 空模式复用「上一条替换模式」（round12 悬置）**：不成立——
  探针 P25b/P25d：vim 用**搜索**模式（`s/b/B/` 后 `s//Q/` 把 b 换成 Q）。
  引擎现行为已正确，悬置关闭。
- **`m^`/`m.` 可设但解析不到**：探针 P22/P48：vim 也静默接受 `m^` 且
  `` `^ `` 仍指向内部状态——引擎与 vim 可观测行为一致，关闭。
- **`gv` 无前次选区（round11 悬置）**：已在 round12/13 实现
  （`NormalCmd::RestoreVisual` + `marks.last_visual` 随编辑平移），本轮
  fuzz 确认路径健壮，记录关闭。

### 新增已知分歧（接全局序号）

38. **递归映射经 builtin 前缀不产生 E223**：`:map j gj`（递归）后输入
    `j`，vim 递归展开至 E223；引擎因 cmd_seq 守卫（修复 #11）在展开的
    `g` 装配后让 `j` 直接完成 builtin——执行一次 gj、不递归。 pathological
    配置下引擎更宽容。
39. **Replace BS 的多次 BS+移动序列**：探针矩阵 9/10 与引擎吻合；P23e
    （`Rab<BS><Right><BS>` 得 'abyz'，引擎得 'axyz'）显示 vim 在恢复后
    的 Right/BS 组合上有额外的栈交互，未建模。
40. **`:s` 的 `c` 交互确认标志不支持**：无宿主确认 UI；现静默当作无 c
    执行全部替换（vim 逐个询问）。使用会改变缓冲，接入方需知。
41. **`:set so=-1` 仍拒绝**：scrolloff 字段为 usize（下游 gpui-vim/
    crossterm-vim 直接读），改 i64 是破坏性变更；vim 允许 -1（居中语义）。
    留待 0.2 的 breaking 窗口。

### 悬而未决（更新）

- config.rs 与 ex_set 对 set 语法的解析**仍是两套实现**（本轮补齐了
  config 侧的注释/查询/重置，分歧收窄；抽共享解析函数仍待做）。
- 命令行提示符无光标模型、`|` 在 `:s` 参数中仍按字面读（`:h :bar`
  语义本身如此，维持）、`state.rs` 拆文件（现 3900+ 行）、Tab 显示宽度、
  visual charwise + linewise 寄存器的 `p` 精确语义——均维持。

### 体验备注（本轮）

- 错误消息对齐 vim 的错误码族：E488（尾参）、E477（多余 !）、E16（越界
  范围）、E348（无词可搜）——宿主状态栏现在能显示与 vim 相同的文案，
  `:5>` 这类输入的报错也从误导性 E16 修正为如实 E492/E16 组合。
- `3:` 现预填 `.,.+2` 范围（vim 同款），提示符里看得见、可继续编辑。
- parity_round8 的两个 O 测试与 round3/round13 的三条断言按新探针
  改写并注明修正缘由（round8 的 probe10 是一次探针误读，测试把误读
  固化了三轮）。
- fuzz round14 一半种子预注册 `nnoremap j gj`，让 cmd_seq 守卫与映射
  展开重放持续接受随机序列轰炸。

### 性能备注（本轮复核）

- bench_probe 全量复跑，**与修复前的基线提交逐项一致**（w 6.0 vs
  6.3µs/key、1MB 行 173 vs 186µs、hlsearch 重扫 0.30 vs 0.30ms、
  `:%s` 6.1 vs 5.8ms、`n` 3.6 vs 3.7µs）——本轮对 motion/search 热路径
  的改动（find_from 重写、`*` 链）无可测回归。与 round13 NOTES 记录的
  2.1µs 相比整体漂移约 3 倍，同一提交内可复现，判定为机器状态差异
  （绝对值以同次运行为准，round13 的数字作废）。
- fuzz round14 与 round13 同预算（16 种子 × 60 轮 × 120 步），耗时持平。

## 〇⁺⁺⁺⁺⁺⁺⁺⁺⁺、第十三轮检视增补（2026-10-02，回归测试在
`tests/parity_round13.rs`（30 例）、fuzz 在 `tests/fuzz_round13.rs`）

本轮三个独立读码 agent 扫 cmdline/config、ops/motions/objects/word、
insert_mode/keymap/tables/tck，加上自读 state.rs 全管线；fuzz 首次把
宿主鼠标事件（`set_cursor_offset` 点击、`set_visual_range` 拖选）与大写
mark、宏字符面纳入键表——五个新 bug 全部由这轮 fuzz 的新不变量抓出。
所有语义声称均先在本机 vim 9.1 探针复核（含块插入 BS 的 5 连探针，
推翻了第八轮的一条既定预期）。

### fuzz 抓取的引擎不变量 bug（5+1）

| # | 问题 | 根因 |
|---|------|------|
| 1 | **块复制落进多字节字符**（宿主 insert_str panic） | `3o` 残留的 count-repeat 曾在块会话 `exit_insert` 时先插 83 字节，块复制偏移只按打字行增量平移、漏掉这 83 字节。三层修：`begin_insert` 清残留 repeat；`replicate_count_insert` 对块会话退出；复制偏移落地到字符边界兜底 |
| 2 | **visual 模式宿主点击塌缩选区** | `set_cursor_offset` 曾把锚点也改成点击点，选区变零宽、`d` 只删一个字符。现按 `:h visual-use` 只移光标并同步活动选区 |
| 3 | **`cmdline_visual` 快照泄漏** | visual `:` 的执行/取消之外的关闭路径（如 `@:` 重放间接退出）不消费快照，宿主经 `visual_selection` 看到陈旧冻结选区。快照只由 prompt 消费 + `handle_key` 末尾清扫 |
| 4 | **`visual_selection` 在 cmdline 模式回报活锚点** | 应回报 prompt 时刻的冻结对（活动锚点可能已被后续编辑顶坏）；快照对已纳入落地清理 |
| 5 | **`marks.active_visual` 不随编辑调整** | 活动选区不在 `for_each_pos` 行走里——visual `:` 下执行 Ex 删改文本后它完全未平移，`parse_range` 拿到中间字符偏移 |
| 6 | **`refresh_highlights` 是死代码** | 大文件宿主 API 被自己的 `hlsearch_live_update` 守卫短路：关闭逐编辑重扫后，显式刷新永远空操作、匹配缓存永不更新。守卫只留在逐编辑路径 |

### 读码+fuzz 修复（均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 7 | **`~` 删组合符与 ZWJ 家族尾巴** | 消费整个字素簇只写回基字符映射——`e`+U+0301 变 `E`（3 字符变 2）、emoji 家族掉成员。簇尾切片原样带回；`gu`/`gU` 本就正确（不对称已消除） |
| 8 | **linewise 粘贴丢尾部空行** | `trim_end_matches('\n')` 吞掉全部分隔换行：寄存器 `"a\n\n`（行 a、""）粘出一行。改 `strip_suffix` 只消费一个 |
| 9 | **visual Y/D/X/C/S 是字符语义** | 探针：v_D/v_X 删覆盖**行**、v_Y 行复制、v_C=v_S 行修改（v_x/v_s 保持字符）。新增 `VisualCmd::LinewiseOp` 走覆盖行跨度 |
| 10 | **块 `A` 中带行不补齐** | 探针 `"123456"/"12"` 块列 0-2 → `"123 X456"/"12  X"`：短行补齐到**追加列**（col_hi 排他端点），旧过滤只补 col_lo 之前的行。附带修 `then_some` 急切求值的减法溢出 |
| 11 | **`ap` 只吞一个空行** | 方向化：文本行吞**全部**后续连续空行；空行会话吞**整个下一段落**（不含它的尾随空行）——三个探针形态全对齐 |
| 12 | **块会话 BS/发散语义与 vim 相反** | 5 连探针：BS 只在打字末尾删最后一个打字字符（中途 BS 空操作，绝不啃行内容）；打字被 `<Del>`/`<C-w>` 删掉的发散会话**不复製**。退出侧 delta 守卫（打字行增量 ≠ 副本长度 → 跳过复制）+ BS 限位。第八轮「发散仍复制」的预期按探针改正 |
| 13 | **`:1,2d 3x` 删错行集** | 尾参垃圾 `unwrap_or(1)` 把 typed range 重锚到末行。0 = 无 count |
| 14 | **`:w!`/`:wq!`/`:x!`/`:noh!` E492** | bang 形式缺失（最常打的命令之一）；`:x` 未修改不写留给宿主（注释契约） |
| 15 | **Ex 缩写缺失** | `:de/:del/:dele/:delet`、`:ya/:yan`、`:jo/:joi`、`:su…:substitute`（静态拼写表；`:ju` 是别的命令，见分歧 #26） |
| 16 | **空操作顶掉 `.` 的重放记录** | `begin_edit` 推测性置位 `recording_mutated`：空行 `x`、空寄存器 `p` 后 `.` 重放空操作而非上一个真修改。置位点移进三个 `edit_*` 漏斗 |
| 17 | **visual `3"<Esc>` 后 count 存活** | Esc 检查移到 `"` 寄存器前缀之前（normal 模式的次序本来就对） |
| 18 | **`q`/`q{reg}` 的 count 泄漏** | 录制开始/停止不清 count，`2q…q` 后 `x` 删两个字符 |
| 19 | **`q/` 劫持宏槽** | `q{reg}` 只收 a-zA-Z0-9；`qA` 追加录制（vim 语义，旧版开独立 A 槽） |
| 20 | **`:5 +2d` E16 / mid-range `%` 语义反转** | 地址内空白跳过（vim get_address 同）；`%` 中段展开为 `1,$`（旧版当 cursor line） |
| 21 | **行首 `"` 注释 E492；`:set ts=4 " note` 误报** | 整行注释静默跳过；`:set` 参数遇 `"` 停止解析 |
| 22 | **insert mapping 前缀吞字符** | `:imap jk <Esc>` 打 `jx`：`j` 被 mapping 前缀答 Consumed、`x` 打破前缀后单槽 IOU 只剩 `x`——j 从缓冲丢失。`pending_unknown_char` 改队列 `take_pending_unknown_chars()`（保留单字符拼写） |
| 23 | **命令行提示符缺编辑键** | C-h=C-BS 规范化；C-w 删词、C-u 清行（cmdline window 级的完整编辑模型仍不做，见悬置）；历史去重改非连续（vim 搬旧重复到末尾） |
| 24 | **`o`/`O`/`cc`/`S` 无视 noautoindent** | 缩进复制/保留受 `autoindent` 选项门控（引擎默认 ai=true 掩盖了这点） |
| 25 | **`let g:mapleader` 静默失效；TAB 分隔的映射进 ignored** | `g:` 作用域拼写生效；map 命令边界与 lhs/rhs 切分接受 TAB |
| 26 | **TCK 参考实现 offset_to_line 中间字符切片 panic** | 宿主逐字复制参考实现会在普通偏移上崩；守卫 + 契约新增「幻影行容忍」条款（`line_range(line_count)` 返回 `len..len`） |
| 27 | **map_depth 跨调用棘轮** | Wait/Done 提前返回不重置：自引用 mapping 数十键后假阳性触发 runaway、清掉刚打的文本 |

### 已证伪的审查发现（探针 vs 读码推断）

- **`:actions x` 误路由进 `:action` 桥**：不成立——`strip_prefix("action")`
  后的边界过滤器要求空格开头，`"s x"` 不匹配，走 E492。
- **块 `A` 对「恰好在 col_hi 结束」的行也要补齐**：不成立——行宽=col_hi
  （排他）时无需补齐，`range.end` 即追加列。

### 新增已知分歧（接全局序号）

35. **块 visual `l` 不越过行尾**：vim 的块可进入虚拟列（短行上 `2l` 把块
    右缘推到 col 2，探针 `"123456"/"12"` 上 `<C-v>j2lA_X` 补齐到 col 2、
    追加在 col 3），引擎的 `l` 停在行尾——需要虚拟列块几何，暂缓。
36. **insert 模式左右方向键不跨行**：vim 的 `<Left>`@col0 去上一行行尾。
    引擎保持原地（这也让块会话更安全）。
37. **命令行提示符无光标模型**：Left/Right/Insert/Tab 补全不做；缓冲区
    只是 append/pop。C-w/C-u/C-h/上下历史已覆盖日常。`:ju[mp]` 仍见 #26。


## 〇⁺⁺⁺⁺⁺⁺⁺⁺、第十二轮检视增补（2026-10-01，回归测试在
`tests/parity_round12.rs`（37 例）、fuzz 在 `tests/fuzz_round12.rs`）

本轮双线并进：两个独立读码 agent 扫 `cmdline.rs`/`options.rs`/`config.rs`
与 `word.rs`/`objects.rs`/`motions.rs`/`search.rs`/`registers.rs`/
`marks.rs`/`insert_mode.rs`，所有声称先在本机 vim 9.1 探针复核、再在引擎
上实证，**两条腿都对上才修**——agent 的读码推断有约 1/3 被探针证伪（见
文末）。fuzz 键表覆盖本轮全部修复面，新增 Ex 范围垃圾参数 600 轮随机
拼接轰炸。

### 新修复（除注明 fuzz/悬置闭环外，均 vim 9.1 探针实证）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **Ex 范围扫描器多字节 panic**（用户可触发） | `:'中d`/`:,'中d`：扫描器 `i += 2` 把 中（3 字节）切在字节中间，`line.split_at` panic。现按字符推进；非法 mark 名报 `E78: Unknown mark`（vim 同款） |
| 2 | **地址偏移只解析第一段** | `:5+2+1d` 应删第 8 行，引擎删第 5 行。偏移链现累加（`with_offset` 循环消化 `+n`/`-n` 段） |
| 3 | **`w`/`b` 停驻纯空白行** | 探针：`abc␤   ␤def␤` 上 `w` 落 def 的 `d`——vim 只停**真空行**。`is_empty_line` 替换 `line_is_blank` 判定 |
| 4 | **`e` 于末词尾返回 `buf.len()`** | `ye` 吞尾部换行（寄存器得 `"c\n`，`p` 劈行）。现返回原地（motion 失败语义），`ye` 得 `"c` |
| 5 | **`d$`/`dg_` 于空行吞换行** | 空行的 Inclusive 端点是 `\n` 位置，扩 1 字节把两行并一行（探针：no-op）。空行/纯空白行改走 Exclusive 空跨度；ops 侧加通用防护：Inclusive 端点遇 `\n` 不吞 |
| 6 | **`aw` 于空白上只取空白段** | 探针矩阵：`yaw`@词间空白 = `"   bar"`、`daw` 删之剩 `"foo"`、空白/真空行上对象跨行伸到下一词、`daw` 连下一行换行一并删（3 行变 1 行）。`blank_run_plus_next_word` 承接全部形态，Delete/Change 侧并线 |
| 7 | **`d-`/`d+` 于缓冲边缘删当前行** | 探针：`d-`@首行、`d+`@末行均 no-op——无相邻行 motion 应失败；原 saturating 吸收越界后 linewise span 误删 |
| 8 | **`2*` 只跳一个匹配** | count 硬编码 1 未传入 `jump_to_match`。`2*` 落第二个下一匹配（探针 col 9） |
| 9 | **`2gN` 自我重复**（读码确认） | backward 迭代 `from = range.start` 恒被自身包含，永远重选同一匹配。第二步起 strict（跳过 containing 偏好）+ 补 wrapscan（首个匹配上 `2gN` wrap 到最后） |
| 10 | **`ci"` 在两组引号之间开下一对完整引号** | 探针：`say "hi" then "bye"` 光标在 then 上 `ci"X` → `say "hi"X"bye"`——**左引号作开引号**配下一引号；左侧无引号才取右侧第一对。原实现清空 bye |
| 11 | **带范围 `:s` 重放丢范围** | `:3,4s`（空参重放）落在光标行——visual 预填 `:'<,'>s` 是高频路径。重放改走 `ex_substitute` 携带给定范围 |
| 12 | **`:s` 不更新搜索状态** | 探针：`:s/x/Y/` 后 `@/` = `x`、`n` 搜 `x`、hlsearch 高亮 `x`。现编辑后 `set_pattern` 同步 |
| 13 | **`:2,2j` 误接两行** | vim `:h :j`：两地址等值的范围什么都不做（单地址 `:2j`/裸 `:j` 照常接下一行）。`parse_range` 现回报地址个数供判定 |
| 14 | **`:d _` 污染编号寄存器** | 黑洞寄存器被参数解析丢弃，内容照进 `"1`。特殊寄存器名（`_ + " . - : % # *`）现被识别，`_` 走黑洞 |
| 15 | **显式 count=1 被当未给 count** | `:1,2d 1`/`:1,2y 1` 应只取第 2 行（EX_COUNT 重锚定）；`count > 1` 判定改为 `count >= 1`（d/y/j 三处） |
| 16 | **`:set ic/hls` 不刷新既有高亮** | 探针：`:set noic` 后 hlsearch 匹配集立即按新规则重渲染。识别搜索类选项，重发布活动模式 |
| 17 | **`/^$` 恒报 E486** | 搜索路径把零宽匹配全滤掉。现保留（`ggn` 探针落空行行首），仅剔除尾换行后的幻影位置 |
| 18 | **visual 模式 `zz`/`zt`/`zb` 缺失**（十一轮悬置闭环） | 注册进 visual trie：锚定滚动上报 + **保留选区**（vim 行为） |
| 19 | **`:marks` 重复行列 `.`** | `m.` 设过的 `.` 与 last_change 各列一行。去重 |
| 20 | **`:5>` 报误导性 E16** | `>` 从范围字母表移除（`'>` 经 mark 分支解析不受影响），`:5>` 现如实报 E492（shift 命令未实现） |

### 被探针证伪的读码怀疑（免得下轮重查）

- `w` 于尾换行缓冲落 `buf.len()` 幻影位：现有 clamp 已正确（v1 断言通过）。
- `N` 从匹配中间跳当前匹配起点：引擎与 vim 一致（`rposition` 语义对）。
- 单行 `daw` 于词间空白只删空白段：同线形态**确实**错（列修复 6），但
  agent 声称的「vim 得 `foobar`」探针结果为 `foo`——最终按探针修。
- `:/^$` 的 vim E486：探针双重转义 bug（`@/` 里实际是 `^\$`），非 vim 行为。

### 新增已知分歧（接续前表编号）

33. **`:>`/`:m`/`:t`/`:co`/`:k` 等 Ex 命令缺失** → E492。`:` 移动/复制/
    缩进是 vim 常用命令；本轮把 `:5>` 从伪 E16 修正为如实 E492。
34. **地址偏移巨值饱和**：`:1+99999999999999999999` 引擎 saturating 到末
    行；vim 报 E16 类错误（溢出检测）。（**round15 已关闭**：实测 vim 报
    E1247 且不执行；round12 #2 的偏移链重写把该路径变成 `unwrap_or(0)`
    ——比本条记录的「饱和」更糟，删错行。现两方向都显式 E1247。）

### 悬而未决（本轮记录、未改动）

- `:s//x/` 空模式复用「上一条搜索模式」，vim 优先「上一条替换模式」、
  无才退回搜索模式（引擎只有一份 last search）。
- `\/` 分隔符转义不存在：`:s/a\/b/x/` 解析切碎（解析层失败）。
- `:set so=-1`：scrolloff 是四个数值选项里唯一合法为负的，`parse::<usize>`
  拒绝。`:se` 缩写、`:setlocal`、`:set ts&` 同缺。
- 命令行历史浏览不重发 incsearch；`|` 命令分隔符被 `:s`/`:d`/`:y`/`:j`
  参数解析静默吞（`:set`/`w`/`q` 至少报错）。
- `:d abc` 寄存器只取首字符、垃圾尾参静默吞（vim 报 E354/E488）。
- `config.rs` 与 `ex_set` 对同一 `set` 语法的解析顺序互相矛盾（`no` 与
  `=` 的优先级两处实现不同），应抽共享解析函数。
- `search::compile` 永远返回 `Some`（死分支）、`SearchState.last_index`
  死状态——低成本清理项。
- `:d`/`:y` 数字首参数=寄存器歧义（第十轮悬置）部分收敛：显式 count
  语义已对齐，`:'a,'bd 1` 的参数面仍需区分。
- `gv` 无前次选区（十一轮）、命令行历史上限（十一轮）、`state.rs` 拆
  文件（九轮，现 3700+ 行）、Replace BS 恢复栈（九轮）、Tab 显示宽度
  （七轮）——均维持。

### 体验备注（本轮）

- **vim 9.1 探针方法论**（下轮直接用）：`-es` 下 `-c` 执行时光标初始在
  **末行行首**；`:normal` 里失败按键（如末行 `j`）**中止剩余序列**——
  定位一律 `execute "normal! gg..."` 显式化；`execute "..."` 内 `\` 需
  双重转义（本轮 `/^$` 探针因此误报一轮）；结果走 `writefile(getline(...))`
  而非 `:%p`（后者受缓冲态干扰）。
- fuzz round12 键表 ~150 键按「本轮修复面」扩容；新增
  `ex_range_garbage_never_panics`：地址/偏移/分隔符/命令随机拼接 600 轮，
  只允许失败不允许 panic。
- `2gN` 的回归断言用 `marks.active_visual()`（单字符匹配的
  `visual_selection` anchor==cursor 是退化形态）。

### 性能备注（本轮复核）

- bench_probe 全量复跑：`w` 2.1µs/键（11KB）、1MB 单行 `w` 107µs/键、
  hlsearch 重扫 0.25ms/次（900KB/万匹配）、`:%s` 600KB 5.3ms、`n` 连跳
  3.7µs/键（ropey）——与十一轮同量级或略优（本轮改动不在热路径）。
- `all_matches` 保留零宽匹配后，`/^` 类模式的匹配数上限仍由
  `take(10_000)` 兜底，incsearch 每键成本不变。

## 〇⁺⁺⁺⁺⁺⁺⁺、第十一轮检视增补（2026-10-01，回归测试并入
`tests/review_regressions.rs`、fuzz 在 `tests/fuzz_round11.rs`）

本轮通读全部 `src/`，读码列可疑点 → vim 9.1 探针实证 → 修复；fuzz 键表
补齐 mark 算子面与本轮修复路径，512 种子离线轰炸抓到一个搜索缓存违规。
两条读码怀疑被探针**证伪/不可观测**未改动：单行块 `O`（vim 的「对角
等列」在等行时是恒等，块操作的列区间与 anchor 方向无关——扩展选区探针
两种形态结果全同）；`077` 前导 0 上 `<C-a>` → `0100`（引擎本来就对，
宽度自然生长）。

### 新修复（1-5、8 语义类均先跑 vim 9.1 探针；6-7 为 fuzz 抓取）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **空命令带范围光标落错行** | 探针：`:2,5<CR>` 光标落第 5 行（范围的**末地址**）；引擎过去落 `first`，`:2,5` 落第 1 行。`:%<CR>` 同族（应落末行） |
| 2 | **多地址范围不截断** | 探针：`:1,2,3d` 于 ['a','b','c','d'] 删第 2-3 行——vim 只保留**最后两个**地址；引擎保留首尾删 1-3 |
| 3 | **空地址被跳过** | 探针：`:,3d` 于第 2 行删 2-3、`:2,d` 于第 3 行删 2-3——空地址默认**当前行**；引擎跳过空段，两条命令都只删了单行 |
| 4 | **`&`/裸 `:s` 重放保留旗标** | 探针：`s/a/B/g` 后 `&`/`:s` 于 "xaxax" 都只换首个匹配——`g` **不**随重放（`:h :&`）。引擎原样重放整条命令行。现在重放前剥掉尾部旗标段 |
| 5 | **mark 跳转作为算子目标整条缺失** | `d'a`/`` d`a ``/`y'a`/`c'a` 静默不动且悬空算子污染下一个键（`complete_char_arg` 的 JumpMark 臂只挪光标）。现镜像 Find 臂：算子下 `Motion::MarkJump` 经 `span_from_motion` 取 span（`'a` 行级跨到 mark 行、`` `a `` 字级排他、方向无关，探针全对齐） |
| 6 | **块插入 pad 编辑的搜索缓存陈旧**（fuzz round11） | 块选 `A` 给短行补空格走 `edit_insert` 但不重跑 hlsearch 扫描，`last_matches` 停在编辑前偏移，多字节文本上 mid-char（"b中" 行被 pad 位移后缓存 end=20 落进 中 内部）。补 `republish_search`，与其余编辑路径同规 |
| 7 | **IME 提交替换路径缓存陈旧** | `VimState::replace_range`（宿主提交组合文本）编辑后不 republish。同款补齐 |
| 8 | **`<C-a>` 光标落在进制前缀字母上产出垃圾** | 探针：`0x1f` 光标在 `x` 上 C-a → `0x20`（vim 把前缀字母视为「数字在光标处」）；引擎把 `x` 当普通文本，抓裸数字 run 拼出 `00x2f`。前缀字母路径锚回前导 `0`，并要求后随合法数位（`0backup` 的 `b` 不算数字，vim 不动） |

### 新增已知分歧（接续前表编号）

32. **`:5,2d` 反向范围**：引擎静默交换首末；vim 交互模式提示
    "Backwards range given, OK to swap (y/n)"、`-es` 下拒绝执行。引擎
    无 y/n 交互通道，取交换语义（IdeaVim 同款取舍）。

### 悬而未决（本轮记录、未改动）

- `gv` 无前次选区：引擎静默不动。vim 大概率报错（`-es` 下 `mode()`
  失真探不准具体错误码），是否补 E448 文本待有可靠探针再定。
- visual 模式 `zz`/`zt`/`zb`：引擎 trie 无行 → 响铃取消 pending；vim
  滚动且保留选区。小缺口，需把三行注册进 visual trie。
- 命令行历史无上限（vim 有 'history' 上限，默认 1 万）。会话级内存
  增长缓慢（每条一行命令文本），加选项+截断的收益低，暂缓。
- `:d`/`:y` 数字首参数（第十轮悬置）、`state.rs` 拆文件（第九轮悬置，
  现 3700+ 行）、Tab 显示宽度（第七轮悬置）——均维持。

### 体验备注（本轮）

- fuzz round11 键表 ~200 键，新增路径按「本轮修复面 + mark 算子面」
  补齐；不变量外加 changelist/jumplist 步进轰炸（`g;`×5 + `<C-i>`×3
  每 7 步穿插）——两个列表的 floor 守卫从此有常驻压力。
- `:marks` 若用 `m.` 设过 `.` mark，列表会出现两行 `.`（items() 与
  last_change 各一行）。纯外观，暂缓。

### 性能备注（本轮复核）

- bench_probe 全量复跑：`w` 3.3µs/键（11KB）、hlsearch 重扫 0.30ms/次、
  `:%s` 600KB 5.0ms、`n` 连跳 3.7µs/键（ropey）——与第十轮同量级。
  新增的 pad-republish 只在块选 `A` 短行补空格路径触发，不在热路径。

## 〇⁺⁺⁺⁺⁺⁺、第十轮检视增补（2026-10-01，回归测试在 `tests/review_regressions.rs`、
fuzz 在 `tests/fuzz_round10.rs`）

本轮通读全部 `src/`，读码列可疑点 → vim 9.1 探针实证 → 修复。fuzz
键表大幅扩容（历史浏览/文本对象全集/count-insert/`:set` 选项/缩进计数/
insert `<C-r>`/带换行 IME 注入）并新增 `search.last_matches` 可寻址
不变量，首轮即抓到一个陈旧缓存违规。两条读码怀疑被探针**证伪**未改动：
`&` 的范围语义（`last_substitute` 存的已是剥离范围后的命令体，`&`/裸
`:s` 都落当前行，vim 探针 `ba|bb|ba` 引擎一致）；宏寄存器重放含 `q`
会清空寄存器（构造探针证明不可达——展开的 `q` 一执行就停止录制并被
弹出，宏内永远录不进可再生的 `q`，与 vim 同）。

### 新修复（1-2、4-5 语义类均先跑 vim 9.1 探针；3 为 fuzz 抓取）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **count-repeat insert 分配无上限** | `99999999i`+长文本+`<Esc>`：`replicate_count_insert` 对 `text.repeat(copies)` 不设防，一个小会话能在退出时生成 GB 级插入（寄存器粘贴路径早有 16MB 防线）。现走同一条 `clamped_repeat_count`；linewise 分支顺带从逐次 `format!` 循环（O(n²)）改为单位串 `repeat` |
| 2 | **`:set ts/sw=` 巨值不设防** | tabstop/shiftwidth 喂 `" ".repeat(n)` 形状的缩进合成（insert Tab、`>>`），`:set sw=99999999999` + 一次缩进就是 GB 级分配。数值选项在 `set_value` 边界截到 1e6（vim 上限 2^31-1，分歧 31） |
| 3 | **insert 模式编辑后搜索缓存陈旧**（fuzz round10） | `<Del>`/`<BS>`/`<C-w>`/`<C-u>` 编辑缓冲后不重跑 hlsearch 扫描，`last_matches` 留着编辑前偏移，删除恰好落进多字节字符中间（"a中b" 删 'a' 后缓存 1..4 指进 中 内部）。`cancel_cmdline` 把这份缓存原样发布给宿主——宿主对高亮做 `offset_to_line` 即 panic（round9 `insert_change_pos` 同类暴露面）。四条路径补 `republish_search`，与 IME 打字路径同规 |
| 4 | **零宽替换整体失效** | `:s/^/>/`、`:s/$/</`、`:s/a*/-/` 静默不动且误报 E486——空匹配被「不计数不替换」策略跳过（第七轮悬置项）。vim 实证：`:s/^/>/` 行首插入（惯用法）、`:s/$/</` 行尾追加、非全局 `x*` 同 vim。空匹配现照常计数展开。残留分歧见分歧 30 |
| 5 | **未设 mark 的范围报 E16** | vim 实证 `:'<,'>d`（无 prior visual）→ `E20: Mark '< not set`；命名 mark 同理。`parse_range` 区分「范围语法错误（E16）」与「mark 未设置（E20，含 mark 名）」 |

### 新功能

- **`VimState::macro_len(reg)`**：宏寄存器步数读接口（状态栏/测试观测；
  宏以步列表存储，文本寄存器通道不可见）。

### 新增已知分歧（接续前表编号）

30. **零宽匹配 + `g` 旗标的行尾推进**：vim `:s/x*/-/g` 于 "xabcx" →
    `-a-b-c-`、`:s/a*/-/g` 于 "bbb" → `-b-b-b`——不吃「行尾位置上的
    最后一个空匹配」；rust regex 会在行尾多产一个空匹配，尾部多一个
    替换字符（`-a-b-c--`/`-b-b-b-`）。`^`/`$`/非全局全部一致。对齐需
    自建带推进规则的替换扫描，成本大于收益。
31. **数值选项上限 1e6**（`ts`/`sw`/`tw`/`so`）：vim 上限 2^31-1。防护
    性收紧（修复 2），1e6 列已远超真实用途。

### 悬而未决（本轮记录、未改动）

- `g&`（`:s` 的全局重复，vim: 按 `%` 范围重放）未实现。
- `:d`/`:y` 首参数为数字时按 COUNT 解析（`:y 1` = count 1），vim 把
  数字也当合法寄存器名（`:1y 2` 进 `"2`）。极边缘，需区分「单参数数字
  = count、显式第二参数数字 = 寄存器」才能对齐，暂缓。
- `publish_incsearch` 每键全缓冲扫描不受 `set_hlsearch_live_update(false)`
  约束（那是编辑路径的开关）；巨文件宿主要自行 `set nois`。
- Replace 模式 BS 的恢复栈在换行分割之后位置失配（第九轮悬置）；
  `state.rs` 拆文件（第九轮悬置）——均维持。
- Tab 显示宽度按 1 记账（第七轮悬置）维持。

### 体验备注（本轮）

- fuzz round10 键表 ~150 键：新增路径按「本轮修复面 + 长尾命令面」
  双线补齐；不变量加 `last_matches` 后，搜索缓存与文本的同步从此有
  常驻压力。
- `Fixture::feed` 逐键语义在写 E20 测试时又踩一次（`'<` 在 normal
  模式是 mark 跳转，要先进 `:`）——helper 注释已有，再记一笔。

### 性能备注（本轮复核）

- insert 四条编辑路径每键多一次 hlsearch 重扫（与打字路径同规）；
  巨文件宿主走 `set_hlsearch_live_update(false)` + `refresh_highlights`
  逃生门，量级不变。
- bench 数字与第九轮同量级（本轮改动不在 bench 热路径上）。

## 〇⁺⁺⁺⁺⁺、第九轮检视增补（2026-10-01，回归测试在 `tests/parity_round9.rs`、
fuzz 在 `tests/fuzz_round9.rs`）

本轮通读全部 `src/`，读码列可疑点 → vim 9.1 探针实证 → 修复；随后把
fuzz 键表扩到本轮修复路径（`<C-w>`/`3D`/`3C`/`r<CR>`/`<Tab>`/`:substitute`
全拼/`:2j 3`/`:bfirst`）并离线重轰炸（256 种子×300 轮×200 步），抓到一个
引擎不变量违规。两条读码怀疑被探针**证伪**未改动：`:s/foo/`（尾分隔符
带空 replacement）引擎本来就按空串替换（与 vim 同）；`2rx` 于多字节行
的光标落位（都在最后一个替换字符上）。

### 新修复（1-6 语义类均先跑 vim 9.1 探针；7 为 fuzz 抓取）

| # | 问题 | 实证/根因 |
|---|------|-----------|
| 1 | **`gq` 把空行/空白行翻倍** | 探针：`ggVGgq` 于 ["","",""]，vim 1:1 保留三行；引擎对每个空行都做一次空段落 flush（空 flush 也输出一行终止 `\n`），三行变七行；纯空白行 `"   "` 还被塌成空行（vim 逐字保留）。`format_lines` 现只在非空段落 flush，分隔行逐字保留 |
| 2 | **`3D`/`3C` 忽略行计数** | 探针：(1,2) `3D` 于 4 行缓冲 → ['a','dddd']；`99D` 于 2 行缓冲 → ['a']。语义是「删 [count] 行，至少到行尾」：覆盖行整行消失，最后覆盖行保留自己的终止 `\n`（计数触到缓冲末行时才一起走）。`D`/`C` 共用 `delete_to_end_span` |
| 3 | **insert `Tab` 按字节列对齐** | 探针：'中文' 行尾 Tab（expandtab ts=4），vim 补 4 空格（按显示列补满到 ts 倍数），引擎按字节列 6%4 只补 2。改用 `display_column` |
| 4 | **insert `<C-w>`/`<C-u>` 行首不并线** | 探针：行首 `i<C-w>x<Esc>`/`i<C-u>x<Esc>` 于 ['aaaa','bbbb'] → ['aaaaxbbbb']（只删换行）。旧行为钳在行首不动（`aaaa\nxbbbb`）。块插入会话中并线响铃拒绝（复制偏移假设每行独立，fuzz round8 行首 BS 同族） |
| 5 | **Ex 命令的 changelist 记在旧光标** | `:s`/`:d`/`:sort`/`:j` 在 `bump()` 之后才挪光标，`.` mark 与 changelist 记下的是命令前的位置（探针：`:4s` 后 `g;` 应落第 4 行）。光标摆放统一移到 `bump` 之前。连带：changelist 两端消息分方向（后退到底 E662、前进到顶 E663，旧行为都报 E662） |
| 6 | **`:s/foo` 缺 replacement 响铃** | 探针：`:s/foo`（无尾分隔符）等价 `:s/foo//`——删除匹配。旧行为响铃不动作。块选 `p` 寄存器行耗尽时同族：vim 直接删覆盖区间（2 行寄存器贴 3 行选区 → 第三行空），旧行为重复寄存器末行 |
| 7 | **`insert_change_pos` mid-char panic**（fuzz） | insert 会话的首敲位置是裸 offset，编辑漏斗不调整；会话中 BS 并线把文本挪动后，`exit_insert` 把落在多字节字符中间的偏移记进 changelist，`:marks` 列 `.` 行时宿主 `offset_to_line` panic。exit 时 floor 到当前文本 |

### 新功能

- **`:{last}j {count}`**：从 range 末行起并 count 行（探针 `:2j 3` 并
  2-4 三行），旧行为忽略计数只并两行。
- **`:[range]s[ubstitute]` 全拼**：归一为 `s` 拼写（vim 单词别名），
  非 `substitute` 前缀的词仍走 E492。
- **`:bfirst`/`:brewind`/`:blast`**（`:bf`/`:bl`）：`VimHost` 新增
  `first_buffer`/`last_buffer` 钩子（默认成功，宿主拒绝响铃）。闭环
  第八轮「悬而未决」第一条。
- **`r<CR>` 光标落新下一行首**（探针 'abc' `r<CR>` → (2,1)），旧行为
  clamp 回上一行。功能本身前几轮已有，本次修落点。

### 悬而未决（本轮记录、未改动）

- Replace 模式 BS 的恢复栈在**换行分割之后**位置失配：`R` 中途打
  `\n` 再 BS，被覆盖字符栈与实际位置脱节（引擎按字符栈恢复，vim 按
  位置恢复）。罕见路径，需要把栈改成 (offset, char) 结构，暂缓。
- 块寄存器普通 `p` 短行补空格、块插入 `I` 短行插行尾等行为与 vim 的
  更深层差异（vim 会扩展/收缩块会话）维持 v1 取舍——纵向移动在块
  会话内仍是响铃拒绝，见第八轮备注。
- `state.rs` 3600+ 行：模块按「管线/编辑漏斗/normal/visual/char-arg」
  分节清楚，但再长就该拆文件了（下次大改时顺手）。

### 体验备注（本轮）

- `Fixture::feed`（测试域）按 `Key::parse` 语义「一个 item 一个键」：
  多字符裸串（如 "up"）是 Named 键、Ex 命令行要逐字符喂——写测试时
  踩过一次，已在 helper 注释里写明。
- `bench_probe` 的 `line_range` 实现每次调用做两次 `split('\n').count()`，
  探针自身的浪费，引擎无关（不影响测量结论的方向）。

### 性能备注（本轮复核）

- bench_probe 全量复跑：`w` 2.5µs/键（11KB）、hlsearch 重扫 0.30ms/次、
  `:%s` 600KB 5.9ms、`n` 连跳 3.7µs/键（ropey）——与第八轮同量级。
  首跑 `w` 6.3µs 为预热噪声，复跑回落。

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

- ~~`:bfirst`/`:blast` 未支持~~（**第九轮已支持**：`VimHost` 新增
  `first_buffer`/`last_buffer` 钩子）。
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

- ~~**`:s` 空匹配**~~ **第十轮已修**：vim 探针推翻悬置（`:s/^/>/`
  行首插入是 vim 惯用法），空匹配现照常计数展开；残差（`g` 旗标行尾
  推进）记分歧 30。
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
11. ~~insert 模式 `<C-w>`/`<C-u>` 在行首不删除换行~~（**第九轮已修复**：
    行首 `<C-w>`/`<C-u>` 与上一行并线，块插入会话内响铃拒绝）。
12. **`:g`、`:sort`、`:normal`、函数/autocmd** 不支持，走 E492 或 rc
    `ignored` 收集（IdeaVim 同款取舍）。
13. **`""yy` 落 `"0`**：`""` 即匿名寄存器，yank 必写 `"0`，行为与 vim
    一致；但 `""dd` 不进数字环（vim 也如此）。
14. ~~visual `:` 执行后 `'<`/`'>` 被按「锚点..当前光标」重写~~
    （**第七轮已修复**：按提示符时的选区范围落账，`gv` 二次语义恢复）。
15. **`o`/`A` 进入 insert 后的块复制不含多行文本**：块插入复制的文本
    含 `\n` 时每行都会粘进（无宿主可见的崩坏，但语义未对齐 vim）。

## 六、性能备注

- **`[profile.test] opt-level = 3`**（第十三轮）：fuzz 套件的自递归宏
  （`qa…@a…q` 后 `@a`）会撑满 10 万键的管线预算，opt1 的调试构建要
  ~78s；opt3 压回秒级。引擎自身的 10 万键预算不动（那是对宿主的
  正式最坏情形承诺）。
- fuzz_round13 的键表附带 4KB 缓冲重置：yy+p 循环能把缓冲翻倍到数 MB，
  每步的全文件扫描是二次方成本；本 fuzz 抓到的 bug 全部在几 KB 缓冲
  上可复现。

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
- 命令行提示符的 C-w/C-u/C-h/上下历史（第十三轮）：`:fo<C-w>` 删词、
  `:<C-u>` 清行是 rc 训练出来的肌肉记忆；此前除字符/回车/Esc/退格/
  上下外全部静默吞掉。
- Ex 缩写（第十三轮）：`:de`、`:su/…/` 等 vim 拼写直接可用，不再 E492。
- visual 点击跟随（第十三轮）：宿主在 visual 模式转发点击不再塌缩选区，
  拖选外的一次点击像 vim 一样延伸选区。
