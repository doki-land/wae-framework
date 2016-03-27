# {{name}}

Scaffolded by `wae new` from the `open-tools` template.

- CLI uses [`@wae/commander`](https://www.npmjs.com/package/@wae/commander): command / option second args are message ids
- Product copy lives under `locales/` (`locales.json` + `<locale>/*.json`)
- Framework chrome (Usage / Options / error codes) comes from `@wae/commander` `commander.*` fallbacks — do not paste it into the product catalog

```bash
pnpm install
pnpm build
node dist/cli.js hello --name world
```

Replace `src/index.ts` / `src/cli.ts` with your real library surface and commands.
