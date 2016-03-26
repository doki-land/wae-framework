# `wae`

WAE Rust **门面库**（不是 CLI bin）。统一导出：

| 依赖 | 内容 |
|------|------|
| `wae-napi` | 薄 Node-API |
| `wae-macros` | `#[function]` / `register_module!` |
| `wae::webview`（feature `webview`，默认开） | Win32 WebView2 |
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
