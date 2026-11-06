#!/usr/bin/env node
/**
 * `@wae/wae` meta CLI: scaffold / build / stage native addons / npm publish.
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createCli, type ParsedOptions } from '@wae/commander';
import { copyDir, removePath, rewritePlaceholders, SCAFFOLD_NAME_PATTERN } from './fs-utils.js';
import { cargoBuildRelease, stageNative } from './native-stage.js';
import { localesRoot, SCAFFOLD_TEMPLATES, type ScaffoldTemplate, templateRoot } from './paths.js';

/** Build the product CLI (tests may call this directly). */
export function buildCli() {
    const cli = createCli('wae').locales(localesRoot()).intro('cli.intro');

    cli.command('new', 'cli.cmd.new')
        .option('--template <name>', 'cli.opt.template', { default: 'open-tools' })
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
        .option('--tag <ref>', 'cli.opt.tag')
        .option('--cargo <path>', 'cli.opt.cargo')
        .option('--execute', 'cli.opt.execute')
        .actionCtx(async (options) => cmdPublish(options));

    return cli;
}

function isScaffoldTemplate(value: string): value is ScaffoldTemplate {
    return (SCAFFOLD_TEMPLATES as readonly string[]).includes(value);
}

async function cmdNew(options: ParsedOptions, args: string[]): Promise<number> {
    const name = args[0]?.trim();
    if (!name) {
        console.error('wae new requires a name: wae new <name> [--template open-tools|open-app] [--out <dir>]');
        return 2;
    }
    if (!SCAFFOLD_NAME_PATTERN.test(name)) {
        console.error(`invalid app name "${name}": use lowercase kebab-case such as "open-finder"`);
        return 2;
    }
    const templateName = typeof options.template === 'string' ? options.template : 'open-tools';
    if (!isScaffoldTemplate(templateName)) {
        console.error(`unknown template "${templateName}": choose one of ${SCAFFOLD_TEMPLATES.join(', ')}`);
        return 2;
    }
    const outDir = typeof options.out === 'string' ? options.out : '.';
    const withGui = templateName === 'open-app' ? true : Boolean(options['with-gui']);
    const template = templateRoot(templateName);
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
    if (templateName === 'open-tools' && !withGui) {
        removePath(path.join(dest, 'gui'));
        removePath(path.join(dest, 'dist', 'gui'));
    }
    console.log(`created ${dest} (template=${templateName}, with_gui=${withGui})`);
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

/** Strict `X.Y.Z` package versions (no ranges, no prerelease). */
const PACKAGE_VERSION_RE = /^\d+\.\d+\.\d+$/;
/** Release tags accepted by the OIDC workflow: `vX.Y.Z`. */
const RELEASE_TAG_RE = /^v(\d+\.\d+\.\d+)$/;

/** One npm package directory selected for verification or publishing. */
export type PublishManifest = { dir: string; name: string; version: string };

/** What `wae publish` may do for the selected packages. */
export type PublishIntent =
    | { kind: 'verify' }
    | { kind: 'placeholder' }
    | { kind: 'refused'; message: string };

/**
 * Decide the publish mode. Real versions are published by the Nifty release
 * workflow; a local `--execute` is limited to the `0.0.0` placeholder stub
 * used to configure the npm trusted publisher.
 */
export function resolvePublishIntent(
    version: string,
    options: { execute: boolean; env: NodeJS.ProcessEnv },
): PublishIntent {
    if (options.execute) {
        if (version !== '0.0.0') {
            return {
                kind: 'refused',
                message: `local publish of ${version} is disabled; push tag v${version} and let the OIDC workflow publish. Local --execute only covers the 0.0.0 placeholder stub`,
            };
        }
        return { kind: 'placeholder' };
    }
    return { kind: 'verify' };
}

/** Read `version` from the `[workspace.package]` table of a Cargo workspace file. */
export function readWorkspaceVersion(cargoPath: string): string | undefined {
    let text: string;
    try {
        text = fs.readFileSync(cargoPath, 'utf8');
    } catch {
        return undefined;
    }
    let inWorkspacePackage = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.replace(/#.*$/, '').trim();
        if (line.startsWith('[')) {
            inWorkspacePackage = line === '[workspace.package]';
            continue;
        }
        if (!inWorkspacePackage) continue;
        const match = line.match(/^version\s*=\s*"([^"]+)"/);
        if (match) return match[1];
    }
    return undefined;
}

/** Read and validate one npm package manifest (real package, exact version). */
function readPublishManifest(dir: string): PublishManifest | string {
    const pack = path.resolve(dir);
    const manifestPath = path.join(pack, 'package.json');
    if (!fs.existsSync(manifestPath)) {
        return `--dir is missing package.json: ${pack}`;
    }
    let manifest: { name?: unknown; version?: unknown; private?: unknown };
    try {
        manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as typeof manifest;
    } catch {
        return `invalid package.json: ${manifestPath}`;
    }
    const name = typeof manifest.name === 'string' ? manifest.name : '';
    const version = typeof manifest.version === 'string' ? manifest.version : '';
    if (!name) {
        return `package.json is missing a name: ${manifestPath}`;
    }
    if (manifest.private === true) {
        return `${name} is private and cannot be published`;
    }
    if (!PACKAGE_VERSION_RE.test(version)) {
        return `${name} has no exact X.Y.Z version: ${version || '<empty>'}`;
    }
    return { dir: pack, name, version };
}

async function cmdPublish(options: ParsedOptions): Promise<number> {
    const raw = options.dir;
    const dirs = Array.isArray(raw) ? raw : typeof raw === 'string' ? [raw] : [];
    if (!dirs.length) {
        console.error('provide at least one --dir');
        return 2;
    }
    const execute = Boolean(options.execute);

    const manifests: PublishManifest[] = [];
    for (const dir of dirs) {
        const manifest = readPublishManifest(dir);
        if (typeof manifest === 'string') {
            console.error(manifest);
            return 1;
        }
        manifests.push(manifest);
    }
    const names = new Set<string>();
    for (const manifest of manifests) {
        if (names.has(manifest.name)) {
            console.error(`duplicate package in --dir list: ${manifest.name}`);
            return 1;
        }
        names.add(manifest.name);
    }
    const version = manifests[0]!.version;
    for (const manifest of manifests) {
        if (manifest.version !== version) {
            console.error(`publish set versions differ: ${manifest.name}@${manifest.version} vs ${version}`);
            return 1;
        }
    }

    const tagRef = typeof options.tag === 'string' ? options.tag : String(process.env.GITHUB_REF_NAME ?? '');
    if (tagRef) {
        const match = RELEASE_TAG_RE.exec(tagRef);
        if (!match) {
            console.error(`release tag must look like v<major>.<minor>.<patch>, got ${tagRef}`);
            return 1;
        }
        if (match[1] !== version) {
            console.error(`tag ${tagRef} does not match package version ${version}`);
            return 1;
        }
    }

    const cargoRaw = typeof options.cargo === 'string' && options.cargo ? options.cargo : 'Cargo.toml';
    const cargoPath = path.resolve(cargoRaw);
    const cargoVersion = readWorkspaceVersion(cargoPath);
    if (cargoVersion === undefined) {
        console.error(`cannot read [workspace.package] version from ${cargoPath}`);
        return 1;
    } else if (cargoVersion !== version) {
        console.error(`Cargo [workspace.package] version ${cargoVersion} does not match npm package version ${version}`);
        return 1;
    }

    const intent = resolvePublishIntent(version, { execute, env: process.env });
    if (intent.kind === 'refused') {
        console.error(intent.message);
        return 2;
    }

    if (intent.kind === 'verify') {
        const tagText = tagRef ? ` for ${tagRef}` : '';
        console.log(`verified ${manifests.length} package(s) at ${version}${tagText}; push tag v${version} to publish via Trusted Publisher (OIDC)`);
        for (const manifest of manifests) {
            console.log(`  ${manifest.name}@${manifest.version} → ${manifest.dir}`);
        }
        return 0;
    }

    console.log(`publishing ${version} as the 0.0.0 placeholder stub (Trusted Publisher setup only)`);
    const args = ['publish', '--access', 'public'];
    for (const manifest of manifests) {
        const result = spawnSync('npm', args, {
            cwd: manifest.dir,
            stdio: 'inherit',
            shell: process.platform === 'win32',
        });
        if (result.status !== 0) {
            console.error(`npm publish failed: ${manifest.name} (${manifest.dir})`);
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
