#!/usr/bin/env node
/**
 * `@wae/wae` meta CLI: scaffold / build / stage native addons / npm publish.
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createCli, type ParsedOptions } from '@wae/commander';
import { copyDir, removePath, rewritePlaceholders } from './fs-utils.js';
import { cargoBuildRelease, stageNative } from './native-stage.js';
import { localesRoot, openToolsTemplateRoot } from './paths.js';

/** Build the product CLI (tests may call this directly). */
export function buildCli() {
    const cli = createCli('wae').locales(localesRoot()).intro('cli.intro');

    cli.command('new', 'cli.cmd.new')
        .option('--with-gui', 'cli.opt.with-gui')
        .option('--out <dir>', 'cli.opt.out', { default: '.' })
        .actionCtx(async (options, args) => cmdNew(options, args));

    cli.command('build', 'cli.cmd.build')
        .option('--project <dir>', 'cli.opt.project', { required: true })
        .option('--package <name>', 'cli.opt.package', { required: true })
        .actionCtx(async (options) => cmdBuild(options));

    cli.command('dist', 'cli.cmd.dist')
        .option('--project <dir>', 'cli.opt.project', { required: true })
        .option('--package <name>', 'cli.opt.package', { required: true })
        .option('--out <dir>', 'cli.opt.dist-out', { required: true })
        .option('--file <name>', 'cli.opt.file', { required: true })
        .actionCtx(async (options) => cmdDist(options));

    cli.command('publish', 'cli.cmd.publish')
        .option('--dir <path>...', 'cli.opt.dir', { required: true })
        .option('--execute', 'cli.opt.execute')
        .actionCtx(async (options) => cmdPublish(options));

    return cli;
}

async function cmdNew(options: ParsedOptions, args: string[]): Promise<number> {
    const name = args[0]?.trim();
    if (!name) {
        console.error('wae new requires a name: wae new <name> [--with-gui] [--out <dir>]');
        return 2;
    }
    const outDir = typeof options.out === 'string' ? options.out : '.';
    const withGui = Boolean(options['with-gui']);
    const template = openToolsTemplateRoot();
    if (!fs.existsSync(template)) {
        console.error(`template directory not found: ${template}`);
        return 1;
    }
    const dest = path.resolve(outDir, name);
    if (fs.existsSync(dest)) {
        console.error(`destination already exists: ${dest}`);
        return 1;
    }
    copyDir(template, dest);
    rewritePlaceholders(dest, name);
    if (!withGui) {
        removePath(path.join(dest, 'gui'));
        removePath(path.join(dest, 'dist', 'gui'));
    }
    console.log(`created ${dest} (with_gui=${withGui})`);
    return 0;
}

async function cmdBuild(options: ParsedOptions): Promise<number> {
    const project = String(options.project);
    const pkg = String(options.package);
    try {
        const root = path.resolve(project);
        cargoBuildRelease(root, pkg);
        console.log(`built -p ${pkg} in ${root}`);
        return 0;
    } catch (err) {
        console.error(`wae build failed: ${err instanceof Error ? err.message : err}`);
        return 1;
    }
}

async function cmdDist(options: ParsedOptions): Promise<number> {
    try {
        const dest = stageNative({
            projectRoot: path.resolve(String(options.project)),
            cargoPackage: String(options.package),
            destDir: path.resolve(String(options.out)),
            destFileName: String(options.file),
        });
        console.log(`staged → ${dest}`);
        return 0;
    } catch (err) {
        console.error(`wae dist failed: ${err instanceof Error ? err.message : err}`);
        return 1;
    }
}

async function cmdPublish(options: ParsedOptions): Promise<number> {
    const raw = options.dir;
    const dirs = Array.isArray(raw) ? raw : typeof raw === 'string' ? [raw] : [];
    if (!dirs.length) {
        console.error('provide at least one --dir');
        return 2;
    }
    const execute = Boolean(options.execute);
    for (const d of dirs) {
        const pack = path.resolve(d);
        if (!fs.existsSync(path.join(pack, 'package.json'))) {
            console.error(`--dir is missing package.json: ${pack}`);
            return 1;
        }
        const args = ['publish', '--access', 'public'];
        if (!execute) args.push('--dry-run');
        const result = spawnSync('npm', args, {
            cwd: pack,
            stdio: 'inherit',
            shell: process.platform === 'win32',
        });
        if (result.status !== 0) {
            console.error(`npm publish failed: ${pack}`);
            return 1;
        }
    }
    return 0;
}

async function main(): Promise<void> {
    process.exitCode = await buildCli().parse(process.argv);
}

const thisFile = fileURLToPath(import.meta.url);
const invoked = process.argv[1] ? path.resolve(process.argv[1]) : '';
if (invoked && path.resolve(thisFile) === invoked) {
    void main();
}
