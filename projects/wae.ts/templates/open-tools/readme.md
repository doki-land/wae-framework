# {{name}}

由 `wae new` 从 `open-tools` 模板生成。

- CLI 基于 [`@wae/commander`](https://www.npmjs.com/package/@wae/commander)：命令 / 选项第二参是 message id
- 产品文案在 `locales/`（`locales.json` + `<locale>/*.json`）
- 框架 chrome（Usage / Options / 错误码）由 `@wae/commander` 的 `commander.*` fallback 提供，不必抄进产品 catalog

```bash
pnpm install
pnpm build
node dist/cli.js hello --name world
```

把 `src/index.ts` / `src/cli.ts` 换成真实库面与命令即可。
