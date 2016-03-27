# `wae-dist`

通用辅助：在产品工作区 `cargo build -p <package> --release`，并把 cdylib 拷到 npm 平台包目录。

不绑定具体下游产品；调用方自行提供 Cargo 包名、产物目录与目标文件名。

经门面 feature `dist` 时以 `wae::dist` 导出。TypeScript 元 CLI `@wae/wae` 的 `dist` 命令有同构实现。
