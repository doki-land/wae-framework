# wae-harness

WAE 自动化 harness：在进程内驱动一个真实 WebView 宿主，并可在测试模式下暴露 loopback 控制通道。

- `Harness::open` / `ready` / `eval` / `wait_for` / `screenshot` / `close`
- `serve_from_env`：`WAE_HARNESS_PORT` 存在时只监听 `127.0.0.1`，`WAE_HARNESS_TOKEN` 可选校验
- 平台缺少脚本执行或截图能力时返回结构化 `Unsupported`，不伪造成功
