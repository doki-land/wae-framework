# `wae-audit`

WAE 系产品共用的审计记录设施：

- 固定 schema `wae.audit/v0`（时间戳、actor、应用与版本、能力、脱敏参数、模式、目标、结果与错误码、耗时、主机、会话）
- 参数脱敏（凭据 / 私钥 / 密码 / token）
- append-only 本机 JSONL，进程内互斥追加，按大小轮转
- 按 actor、时间、能力过滤的查询接口

`actor` 只记录身份（`user` / `agent`），角色与权限判定属于独立授权模型，不写入 `actor`。

门面经 feature `audit` 以 `wae::audit` 导出，并在门面提供 napi 侧的记录与查询入口。
本 crate 不远程上报，不做服务端集中存储，也不提供密码学防篡改证明。
