#!/usr/bin/env node
/**
 * Product CLI skeleton: command tree + locales via `@wae/commander`.
 * After scaffolding, replace `hello` with real commands.
 */
import fs from 'node:fs';
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

/**
 * `main` runs this CLI, `imported` keeps it quiet for tests and libraries, and
 * `unresolved` fails instead of reporting a silent success for a broken entry.
 */
function relationToEntry(invoked: string): 'main' | 'imported' | 'unresolved' {
    const self = fileURLToPath(import.meta.url);
    try {
        // Package-manager symlinks and junctions point at another path for this file.
        if (fs.realpathSync(invoked) === fs.realpathSync(self)) return 'main';
    }
    catch {
        // A dangling link still counts as a launch attempt when it carries this entry's name.
    }
    if (path.basename(invoked) !== path.basename(self)) return 'imported';
    // Another CLI can share the file name, so only the same package is a broken entry.
    return nearestPackageName(invoked) === nearestPackageName(self) ? 'unresolved' : 'imported';
}

function nearestPackageName(target: string): string | undefined {
    let dir = path.dirname(target);
    while (true) {
        const manifest = path.join(dir, 'package.json');
        if (fs.existsSync(manifest)) {
            try {
                const parsed = JSON.parse(fs.readFileSync(manifest, 'utf8')) as { name?: unknown };
                return typeof parsed.name === 'string' ? parsed.name : undefined;
            }
            catch {
                return undefined;
            }
        }
        const parent = path.dirname(dir);
        if (parent === dir) return undefined;
        dir = parent;
    }
}

const entry = process.argv[1];
if (entry !== undefined) {
    const relation = relationToEntry(entry);
    if (relation === 'main') {
        process.exitCode = await cli.parse(process.argv);
    }
    else if (relation === 'unresolved') {
        console.error(`[{{name}}] cannot resolve entry "${entry}" to "${fileURLToPath(import.meta.url)}"`);
        process.exitCode = 1;
    }
}
