# `wae-dist`

原生 npm 平台包暂存辅助库：在任意产品 Cargo 工作区中 `cargo build -p <package> --release`，再把 cdylib 拷贝到调用方指定的平台包目录与文件名。

本 crate **不知道**任何下游产品名字；产品布局、包名与产物文件名全部由 [`StageNative`](src/lib.rs) 参数提供。

```rust
use wae_dist::{StageNative, stage_native};

stage_native(&StageNative {
    project_root: "...".into(),
    cargo_package: "my-tool-napi".into(),
    dest_dir: ".../my-tool-win32-x64".into(),
    dest_file_name: "my-tool.win32-x64.node".into(),
})?;
```
