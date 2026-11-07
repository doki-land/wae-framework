#!/usr/bin/env node
/**
 * `@wae/wae` meta CLI: scaffold / build / stage native addons / npm publish.
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { createCli, type ParsedOptions } from '@wae/commander';
import { BUILD_CODES, WaeBuildError } from './build-error.js';
import {
    loadConfig,
    WaeConfigError,
    WAE_CONFIG_FILE,
    type WaeNativeConfig,
    type WaeNativeTarget,
} from './config.js';
import { defineConfig } from './config.js';
import { runBuild, readWorkspaceVersion } from './build.js';
import { entryRelation, unresolvedEntryMessage } from './entry.js';

export { readWorkspaceVersion };
import { copyDir, removePath, rewritePlaceholders, SCAFFOLD_NAME_PATTERN } from './fs-utils.js';
import {
    cargoBuildNative,
    findNativeArtifact,
    nativeBuildRequest,
    selectNativeTarget,
    stageBuiltArtifact,
    stageNative,
} from './native-stage.js';
import {
    inspectNative,
    loadNative,
    NativeLoadError,
    NativeUnsupportedError,
    NATIVE_ARCH_NAMES,
    NATIVE_PLATFORM_NAMES,
    capabilityMismatch,
    type NativeAuditFilter,
} from './native.js';
import { localesRoot, packageVersion, SCAFFOLD_TEMPLATES, type ScaffoldTemplate, templateRoot } from './paths.js';

export { defineConfig };
export type {
    WaeActorSource,
    WaeAuditConfig,
    WaeBuildConfig,
    WaeBuildStep,
    WaeBuildStepName,
    WaeCiConfig,
    WaeConfig,
    WaeInstallConfig,
    WaeNativeConfig,
    WaeNativeTarget,
    WaePlatformPackage,
    WaeReleaseConfig,
    WaeReleasePackage,
    WaeSkillsConfig,
    WaeSkillsPackage,
} from './config.js';

/** Build the product CLI (tests may call this directly). */
export function buildCli() {
    const cli = createCli('wae').locales(localesRoot()).intro('cli.intro');

    cli.command('new', 'cli.cmd.new')
        .option('--template <name>', 'cli.opt.template', { default: 'open-tools' })
        .option('--with-gui', 'cli.opt.with-gui')
        .option('--out <dir>', 'cli.opt.out', { default: '.' })
        .actionCtx(async (options, args) => cmdNew(options, args));

    cli.command('build', 'cli.cmd.build')
        .option('--project <dir>', 'cli.opt.project')
        .option('--package <name>', 'cli.opt.package')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
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

    const native = cli.command('native', 'cli.cmd.native');
    native.command('info', 'cli.cmd.native-info')
        .option('--json', 'cli.opt.json')
        .actionCtx(async (options) => cmdNativeInfo(options));
    native.command('build', 'cli.cmd.native-build')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
        .actionCtx(async (options) => cmdNativeBuild(options));
    native.command('stage', 'cli.cmd.native-stage')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
        .actionCtx(async (options) => cmdNativeStage(options));
    native.command('smoke', 'cli.cmd.native-smoke')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--json', 'cli.opt.json')
        .actionCtx(async (options) => cmdNativeSmoke(options));
    native.command('audit', 'cli.cmd.native-audit')
        .option('--kind <kind>', 'cli.opt.kind', { enum: ['user', 'agent'] })
        .option('--id <id>', 'cli.opt.actor-id')
        .option('--capability <name>', 'cli.opt.capability')
        .option('--from <ms>', 'cli.opt.from', { coerce: (raw) => nonNegativeInt(raw, '--from') })
        .option('--to <ms>', 'cli.opt.to', { coerce: (raw) => nonNegativeInt(raw, '--to') })
        .option('--limit <n>', 'cli.opt.limit', { coerce: (raw) => nonNegativeInt(raw, '--limit') })
        .option('--json', 'cli.opt.json')
        .actionCtx(async (options) => cmdNativeAudit(options));

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
    rewritePlaceholders(dest, name, packageVersion());
    formatScaffold(dest);
    if (templateName === 'open-tools' && !withGui) {
        removePath(path.join(dest, 'gui'));
        removePath(path.join(dest, 'dist', 'gui'));
    }
    console.log(`created ${dest} (template=${templateName}, with_gui=${withGui})`);
    return 0;
}

/**
 * Format generated Rust sources with the template's `rustfmt.toml`.
 *
 * A line that contains `{{name}}` can cross the width limit depending on the
 * chosen name, so the template cannot be pre-formatted for every name. Leaving
 * the scaffold unformatted makes `check:rust` fail on the very first run.
 */
function formatScaffold(dest: string): void {
    if (!fs.existsSync(path.join(dest, 'crates'))) return;
    const result = spawnSync('cargo', ['fmt', '--all'], { cwd: dest, stdio: 'inherit' });
    if (result.status !== 0) {
        console.error(`[wae] skipped rustfmt: ${result.error?.message ?? `cargo fmt exited ${String(result.status)}`}`);
    }
}

async function cmdBuild(options: ParsedOptions): Promise<number> {
    const startDir = typeof options.project === 'string' ? path.resolve(options.project) : process.cwd();
    try {
        await runBuild({
            startDir,
            ...(typeof options.package === 'string' ? { packageOverride: options.package } : {}),
            ...(typeof options.target === 'string' ? { targetOverride: options.target } : {}),
            dryRun: options['dry-run'] === true,
        });
        return 0;
    } catch (error) {
        return buildFailure('build', error);
    }
}

/** Render a structured build/config failure as JSON and return exit code 1. */
function buildFailure(scope: string, error: unknown): number {
    if (error instanceof WaeBuildError || error instanceof WaeConfigError) {
        console.error(JSON.stringify(error.toJSON(), null, 2));
        return 1;
    }
    console.error(`wae ${scope} failed: ${error instanceof Error ? error.message : String(error)}`);
    return 1;
}

type NativePlan = { root: string; native: WaeNativeConfig; target: WaeNativeTarget };

/** Load the config and select one native target (host by default). */
function nativePlan(options: ParsedOptions): NativePlan {
    const startDir = typeof options.project === 'string' ? path.resolve(options.project) : process.cwd();
    const loaded = loadConfig(startDir);
    if (loaded.config?.native === undefined) {
        throw new WaeBuildError(
            BUILD_CODES.targetMissing,
            `\`native\` is not configured in ${WAE_CONFIG_FILE}`,
            { attempts: [loaded.path ?? path.join(loaded.root, WAE_CONFIG_FILE)] },
        );
    }
    return {
        root: loaded.root,
        native: loaded.config.native,
        target: selectNativeTarget(loaded.config.native, typeof options.target === 'string' ? options.target : undefined),
    };
}

async function cmdNativeBuild(options: ParsedOptions): Promise<number> {
    try {
        const plan = nativePlan(options);
        const key = `${plan.target.platform}-${plan.target.arch}`;
        if (options['dry-run'] === true) {
            console.log(`[wae] native build would ${plan.native.stageOnly ? 'stage' : 'build'} \`${key}\` → ${path.join(plan.root, plan.target.dir, plan.target.nodeFile)}`);
            return 0;
        }
        const request = nativeBuildRequest(plan.root, plan.native, plan.target);
        if (plan.native.stageOnly !== true) {
            cargoBuildNative(request);
        }
        const artifact = findNativeArtifact(request);
        const dest = stageBuiltArtifact(artifact, path.resolve(plan.root, plan.target.dir), plan.target.nodeFile);
        console.log(`[wae] native addon staged at ${dest}`);
        return 0;
    } catch (error) {
        return buildFailure('native build', error);
    }
}

async function cmdNativeStage(options: ParsedOptions): Promise<number> {
    try {
        const plan = nativePlan(options);
        const dest = path.join(plan.root, plan.target.dir, plan.target.nodeFile);
        if (options['dry-run'] === true) {
            console.log(`[wae] native stage would copy the built cdylib for \`${plan.native.crate}\` → ${dest}`);
            return 0;
        }
        const artifact = findNativeArtifact(nativeBuildRequest(plan.root, plan.native, plan.target));
        const staged = stageBuiltArtifact(artifact, path.resolve(plan.root, plan.target.dir), plan.target.nodeFile);
        console.log(`[wae] native addon staged at ${staged}`);
        return 0;
    } catch (error) {
        return buildFailure('native stage', error);
    }
}

/**
 * The addon contract shared by every project: `version()` and `capabilities()`
 * are required. `platform`, `arch`, `audit`, `probe()` and `auditQuery()` are
 * part of the wae native-addon shape; when an app omits one, the smoke reports
 * it as not checked instead of pretending it passed.
 */
type SmokeAddon = {
    version(): string;
    capabilities(): Record<string, unknown>;
    probe?: () => { ok?: boolean };
    auditQuery?: (filter?: { capability?: string; limit?: number }) => string;
};

async function cmdNativeSmoke(options: ParsedOptions): Promise<number> {
    // The audit directory is borrowed for this run only: a temp one is created on demand and
    // removed afterwards, and an inherited value is left exactly as the caller set it.
    const previousAuditDir = process.env.WAE_AUDIT_DIR;
    let tempAuditDir: string | undefined;
    try {
        const plan = nativePlan(options);
        const key = `${plan.target.platform}-${plan.target.arch}`;
        const dir = path.resolve(plan.root, plan.target.dir);
        const addonPath = path.join(dir, plan.target.nodeFile);
        if (!fs.existsSync(addonPath)) {
            throw new WaeBuildError(
                BUILD_CODES.artifactMissing,
                `native smoke cannot find the staged addon at ${addonPath}; run \`wae native build\` first`,
                { platform: key, attempts: [addonPath] },
            );
        }
        const manifestPath = path.join(dir, 'package.json');
        const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as {
            name: string;
            version: string;
            os: string[];
            cpu: string[];
        };
        if (!manifest.os.includes(plan.target.platform) || !manifest.cpu.includes(plan.target.arch)) {
            throw new WaeBuildError(
                BUILD_CODES.platformPackageInvalid,
                `platform package declares os/cpu ${manifest.os.join(',')}/${manifest.cpu.join(',')} but the target is ${key}`,
                { platform: key, attempts: [manifestPath] },
            );
        }

        if (previousAuditDir === undefined) {
            tempAuditDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-native-audit-'));
            process.env.WAE_AUDIT_DIR = tempAuditDir;
        }
        const addon = createRequire(import.meta.url)(addonPath) as SmokeAddon;
        const version = addon.version();
        if (version !== manifest.version) {
            throw new WaeBuildError(
                BUILD_CODES.platformPackageInvalid,
                `native addon version ${version || '<empty>'} does not match ${manifest.name} ${manifest.version}`,
                { platform: key },
            );
        }
        const capabilities = addon.capabilities();
        if (typeof capabilities !== 'object' || capabilities === null) {
            throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, 'capabilities() did not return an object', {
                platform: key,
            });
        }
        const expectedPlatform = NATIVE_PLATFORM_NAMES[plan.target.platform];
        const expectedArch = NATIVE_ARCH_NAMES[plan.target.arch];
        const mismatch = capabilityMismatch(capabilities, { platform: expectedPlatform, arch: expectedArch, version });
        if (mismatch !== undefined) {
            throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, mismatch, { platform: key });
        }
        let probeState = 'not exported';
        if (typeof addon.probe === 'function') {
            const probe = addon.probe();
            if (probe.ok !== true) {
                throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, 'native probe did not report ok', { platform: key });
            }
            probeState = 'ok';
        }
        let auditEntries = 0;
        let auditState = 'not exported';
        if (typeof addon.auditQuery === 'function') {
            const entries = JSON.parse(addon.auditQuery({ limit: 5 })) as unknown;
            if (!Array.isArray(entries) || entries.length === 0) {
                throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, 'auditQuery did not return any audit entry', {
                    platform: key,
                });
            }
            auditEntries = entries.length;
            auditState = `${auditEntries} entries`;
        }
        const summary = {
            addon: addonPath,
            version,
            platform: capabilities.platform ?? expectedPlatform,
            arch: capabilities.arch ?? expectedArch,
            probe: probeState,
            audit: auditState,
        };
        if (options.json === true) {
            console.log(JSON.stringify(summary, null, 2));
        } else {
            console.log(
                `[wae] native smoke ok: ${addonPath} version=${version} platform=${String(summary.platform)} arch=${String(summary.arch)} probe=${probeState} audit=${auditState}`,
            );
        }
        return 0;
    } catch (error) {
        if (error instanceof NativeLoadError || error instanceof NativeUnsupportedError) {
            return nativeFailure(error);
        }
        return buildFailure('native smoke', error);
    } finally {
        if (tempAuditDir !== undefined) {
            fs.rmSync(tempAuditDir, { recursive: true, force: true });
        }
        if (previousAuditDir === undefined) {
            delete process.env.WAE_AUDIT_DIR;
        } else {
            process.env.WAE_AUDIT_DIR = previousAuditDir;
        }
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

/** Strict non-negative integer option value (parse failure exits 2). */
function nonNegativeInt(raw: string | boolean | string[], flag: string): number {
    const text = typeof raw === 'string' ? raw.trim() : '';
    if (!/^\d+$/.test(text)) {
        throw new Error(`${flag} expects a non-negative integer, got ${typeof raw === 'string' ? raw : String(raw)}`);
    }
    return Number(text);
}

/** Render a structured native failure (code, capability, platform, attempts) as JSON. */
function nativeFailure(error: unknown): number {
    if (error instanceof NativeUnsupportedError || error instanceof NativeLoadError) {
        console.error(JSON.stringify(error.toJSON(), null, 2));
        return 1;
    }
    console.error(`wae native failed: ${error instanceof Error ? error.message : error}`);
    return 1;
}

async function cmdNativeInfo(options: ParsedOptions): Promise<number> {
    try {
        const report = inspectNative();
        if (options.json === true) {
            console.log(JSON.stringify(report, null, 2));
            return 0;
        }
        console.log(`platform: ${report.platformKey}`);
        console.log(`path: ${report.path}`);
        console.log(`version: ${report.version}`);
        console.log(`probe: ${report.probe.ok ? 'ok' : 'failed'} (${report.probe.platform}/${report.probe.arch})`);
        console.log(`audit: ${String(report.capabilities.audit)} (probe entries: ${report.auditEntries})`);
        return 0;
    } catch (error) {
        return nativeFailure(error);
    }
}

async function cmdNativeAudit(options: ParsedOptions): Promise<number> {
    const filter: NativeAuditFilter = {};
    if (typeof options.kind === 'string') {
        filter.kind = options.kind === 'agent' ? 'agent' : 'user';
    }
    if (typeof options.id === 'string' && options.id) {
        filter.id = options.id;
    }
    if (typeof options.capability === 'string' && options.capability) {
        filter.capability = options.capability;
    }
    if (typeof options.from === 'number') {
        filter.fromMs = options.from;
    }
    if (typeof options.to === 'number') {
        filter.toMs = options.to;
    }
    if (typeof options.limit === 'number') {
        filter.limit = options.limit;
    }
    try {
        const addon = loadNative();
        const text = addon.auditQuery(Object.keys(filter).length ? filter : undefined);
        if (options.json === true) {
            console.log(text);
            return 0;
        }
        try {
            console.log(JSON.stringify(JSON.parse(text), null, 2));
        } catch {
            console.log(text);
        }
        return 0;
    } catch (error) {
        return nativeFailure(error);
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
        // `npm` is a `.cmd` shim on Windows, so it needs a shell there. Pass a
        // single command string rather than an argument array.
        const result = process.platform === 'win32'
            ? spawnSync(`npm ${args.join(' ')}`, { cwd: manifest.dir, stdio: 'inherit', shell: true })
            : spawnSync('npm', args, { cwd: manifest.dir, stdio: 'inherit' });
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

const invoked = process.argv[1] ? path.resolve(process.argv[1]) : '';
const relation = entryRelation(import.meta.url, invoked);
if (relation === 'main') {
    void main();
}
else if (relation === 'unresolved') {
    console.error(`[wae] ${unresolvedEntryMessage(import.meta.url, invoked)}`);
    process.exitCode = 1;
}
