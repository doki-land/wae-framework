# wae-plugins

声明式插件宿主。插件只提交候选与处置请求，判定与执行留在应用 core。

- `manifest`：`plugin.json` 严格解析与整包原子拒绝。
- `scope`：作用域解析、运行期交集收窄与候选越界判定。
- `candidate`：`wae.plugin-candidate/v0` 归因结构与证据复算。
- `error`：稳定错误码与退出码的唯一集中定义处。

首版只接受 `wae.plugin/v0` 数据包，不执行代码，不提供文件句柄。
