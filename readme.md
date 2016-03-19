# WAE

纯 Rust 元工具仓：脚手架 + 构建编排 + 原生 npm 分发辅助。面向「Rust 能力 + 薄 N-API + TS 重组（可选自研 WebView）」的工具产品。

```text
projects/wae.rs/crates/
  wae                 # clap 元 CLI：new / build / dist / publish
  wae-napi            # 自研薄 Node-API（dyn-symbols，无 napi-rs）
  wae-webview-win32   # 自研 WebView2 宿主（pumpEvents）
  wae-dist            # 通用 cdylib → 平台包暂存
```

本仓 **不**内嵌或默认指向任何下游产品仓库；产品通过 path / 版本依赖上述 crate，并由调用方传入 `--project` / `--package` 等参数。

## 检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## N-API 链接模型

`wae-napi` 采用 dyn-symbols：addon 在 `napi_register_module_v1` 时用 `libloading` 从宿主解析 `napi_*`，不在链接期依赖 `node.exe`。
