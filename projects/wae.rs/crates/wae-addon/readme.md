# wae-addon

WAE 门面的 Node-API addon（cdylib）。使用 `wae-napi` 的手写 C ABI（dyn-symbols），不依赖 napi-rs。

## 导出

| JS 名 | 说明 |
|-------|------|
| `version()` | 当前 addon 版本 |
| `capabilities()` | `platform` / `arch` / `audit` / `version` |
| `probe()` | 最小能力探针，经 `wae-audit` 记录 `wae.native.probe` |
| `auditQuery(filter?)` | 审计查询，返回 JSON 数组文本 |

产物查找、暂存与平台包校验由 `@wae/wae` 的 `native-stage.ts` 单一实现，本 addon 不重复这套规则。

## 构建

在仓库根执行 `pnpm build:napi`，产物由 `wae native build` 暂存到对应平台包。
