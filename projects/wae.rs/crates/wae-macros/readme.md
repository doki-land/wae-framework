# `wae-macros`

WAE 过程宏（配合门面 crate `wae` 使用）。

## 宏

- `#[wae::function]` / `#[wae::function(js_name = "...")]`：把 `fn(CallContext) -> Result<JsValue>` 包成 Node-API `extern "C"` 回调
- `wae::register_module![a, b, …]`：生成 `napi_register_module_v1`（含 `setup` + `register_exports`）

请通过门面 `wae` 引用（`use wae::{function, register_module}`），不要单独依赖本 crate 写业务代码。
