/**
 * Build the WAE Node-API addon on the host target and stage it into the matching platform package.
 *
 * Candidate lookup is shared with the product CLI through `findCdylib`, so the artifact naming
 * rules exist in one place. Cross-compiled artifacts are not built here: run this script only on
 * the native runner that will load the result.
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { findCdylib } from '../projects/wae.ts/packages/wae/src/native-stage.ts';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CARGO_PACKAGE = 'wae-addon';
const PLATFORMS_DIR = path.join(ROOT, 'projects', 'wae.ts', 'platforms');

export type NativeTarget = {
    platform: 'win32' | 'darwin' | 'linux';
    arch: 'x64' | 'arm64';
    triple: string;
};

const TARGETS: Record<string, NativeTarget> = {
    'win32-x64': { platform: 'win32', arch: 'x64', triple: 'x86_64-pc-windows-msvc' },
    'darwin-arm64': { platform: 'darwin', arch: 'arm64', triple: 'aarch64-apple-darwin' },
    'linux-x64': { platform: 'linux', arch: 'x64', triple: 'x86_64-unknown-linux-gnu' },
};

/** Targets that have a published platform package. */
export function supportedTargets(): string[] {
    return Object.keys(TARGETS);
}

/** Resolve the host target, or an explicit `WAE_NATIVE_TARGET` key. */
export function resolveTarget(platform = process.platform, arch = process.arch): NativeTarget {
    const key = `${platform}-${arch}`;
    const target = TARGETS[key];
    if (!target) {
        throw new Error(`unsupported native target \`${key}\` (supported: ${supportedTargets().join(', ')})`);
    }
    return target;
}

/** Target key, e.g. `win32-x64`. */
export function targetKey(target: NativeTarget): string {
    return `${target.platform}-${target.arch}`;
}

/** npm platform package name, e.g. `@wae/wae-win32-x64`. */
export function platformPackageName(target: NativeTarget): string {
    return `@wae/wae-${targetKey(target)}`;
}

/** Addon file name inside the platform package, e.g. `wae.win32-x64.node`. */
export function nativeFileName(target: NativeTarget): string {
    return `wae.${targetKey(target)}.node`;
}

/** Platform package directory. */
export function platformDir(root: string, target: NativeTarget): string {
    return path.join(root, 'projects', 'wae.ts', 'platforms', `wae-${targetKey(target)}`);
}

/** Cargo release directory for the target (explicit `--target` builds land in a triple subdirectory). */
export function releaseDir(root: string, target: NativeTarget, explicitTarget: boolean): string {
    return explicitTarget ? path.join(root, 'target', target.triple, 'release') : path.join(root, 'target', 'release');
}

/** Resolve the built cdylib, sharing the candidate list with the product CLI. */
export function findNapiArtifact(root: string, target: NativeTarget, explicitTarget: boolean): string {
    return findCdylib(releaseDir(root, target, explicitTarget), CARGO_PACKAGE);
}

/** Build and stage the addon; returns the staged `.node` path. */
export function stageNative(root: string, target: NativeTarget, explicitTarget: boolean): string {
    const artifact = findNapiArtifact(root, target, explicitTarget);
    const destinationDir = platformDir(root, target);
    fs.mkdirSync(destinationDir, { recursive: true });
    const destination = path.join(destinationDir, nativeFileName(target));
    fs.copyFileSync(artifact, destination);
    return destination;
}

/** Run `cargo build -p wae-addon --release` for the target. */
export function cargoBuildNative(root: string, target: NativeTarget, explicitTarget: boolean): void {
    const args = ['build', '-p', CARGO_PACKAGE, '--release'];
    if (explicitTarget) {
        args.push('--target', target.triple);
    }
    const result = spawnSync('cargo', args, { cwd: root, stdio: 'inherit', shell: false });
    if (result.status !== 0) {
        throw new Error(`cargo build -p ${CARGO_PACKAGE} failed (exit ${result.status ?? 'null'})`);
    }
}

/** Build for the host target and stage it, refusing cross-compiled verification. */
export function main(root = ROOT): string {
    const configured = process.env.WAE_NATIVE_TARGET?.trim();
    const target = resolveTarget();
    if (configured && configured !== targetKey(target)) {
        throw new Error(
            `\`WAE_NATIVE_TARGET\` ${configured} does not match the host \`${targetKey(target)}\`; build on the native runner instead of cross-compiling`,
        );
    }
    cargoBuildNative(root, target, false);
    const staged = stageNative(root, target, false);
    console.log(`[wae] native addon staged at ${staged}`);
    return staged;
}

const entry = process.argv[1] ? path.resolve(process.argv[1]) : '';
if (entry === path.resolve(fileURLToPath(import.meta.url))) {
    main();
}
