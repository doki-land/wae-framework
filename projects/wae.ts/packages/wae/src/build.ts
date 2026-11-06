/**
 * `wae build` orchestration: native → ts → ui → skills → staging.
 *
 * Every configured step must produce the artifact it declared. A missing target,
 * artifact, platform package, or release-set mismatch fails with a stable code
 * instead of being skipped.
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { BUILD_CODES, WaeBuildError } from './build-error.js';
import {
    loadConfig,
    parseStaticObject,
    WAE_CONFIG_FILE,
    type WaeBuildStepName,
    type WaeConfig,
    type WaeNativeConfig,
    type WaeNativeTarget,
    type WaePlatformPackage,
    type WaeSkillsPackage,
} from './config.js';
import { cargoBuildNative, findNativeArtifact, stageBuiltArtifact } from './native-stage.js';
import { auditSkills } from './skills-audit.js';

/** Exact `X.Y.Z` versions, no ranges or prerelease. */
export const EXACT_VERSION_RE = /^\d+\.\d+\.\d+$/;

export type BuildStepStatus = 'ok' | 'skipped' | 'planned';

export type BuildStepReport = {
    name: WaeBuildStepName | 'default';
    status: BuildStepStatus;
    detail: string;
};

export type BuildReport = {
    root: string;
    source: 'file' | 'default';
    configPath?: string;
    dryRun: boolean;
    steps: BuildStepReport[];
};

export type BuildOptions = {
    /** Directory where config discovery starts. */
    startDir: string;
    /** Overrides `native.crate` only; never replaces the config file. */
    packageOverride?: string;
    /** Overrides the selected native target only. */
    targetOverride?: string;
    dryRun?: boolean;
    log?: (line: string) => void;
};

type BuildContext = {
    root: string;
    config: WaeConfig;
    dryRun: boolean;
    packageOverride?: string;
    log: (line: string) => void;
};

type StepResult = { step: BuildStepReport; selected?: WaeNativeTarget };

/** Run the canonical build and return a per-step report. */
export function runBuild(options: BuildOptions): BuildReport {
    const log = options.log ?? ((line: string) => console.log(line));
    const loaded = loadConfig(options.startDir);
    const dryRun = Boolean(options.dryRun);
    const report: BuildReport = {
        root: loaded.root,
        source: loaded.source,
        ...(loaded.path === undefined ? {} : { configPath: loaded.path }),
        dryRun,
        steps: [],
    };

    if (loaded.config === undefined) {
        const step = runDefaultBuild(loaded.root, {
            dryRun,
            packageOverride: options.packageOverride,
            log,
        });
        report.steps.push(step);
        return report;
    }

    log(`[build] config: ${loaded.path ?? '<default>'}`);
    const context: BuildContext = {
        root: loaded.root,
        config: loaded.config,
        dryRun,
        ...(options.packageOverride === undefined ? {} : { packageOverride: options.packageOverride }),
        log,
    };

    const native = runStep('native', () => runNativeStep(context, options.targetOverride), log);
    const selected = native.selected;
    report.steps.push(native.step);
    report.steps.push(runStep('ts', () => runTsStep(context), log).step);
    report.steps.push(runStep('ui', () => runUiStep(context), log).step);
    report.steps.push(runStep('skills', () => runSkillsStep(context), log).step);
    report.steps.push(
        runStep('staging', () => runStagingStep(context, selected), log).step,
    );
    log(`[build] ok (${dryRun ? 'dry run' : 'built'} ${report.steps.length} step(s))`);
    return report;
}

function runStep(name: WaeBuildStepName, body: () => StepResult, log: (line: string) => void): StepResult {
    const result = body();
    const prefix = result.step.status === 'skipped' ? 'skipped — ' : '';
    log(`[build] ${name}: ${prefix}${result.step.detail}`);
    return result;
}

function runNativeStep(context: BuildContext, targetOverride: string | undefined): StepResult {
    const native = context.config.native;
    const crate = context.packageOverride ?? native?.crate;

    if (native === undefined) {
        if (context.packageOverride === undefined) {
            if (context.config.packages !== undefined) {
                throw new WaeBuildError(
                    BUILD_CODES.targetMissing,
                    `\`packages\` declares platform packages but \`native\` has no target to build them from`,
                    { attempts: context.config.packages.map((pkg) => pkg.name) },
                );
            }
            return { step: { name: 'native', status: 'skipped', detail: '`native` is not configured' } };
        }
        if (context.dryRun) {
            return { step: { name: 'native', status: 'planned', detail: `would build crate \`${crate}\`` } };
        }
        cargoBuildNative({ root: context.root, crate: crate! });
        const artifact = findNativeArtifact({ root: context.root, crate: crate! });
        return { step: { name: 'native', status: 'ok', detail: `built \`${crate}\` → ${artifact}` } };
    }

    const selected = selectTarget(native, targetOverride);
    const crossCompiled = `${selected.platform}-${selected.arch}` !== hostKey();
    if (context.dryRun) {
        return {
            step: {
                name: 'native',
                status: 'planned',
                detail: `would ${native.stageOnly ? 'stage' : 'build'} \`${crate}\` for \`${selected.platform}-${selected.arch}\` → ${path.join(selected.dir, selected.nodeFile)}`,
            },
            selected,
        };
    }

    if (native.stageOnly !== true) {
        cargoBuildNative({
            root: context.root,
            crate: crate!,
            ...(native.cdylib === undefined ? {} : { cdylib: native.cdylib }),
            profile: native.profile ?? 'release',
            ...(native.buildDir === undefined ? {} : { buildDir: native.buildDir }),
            ...(crossCompiled ? { triple: selected.triple } : {}),
        });
    }
    const artifact = findNativeArtifact({
        root: context.root,
        crate: crate!,
        ...(native.cdylib === undefined ? {} : { cdylib: native.cdylib }),
        profile: native.profile ?? 'release',
        ...(native.buildDir === undefined ? {} : { buildDir: native.buildDir }),
        ...(crossCompiled ? { triple: selected.triple } : {}),
    });
    const destDir = path.resolve(context.root, selected.dir);
    if (!fs.existsSync(destDir)) {
        throw new WaeBuildError(
            BUILD_CODES.platformPackageInvalid,
            `platform package directory is missing: ${destDir}`,
            { platform: `${selected.platform}-${selected.arch}`, attempts: [destDir] },
        );
    }
    const dest = stageBuiltArtifact(artifact, destDir, selected.nodeFile);
    return {
        step: { name: 'native', status: 'ok', detail: `staged \`${selected.platform}-${selected.arch}\` → ${dest}` },
        selected,
    };
}

function hostKey(): string {
    return `${process.platform}-${process.arch}`;
}

function selectTarget(native: WaeNativeConfig, override: string | undefined): WaeNativeTarget {
    const host = hostKey();
    const envKey = native.targetEnv === undefined ? undefined : process.env[native.targetEnv];
    const key = override ?? envKey?.trim() ?? host;
    const declared = native.targets.map((target) => `${target.platform}-${target.arch}`);
    const target = native.targets.find((entry) => `${entry.platform}-${entry.arch}` === key);
    if (!target) {
        throw new WaeBuildError(
            BUILD_CODES.targetMissing,
            `native target \`${key}\` is not declared in ${WAE_CONFIG_FILE}; declared: ${declared.join(', ')}`,
            { platform: key, attempts: declared },
        );
    }
    if (key !== host && (native.crossCompile ?? 'reject') === 'reject') {
        throw new WaeBuildError(
            BUILD_CODES.targetMissing,
            `native target \`${key}\` differs from host \`${host}\`; cross compilation is rejected, run the build on that platform`,
            { platform: key, attempts: declared },
        );
    }
    return target;
}

function runTsStep(context: BuildContext): StepResult {
    const ts = context.config.build?.ts;
    if (ts === undefined) {
        return { step: { name: 'ts', status: 'skipped', detail: '`build.ts` is not configured' } };
    }
    if (context.dryRun) {
        return {
            step: { name: 'ts', status: 'planned', detail: `would build ${ts.packages.map((name) => `\`${name}\``).join(', ')}` },
        };
    }
    for (const name of ts.packages) {
        const dir = findPackageDir(context.root, name);
        if (dir === undefined) {
            throw new WaeBuildError(
                BUILD_CODES.targetMissing,
                `package \`${name}\` listed in \`build.ts.packages\` was not found under ${context.root}`,
                { attempts: [name] },
            );
        }
        runPnpm(context.root, ['--filter', name, 'run', 'build']);
        assertPackageOutput(dir, name);
    }
    return { step: { name: 'ts', status: 'ok', detail: `built ${ts.packages.join(', ')}` } };
}

function runUiStep(context: BuildContext): StepResult {
    const ui = context.config.build?.ui;
    if (ui === undefined) {
        return { step: { name: 'ui', status: 'skipped', detail: '`build.ui` is not configured' } };
    }
    if (context.dryRun) {
        return { step: { name: 'ui', status: 'planned', detail: `would build \`${ui.package}\`` } };
    }
    const dir = findPackageDir(context.root, ui.package);
    if (dir === undefined) {
        throw new WaeBuildError(
            BUILD_CODES.targetMissing,
            `UI package \`${ui.package}\` listed in \`build.ui\` was not found under ${context.root}`,
            { attempts: [ui.package] },
        );
    }
    runPnpm(context.root, ['--filter', ui.package, 'run', 'build']);

    const stage = context.config.build?.stage ?? [];
    const sources = stage.map((entry) => path.resolve(context.root, entry.from));
    const missing = sources.filter((source) => !fs.existsSync(source));
    if (missing.length) {
        throw new WaeBuildError(
            BUILD_CODES.artifactMissing,
            `the UI build did not produce its declared stage source(s): ${missing.join(', ')}`,
            { attempts: missing },
        );
    }
    return { step: { name: 'ui', status: 'ok', detail: `built \`${ui.package}\`` } };
}

function runSkillsStep(context: BuildContext): StepResult {
    const skills = context.config.skills;
    if (skills === undefined) {
        return { step: { name: 'skills', status: 'skipped', detail: '`skills` is not configured' } };
    }
    if (context.dryRun) {
        return {
            step: {
                name: 'skills',
                status: 'planned',
                detail: `would audit ${skills.packages.length} skill package(s)`,
            },
        };
    }
    let skillsCount = 0;
    for (const pkg of skills.packages) {
        const result = auditSkills(context.root, pkg, (line) => context.log(`[build] skills: ${line}`));
        skillsCount += result.skills;
    }
    return { step: { name: 'skills', status: 'ok', detail: `${skillsCount} skill(s) across ${skills.packages.length} package(s)` } };
}

function runStagingStep(context: BuildContext, selected: WaeNativeTarget | undefined): StepResult {
    const build = context.config.build;
    const stage = build?.stage ?? [];

    if (context.dryRun) {
        return {
            step: {
                name: 'staging',
                status: 'planned',
                detail: `would apply ${stage.length} stage cop(ies) and validate ${platformPackages(context).length} platform package(s)`,
            },
            selected,
        };
    }

    let copies = 0;
    for (const entry of stage) {
        const from = path.resolve(context.root, entry.from);
        if (!fs.existsSync(from)) {
            throw new WaeBuildError(
                BUILD_CODES.artifactMissing,
                `stage source is missing: ${from}`,
                { attempts: [from] },
            );
        }
        const to = path.resolve(context.root, entry.to);
        if (entry.clean) fs.rmSync(to, { recursive: true, force: true });
        copyTree(from, to, entry.ignore ?? []);
        if (!fs.existsSync(to)) {
            throw new WaeBuildError(BUILD_CODES.artifactMissing, `stage target was not written: ${to}`, {
                attempts: [to],
            });
        }
        copies += 1;
    }

    validatePlatformPackages(context, selected);
    validateReleaseSet(context);

    for (const output of build?.outputs ?? []) {
        const file = path.resolve(context.root, output);
        if (!fs.existsSync(file)) {
            throw new WaeBuildError(BUILD_CODES.artifactMissing, `declared build output is missing: ${file}`, {
                attempts: [file],
            });
        }
    }

    const detail = [
        `${copies} stage cop(ies)`,
        `${platformPackages(context).length} platform package(s)`,
        context.config.release === undefined ? 'release not configured' : `release set ${context.config.release.packages.length} package(s)`,
    ].join(', ');
    return { step: { name: 'staging', status: 'ok', detail: `validated ${detail}` }, selected };
}

function platformPackages(context: BuildContext): WaePlatformPackage[] {
    if (context.config.packages !== undefined) return context.config.packages;
    if (context.config.native === undefined) return [];
    return context.config.native.targets.map((target) => ({
        name: target.package,
        dir: target.dir,
        os: [target.platform],
        cpu: [target.arch],
        main: target.nodeFile,
    }));
}

function validatePlatformPackages(context: BuildContext, selected: WaeNativeTarget | undefined): void {
    const selectedKey = selected === undefined ? undefined : `${selected.platform}-${selected.arch}`;
    const native = context.config.native;
    for (const pkg of platformPackages(context)) {
        const dir = path.resolve(context.root, pkg.dir);
        const manifestPath = path.join(dir, 'package.json');
        const target = native?.targets.find((entry) => entry.package === pkg.name);
        const platform = target === undefined ? `${pkg.os.join(',')}-${pkg.cpu.join(',')}` : `${target.platform}-${target.arch}`;
        const invalid = (message: string): never => {
            throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, message, {
                platform,
                attempts: [manifestPath],
            });
        };

        if (!fs.existsSync(manifestPath)) invalid(`platform package manifest is missing: ${manifestPath}`);
        const manifest = readJson(manifestPath, invalid);
        if (manifest.name !== pkg.name) invalid(`platform package name is \`${String(manifest.name)}\`, expected \`${pkg.name}\``);
        if (typeof manifest.version !== 'string' || !EXACT_VERSION_RE.test(manifest.version)) {
            invalid(`platform package \`${pkg.name}\` needs an exact X.Y.Z version, got \`${String(manifest.version)}\``);
        }
        if (!sameStringSet(manifest.os, pkg.os)) {
            invalid(`platform package \`${pkg.name}\` declares os ${format(manifest.os)} but config declares ${format(pkg.os)}`);
        }
        if (!sameStringSet(manifest.cpu, pkg.cpu)) {
            invalid(`platform package \`${pkg.name}\` declares cpu ${format(manifest.cpu)} but config declares ${format(pkg.cpu)}`);
        }
        if (manifest.main !== pkg.main) {
            invalid(`platform package \`${pkg.name}\` declares main \`${String(manifest.main)}\`, expected \`${pkg.main}\``);
        }
        const rawFiles: unknown = manifest.files;
        const files = Array.isArray(rawFiles) ? rawFiles : invalid(`platform package \`${pkg.name}\` must declare a \`files\` array`);
        if (files.length === 0 || !files.includes(pkg.main)) {
            invalid(`platform package \`${pkg.name}\` must list \`${pkg.main}\` in \`files\``);
        }
        for (const entry of files) {
            if (typeof entry !== 'string' || (entry !== pkg.main && !/^(?:readme|license|package)(?:\.(?:md|markdown|txt|json))?$/i.test(entry))) {
                invalid(`platform package \`${pkg.name}\` may only ship the addon and package metadata, found \`${String(entry)}\``);
            }
        }

        if (context.dryRun || selectedKey === undefined || target === undefined) continue;
        if (`${target.platform}-${target.arch}` !== selectedKey) continue;
        const artifact = path.join(dir, pkg.main);
        if (!fs.existsSync(artifact)) {
            throw new WaeBuildError(
                BUILD_CODES.artifactMissing,
                `platform package \`${pkg.name}\` is missing its staged artifact \`${pkg.main}\``,
                { platform: selectedKey, attempts: [artifact] },
            );
        }
    }
}

function validateReleaseSet(context: BuildContext): void {
    const release = context.config.release;
    if (release === undefined) return;
    try {
        new RegExp(release.tagPattern);
    } catch {
        mismatch(`\`release.tagPattern\` is not a valid regular expression: \`${release.tagPattern}\``);
    }

    const entries: { name: string; version: string; dir: string }[] = [];
    const names = new Set<string>();
    for (const entry of release.packages) {
        const dir = path.resolve(context.root, entry.dir);
        const manifestPath = path.join(dir, 'package.json');
        if (!fs.existsSync(manifestPath)) {
            mismatch(`release package manifest is missing: ${manifestPath}`, manifestPath);
        }
        const manifest = readJson(manifestPath, (message, attempts) => mismatch(message, attempts?.[0] ?? manifestPath));
        const name = typeof manifest.name === 'string' ? manifest.name : '';
        if (!name) mismatch(`release package \`${entry.dir}\` has no name`);
        if (entry.name !== undefined && entry.name !== name) {
            mismatch(`release entry \`${entry.name}\` points at package \`${name}\``);
        }
        if (manifest.private === true) mismatch(`release package \`${name}\` is private and cannot be published`);
        if (typeof manifest.version !== 'string' || !EXACT_VERSION_RE.test(manifest.version)) {
            mismatch(`release package \`${name}\` needs an exact X.Y.Z version, got \`${String(manifest.version)}\``);
        }
        if (names.has(name)) mismatch(`release set lists \`${name}\` more than once`);
        names.add(name);
        entries.push({ name, version: manifest.version, dir });
    }

    const versions = new Set(entries.map((entry) => entry.version));
    if (versions.size > 1) {
        mismatch(`release set versions differ: ${entries.map((entry) => `${entry.name}@${entry.version}`).join(', ')}`);
    }

    const expected = publishablePackages(context);
    for (const name of expected) {
        if (!names.has(name)) mismatch(`release set is missing publishable package \`${name}\``);
    }
    for (const name of names) {
        if (!expected.has(name)) mismatch(`release set contains \`${name}\` which is not a publishable package in this project`);
    }

    const platformNames = new Set((context.config.native?.targets ?? []).map((target) => target.package));
    let sawAppPackage = false;
    for (const entry of entries) {
        if (platformNames.has(entry.name)) {
            if (sawAppPackage) mismatch(`platform package \`${entry.name}\` must be listed before app packages`);
            continue;
        }
        sawAppPackage = true;
    }

    const nifty = readNiftyPublishSet(context.root);
    if (nifty !== undefined) {
        for (const name of nifty) {
            if (!names.has(name)) mismatch(`\`nifty.config.ts#publish.packages\` lists \`${name}\`, which is not in \`wae.config.ts#release.packages\``);
        }
        for (const name of names) {
            if (!nifty.has(name)) mismatch(`\`wae.config.ts#release.packages\` lists \`${name}\`, which is not in \`nifty.config.ts#publish.packages\``);
        }
    }

    const cargoPath = path.join(context.root, 'Cargo.toml');
    if (fs.existsSync(cargoPath)) {
        const cargoVersion = readWorkspaceVersion(cargoPath);
        const version = entries[0]?.version;
        if (cargoVersion !== undefined && version !== undefined && cargoVersion !== version) {
            mismatch(`Cargo [workspace.package] version ${cargoVersion} does not match npm release version ${version}`);
        }
    }
}

function publishablePackages(context: BuildContext): Set<string> {
    const expected = new Set<string>();
    for (const pkg of platformPackages(context)) {
        const manifest = tryReadManifest(path.resolve(context.root, pkg.dir));
        if (manifest !== undefined && manifest.private !== true) expected.add(pkg.name);
    }
    for (const name of context.config.build?.ts?.packages ?? []) {
        const dir = findPackageDir(context.root, name);
        const manifest = dir === undefined ? undefined : tryReadManifest(dir);
        if (manifest !== undefined && manifest.private !== true) expected.add(name);
    }
    const ui = context.config.build?.ui?.package;
    if (ui !== undefined) {
        const dir = findPackageDir(context.root, ui);
        const manifest = dir === undefined ? undefined : tryReadManifest(dir);
        if (manifest !== undefined && manifest.private !== true) expected.add(ui);
    }
    return expected;
}

/** Read `publish.packages` from `nifty.config.ts`; `undefined` when absent or not declared. */
function readNiftyPublishSet(root: string): Set<string> | undefined {
    const file = path.join(root, 'nifty.config.ts');
    if (!fs.existsSync(file)) return undefined;
    let parsed: Record<string, unknown>;
    try {
        parsed = parseStaticObject(fs.readFileSync(file, 'utf8'), file);
    } catch (error) {
        mismatch(`cannot parse \`nifty.config.ts\`: ${error instanceof Error ? error.message : String(error)}`);
    }
    const publish = parsed.publish;
    if (!isRecord(publish) || !Array.isArray(publish.packages)) return undefined;
    const names = new Set<string>();
    for (const entry of publish.packages) {
        if (typeof entry === 'string') names.add(entry);
        else if (isRecord(entry) && typeof entry.name === 'string') names.add(entry.name);
    }
    return names.size === 0 ? undefined : names;
}

function runDefaultBuild(
    root: string,
    options: { dryRun: boolean; packageOverride?: string; log: (line: string) => void },
): BuildStepReport {
    const manifest = tryReadManifest(root);
    if (options.packageOverride !== undefined) {
        if (options.dryRun) {
            return { name: 'default', status: 'planned', detail: `would build crate \`${options.packageOverride}\`` };
        }
        cargoBuildNative({ root, crate: options.packageOverride });
        const artifact = findNativeArtifact({ root, crate: options.packageOverride });
        return { name: 'default', status: 'ok', detail: `built \`${options.packageOverride}\` → ${artifact}` };
    }
    if (fs.existsSync(path.join(root, 'pnpm-workspace.yaml'))) {
        if (options.dryRun) {
            return { name: 'default', status: 'planned', detail: 'would build every workspace package' };
        }
        runPnpm(root, ['-r', '--filter', '!.', 'run', 'build']);
        return { name: 'default', status: 'ok', detail: 'built workspace packages' };
    }
    const script = manifest?.scripts;
    const build = isRecord(script) && typeof script.build === 'string' ? script.build : undefined;
    if (build !== undefined && !/\bwae\s+build\b/.test(build)) {
        if (options.dryRun) {
            return { name: 'default', status: 'planned', detail: `would run \`pnpm run build\` (${build})` };
        }
        runPnpm(root, ['run', 'build']);
        const main = typeof manifest?.main === 'string' ? manifest.main : undefined;
        if (main !== undefined && !fs.existsSync(path.resolve(root, main))) {
            throw new WaeBuildError(BUILD_CODES.artifactMissing, `\`pnpm run build\` did not produce \`${main}\``, {
                attempts: [path.resolve(root, main)],
            });
        }
        return { name: 'default', status: 'ok', detail: 'ran the root `build` script' };
    }
    throw new WaeBuildError(
        BUILD_CODES.targetMissing,
        `no ${WAE_CONFIG_FILE} and no default build target under ${root}; add a config or pass \`--package\``,
        { attempts: [path.join(root, WAE_CONFIG_FILE), path.join(root, 'pnpm-workspace.yaml')] },
    );
}

function findPackageDir(root: string, name: string): string | undefined {
    const skip = new Set(['node_modules', '.git', 'target', 'dist', 'build', 'coverage', '.turbo', '.pnpm-store']);
    const walk = (dir: string, depth: number): string | undefined => {
        if (depth > 6) return undefined;
        const manifest = path.join(dir, 'package.json');
        if (fs.existsSync(manifest)) {
            try {
                const parsed = JSON.parse(fs.readFileSync(manifest, 'utf8')) as Record<string, unknown>;
                if (parsed.name === name) return dir;
            } catch {
                // Keep walking: a malformed nested manifest must not hide the real package.
            }
        }
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            if (!entry.isDirectory() || entry.name.startsWith('.') || skip.has(entry.name)) continue;
            const found = walk(path.join(dir, entry.name), depth + 1);
            if (found !== undefined) return found;
        }
        return undefined;
    };
    return walk(path.resolve(root), 0);
}

function assertPackageOutput(dir: string, name: string): void {
    const manifest = tryReadManifest(dir);
    const output = manifest === undefined ? undefined : firstString(manifest.main, moduleEntry(manifest.exports));
    if (output === undefined) {
        throw new WaeBuildError(BUILD_CODES.artifactMissing, `package \`${name}\` declares no \`main\` or export entry`, {
            attempts: [dir],
        });
    }
    const file = path.resolve(dir, output);
    if (!fs.existsSync(file)) {
        throw new WaeBuildError(BUILD_CODES.artifactMissing, `package \`${name}\` build did not produce \`${output}\``, {
            attempts: [file],
        });
    }
}

function moduleEntry(value: unknown): string | undefined {
    if (!isRecord(value)) return undefined;
    const root = value['.'];
    if (typeof root === 'string') return root;
    if (isRecord(root)) return firstString(root.types, root.default, root.import, root.require);
    return undefined;
}

function firstString(...values: unknown[]): string | undefined {
    for (const value of values) {
        if (typeof value === 'string' && value) return value;
    }
    return undefined;
}

function runPnpm(root: string, args: string[]): void {
    // Windows resolves `pnpm` through a `.cmd` shim, which needs a shell. Pass a
    // single command string so Node never concatenates an argument array.
    const command = process.platform === 'win32' ? ['pnpm', ...args.map(shellQuote)].join(' ') : 'pnpm';
    const result = process.platform === 'win32'
        ? spawnSync(command, { cwd: root, stdio: 'inherit', shell: true })
        : spawnSync(command, args, { cwd: root, stdio: 'inherit' });
    if (result.status !== 0) {
        throw new WaeBuildError(
            BUILD_CODES.stepFailed,
            `pnpm ${args.join(' ')} failed in ${root} (exit ${result.status ?? 'null'})`,
        );
    }
}

function shellQuote(value: string): string {
    return /^[A-Za-z0-9@/:._-]+$/.test(value) ? value : `"${value.replace(/"/g, '\\"')}"`;
}

function copyTree(from: string, to: string, ignore: string[]): void {
    const ignored = new Set(ignore);
    fs.cpSync(from, to, {
        recursive: true,
        force: true,
        filter: (source) => !ignored.has(path.basename(source)),
    });
}

function readJson(file: string, fail: (message: string, attempts?: string[]) => never): Record<string, unknown> {
    try {
        return JSON.parse(fs.readFileSync(file, 'utf8')) as Record<string, unknown>;
    } catch (error) {
        fail(`cannot read ${file}: ${error instanceof Error ? error.message : String(error)}`, [file]);
    }
}

function tryReadManifest(dir: string): Record<string, unknown> | undefined {
    const file = path.join(dir, 'package.json');
    if (!fs.existsSync(file)) return undefined;
    try {
        return JSON.parse(fs.readFileSync(file, 'utf8')) as Record<string, unknown>;
    } catch {
        return undefined;
    }
}

function format(value: unknown): string {
    return Array.isArray(value) ? value.map(String).join(',') : '<missing>';
}

function sameStringSet(value: unknown, expected: string[]): boolean {
    if (!Array.isArray(value)) return false;
    const actual = value.map(String).sort();
    const wanted = [...expected].sort();
    return actual.length === wanted.length && actual.every((entry, index) => entry === wanted[index]);
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function mismatch(message: string, attempt?: string): never {
    throw new WaeBuildError(BUILD_CODES.releaseSetMismatch, message, attempt === undefined ? {} : { attempts: [attempt] });
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
