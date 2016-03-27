# `@wae/wae`

WAE meta CLI (sole bin: `wae`). Built on `@wae/commander` for scaffolding, building, staging native npm addons, and publishing.

## Commands

```bash
wae new <name> [--with-gui] [--out <dir>]
wae build --project <dir> --package <name>
wae dist --project <dir> --package <name> --out <dir> --file <name>
wae publish --dir <path> [--dir <path>...] [--execute]
```

Product strings live in this package's `locales/`. Framework chrome comes from `@wae/commander`.

## Develop

```bash
pnpm --filter @wae/wae build
pnpm --filter @wae/wae test
pnpm exec wae --help
```
