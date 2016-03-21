#!/usr/bin/env node
/**
 * 产品 CLI 骨架：命令树 + locales 由 `@wae/commander` 驱动。
 * 生成后把 `hello` 换成真实业务命令即可。
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createCli } from '@wae/commander';
import { hello } from './index.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.join(here, '..', 'locales');

const cli = createCli('{{name}}')
    .locales(localesDir)
    .intro('cli.intro');

cli.command('hello', 'cli.cmd.hello')
    .option('--name <text>', 'cli.opt.name', { default: '{{name}}' })
    .actionCtx(async (options, _args, ctx) => {
        const name = typeof options.name === 'string' ? options.name : '{{name}}';
        ctx.stdout(hello(name));
        return 0;
    });

const code = await cli.parse(process.argv);
process.exitCode = code;
