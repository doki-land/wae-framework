/**
 * Native harness bindings loaded from the shared WAE platform package.
 *
 * There is no harness-specific `.node`: `@wae/wae-harness` consumes exactly the
 * same `@wae/wae-<platform>-<arch>` packages as `@wae/wae`, and only uses the
 * `harness*` exports. Missing platform or missing artifact fails with a
 * structured error instead of a JavaScript fallback.
 */
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

/** The harness-relevant part of the shared platform addon. */
export interface HarnessNative {
    harnessOpen(optionsJson: string): number;
    harnessEval(handle: number, script: string): string;
    harnessWaitFor(handle: number, script: string, timeoutMs: number): string;
    harnessScreenshot(handle: number, path: string): string;
    harnessPump(handle: number): boolean;
    harnessPost(handle: number, text: string): void;
    harnessTakeIpc(handle: number): string;
    harnessClose(handle: number): void;
}

/** Structured failure for a missing platform, artifact or invalid addon. */
export class HarnessNativeError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = 'HarnessNativeError';
        this.code = code;
    }
}

const PLATFORM_PACKAGES: Record<string, string> = {
    'win32-x64': '@wae/wae-win32-x64',
    'darwin-arm64': '@wae/wae-darwin-arm64',
    'linux-x64': '@wae/wae-linux-x64',
};

const HERE = path.dirname(fileURLToPath(import.meta.url));
const NATIVE_REQUIRE = createRequire(import.meta.url);

const NATIVE_FUNCTIONS = [
    'harnessOpen',
    'harnessEval',
    'harnessWaitFor',
    'harnessScreenshot',
    'harnessPump',
    'harnessPost',
    'harnessTakeIpc',
    'harnessClose',
] as const;

/** Platform keys with a published shared addon. */
export function supportedPlatforms(): string[] {
    return Object.keys(PLATFORM_PACKAGES);
}

/** Shared platform package name for a `platform-arch` key. */
export function platformPackageName(key: string): string | undefined {
    return PLATFORM_PACKAGES[key];
}

/** The shared platform package map, exported for parity checks against `@wae/wae`. */
export function sharedPlatformPackages(): Record<string, string> {
    return { ...PLATFORM_PACKAGES };
}

/** Candidate specifiers: the installed package, then the repository checkout. */
export function nativeCandidates(platform: string = process.platform, arch: string = process.arch): string[] {
    const key = `${platform}-${arch}`;
    const packageName = PLATFORM_PACKAGES[key];
    if (packageName === undefined) {
        return [];
    }
    return [packageName, path.resolve(HERE, '..', '..', '..', 'platforms', `wae-${key}`)];
}

/** Load the shared platform addon and verify the harness exports exist. */
export function loadHarnessNative(): HarnessNative {
    const key = `${process.platform}-${process.arch}`;
    const packageName = PLATFORM_PACKAGES[key];
    if (packageName === undefined) {
        throw new HarnessNativeError(
            'harness_platform_unsupported',
            `no shared WAE addon is published for ${key}`,
        );
    }
    const attempts: string[] = [];
    let last: unknown = null;
    for (const candidate of nativeCandidates()) {
        attempts.push(candidate);
        if (!candidate.startsWith('@') && !fs.existsSync(candidate)) {
            continue;
        }
        try {
            const addon = NATIVE_REQUIRE(candidate) as Partial<HarnessNative>;
            const missing = NATIVE_FUNCTIONS.filter((name) => typeof addon[name] !== 'function');
            if (missing.length > 0) {
                throw new Error(`addon is missing harness exports: ${missing.join(', ')}`);
            }
            return addon as HarnessNative;
        } catch (error) {
            last = error;
        }
    }
    throw new HarnessNativeError(
        'harness_native_unavailable',
        `cannot load ${packageName} (attempted: ${attempts.join(', ')}): ${String(last)}`,
    );
}
