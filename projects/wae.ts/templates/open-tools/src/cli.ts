#!/usr/bin/env node
/**
 * Product CLI skeleton: command tree + locales via `@wae/commander`.
 * After scaffolding, replace `hello` with real commands.
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
