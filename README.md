# vimcore

纯 Rust 的**宿主无关 Vim 引擎**：模式机、motions、operators、text objects、
寄存器、搜索、marks、宏、`.` 重放、Ex 命令。零 GUI、零终端依赖（运行时仅
`regex` + `unicode-width`），通过两个宿主 trait 接入任何 Rust UI——gpui、
ratatui、自绘 ANSI 皆可。

```
┌──────────────────────────────────────────────┐
│ 你的应用（gpui / ratatui / 自绘 ANSI / …）    │
├──────────────────────────────────────────────┤
│ 前端集成 crate（各管一段）                    │
│  · gpui-vim      keystroke 拦截 + 模式 UI    │
│  · crossterm-vim 键转换 + 缓冲 + 宿主副作用  │
│  · ratatui-vim   状态绘制层                  │
├──────────────────────────────────────────────┤
│ vimcore（本 crate）  纯引擎：                │
│  · 模式机 · 按键流水线 · Trie 命令表         │
│  · motion · operator · text object          │
│  · 寄存器 · 搜索 · marks · 宏 · `.` · Ex    │
├──────────────────────────────────────────────┤
│ 宿主 trait：VimBuffer(Mut) + VimHost（你实现）│
└──────────────────────────────────────────────┘
```

分层形态承自 IdeaVim 的 `vim-engine`：引擎持有光标与模式，宿主只实现
buffer 读写与视口/剪贴板/undo 钩子。

## 用法

```rust
use vimcore::buffer::VimBufferMut;
use vimcore::key::Key;
use vimcore::state::{Ctx, KeyResult, VimState};

let mut vim = VimState::new();
let mut buf = MyBuffer::default();   // 实现 VimBuffer + VimBufferMut
let mut host = MyHost::default();    // 实现 VimHost（视口/剪贴板/undo 钩子）

let mut ctx = Ctx { buf: &mut buf, host: &mut host };
match vim.handle_key(&mut ctx, Key::parse("dd")) {
    KeyResult::Handled => {}   // 引擎已处理，状态与 buffer 已更新
    KeyResult::Unknown => {}   // 引擎不要这个键，还给宿主
}
```

引擎内部统一 UTF-8 字节偏移，渲染所需的全部状态（模式、光标、可视选区、
搜索高亮）都可读。

## 接入质量有测试下限（TCK）

宿主在自己的 buffer / host 实现上跑引擎自带的契约测试，把「接入质量靠读
文档」变成「有可执行验收」：

```rust,ignore
#[test]
fn my_buffer_meets_engine_contract() {
    let mut buf = MyBuffer::default();
    vimcore::tck::buffer_edit_contract(&mut buf).unwrap();
    let mut host = MyHost::default();
    vimcore::tck::engine_smoke_contract(&mut buf, &mut host).unwrap();
}
```

## 实际消费方

- [gpui-vim](https://github.com/beyondlex/gpui_vim)：GPUI 集成（[PandaGit](https://github.com/beyondlex/pandagit) 在用）
- [crossterm-vim](https://github.com/beyondlex/crossterm_vim)：crossterm 通用集成 → ratatui-vim 绘制层（[jk](https://github.com/beyondlex/jks) 在用）

## 开发

```bash
cargo test            # 无头测试：引擎全量 + TCK 契约 + 随机 fuzz
cargo run --release --example bench_probe   # 宿主实现的性能探针
```

`NOTES.md` 记录审查结论：已修复的 bug（vim 9.1 探针实证）、与 vim 的
已知分歧、性能与体验备注——接入前值得通读。

## 名字沿革

`vim-core` → `gpui-vim-core`（2026-09-20，随 gpui_vim 上 crates.io）→
`vimcore`（2026-09-28，独立建仓）。引擎从来不属于任何一个前端，名字不再
锚定 gpui。

## License

MIT OR Apache-2.0
