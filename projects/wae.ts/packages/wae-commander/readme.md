# `@wae/commander`

i18n-first TypeScript CLI framework under the `@wae` scope. **Not** bound to `@wae/wae`.

- Fluent registration; command / option second args are **message ids**
- **Help is derived** from the command tree — do not paste Usage walls into catalogs
- **Locales directory loading**: `locales.json` + `<locale>/*.json` → flatten → `LocalizePlugin`
- Framework chrome/err ids are `commander.*` with tiny English fallbacks (products may override)
- Interactive help uses a small ANSI theme; `NO_COLOR` disables it, `FORCE_COLOR` enables it outside a TTY
- **No product language packs** in this package

## Quick start

```ts
import { createCli } from '@wae/commander';

const cli = createCli('my-cli')
  .locales(new URL('./locales', import.meta.url).pathname) // or absolute path
  .intro('cli.intro')
  .command('build', 'cli.cmd.build')
  .option('--out-dir, -o <dir>', 'cli.opt.out-dir')
  .action(async (options) => {
    /* options._ positionals; options['out-dir'] … */
  });

await cli.parse(process.argv);
```

Locales layout (any product):

```text
<localesRoot>/
  locales.json          # { defaultLocale, locales: [{ id }], fallback? }
  en-US/*.json          # flat or nested → dotted ids
  zh-CN/*.json          # optional
```

`.locales(root)` also registers root `--locale <id>` (peeled before the command). Later `.use(plugin)` overrides the locales plugin.

## Layering

```text
@wae/commander  = CLI tree + locales loader + commander.* chrome
@wae/wae        = meta CLI bin + command registration + locales/ content
Downstream apps = product bin + command registration + locales/
```
