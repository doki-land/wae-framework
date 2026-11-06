# wae-addon

WAE 门面的 Node-API addon（cdylib）。使用 `wae-napi` 的手写 C ABI（dyn-symbols），不依赖 napi-rs。

## 导出

| JS 名 | 说明 |
|-------|------|
| `version()` | 当前 addon 版本 |
| `capabilities()` | `platform` / `arch` / `audit` / `version` / `atomic` |
| `probe()` | 最小能力探针，经 `wae-audit` 记录 `wae.native.probe` |
| `auditQuery(filter?)` | 审计查询，返回 JSON 数组文本 |
| `findArtifact({ releaseDir, package })` | 按 Cargo 包名查找 cdylib，成功 `{ ok, value: { path } }` |
| `stageArtifact({ artifact, destDir, fileName })` | 把产物复制为平台包内的 `.node`，`fileName` 必须以 `.node` 结尾 |
| `validatePlatformPackage({ dir })` | 校验平台包 `os` / `cpu` / `files` / `main`，并要求产物真实存在 |

原子能力只接受调用方传入的显式路径与包名，不内嵌项目名或仓路径，也不承担构建编排。
预期失败不抛异常，返回 `{ ok: false, code, message }`，`code` 为 `artifact_missing`、
`platform_package_invalid`、`invalid_argument` 或 `io_error`。

## 构建

在仓库根执行 `pnpm build:napi`，产物由 `scripts/build-napi.ts` 暂存到对应平台包。
