/**
 * Load the native WAE addon from the exact platform package.
 *
 * The addon is optional and platform specific. Resolution is exact: when the platform is not
 * published, or the platform package is installed without its artifact, this throws a structured
 * `NativeUnsupportedError` instead of falling back to a JavaScript implementation.
 *
 * Callers require the platform package (or its checkout directory). `package.json#main` owns the
 * `.node` filename — loaders never concatenate it.
 */
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

export type NativeCapabilities = {
    platform: string;
    arch: string;
    audit: boolean;
    version: string;
    atomic?: string[];
};

export type NativeProbe = {
    ok: boolean;
    platform: string;
    arch: string;
};

export type NativeAuditFilter = {
    kind?: 'user' | 'agent';
    id?: string;
    capability?: string;
    fromMs?: number;
    toMs?: number;
    limit?: number;
};

export type WaeNative = {
    version(): string;
    capabilities(): NativeCapabilities;
    probe(): NativeProbe;
    auditQuery(filter?: NativeAuditFilter): string;
};

/** Capability name reported by unsupported and load failures. */
export const NATIVE_CAPABILITY = 'native-addon';

/** Node platform id → Rust target OS name reported by `capabilities()`. */
export const NATIVE_PLATFORM_NAMES: Record<string, string> = {
    win32: 'windows',
    darwin: 'macos',
    linux: 'linux',
};

/** Node arch id → Rust target architecture name reported by `capabilities()`. */
export const NATIVE_ARCH_NAMES: Record<string, string> = {
    x64: 'x86_64',
    arm64: 'aarch64',
};

/** Platforms with a published native addon package. */
const PLATFORM_PACKAGES: Record<string, string> = {
    'win32-x64': '@wae/wae-win32-x64',
    'darwin-arm64': '@wae/wae-darwin-arm64',
    'linux-x64': '@wae/wae-linux-x64',
};

const NATIVE_REQUIRE = createRequire(import.meta.url);
const HERE = path.dirname(fileURLToPath(import.meta.url));

/** Supported platform keys, e.g. `win32-x64`. */
export function supportedPlatforms(): string[] {
    return Object.keys(PLATFORM_PACKAGES);
}

/** Platform package name for a key, or `undefined` when the platform is not published. */
export function platformPackageName(key: string): string | undefined {
    return PLATFORM_PACKAGES[key];
}

/**
 * Addon file name inside the platform package.
 *
 * Staging / build tooling may still need this. Runtime loaders must not use it —
 * they require the package and let `package.json#main` choose the file.
 */
export function nativeFileName(key: string): string {
    return `wae.${key}.node`;
}

/** Structured failure for a missing platform or a missing platform artifact. */
export class NativeUnsupportedError extends Error {
    readonly code = 'unsupported_platform' as const;
    readonly capability = NATIVE_CAPABILITY;
    readonly platform: string;
    readonly attempts: readonly string[];

    constructor(platform: string, attempts: readonly string[], message: string) {
        super(message);
        this.name = 'NativeUnsupportedError';
        this.platform = platform;
        this.attempts = attempts;
    }

    toJSON() {
        return {
            code: this.code,
            capability: this.capability,
            platform: this.platform,
            attempts: this.attempts,
            message: this.message,
        };
    }
}

/** Structured failure when an existing `.node` cannot be loaded by the current Node runtime. */
export class NativeLoadError extends Error {
    readonly code = 'native_load_failed' as const;
    readonly capability = NATIVE_CAPABILITY;
    readonly platform: string;
    readonly path: string;

    constructor(platform: string, addonPath: string, message: string, cause: unknown) {
        super(message, { cause });
        this.name = 'NativeLoadError';
        this.platform = platform;
        this.path = addonPath;
    }

    toJSON() {
        return {
            code: this.code,
            capability: this.capability,
            platform: this.platform,
            path: this.path,
            message: this.message,
        };
    }
}

export type NativeResolveOptions = {
    platform?: string;
    arch?: string;
    here?: string;
    /** Resolve a package specifier the way Node would; overridable for tests. */
    resolve?: (specifier: string) => string;
    /** File existence check; overridable for tests. */
    exists?: (file: string) => boolean;
};

/**
 * Module specifiers to try: the installed platform package name, then the
 * repository checkout directory that contains that package's `package.json`.
 */
export function nativeCandidates(options: NativeResolveOptions = {}): string[] {
    const platform = options.platform ?? process.platform;
    const arch = options.arch ?? process.arch;
    const key = `${platform}-${arch}`;
    const packageName = PLATFORM_PACKAGES[key];
    if (!packageName) {
        return [];
    }

    const here = options.here ?? HERE;
    // packages/wae/<src|dist> → wae.ts/platforms/<package>
    const localPackage = path.resolve(here, '..', '..', '..', 'platforms', `wae-${key}`);
    return [packageName, localPackage];
}

/** Resolve the addon path, throwing `NativeUnsupportedError` when no candidate exists. */
export function resolveNativePath(options: NativeResolveOptions = {}): string {
    const platform = options.platform ?? process.platform;
    const arch = options.arch ?? process.arch;
    const key = `${platform}-${arch}`;
    const candidates = nativeCandidates(options);
    if (candidates.length === 0) {
        throw new NativeUnsupportedError(
            key,
            [],
            `no native addon is published for \`${key}\` (supported: ${supportedPlatforms().join(', ')})`,
        );
    }

    const resolve = options.resolve ?? ((specifier: string) => NATIVE_REQUIRE.resolve(specifier));
    const exists = options.exists ?? fs.existsSync;
    const attempts: string[] = [];

    for (const candidate of candidates) {
        try {
            const resolved = resolve(candidate);
            attempts.push(resolved);
            if (exists(resolved)) {
                return resolved;
            }
        }
        catch {
            attempts.push(candidate);
        }
    }

    const packageName = PLATFORM_PACKAGES[key];
    throw new NativeUnsupportedError(
        key,
        attempts,
        `\`${packageName}\` could not be resolved for \`${key}\` through \`package.json#main\`; install the platform package or run \`pnpm build:napi\``,
    );
}

/** Load the addon from the platform package. */
export function loadNative(options: NativeResolveOptions & { load?: (file: string) => unknown } = {}): WaeNative {
    const addonPath = resolveNativePath(options);
    return loadAt(addonPath, platformKey(options), options.load ?? ((file: string) => NATIVE_REQUIRE(file)));
}

/** Result of resolving and loading the addon, as reported by `wae native info`. */
export type NativeInspection = {
    platformKey: string;
    path: string;
    version: string;
    capabilities: NativeCapabilities;
    probe: NativeProbe;
    /** Audit entries recorded for the probe call, proving the audit facility is wired. */
    auditEntries: number;
};

/** Resolve the addon, load it, probe it, and report what it exposes. */
export function inspectNative(
    options: NativeResolveOptions & { load?: (file: string) => unknown } = {},
): NativeInspection {
    const addonPath = resolveNativePath(options);
    const addon = loadAt(addonPath, platformKey(options), options.load ?? ((file: string) => NATIVE_REQUIRE(file)));
    const probe = addon.probe();
    const entries = JSON.parse(addon.auditQuery({ capability: 'wae.native.probe', limit: 5 })) as unknown;
    return {
        platformKey: platformKey(options),
        path: addonPath,
        version: addon.version(),
        capabilities: addon.capabilities(),
        probe,
        auditEntries: Array.isArray(entries) ? entries.length : 0,
    };
}

function platformKey(options: NativeResolveOptions): string {
    return `${options.platform ?? process.platform}-${options.arch ?? process.arch}`;
}

function loadAt(addonPath: string, key: string, load: (file: string) => unknown): WaeNative {
    try {
        return load(addonPath) as WaeNative;
    } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        throw new NativeLoadError(
            key,
            addonPath,
            `failed to load the WAE native addon at ${addonPath}: ${detail}`,
            error,
        );
    }
}
