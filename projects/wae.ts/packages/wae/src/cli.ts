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
import { createCli, CliError, EXIT, type CommandContext, type CreateCliOptions, type ParsedOptions } from '@wae/commander';
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
export { WAE_WINDOW_CODES, WAE_WINDOW_HOST_SUPPORT, WAE_WINDOW_MODES, resolveWindowPolicy, windowModeSupport } from './window.js';
export type { WaeChildWindows, WaeSecondLaunch, WaeWindowConfig, WaeWindowHostSupport, WaeWindowMode, WaeWindowPolicy } from './window.js';

/**
 * Build the product CLI (tests may call this directly).
 *
 * `runtime` lets an in-process host inject stdin/stdout/stderr, TTY state and
 * env; the engine's default (real `process` streams) is used when omitted.
 */
export function buildCli(runtime: CreateCliOptions = {}) {
    const cli = createCli('wae', runtime).locales(localesRoot()).intro('cli.intro');

    cli.command('new', 'cli.cmd.new')
        .option('--template <name>', 'cli.opt.template')
        .option('--with-gui', 'cli.opt.with-gui')
        .option('--out <dir>', 'cli.opt.out', { default: '.' })
        .interactive({
            policy: 'optional',
            promptKeys: { name: 'cli.prompt.new-name', template: 'cli.prompt.new-template' },
            // The engine validates bare refs against option keys; the positional
            // `name` is declared through the `config:` namespace.
            nonInteractiveInputs: ['config:name', 'template', 'out', 'with-gui'],
        })
        .actionCtx(async (options, args, ctx) => cmdNew(options, args, ctx));

    cli.command('build', 'cli.cmd.build')
        .option('--project <dir>', 'cli.opt.project')
        .option('--package <name>', 'cli.opt.package')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
        .actionCtx(async (options, _args, ctx) => cmdBuild(options, ctx));

    cli.command('dist', 'cli.cmd.dist')
        .option('--project <dir>', 'cli.opt.project', { required: true })
        .option('--package <name>', 'cli.opt.package', { required: true })
        .option('--out <dir>', 'cli.opt.dist-out', { required: true })
        .option('--file <name>', 'cli.opt.file', { required: true })
        .actionCtx(async (options, _args, ctx) => cmdDist(options, ctx));

    cli.command('publish', 'cli.cmd.publish')
        .option('--dir <path>...', 'cli.opt.dir', { required: true })
        .option('--tag <ref>', 'cli.opt.tag')
        .option('--cargo <path>', 'cli.opt.cargo')
        .option('--execute', 'cli.opt.execute')
        .actionCtx(async (options, _args, ctx) => cmdPublish(options, ctx));

    const native = cli.command('native', 'cli.cmd.native');
    native.command('info', 'cli.cmd.native-info')
        .option('--json', 'cli.opt.json')
        .actionCtx(async (_options, _args, ctx) => cmdNativeInfo(ctx));
    native.command('build', 'cli.cmd.native-build')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
        .actionCtx(async (options, _args, ctx) => cmdNativeBuild(options, ctx));
    native.command('stage', 'cli.cmd.native-stage')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--dry-run', 'cli.opt.dry-run')
        .actionCtx(async (options, _args, ctx) => cmdNativeStage(options, ctx));
    native.command('smoke', 'cli.cmd.native-smoke')
        .option('--project <dir>', 'cli.opt.project')
        .option('--target <key>', 'cli.opt.target')
        .option('--json', 'cli.opt.json')
        .actionCtx(async (options, _args, ctx) => cmdNativeSmoke(options, ctx));
    native.command('audit', 'cli.cmd.native-audit')
        .option('--kind <kind>', 'cli.opt.kind', { enum: ['user', 'agent'] })
        .option('--id <id>', 'cli.opt.actor-id')
        .option('--capability <name>', 'cli.opt.capability')
        .option('--from <ms>', 'cli.opt.from', { coerce: (raw) => nonNegativeInt(raw, '--from') })
        .option('--to <ms>', 'cli.opt.to', { coerce: (raw) => nonNegativeInt(raw, '--to') })
        .option('--limit <n>', 'cli.opt.limit', { coerce: (raw) => nonNegativeInt(raw, '--limit') })
        .option('--json', 'cli.opt.json')
        .actionCtx(async (options, _args, ctx) => cmdNativeAudit(options, ctx));

    return cli;
}

function isScaffoldTemplate(value: string): value is ScaffoldTemplate {
    return (SCAFFOLD_TEMPLATES as readonly string[]).includes(value);
}

async function cmdNew(options: ParsedOptions, args: string[], ctx: CommandContext): Promise<number> {
    let name = args[0]?.trim();
    if (!name && ctx.isInteractive()) {
        name = (
            await ctx.prompt.input({
                key: 'cli.prompt.new-name',
                validate: (value) =>
                    SCAFFOLD_NAME_PATTERN.test(value.trim()) ? null : { code: 'cli.err.invalid-name', args: { name: value.trim() } },
            })
        ).trim();
    }
    if (!name) {
        ctx.emitFailure({ code: 'name_required', message: ctx.t('cli.err.new-name-required') });
        return EXIT.USAGE;
    }
    if (!SCAFFOLD_NAME_PATTERN.test(name)) {
        ctx.emitFailure({ code: 'invalid_name', message: ctx.t('cli.err.invalid-name', { name }) });
        return EXIT.USAGE;
    }
    let templateName = typeof options.template === 'string' ? options.template : undefined;
    if (templateName === undefined && ctx.isInteractive()) {
        templateName = await ctx.prompt.select({
            key: 'cli.prompt.new-template',
            choices: SCAFFOLD_TEMPLATES.map((id) => ({ value: id, labelKey: `cli.choice.template.${id}` })),
        });
    }
    templateName ??= 'open-tools';
    if (!isScaffoldTemplate(templateName)) {
        ctx.emitFailure({
            code: 'unknown_template',
            message: ctx.t('cli.err.unknown-template', { template: templateName, choices: SCAFFOLD_TEMPLATES.join(', ') }),
        });
        return EXIT.USAGE;
    }
    const outDir = typeof options.out === 'string' ? options.out : '.';
    const withGui = templateName === 'open-app' ? true : Boolean(options['with-gui']);
    const template = templateRoot(templateName);
    if (!fs.existsSync(template)) {
        ctx.emitFailure({ code: 'template_missing', message: ctx.t('cli.err.template-missing', { template }) });
        return EXIT.FAILURE;
    }
    const dest = path.resolve(outDir, name);
    if (fs.existsSync(dest)) {
        ctx.emitFailure({ code: 'destination_exists', message: ctx.t('cli.err.destination-exists', { dest }) });
        return EXIT.FAILURE;
    }
    try {
        copyDir(template, dest);
        rewritePlaceholders(dest, name, packageVersion());
        formatScaffold(dest, ctx);
        if (templateName === 'open-tools' && !withGui) {
            removePath(path.join(dest, 'gui'));
            removePath(path.join(dest, 'dist', 'gui'));
        }
    } catch (error) {
        return emitDomainFailure(ctx, error);
    }
    ctx.emitResult(
        { created: dest, template: templateName, withGui },
        ctx.t('cli.result.new-created', { dest, template: templateName, withGui: String(withGui) }),
    );
    return EXIT.OK;
}

/**
 * Format generated Rust sources with the template's `rustfmt.toml`.
 *
 * A line that contains `{{name}}` can cross the width limit depending on the
 * chosen name, so the template cannot be pre-formatted for every name. Leaving
 * the scaffold unformatted makes `check:rust` fail on the very first run.
 */
function formatScaffold(dest: string, ctx: CommandContext): void {
    if (!fs.existsSync(path.join(dest, 'crates'))) return;
    const result = spawnSync('cargo', ['fmt', '--all'], { cwd: dest, stdio: 'inherit' });
    if (result.status !== 0) {
        const detail = result.error?.message ?? `cargo fmt exited ${String(result.status)}`;
        ctx.stderr(ctx.t('cli.err.rustfmt-skipped', { detail }));
    }
}

async function cmdBuild(options: ParsedOptions, ctx: CommandContext): Promise<number> {
    const startDir = typeof options.project === 'string' ? path.resolve(options.project) : process.cwd();
    try {
        const report = await runBuild({
            startDir,
            ...(typeof options.package === 'string' ? { packageOverride: options.package } : {}),
            ...(typeof options.target === 'string' ? { targetOverride: options.target } : {}),
            dryRun: options['dry-run'] === true,
            // Step progress is diagnostic output; stdout stays reserved for the envelope.
            log: (line) => ctx.stderr(line),
        });
        ctx.emitResult(report, ctx.t('cli.result.build-done', { root: report.root }));
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
    }
}

/**
 * Map a caught domain error onto the failure envelope and its exit code.
 * Structured errors keep their stable code; the failure envelope only carries
 * `{code, message}`, so the `toJSON()` details are serialized into the message.
 * An unsupported platform exits CAPABILITY, everything else FAILURE.
 */
function emitDomainFailure(ctx: CommandContext, error: unknown): number {
    if (error instanceof CliError) throw error;
    if (error instanceof NativeUnsupportedError) {
        ctx.emitFailure({ code: error.code, message: JSON.stringify(error.toJSON()) });
        return EXIT.CAPABILITY;
    }
    if (error instanceof WaeBuildError || error instanceof WaeConfigError || error instanceof NativeLoadError) {
        ctx.emitFailure({ code: error.code, message: JSON.stringify(error.toJSON()) });
        return EXIT.FAILURE;
    }
    ctx.emitFailure({ code: 'action_failed', message: error instanceof Error ? error.message : String(error) });
    return EXIT.FAILURE;
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

async function cmdNativeBuild(options: ParsedOptions, ctx: CommandContext): Promise<number> {
    try {
        const plan = nativePlan(options);
        const key = `${plan.target.platform}-${plan.target.arch}`;
        const dest = path.join(plan.root, plan.target.dir, plan.target.nodeFile);
        if (options['dry-run'] === true) {
            const action = plan.native.stageOnly ? 'stage' : 'build';
            ctx.emitResult(
                { dryRun: true, action, target: key, dest },
                ctx.t('cli.result.native-build-plan', { action, target: key, dest }),
            );
            return EXIT.OK;
        }
        const request = nativeBuildRequest(plan.root, plan.native, plan.target);
        if (plan.native.stageOnly !== true) {
            cargoBuildNative(request);
        }
        const artifact = findNativeArtifact(request);
        const staged = stageBuiltArtifact(artifact, path.resolve(plan.root, plan.target.dir), plan.target.nodeFile);
        ctx.emitResult({ staged }, ctx.t('cli.result.native-staged', { dest: staged }));
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
    }
}

async function cmdNativeStage(options: ParsedOptions, ctx: CommandContext): Promise<number> {
    try {
        const plan = nativePlan(options);
        const dest = path.join(plan.root, plan.target.dir, plan.target.nodeFile);
        if (options['dry-run'] === true) {
            ctx.emitResult(
                { dryRun: true, action: 'stage', crate: plan.native.crate, dest },
                ctx.t('cli.result.native-stage-plan', { crate: plan.native.crate, dest }),
            );
            return EXIT.OK;
        }
        const artifact = findNativeArtifact(nativeBuildRequest(plan.root, plan.native, plan.target));
        const staged = stageBuiltArtifact(artifact, path.resolve(plan.root, plan.target.dir), plan.target.nodeFile);
        ctx.emitResult({ staged }, ctx.t('cli.result.native-staged', { dest: staged }));
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
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

async function cmdNativeSmoke(options: ParsedOptions, ctx: CommandContext): Promise<number> {
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
        ctx.emitResult(
            summary,
            ctx.t('cli.result.native-smoke-ok', {
                addon: addonPath,
                version,
                platform: String(summary.platform),
                arch: String(summary.arch),
                probe: probeState,
                audit: auditState,
            }),
        );
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
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

async function cmdDist(options: ParsedOptions, ctx: CommandContext): Promise<number> {
    try {
        const dest = stageNative({
            projectRoot: path.resolve(String(options.project)),
            cargoPackage: String(options.package),
            destDir: path.resolve(String(options.out)),
            destFileName: String(options.file),
        });
        ctx.emitResult({ staged: dest }, ctx.t('cli.result.dist-staged', { dest }));
        return EXIT.OK;
    } catch (err) {
        return emitDomainFailure(ctx, err);
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

async function cmdNativeInfo(ctx: CommandContext): Promise<number> {
    try {
        const report = inspectNative();
        const human = [
            `platform: ${report.platformKey}`,
            `path: ${report.path}`,
            `version: ${report.version}`,
            `probe: ${report.probe.ok ? 'ok' : 'failed'} (${report.probe.platform}/${report.probe.arch})`,
            `audit: ${String(report.capabilities.audit)} (probe entries: ${report.auditEntries})`,
        ].join('\n');
        ctx.emitResult(report, human);
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
    }
}

async function cmdNativeAudit(options: ParsedOptions, ctx: CommandContext): Promise<number> {
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
        let data: unknown;
        try {
            data = JSON.parse(text);
        } catch {
            data = text;
        }
        ctx.emitResult(data, typeof data === 'string' ? data : JSON.stringify(data, null, 2));
        return EXIT.OK;
    } catch (error) {
        return emitDomainFailure(ctx, error);
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

/** Structured manifest/validation failure: stable code + locale key + safe args. */
type PublishFailure = { code: string; key: string; args: Record<string, string> };

/** Read and validate one npm package manifest (real package, exact version). */
function readPublishManifest(dir: string): PublishManifest | PublishFailure {
    const pack = path.resolve(dir);
    const manifestPath = path.join(pack, 'package.json');
    if (!fs.existsSync(manifestPath)) {
        return { code: 'missing_package_json', key: 'cli.err.missing-package-json', args: { dir: pack } };
    }
    let manifest: { name?: unknown; version?: unknown; private?: unknown };
    try {
        manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as typeof manifest;
    } catch {
        return { code: 'invalid_package_json', key: 'cli.err.invalid-package-json', args: { file: manifestPath } };
    }
    const name = typeof manifest.name === 'string' ? manifest.name : '';
    const version = typeof manifest.version === 'string' ? manifest.version : '';
    if (!name) {
        return { code: 'missing_package_name', key: 'cli.err.missing-package-name', args: { file: manifestPath } };
    }
    if (manifest.private === true) {
        return { code: 'private_package', key: 'cli.err.private-package', args: { name } };
    }
    if (!PACKAGE_VERSION_RE.test(version)) {
        return { code: 'inexact_version', key: 'cli.err.inexact-version', args: { name, version: version || '<empty>' } };
    }
    return { dir: pack, name, version };
}

async function cmdPublish(options: ParsedOptions, ctx: CommandContext): Promise<number> {
    const raw = options.dir;
    const dirs = Array.isArray(raw) ? raw : typeof raw === 'string' ? [raw] : [];
    if (!dirs.length) {
        ctx.emitFailure({ code: 'dirs_required', message: ctx.t('cli.err.dirs-required') });
        return EXIT.USAGE;
    }
    const execute = Boolean(options.execute);

    const manifests: PublishManifest[] = [];
    for (const dir of dirs) {
        const manifest = readPublishManifest(dir);
        if ('key' in manifest) {
            ctx.emitFailure({ code: manifest.code, message: ctx.t(manifest.key, manifest.args) });
            return EXIT.FAILURE;
        }
        manifests.push(manifest);
    }
    const names = new Set<string>();
    for (const manifest of manifests) {
        if (names.has(manifest.name)) {
            ctx.emitFailure({ code: 'duplicate_package', message: ctx.t('cli.err.duplicate-package', { name: manifest.name }) });
            return EXIT.FAILURE;
        }
        names.add(manifest.name);
    }
    const version = manifests[0]!.version;
    for (const manifest of manifests) {
        if (manifest.version !== version) {
            ctx.emitFailure({
                code: 'versions_differ',
                message: ctx.t('cli.err.versions-differ', { name: manifest.name, version: manifest.version, expected: version }),
            });
            return EXIT.FAILURE;
        }
    }

    const tagRef = typeof options.tag === 'string' ? options.tag : String(process.env.GITHUB_REF_NAME ?? '');
    if (tagRef) {
        const match = RELEASE_TAG_RE.exec(tagRef);
        if (!match) {
            ctx.emitFailure({ code: 'bad_tag_format', message: ctx.t('cli.err.bad-tag-format', { tag: tagRef }) });
            return EXIT.FAILURE;
        }
        if (match[1] !== version) {
            ctx.emitFailure({ code: 'tag_mismatch', message: ctx.t('cli.err.tag-mismatch', { tag: tagRef, version }) });
            return EXIT.FAILURE;
        }
    }

    const cargoRaw = typeof options.cargo === 'string' && options.cargo ? options.cargo : 'Cargo.toml';
    const cargoPath = path.resolve(cargoRaw);
    const cargoVersion = readWorkspaceVersion(cargoPath);
    if (cargoVersion === undefined) {
        ctx.emitFailure({ code: 'cargo_version_missing', message: ctx.t('cli.err.cargo-version-missing', { file: cargoPath }) });
        return EXIT.FAILURE;
    } else if (cargoVersion !== version) {
        ctx.emitFailure({
            code: 'cargo_version_mismatch',
            message: ctx.t('cli.err.cargo-version-mismatch', { cargo: cargoVersion, version }),
        });
        return EXIT.FAILURE;
    }

    const intent = resolvePublishIntent(version, { execute, env: process.env });
    if (intent.kind === 'refused') {
        ctx.emitFailure({ code: 'publish_refused', message: ctx.t('cli.err.publish-refused', { version }) });
        return EXIT.USAGE;
    }

    if (intent.kind === 'verify') {
        const tagText = tagRef ? ctx.t('cli.result.publish-verified-tag', { tag: tagRef }) : '';
        const lines = [ctx.t('cli.result.publish-verified', { count: String(manifests.length), version, tagSuffix: tagText })];
        for (const manifest of manifests) {
            lines.push(`  ${manifest.name}@${manifest.version} → ${manifest.dir}`);
        }
        ctx.emitResult({ version, tag: tagRef || null, packages: manifests }, lines.join('\n'));
        return EXIT.OK;
    }

    ctx.emitResult({ version, placeholder: true }, ctx.t('cli.result.publishing-placeholder', { version }));
    const args = ['publish', '--access', 'public'];
    for (const manifest of manifests) {
        // `npm` is a `.cmd` shim on Windows, so it needs a shell there. Pass a
        // single command string rather than an argument array.
        const result = process.platform === 'win32'
            ? spawnSync(`npm ${args.join(' ')}`, { cwd: manifest.dir, stdio: 'inherit', shell: true })
            : spawnSync('npm', args, { cwd: manifest.dir, stdio: 'inherit' });
        if (result.status !== 0) {
            ctx.emitFailure({ code: 'npm_publish_failed', message: ctx.t('cli.err.npm-publish-failed', { name: manifest.name, dir: manifest.dir }) });
            return EXIT.FAILURE;
        }
    }
    return EXIT.OK;
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
