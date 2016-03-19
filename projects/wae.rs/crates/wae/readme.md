# `wae`

纯 Rust 元 CLI：为「Rust 核心 + 薄 N-API + TS 重组」类原生 npm 工具提供脚手架、构建与分发编排。

**不**绑定任何具体下游产品；`--project` / `--package` / `--out` 均由调用方显式传入。

## 命令

| 命令 | 作用 |
|---|---|
| `wae new <name>` | 从本地模板生成工具骨架（可选 `--with-gui`） |
| `wae build --project <dir> --package <crate>` | 在产品工作区 release 构建指定 Cargo 包 |
| `wae dist --project <dir> --package <crate> --out <平台包目录> --file <文件名>` | 构建并将 cdylib 暂存为 `.node` |
| `wae publish --dir <npm包> [--execute]` | 对给定 npm 目录执行 publish（默认 dry-run） |

```bash
cargo run -p wae -- new demo --with-gui --out .
cargo run -p wae -- build --project path/to/tool --package tool-napi
cargo run -p wae -- dist --project path/to/tool --package tool-napi --out path/to/tool-win32-x64 --file tool.win32-x64.node
```
