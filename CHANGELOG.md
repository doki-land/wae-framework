# Changelog

## 0.0.6

- `wae.config.ts` 支持显式窗口策略 `window.mode`（`single` / `independent` / `primary-secondary`）与 `secondLaunch`，`wae build` 在宿主不支持的模式上以 `window_mode_unsupported` 失败，不再静默降级。`wae new` 生成的工程默认声明 `single` + `focus`。
- 新增 `wae-plugins` 宿主：严格解析 `plugin.json`，集中插件错误码，收窄作用域，并为候选处置附加归因与证据。
- WebView 宿主：首次导航完成后才显示窗口，设置窗口图标，关闭 WebView2 occlusion，并在宿主合同中加入窗口聚焦。
- 新增 `wae-shell`，用系统默认程序打开文件。
- Win32 文件拖放经 `accept_file_drop` 路由到页面。
- 修复 Linux 后端重复 `is_alive` 导致的构建失败，插件清单校验改用 `contains` 判断切片成员。

### 从 0.0.5 升级

把工程根 `devDependencies` 的 `@wae/wae` 与子包 `dependencies` 的 `@wae/commander` 一起改为 `0.0.6`，执行 `pnpm install` 后重新构建。需要显式窗口策略的工程可以补上 `window` 键。也可以直接重新运行 `wae new` 生成新工程。

## 0.0.5

- 修复 pnpm 符号链接下 CLI 入口守卫失效：`wae build` 不再退出 0 却没有任何产物。入口文件无法解析到自身时改为非零退出并打印诊断，不再静默成功。
- 生成工程的 `@wae/wae` / `@wae/commander` 依赖改由 `wae new` 写入当前 CLI 版本（精确版本），模板不再保存手写的旧版本或 `0.0.0` 占位版本。
- 模板根清单补上 `@wae/wae` 依赖，并把生成工程的 bin 链接进根 `node_modules/.bin`，`pnpm exec <app>` 可直接使用。
- 脚手架在占位符替换后执行 `cargo fmt --all`，生成工程第一次 `pnpm check:rust` 不再因格式化失败；模板同时修正 `clippy::explicit_auto_deref`。
- skills 审计遇到缺少 `describe()` 的 `@wae/commander` 时给出可操作的构建错误。

### 从 0.0.4 升级

0.0.4 生成的工程在 pnpm 符号链接安装下无法真正执行 `wae build`。请把工程根 `devDependencies` 的 `@wae/wae` 和子包 `dependencies` 的 `@wae/commander` 一起改为 `0.0.5`，执行 `pnpm install` 后重新构建。也可以直接重新运行 `wae new` 生成新工程。
