# `@wae/commander`

面向 i18n 的 TypeScript CLI 框架。**不**绑定 `@wae/wae`。

- 流式注册；命令 / 选项第二参数是 **message id**
- **帮助从命令树派生**，不要把 Usage 大段塞进语言包
- **locales 目录加载**：`locales.json` + `<locale>/*.json` → flatten → `LocalizePlugin`
- 框架 chrome / 错误 id 为 `commander.*`，自带极简英文 fallback（产品可覆盖）
- 交互式帮助用小型 ANSI 主题；`NO_COLOR` 关闭，`FORCE_COLOR` 在非 TTY 下强制开启
- **本包不含** 产品语言包

## 快速开始

```ts
import { createCli } from '@wae/commander';

const cli = createCli('my-cli')
  .locales(new URL('./locales', import.meta.url).pathname)
  .intro('cli.intro')
  .command('build', 'cli.cmd.build')
  .option('--out-dir, -o <dir>', 'cli.opt.out-dir')
  .action(async (options) => {
    /* options._ 为位置参数；options['out-dir'] … */
  });

await cli.parse(process.argv);
```

## 分层

```text
@wae/commander  = CLI 树 + locales 加载 + commander.* chrome
@wae/wae        = 元工具 bin + 命令注册 + locales/ 内容
下游应用         = 产品 bin + 命令注册 + locales/
```
