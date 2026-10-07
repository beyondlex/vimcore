# vimcore 独立审计（第二轮）：新发现 Bug 清单（2026-10-07，60 项）

> **状态：60 项已全部修复**（2026-10-08，同日完成）。回归探针全部转绿：
> `tests/audit2_{a,b,c,d,e,f}.rs` 共 90 个用例全绿，全量 `cargo test`
> 904 例无失败。修复要点：A 系列 TAB 虚列模型 + 节/方法跳转；B 系列
> aw 空白对象按 oracle 矩阵重写 + count 扩展跨行 + cw≡ce 特例（含
> U+3000）；C 系列 noeol 粘贴字节口径（fixendofline）+ `"=` 表达式
> 寄存器 + 只读寄存器 E354 + `"Add` 非对称 unnamed；D 系列六个插入
> 控制键 + `".` 保真度 + 块插入点重放 + `q"`/可视 `@`；E 系列块可视
> o/O、`'>` 落点、undo-marks、`gv` 钳制、`` `[/`] ``、可视 `<C-o>`、
> `g;` 方向；F 系列搜索偏移重放/算子可视锚定/gd/g*/g#/翻页 count；
> G 系列 `:{range}pu`、`:delm A-Z`、地址偏移记号化、裸地址钳制、
> `<Up>` 前缀匹配、命令行 `<C-v>`、退出拼写族；H 系列 `:set` 拼写与
> 查询格式、`:retab 0`。
>
> 审计期间另证伪 2 条旧钉：`<Up>` 历史无前缀匹配的旧行为、`v-B` 块列
> 虚列（#35 已知账）保持不动。

> **状态：59 项新发现，全部未修复**。每项都带一个当前**失败**的探针测试，
> 修复后可直接转回归。探针分布在 6 个文件：`tests/audit2_{a,b,c,d,e,f}.rs`
> （合计 76 个失败测试；`audit2_d` 另有 1 个占位测试是绿的）。仓库其余
> 819 个既有测试全部保持通过。
>
> **实证方式**：语义类结论全部用本机 vim 9.1（patches 1-1752）逐一对照——
> 绝大多数走 `-Nu NONE -N -i NONE -s` typeahead 通道；命令行方向键（G8）
> 走 expect 驱动的真 PTY（round 28 的教训：方向键在 typeahead 通道不可达）。
> 唯一例外是 **E10**（响铃不可观测），已在条目内标注。审计前逐项对照了
> NOTES.md 的 77 项已知分歧与 2026-10-06 的 50 项已修复清单（BUG_AUDIT.md），
> 下列条目均不在既有账上；与昨天 F3（搜索偏移）相关的 4 条搜索条目是
> 该修复的**后续缺口**（偏移已能解析，但重放/算子/数字尾/落点语义仍错）。
>
> 严重度：**P0** 崩溃 / **P1** 行为错误或数据损坏 / **P2** 功能缺失或偏差 /
> **P3** 反馈缺失与边角。本轮没有 P0——昨天的 saturating 修复之后，各域的
> 敌意扫描（视口极值 × 滚动全家、空缓冲 × 37 种操作、21 位 count、
> NUL/裸 CR 文本）都没有再打出 panic。

本轮探针的攻击面：意料之外的操作序列（算子中途寄存器前缀、可视模式里的
`@`、undo 后的 mark 与 gv）+ 不寻常的数据（无尾换行缓冲、TAB 与 CJK 混排、
U+3000 表意空格、连字/组合变音字符、大小写混用标签、空行作对象边界）。

---

## A. Motion 与列计算（TAB 宽度模型）

- [x] **A1.（P1）`{count}|` 把 TAB 当 1 个显示格，不按 `'tabstop'` 展开** —
  `offset_for_display_column` 用 unicode-width 计列，TAB 宽 1；vim 按
  `'ts'`（默认 8）的虚列落位。
  - 复现：`ab\tc\n`，(0,0)，`5|`
  - 预期：光标落 TAB 上（vim col 3，offset 2）；实际：offset 3（落 `c`）
  - 波及：`4|`–`10|` 全段与 `d4|` 式算子跨度。oracle 实证。
  - 探针：`tests/audit2_a.rs::pipe_column_ignores_tabstop`、
    `tests/audit2_f.rs::f7_tab_counts_one_cell_in_column_math`

- [x] **A2.（P1）`j`/`k` 的列记忆同样不按 virtcol 记 TAB** — 同一根因：
  列状态机里没有 TAB 展开，跨 TAB 行后虚列漂移。
  - 复现：`a\tb\n0123456789x\n`，(0,2)（`b`，虚列 9），`j`
  - 预期：第二行 col 9（offset 12，字符 `8`）；实际：offset 6（`2`）
  - `C-d`/`C-u`/`C-f`/`C-b` 的列保持同族。oracle 实证。
  - 探针：`tests/audit2_a.rs::j_preserves_virtcol_with_tabstop`

- [x] **A3.（P2）节跳转 `]]`/`[[`/`][`/`[]`/`]m`/`[m` 全部未绑定** —
  响铃不动。vim：`]]` 跳下一列 1 的 `{`，`[[` 跳上一列 1 的 `{`
  （列 1 的 `}` 不拦截 `[[`）。
  - 复现：`foo\n{\nbar\n}\nbaz\n`，(0,0)，`]]`
  - 预期：光标到第 2 行（col 1 的 `{`）；实际：响铃，原地
  - oracle 实证。
  - 探针：`tests/audit2_a.rs::section_motions_brace_column_unbound`

## B. Text objects 与 count 扩展

- [x] **B1.（P1）对象 count 扩展在行边界处中断** — `object_span_count`
  从 `span.end` 重新探测，偏移被 floor 回上一行末词后「无进展」早退。
  三个形状（oracle 全部实证）：
  - a) `foo\n\nbar\n`，(0,0)，`d2aw`：vim 整缓冲删空（空行算一个词）；
    实际 `\n\nbar\n`（count 在空行处停止）
  - b) `ab cd\nef gh\n`，(0,2)，`d2aw`：vim 得单行 `abgh`（第二个 aw
    跨进 `ef`）；实际 `ab\nef gh\n`
  - c) `ab \n\ncd\n`，(0,1)，`2cwX<Esc>`：vim 得 `aX`；实际 `aX \n\ncd\n`
  - 探针：`tests/audit2_a.rs::d2aw_count_stops_at_empty_line`、
    `d2aw_count_stops_at_line_break`、`d2aw_punct_line_final_stops_at_line_break`、
    `c2w_across_empty_line_reaches_last_word`

- [x] **B2.（P1）`daw` 的尾随空白族误吞** — `blank_run_plus_next_word`
  把 `\n` 当普通空白、扫到 EOF 还会返回部分 span。三个形状（oracle 全部
  实证）：
  - a) `a   \n\nb\n`，(0,3)（末空格），`daw`：vim 得 `a\nb\n`；实际 `a`
    （连 `b` 与行结构一起吞掉）
  - b) `foo\n   \n\nbar\n`，(1,0)，`daw`（TAB 变体同）：vim 得 `foo\nbar\n`；
    实际 `foo\n`
  - c) `a   \n`，(0,3)，`daw`：vim 原样不动（后面没有词，`daw` 应为
    no-op）；实际 `a`（空白与换行被删）
  - 探针：`tests/audit2_a.rs::daw_after_trailing_ws_eats_next_word_across_empty_line`、
    `daw_on_ws_only_line_eats_next_word_across_empty_line`、
    `daw_trailing_ws_without_next_word_is_noop`、
    `daw_ws_only_last_line_without_next_word_is_noop`

- [x] **B3.（P1）`cw` 不保护 U+3000 表意空格** — cw 的截尾 trim 只认
  ASCII 空白与 TAB，全角空格被当普通字符删掉；vim 的 `cw`（= `ce`）
  永不触碰尾部任何空白。
  - 复现：`a\u{3000}b\n`（全角空格），(0,0)，`cwX<Esc>`
  - 预期：`X\u{3000}b\n`；实际：`Xb\n`。oracle 实证。
  - 探针：`tests/audit2_a.rs::cw_keeps_wide_space_vim_ce_semantics`

- [x] **B4.（P1）`it`/`at` 的标签名匹配大小写敏感** — HTML 标签在 vim
  的 tag 对象里大小写不敏感。
  - 复现 a：`<P>x</p>\n`，(0,3)，`dit`。预期 `<P></p>\n`；实际不变
  - 复现 b：`<p>x</P>\n`，(0,3)，`dit`。预期 `<p></P>\n`；实际不变
  - oracle 实证（本机复核通过）。
  - 探针：`tests/audit2_a.rs::tag_object_matches_open_tag_case_insensitively`、
    `tag_object_matches_close_tag_case_insensitively`

## C. 无尾换行（noeol）粘贴族与寄存器

- [x] **C1.（P1）noeol 场景下 linewise `p`/`P` 丢末尾换行** — `put_ex`
  在 `after && !has_newline` 分支把寄存器自带的结尾 `\n` 消费掉不补回；
  空寄存器被强转成 `"\n"` 后又被剥成空串。
  - a) 空缓冲，`yyp`：vim 2 个空行（`yy3p` 得 4 行）；实际仍 1 行
  - b) `a\n`，(0,0)，`ddp`：vim 文件字节 `\na\n`；实际 `\na`（末字节静默丢失）
  - c) `abc\n`，(0,0)，`ddp`：vim `\nabc\n`；实际 `\nabc`
  - oracle 实证（含 `od -c` 字节级对照）。
  - 探针：`tests/audit2_b.rs::f1_empty_linewise_put_on_empty_buffer_loses_line`、
    `f2_put_below_noel_line_drops_final_newline`、
    `tests/audit2_f.rs::f10_linewise_put_into_emptied_buffer_loses_final_newline`

- [x] **C2.（P1）noeol 末行 yank 后 count-repeat 水平粘连** — 寄存器文本
  没有自带 `\n` 时 `repeat(count)` 直接水平拼接，行分隔符缺失。
  - 复现：`abc`（无尾换行），(0,0)，`yy3p`
  - 预期：4 行 `[abc,abc,abc,abc]`；实际 `abc\nabcabcabc`。oracle 实证。
  - 探针：`tests/audit2_b.rs::f4_linewise_count_repeat_glues_without_separators`

- [x] **C3.（P1）noeol 下 `]p` 把粘贴内容并进当前行** — `PasteIndent`
  的插入点取 `line_range(cur).end.min(len)`，末行无换行时即为缓冲末尾
  且不补分隔符，属于数据损坏形状。
  - 复现：`ab\ncd`（无尾换行），(0,0)，`yy` `j` `]p`
  - 预期：3 行 `[ab,cd,ab]`；实际 `ab\ncdab\n`。oracle 实证。
  - 探针：`tests/audit2_b.rs::f3_bracket_p_below_noel_line_glues`

- [x] **C4.（P2）大小写算子用 Rust 全 Unicode 映射，vim 用简单映射** —
  昨天只对 `ß` 做了特判，同根因的其余字符仍在：连字（ﬁ）、无简单映射符
  （ǰ）、İ 的小写展开（`i` + 组合点）。
  - a) `ﬁx` 上 `~`：vim 不变；实际 `FIx`
  - b) `İx` 上 `gul`：vim `ix`；实际 `i\u{307}x`
  - c) `ǰx` 上 `~`：vim 不变；实际 `J\u{30c}x`
  - oracle 实证。
  - 探针：`tests/audit2_b.rs::f5_case_operators_full_vs_simple_unicode`

- [x] **C5.（P2）`gP` 行级粘贴光标落粘贴块首行，vim 落块之后一行**
  - 复现：`aaa\nbbb\nccc\n`，(1,0)，`yy` `G` `gP`
  - 预期：光标在第 4 行（1-based，两行块时第 5 行）；实际：块首行。
    oracle 实证。
  - 探针：`tests/audit2_b.rs::f6_gp_linewise_cursor_after_block`

- [x] **C6.（P2）operator-pending 接受 `"{reg}` 前缀并静默执行删除** —
  vim 里 `"` 在 operator-pending 下非法：算子取消并响铃，后续按键照常
  解释（`a` 进插入模式）。
  - 复现：`foo bar`，(0,0)，`d` `"` `a` `w` `<Esc>`
  - 预期：`fwoo bar`、寄存器 a 空；实际：`bar`、寄存器 a = `foo `。
    oracle 实证。
  - 探针：`tests/audit2_b.rs::f7_register_prefix_rejected_under_operator`

- [x] **C7.（P2）只读寄存器可经前缀写入（数据丢失形状）** — vim 拒绝向
  `%`/`:`/`.`/`/` 写入；引擎照删并把删除内容存进对应槽。
  - 复现：`abc\n`，(0,0)，`"` `%` `d` `d`。预期：缓冲不变；实际：缓冲空、
    `"%` 槽被覆盖（`".dd` 后 `".p` 会粘出被删行而非最后插入文本）。
    四个寄存器逐一实证。
  - 探针：`tests/audit2_b.rs::f8_readonly_registers_reject_prefix_write`

- [x] **C8.（P3）`"Add` 后未命名寄存器指向合并结果，vim 指向新片段**
  - 复现：`ab\ncd\nef\n`，(1,0)，`"ayy` `j` `"Add` `p`
  - 预期：`p` 只粘出新删除的 `ef`；实际粘出合并体 `cd\nef`（append
    路径重置了 unnamed）。oracle 实证。
  - 探针：`tests/audit2_b.rs::f9_uppercase_append_unnamed_gets_new_piece`

- [x] **C9.（P2）`"=` 表达式寄存器缺失** — Normal 侧 `"=2+3<CR>p` 无物
  可粘；Insert 侧更糟：`i<C-r>=` 后的 `1+1` 被**当文本打进缓冲**。
  - 复现：`ab`，(0,0)，`"=2+3<CR>p`。预期 `a5b`；实际不变。
  - 复现：`x\n`，(0,0)，`i<C-r>=1+1<CR><Esc>`。预期 `2x\n`；实际
    `1+1\nx\n`。oracle 实证。
  - 探针：`tests/audit2_b.rs::f10_expression_register_put`、
    `tests/audit2_c.rs::ctrl_r_equals_expression_register`

## D. 插入模式、`".`/`.` 重放与宏

- [x] **D1.（P2）`i<C-t>`（插入一个 shiftwidth）未绑定** — vim 在行首
  插一个 `'sw'`（`sw=4 et` 时 4 空格），光标不动；引擎下该键为
  Unknown，缓冲只多了打字文本。
  - 复现：`abc\n`，(0,0)，`i` + `ab` + `<C-t>` + `<Esc>`。预期 `\tababc\n`；
    实际 `ababc\n`。oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_t_inserts_shiftwidth_indent`

- [x] **D2.（P2）`i<C-d>`/`0<C-d>`（移除缩进）未绑定** —
  - 复现 a：`        abc\n`（8 空格），(0,0)，`i<C-d>x<Esc>`：预期
    `xabc\n`；实际 `x        abc\n`
  - 复现 b：`i0<C-d><Esc>`：预期缩进全移且 `0` 不落盘；实际无效果。
    oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_d_removes_shiftwidth_indent`

- [x] **D3.（P2）`i<C-y>`（抄上一行字符）未绑定** —
  - 复现：`abcdef\nxy\n`，(1,1)，`i` + `x` + `<C-y>×3` + `<Esc>`
  - 预期：`xxcdey\n`（上一行耗尽后静默）；实际 `abcdef\nxxy\n`。
    oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_y_copies_char_from_line_above`

- [x] **D4.（P2）`i<C-e>`（抄下一行字符）未绑定** —
  - 复现：`xyz\nab\n`，(0,0)，`i<C-e>×4<Esc>`。预期 `abxyz\nab\n`；
    实际缓冲不变。oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_e_copies_char_from_line_below`

- [x] **D5.（P2）`i<C-a>`（重插上次插入文本）未绑定** —
  - 复现：`foo\nbar\n`，(0,0)，`ihello <Esc>jA<C-a>!<Esc>`
  - 预期：`barhello !\n`；实际 `bar!\n`。oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_a_inserts_last_inserted_text`

- [x] **D6.（P2）`i<C-@>`（重插上次插入并退出插入模式）未绑定** —
  键被静默吞掉，会话还留在 Insert。
  - 复现：`x\n`，(0,0)，`ihi<Esc>i<C-@>`。预期 `hhiix\n` 且回到 Normal
    （其后 `Z` 是 Normal 命令）；实际 `hix\n` 且仍在 Insert。oracle 实证。
  - 探针：`tests/audit2_c.rs::ctrl_at_inserts_last_insert_then_leaves_insert`

- [x] **D7.（P1）`".` 寄存器保真度三缺陷** — 昨天补了 `".` 寄存器本身，
  但内容保真度有三个缺口（oracle 全部实证）：
  - a) 空插入会话不清 `".`：`i<Esc>` 结束后 vim 清空该寄存器（代码注释
    的假设被 oracle 推翻），引擎里陈旧文本继续被 `<C-r>.`/`<C-a>` 粘出
  - b) C-w/C-u 删除的文本不裁剪：`ifo<C-w><Esc>` 后 vim 的 `".` 为空、
    `<C-a>` 不再插入；引擎仍存完整 `fo`
  - c) BS 应作为退格重放：`iabc<BS><Esc>` 后 `<C-r>.` 在 vim 里净效果
    是 `ab`（寄存器含 `<80>kb`）；引擎粘出未删的 `abc`
  - 探针：`tests/audit2_c.rs::empty_insert_session_clears_last_insert_register`、
    `ctrl_w_and_ctrl_u_trim_last_insert_register`、
    `backspace_in_session_replays_as_backspace_from_last_insert`

- [x] **D8.（P2）`q"`（录进未命名寄存器）被拒** — vim 接受
  `q{0-9a-zA-Z"}`，且 `@"` 可重放刚录的宏；引擎按字母数字过滤拒绝 `"`。
  - 复现：`ZZ\n`，(0,0)，`q"` `ix<Esc>` `q` `@"`
  - 预期：`xxZZ\n`；实际 `xZZ\n` + 两次响铃（录制从未开始）。oracle 实证。
  - 探针：`tests/audit2_c.rs::q_quote_records_into_unnamed_register`

- [x] **D9.（P2）行可视模式 `@{reg}` 未绑定** — vim 在可视模式执行
  `@a` 时寄存器内容带着选区跑（宏里的 `x` 删掉整行选中）。
  - 复现：`aa\nbb\ncc\n`，(0,0)，`qaxq<Esc>jV@a`
  - 预期：`a\ncc\n`；实际三行不动 + 响铃。oracle 实证。
  - 探针：`tests/audit2_c.rs::at_in_linewise_visual_runs_the_register`

- [x] **D10.（P1）`.` 重放 `gi` 会话时把 `^` 跳转也重放** — 点重放应
  只重放插入动作本身，不含 `gi` 的定位。
  - 复现：`foo bar\nqux\n`，(0,0)，`gi` + `X` + `<Esc>` + `ww` + `.`
  - 预期：`Xfoo bar\nXqux\n`；实际 `XXfoo bar\nqux\n`（跳回 `^` 再插）。
    oracle 实证。
  - 探针：`tests/audit2_c.rs::dot_after_gi_inserts_at_current_cursor`

- [x] **D11.（P1）`.` 丢弃块插入录制** — 块插入完成后 `recording_blocked`
  把整段录制丢掉，`.` 只能重复更早的变更（b1–b10 矩阵均一致）。
  - 复现：`aa\nbb\ncc\n`，(0,0)，`<C-v>j` + `I` + `-` + `<Esc>` + `w` + `.`
  - 预期：`--aa\n--bb\ncc\n`；实际 `"-aa\n-bb\ncc\n` + 响铃。oracle 实证。
  - 探针：`tests/audit2_c.rs::dot_after_block_insert_redoes_block_insert`

- [x] **D12.（P2）`3i` 里的退格不应取消 count 重复** — 方向键取消重复
  是对的（引擎与 vim 一致），但退格在 vim 里不取消。
  - 复现：`zz\n`，(0,0)，`3i` + `fo` + `<BS>` + `<Esc>`
  - 预期：`fffzz\n`；实际 `fzz\n`。oracle 实证。
  - 探针：`tests/audit2_c.rs::count_insert_repeat_survives_backspace`

## E. 可视、块可视、marks 与 changelist

- [x] **E1.（P1）块可视 `o` 按字节交换角点，矩形不变量破坏** — 角点
  交换应按（行，虚列）做，实际拿原始字节偏移互换。三个形状（oracle
  全部实证）：
  - a) `abcdef\nabcdef\n`，(0,0)，`<C-v>jlod`：vim 删 2 列得 `cdef\ncdef`；
    实际删 1 列
  - b) `<C-v>lod`（单行块）：vim 删 2 列；实际塌成 1 列
  - c) `abcdef\nab\n`，(0,3)，`<C-v>jod`：短行的字节列反推角点把块向左
    扩列，得 `aef\na`；vim `abef\nab`
  - 探针：`tests/audit2_e.rs::block_o_swaps_raw_bytes_instead_of_row_col_corners`

- [x] **E2.（P1）块可视 `O` 留下过期 virtual col** —
  - 复现：`abcdef\nabcdef\n`，(0,0)，`<C-v>jlOd`
  - 预期（删 2 列）：`cdef\ncdef\n`；实际 `acdef\nacdef\n`（span 错成
    1..2，只删 `b` 列）。oracle 实证。
  - 探针：`tests/audit2_e.rs::block_O_leaves_stale_virtual_column`

- [x] **E3.（P2）`'>` 存的是排他端点，两处落位偏差** —
  - a) charwise：`vllly` 后 `` `> ``：vim 落最后选中字符（offset 3）；
    实际 offset 4（排他尾字节）
  - b) linewise：`Vjy` 后 `` `> ``：vim 落末选中行的**末字符**；
    实际落（末行，col 0）
  - oracle 实证。
  - 探针：`tests/audit2_e.rs::tick_gt_charwise_lands_one_past_last_selected_char`、
    `tick_gt_linewise_lands_at_line_start_instead_of_last_char`

- [x] **E4.（P1）undo 不恢复被删行的 mark** — 宿主 undo 只换文本，引擎
  的 `adjust_delete` 已把 mark 塌到 0。
  - 复现：`  foo\nbar\nbaz\n`，(0,2)，`ma` `dd` `u` `` `a ``
  - 预期：mark 随文本回来（vim `getpos("'a")` = 第 1 行 col 5）；实际
    落 offset 0。oracle 实证。
  - 探针：`tests/audit2_e.rs::undo_does_not_restore_marks_of_deleted_line`

- [x] **E5.（P1）选中行被 `dd` 后 `gv` 跨行复选** — vim 把 `'<`/`'>`
  折叠到幸存行；引擎从 `floor(hi-1)` 反推光标、钳到上一行末字符，选出
  跨行区域，`d` 把两行合并（`ac\ndd`）。
  - 复现：`aa\nbb\ncc\ndd\n`，(1,0)，`vly` `dd` `gv` `d`
  - 预期：`aa\n\ndd\n`（重选 `cc` 行后删）；实际 `ac\ndd\n`。oracle 实证。
  - 探针：`tests/audit2_e.rs::gv_after_line_delete_reselects_across_line_break`

- [x] **E6.（P2）`` `[ ``/`` `] ``/`'[`/`']`（最后 yank/变更范围）未绑定** —
  响铃不动。vim：`yiw` 后 `` `] `` 落最后 yank 字符。
  - 复现：`foo bar\n`，(0,0)，`yiw` `` `] ``。预期 offset 2；实际响铃
    原地。oracle 实证。
  - 探针：`tests/audit2_e.rs::bracket_marks_unbound`

- [x] **E7.（P2）可视模式 `<C-o>` 未绑定** — vim 在可视模式按 `<C-o>`
  执行一次 jumplist 后跳并退出到 Normal（`visualmode()` 仍记得选区）；
  引擎把键交给宿主（Unknown，零反馈），停在 Visual。
  - 复现：6 行缓冲，(0,0)，`G` `v` `kk` `<C-o>`
  - oracle 实证（mode()=n、visualmode()=v）。
  - 探针：`tests/audit2_e.rs::ctrl_o_in_visual_should_exit_to_normal_mode`

- [x] **E8.（P1）`g;` 从最旧变更开始走，且首按假报 E662** — vim 的
  changelist 步进是最新优先、静默。
  - 复现：两次变更（`x`，然后 `A` + ` z`）后按 `g;`
  - 预期：落最新变更（offset 3）；实际落最旧（offset 1）；再次步进时
    还假报 `E662: At start of changelist` + 响铃（vim 同场景静默）。
    oracle 实证。
  - 探针：`tests/audit2_e.rs::changelist_walk_starts_at_oldest_and_falsely_reports_E662`

- [x] **E9.（P3）`^` 与 `gi` 停在被插入字符上，vim 停其后一位** —
  - 复现：`hello\n`，(0,1)，`l` + 插入 `XY` + `<Esc>`，`` `^ ``
  - 预期：offset 3（`Y` 之后）；实际 offset 2；`gi` + `Z` 得 `hXZYello`
    vs vim `hXYZello`。oracle 实证。
  - 探针：`tests/audit2_e.rs::caret_mark_and_gi_stop_on_last_inserted_char`

- [x] **E10.（P3）可视 `<` 无缩进可删时静默（待复核）** — Normal 路径
  的 `<<` 无缩进响铃已在昨天 L3 修复，可视算子路径漏了同一反馈；vim
  侧响铃在 typeahead 通道不可观测，此条的**文本行为**双方一致（都不动），
  差异只在响铃。修复 L3 时的 oracle 结论支持 vim 会响。
  - 复现：`foo\n`，(0,0)，`V<`
  - 探针：`tests/audit2_e.rs::visual_indent_left_noop_is_silent`

## F. 搜索与偏移

- [x] **F1.（P2）`gd`/`gD` 未绑定，且 `gd` 会误启 delete 算子** — trie
  未命中后 `g` 被丢、`d` 重新入队武装算子，后续 `x` 响铃报非法 motion。
  虽然无数据损坏，但形状错误；vim 里 `gd` 跳到局部声明。
  - 复现：`int x;\nuse x here\n`，(1,4)，`g d x`
  - 预期：跳到声明后 `x` 删除字符（`int ;`）；实际原地 + 响铃。
    oracle 实证。
  - 探针：`tests/audit2_f.rs::f1_gd_unbound_arms_delete_operator`

- [x] **F2.（P2）`g*`/`g#` 缺失，退化为整词 `*`/`#` 并多响一次铃** —
  - 复现：`foo bar\nfoofoo\nfoo\n`，(0,0)，`g *`
  - 预期：子串匹配、落 offset 8；实际整词匹配落 offset 15 + 响铃。
    `g#` 同族实证。
  - 探针：`tests/audit2_f.rs::f2_gstar_ghash_missing_degrade_to_whole_word`

- [x] **F3.（P2）`n`/`N` 不重放搜索偏移，首跳锚点也错位** —
  `Motion::SearchNext` 从不调用 `apply_search_offset`（与该字段自己的
  doc 注释矛盾）。
  - 复现：`foo x\nfoo y\n`，(0,0)，`/foo/e<CR>` `n` `n`
  - 预期（oracle 逐步）：2 → 8 → 2（每次都落匹配尾）；实际 8 → 0 → 0
    （首跳跳过光标处匹配、`n` 落裸匹配首）。
  - 探针：`tests/audit2_f.rs::f3_n_does_not_reapply_search_offset`

- [x] **F4.（P2）算子/可视模式下的搜索提示整体丢弃偏移** — cmdline 只在
  纯 motion 路径应用偏移。
  - 复现：`x foo y\n`，(0,0)，`d/foo/e<CR>`：预期 ` y\n`；实际 `foo y\n`
  - 可视同族：`v/foo/e<CR>` 光标落匹配首而非尾。oracle 实证。
  - 探针：`tests/audit2_f.rs::f4_operator_visual_search_ignore_offsets`

- [x] **F5.（P1）`b`/`e`/`s` 带数字尾时把数字当行数，vim 是字符数** —
  `parse_search_offset` 把数字建成 `line_shift`。
  - a) `/foo/b2`（匹配在第 3 行 col 5）：预期同行 col 7；实际移 2 行
  - b) `/foo/e2`：预期下一行 col 2；实际差两行
  - c) `/foo/s-2`：预期同行 col 3；实际跳到第 1 行。oracle 实证。
  - 探针：`tests/audit2_f.rs::f5_offset_sbe_count_is_chars_not_lines`

- [x] **F6.（P2）纯行偏移（`[N]`/`+N`/`-N`）保持匹配列，vim 落 col 1**
  - 复现：`x foo y\n    ind\nzzz\n`，(0,0)，`/foo/+1<CR>`
  - 预期：第二行 col 1（offset 8）；实际 offset 10（匹配列保留）。
    oracle 实证。
  - 探针：`tests/audit2_f.rs::f6_line_offset_lands_column1`

- [x] **F7.（P3）反向 `?` 的 incsearch 预览把前向最近匹配标为当前** —
  - 复现：`foo\nbar\nfoo bar\n`，(2,5)，`?o`（不回车）
  - 预期：current_highlight = 10..11（回车将落的位置）；实际 1..2。
  - 探针：`tests/audit2_f.rs::f8_incsearch_backwards_preview_marks_forward_match`

- [x] **F8.（P2）Ex 范围地址拒绝偏移尾** — `/pat/` 分支只剥尾部终止符，
  `foo/+1` 整体被当模式。
  - 复现：`foo\nbar\nbaz foo\n`，(0,0)，`:/foo/+1<CR>`
  - 预期：光标落 offset 8、无消息；实际原地 + `E16: Invalid range`。
    oracle 实证。
  - 探针：`tests/audit2_f.rs::f9_ex_range_address_rejects_offset_tail`

- [x] **F9.（P2）`[count]<C-f>`（`C-b`/`C-d`/`C-u` 同族）忽略 count** —
  - 复现：200 行缓冲，viewport (0,9)，`2<C-f>`
  - 预期：落第 19 行（oracle：1 次 → 行 10，2 次 → 行 19）；实际只滚
    一页（行 10）。oracle 实证（真控制字节投递）。
  - 探针：`tests/audit2_f.rs::f11_page_scroll_ignores_count`

## G. Ex 命令与命令行

- [x] **G1.（P1）`:{range}pu` 丢弃范围** — `ex_put` 完全不接收 range；
  `:0pu`（粘到文件顶）、`:5pu`、`:$pu` 全部落到光标行。
  - 复现：`a\nb\nc\nd\ne\n`，(2,0)，`:1y a<CR>` `:0pu a<CR>`
  - 预期：`a|a|b|c|d|e`；实际 `a\nb\nc\na\nd\ne\n`。oracle 实证（本机
    复核：vim 确实粘到顶部）。
  - 探针：`tests/audit2_d.rs::put_range_address_moves_insertion_point`

- [x] **G2.（P1）`:le {indent}` 参数被静默丢弃** — `ex_align` 里
  `let _ = indent;`。
  - 复现：`ab\n`，`:le 4<CR>`：预期 `    ab\n`；实际 `ab\n`
  - 范围形 `:1,2le 2<CR>` 同坏。oracle 实证。
  - 探针：`tests/audit2_d.rs::left_align_honors_indent_argument`

- [x] **G3.（P1）`noet` 下 `:ce`/`:ri` 只用空格填充，vim 先 TAB 后空格**
  - 复现：`ab\n`，`:set noet<CR>` `:ce 40<CR>`
  - 预期（od 实证）：`\t\t   ab`（19 列 = 2 个 TAB + 3 空格，ts=8）；
    实际 19 个字面空格。`et` 下 vim 才全用空格（也实证过）。
  - 探针：`tests/audit2_d.rs::center_padding_composes_tabs_under_noet`

- [x] **G4.（P1）`:delm A-Z` 删掉的是小写 mark `a`–`z`** — 范围两端点被
  lowercase，用户没点名的 mark 被毁。
  - 复现：`ma` 后 `:delm A-Z<CR>` 再 `:'a<CR>`
  - 预期：`a` 幸存；实际 `E20: Mark 'a not set`。oracle 实证。
  - 探针：`tests/audit2_d.rs::delmarks_uppercase_range_spares_lowercase`

- [x] **G5.（P2）`:put` 光标落第一粘贴行，vim 落最后粘贴行** —
  - 复现：`one\ntwo\nthree\n`，(2,0)，`:1,3y b<CR>` `:pu b<CR>`
  - 预期：光标行 6（1-based；3 行块的末行）；实际行 4。oracle 实证。
  - 探针：`tests/audit2_d.rs::put_cursor_on_last_pasted_line`

- [x] **G6.（P2）`:wqa`/`:exit`/`:qa`/`:xa`/`:wqall` 全部 E492** —
  - 复现：`:wqa<CR>`
  - 预期：save + close 请求、无消息；实际 `E492: Not an editor command:
    wqa`、`close_requested` 不置位。`:exit`、`:qa` 同族实证。
  - 探针：`tests/audit2_d.rs::quit_all_spellings_wqa_exit_exist`

- [x] **G7.（P1）地址偏移里数字前的空格被吞，静默算错行** — 引擎把
  `+` 读成裸 +1 就停了，比 vim 少走 1 行且无任何报错。
  - 复现：`l1..l10`，`:5 + 2d<CR>`
  - 预期（oracle 三连证）：删第 8 行（`:5 + 1d` 删第 7 行；`:5 + 2d 2`
    删第 8–9 行）；实际删第 6 行。
  - 探针：`tests/audit2_d.rs::address_offset_accepts_space_before_count`

- [x] **G8.（P2）命令行 `<Up>` 无前缀匹配，总召回最新历史** —
  `:h c_<Up>`：召回「以当前输入开头」的更早命令行。
  - 复现：5 行缓冲 `m1..m5`，`:1d<CR>:2d<CR>:3d<CR>`（剩 `m2\nm4\n`），
    然后 `:` `2` `<Up>` `<CR>`
  - 预期：召回 `:2d`、删掉 `m4`（剩 1 行）；实际召回最新 `:3d`，在
    2 行缓冲上报 E16 且什么都不删。PTY 实证（expect 逐键投递，
    本机复核通过）。
  - 探针：`tests/audit2_d.rs::cmdline_up_recalls_prefix_matching_history`

- [x] **G9.（P2）`:marks {name}` 过滤参数缺失（E492）** — vim 支持指定
  要列出的 mark。
  - 复现：`:marks a<CR>`。预期只列 a 一行；实际 E492。oracle 实证。
  - 探针：`tests/audit2_d.rs::marks_accepts_name_arguments`

- [x] **G10.（P3）命令行 `<C-v>` 字面引用缺失** — `<C-v><Esc>` 被吞、
  Esc 直接取消整个提示符；vim 插入字面 `^[`，回车后报
  `E492: Not an editor command: ^[`。
  - 探针：`tests/audit2_d.rs::cmdline_ctrl_v_quotes_next_key`

## H. `:set` 选项

- [x] **H1.（P2）`:set` 拼写族三缺口** — `:set invic`（取反拼写）、
  `:set ic&`（布尔重置为默认）、`:set foo?`（未知名查询）行为缺失：
  前两者报 E518，第三者只响铃无任何状态消息（vim 报
  `E518: Unknown option: foo?`）。oracle 实证。
  - 探针：`tests/audit2_d.rs::set_inv_spelling_toggles_boolean`、
    `set_ampersand_resets_boolean_option`、
    `set_unknown_name_query_reports_e518`

- [x] **H2.（P3）`:set ts?` 查询输出缺 vim 的两空格缩进** — vim 输出
  `  tabstop=8`（布尔选项为 `  ignorecase`）；引擎输出无缩进的裸文本。
  - 探针：`tests/audit2_d.rs::set_query_output_two_space_padding`

- [x] **H3.（P3）`:retab 0` 被拒（E488），vim 接受且 0 = 用当前
  `'tabstop'`** —
  - 复现：noet 下 8 空格行 `:retab! 0<CR>`。预期 `\tab` 无错；实际
    `E488: Trailing characters: 0`、缓冲不动。oracle 实证。
  - 探针：`tests/audit2_d.rs::retab_zero_uses_current_tabstop`

---

## 本轮证伪清单（oracle 已证引擎与 vim 一致，免下轮重查）

- `ge` 无词尾时回落首字符；`dit` 在闭标签 `<` 上取自身块（objects.rs
  里「enclosing」注释与 vim 相反，注释该改）；`d}`/`d{` 的列 1 linewise
  提升；`}`/`{` 在空白串内部、`}` 于 EOF；`d2w` 跨行；`j` 在 `|` 之后的
  列保持；`2Fb` 整体失败；相邻行 `daw` 吸词；`2t` 异目标计数。
- `vi(` 不爬层；空行上 `vx` 删整行；`3vj` count 被 vim 丢弃；`vas` 于
  EOF 选全缓冲；块 `I`/块 `A` 的短行与填充模型；`v$p` 选区形状；可视
  算子 count 不放大选区（`vll3d`）；`V>`/`V<` 光标规则；linewise `o`/`O`
  保持选区；`%`/`gJ`/`vJ`/`~` 的 count。
- `2D`/`2C` 行中起、count 触末行；`]p` 配 charwise 寄存器退化；`guw`/
  `gp` 行级光标；`v_D`；`:s` 更新 `"/`；`append_to_named` 无幽灵空行。
- `<C-w>` 前后 CJK 词删除；`i<C-r>` 后 Esc；空寄存器 `@z`；`.` 重放
  `I`/`gI` 的定位；`3i` 含方向键取消重复。
- E348（空行 `*`）、E35（冷启动 `n`）、TAB 上 `~` 原地前进、两行缓冲
  `ddp`、`2*` 跳过语义、`;` 的 count、NUL 上 `x`。
- 打包 count（`:d3`）、`:4,5d 99` 钳制、`:y` 不动光标、`:5` 落首非空白、
  `:%s///n` 消息文本、`:y A`/`:d A` 大写追加、`:set ts?`/`ts ?` 查询形、
  `:ce -5` 的 vim 侧静默、超宽行 `:ce`/`:ri`。

## 敌意扫描结果（无 panic）

视口极值（`(usize::MAX, *)`、`(0,0)`、first>last）× 滚动/跳转全家、空缓冲
× 37 种操作形状、单行缓冲、裸 `\r`/NUL 文本、21 位 count、畸形偏移尾——
零 panic（昨天 A1 的 saturating 修复覆盖住了 H/M/L 之外的键位）。
