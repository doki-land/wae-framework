# `@wae/wae`

WAE 元 CLI（唯一 bin：`wae`）。基于 `@wae/commander`，面向「Rust 能力 + 薄 N-API + TS 重组」的工具产品脚手架与分发。

## 命令

```bash
wae new <name> [--with-gui] [--out <dir>]
wae build --project <dir> --package <name>
wae dist --project <dir> --package <name> --out <dir> --file <name>
wae publish --dir <path> [--dir <path>...] [--execute]
```

产品文案在本包 `locales/`；框架 chrome 由 `@wae/commander` 提供。

## 开发

```bash
pnpm --filter @wae/wae build
pnpm --filter @wae/wae test
pnpm exec wae --help
```
