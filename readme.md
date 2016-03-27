# WAE

面向「Rust 能力 + 薄 N-API + TS 重组（可选自研 WebView）」的工具产品元仓。

- **元 CLI**：TypeScript 包 `@wae/wae`（唯一 bin：`wae`）
- **应用 CLI 框架**：`@wae/commander`（i18n-first 命令树）
- **Rust 库**：门面 `wae` + 宏 `wae-macros` + 薄 N-API / Win32 WebView2 / dist（**无** Rust 产品 bin）

```text
projects/wae.ts/packages/
  wae-commander       # @wae/commander
  wae                 # @wae/wae（new / build / dist / publish）
projects/wae.ts/templates/
  open-tools          # wae new 模板（commander + locales）
projects/wae.rs/crates/
  wae                 # Rust 门面库（re-export + #[function] / register_module!）
  wae-macros          # 过程宏
  wae-napi            # 自研薄 Node-API（dyn-symbols，无 napi-rs）
  wae-webview-win32   # 自研 WebView2 宿主（pumpEvents）→ wae::webview
  wae-dist            # 通用 cdylib → 平台包暂存 → wae::dist
```

本仓 **不**内嵌或默认指向任何下游产品仓库；产品通过 path / 版本依赖上述包与 crate。

语言约定见 [`AGENTS.md`](./AGENTS.md)：npm `@wae/*` 与模板用英文，Cargo 文档用中文。

## 检查

```bash
pnpm install
pnpm run build:ts
pnpm run test:ts
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
pnpm exec wae --help
pnpm exec wae new demo-tool --out .
```

## N-API 链接模型

`wae-napi` 采用 dyn-symbols：addon 在 `napi_register_module_v1` 时用 `libloading` 从宿主解析 `napi_*`，不在链接期依赖 `node.exe`。
