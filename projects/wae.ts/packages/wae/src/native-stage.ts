/**
 * Native build and staging primitives shared by `wae build`, `wae native`, and `wae dist`.
 *
 * There is exactly one candidate lookup (`findNativeArtifact`). Product scripts
 * must not re-implement it.
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { BUILD_CODES, WaeBuildError } from './build-error.js';
import { WAE_CONFIG_FILE, type WaeNativeConfig, type WaeNativeTarget } from './config.js';

/** `<platform>-<arch>` of the running host. */
export function hostKey(): string {
    return `${process.platform}-${process.arch}`;
}

/**
 * Resolve exactly one native target: CLI override first, then `native.targetEnv`,
 * then the host. Cross targets require `crossCompile: 'allow'`.
 */
export function selectNativeTarget(native: WaeNativeConfig, override: string | undefined): WaeNativeTarget {
    const host = hostKey();
    const envKey = native.targetEnv === undefined ? undefined : process.env[native.targetEnv]?.trim();
    // A blank override or env value means "not set", otherwise a CI run that exports
    // an empty target variable would fail instead of selecting the host.
    const requested = override?.trim() ?? envKey;
    const key = requested === undefined || requested === '' ? host : requested;
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

/** Build request for one resolved target; `triple` is set only for cross targets. */
export function nativeBuildRequest(root: string, native: WaeNativeConfig, target: WaeNativeTarget): NativeBuildOptions {
    const cross = `${target.platform}-${target.arch}` !== hostKey();
    return {
        root,
        crate: native.crate,
        ...(native.cdylib === undefined ? {} : { cdylib: native.cdylib }),
        profile: native.profile ?? 'release',
        ...(native.buildDir === undefined ? {} : { buildDir: native.buildDir }),
        ...(cross ? { triple: target.triple } : {}),
    };
}

export type NativeBuildOptions = {
    root: string;
    crate: string;
    /** cdylib stem; defaults to the crate name with `-` replaced by `_`. */
    cdylib?: string;
    profile?: 'debug' | 'release';
    /** Cargo build directory; defaults to `<root>/target`. */
    buildDir?: string;
    /** Cargo `--target` triple; only set when cross compilation is explicitly allowed. */
    triple?: string;
    /** Extra environment for cargo; `CARGO_BUILD_JOBS` defaults to 2. */
    env?: NodeJS.ProcessEnv;
};

/** cdylib file names tried for one crate, in platform order. */
export function nativeArtifactCandidates(options: NativeBuildOptions): string[] {
    const stem = options.cdylib ?? options.crate.replaceAll('-', '_');
    const buildDir = path.resolve(options.root, options.buildDir ?? 'target');
    const profile = options.profile ?? 'release';
    const dirs = options.triple
        ? [path.join(buildDir, options.triple, profile), path.join(buildDir, profile)]
        : [path.join(buildDir, profile)];
    const names = [`${stem}.dll`, `${stem}.node`, `lib${stem}.so`, `lib${stem}.dylib`];
    return dirs.flatMap((dir) => names.map((name) => path.join(dir, name)));
}

/** Run `cargo build -p <crate>` for the selected profile. */
export function cargoBuildNative(options: NativeBuildOptions): void {
    const cargoToml = path.join(path.resolve(options.root), 'Cargo.toml');
    if (!fs.existsSync(cargoToml)) {
        throw new WaeBuildError(BUILD_CODES.targetMissing, `project root is missing Cargo.toml: ${options.root}`, {
            attempts: [cargoToml],
        });
    }
    const args = ['build', '-p', options.crate];
    if ((options.profile ?? 'release') === 'release') args.push('--release');
    if (options.triple) args.push('--target', options.triple);
    // `cargo` resolves to a real executable on every supported host, so no shell.
    const result = spawnSync('cargo', args, {
        cwd: path.resolve(options.root),
        stdio: 'inherit',
        env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? '2', ...options.env },
    });
    if (result.error !== undefined) {
        throw new WaeBuildError(
            BUILD_CODES.stepFailed,
            `cargo ${args.join(' ')} could not start in ${options.root}: ${result.error.message}`,
        );
    }
    if (result.signal !== null) {
        throw new WaeBuildError(
            BUILD_CODES.stepFailed,
            `cargo ${args.join(' ')} was killed by ${result.signal} in ${options.root}`,
        );
    }
    if (result.status !== 0) {
        throw new WaeBuildError(
            BUILD_CODES.stepFailed,
            `cargo ${args.join(' ')} failed in ${options.root} (exit ${String(result.status)})`,
        );
    }
}

/** Resolve the built cdylib; throws `build_artifact_missing` instead of returning undefined. */
export function findNativeArtifact(options: NativeBuildOptions): string {
    const candidates = nativeArtifactCandidates(options);
    for (const candidate of candidates) {
        if (fs.existsSync(candidate)) return candidate;
    }
    throw new WaeBuildError(
        BUILD_CODES.artifactMissing,
        `no native artifact for crate \`${options.crate}\` (stem \`${options.cdylib ?? options.crate.replaceAll('-', '_')}\`)`,
        { attempts: candidates },
    );
}

/** Copy one built artifact into a platform package as `<name>.node`. */
export function stageBuiltArtifact(
    artifact: string,
    destDir: string,
    nodeFile: string,
): string {
    if (!nodeFile.endsWith('.node')) {
        throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, `platform artifact name must end with \`.node\`: ${nodeFile}`);
    }
    if (!fs.existsSync(artifact)) {
        throw new WaeBuildError(BUILD_CODES.artifactMissing, `native artifact is missing: ${artifact}`, { attempts: [artifact] });
    }
    fs.mkdirSync(destDir, { recursive: true });
    const dest = path.join(destDir, nodeFile);
    fs.copyFileSync(artifact, dest);
    if (!fs.existsSync(dest)) {
        throw new WaeBuildError(BUILD_CODES.artifactMissing, `staged artifact was not written: ${dest}`, { attempts: [dest] });
    }
    return dest;
}

/** Build and stage a cdylib into a platform package (the `wae dist` primitive). */
export function stageNative(req: {
    projectRoot: string;
    cargoPackage: string;
    destDir: string;
    destFileName: string;
}): string {
    if (!req.cargoPackage.trim()) throw new WaeBuildError(BUILD_CODES.targetMissing, 'cargo package must not be empty');
    if (!req.destFileName.trim()) throw new WaeBuildError(BUILD_CODES.platformPackageInvalid, 'dest file name must not be empty');
    cargoBuildRelease(req.projectRoot, req.cargoPackage);
    const artifact = findNativeArtifact({ root: req.projectRoot, crate: req.cargoPackage });
    return stageBuiltArtifact(artifact, req.destDir, req.destFileName);
}

/** Run `cargo build -p … --release` in `projectRoot` (legacy single-crate entry). */
export function cargoBuildRelease(projectRoot: string, cargoPackage: string): void {
    cargoBuildNative({ root: projectRoot, crate: cargoPackage, profile: 'release' });
}
