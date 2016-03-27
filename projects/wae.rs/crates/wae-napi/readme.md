# `wae-napi`

自研薄 Node-API 绑定（**不**依赖 `napi` / `napi-derive` / `napi-build`）。

目录名 `wae-napi`，crate 名同为 `wae-napi`。

## 链接模型

对齐 napi-rs 的 **dyn-symbols**：addon 加载时通过 `libloading` 从宿主进程（`node.exe` / `libnode` 等）解析 `napi_*`，编译期不链 `node.exe`、不伪造 import lib。

## 用法要点

- 在 `napi_register_module_v1` 开头调用 [`setup`](src/lib.rs)
- 用 [`Env`](src/lib.rs) / [`JsValue`](src/lib.rs) / [`register_exports`](src/lib.rs) / [`catch_callback`](src/lib.rs) 编写导出

下游产品优先依赖门面 crate `wae`（及 `#[wae::function]` / `register_module!`）。
