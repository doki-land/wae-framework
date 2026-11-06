# `wae`

WAE Rust **门面库**（不是 CLI bin）。统一导出：

| 依赖 | 内容 |
|------|------|
| `wae-napi` | 薄 Node-API |
| `wae-macros` | `#[function]` / `register_module!` |
| `wae::webview`（feature `webview`，默认开） | 按目标平台选择：Windows Win32 WebView2 / macOS WKWebView / Linux WebKitGTK |
| `wae::dist`（feature `dist`，默认开） | cdylib 暂存 |

## 用法

```toml
[dependencies]
wae = { path = "../wae" }
# 或关闭可选面：
# wae = { path = "../wae", default-features = false }
```

```rust
use wae::{function, register_module, CallContext, JsValue, Result};

#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(env!("CARGO_PKG_VERSION"))
}

#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    ctx.env().undefined()
}

register_module![version, open_gui];
```

产品命令行请用 `@wae/wae` / `@wae/commander`。

## 跨平台约束

`wae::webview` 在 Windows / macOS / Linux 暴露同一组符号与签名。下游应用（含 `napi` 层）
不得写平台 `cfg`，也不得用字符串表达「仅 Windows 支持」；平台缺少能力时使用
`WebViewError::Unsupported { capability, platform }` 结构化判别。
详见工作区 `AGENTS.md` 索引中的 WAE 设计文档。

