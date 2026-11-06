/**
 * Load the staged `.node` with a real `require()` and call one capability. This is the native
 * runner smoke: the same file that ships in the platform package must load and answer.
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

import { nativeFileName, platformDir, resolveTarget } from './build-napi.ts';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const PLATFORM_NAMES: Record<string, string> = { win32: 'windows', darwin: 'macos', linux: 'linux' };
const ARCH_NAMES: Record<string, string> = { x64: 'x86_64', arm64: 'aarch64' };

type WaeNative = {
    version(): string;
    capabilities(): { platform: string; arch: string; audit: boolean; version: string; atomic: string[] };
    probe(): { ok: boolean; platform: string; arch: string };
    auditQuery(filter?: { capability?: string; limit?: number }): string;
};

export function main(root = ROOT): void {
    const target = resolveTarget();
    const dir = platformDir(root, target);
    const addonPath = path.join(dir, nativeFileName(target));
    if (!fs.existsSync(addonPath)) {
        throw new Error(`native addon is missing at ${addonPath}; run \`pnpm build:napi\` first`);
    }

    const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8')) as {
        name: string;
        version: string;
        os: string[];
        cpu: string[];
    };
    if (manifest.os.includes(target.platform) === false || manifest.cpu.includes(target.arch) === false) {
        throw new Error(
            `platform package declares os/cpu ${manifest.os.join(',')}/${manifest.cpu.join(',')} but runner is ${target.platform}/${target.arch}`,
        );
    }

    process.env.WAE_AUDIT_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-native-audit-'));

    const native: WaeNative = createRequire(import.meta.url)(addonPath);
    const version = native.version();
    if (version !== manifest.version) {
        throw new Error(`native addon version ${version || '<empty>'} does not match ${manifest.name} ${manifest.version}`);
    }

    const capabilities = native.capabilities();
    if (capabilities.platform !== PLATFORM_NAMES[target.platform] || capabilities.arch !== ARCH_NAMES[target.arch]) {
        throw new Error(
            `capabilities mismatch: got ${capabilities.platform}/${capabilities.arch}, expected ${PLATFORM_NAMES[target.platform]}/${ARCH_NAMES[target.arch]}`,
        );
    }
    if (capabilities.version !== version) {
        throw new Error(`capabilities version ${capabilities.version} does not match version() ${version}`);
    }
    if (!Array.isArray(capabilities.atomic) || capabilities.atomic.length === 0) {
        throw new Error('capabilities did not list atomic native exports');
    }

    const probe = native.probe();
    if (probe.ok !== true) {
        throw new Error('native probe did not report ok');
    }

    const entries = JSON.parse(native.auditQuery({ capability: 'wae.native.probe', limit: 5 }));
    if (!Array.isArray(entries) || entries.length === 0) {
        throw new Error('audited probe did not produce a queryable audit entry');
    }

    console.log(
        `[wae] native smoke ok: ${addonPath} version=${version} platform=${capabilities.platform} arch=${capabilities.arch} auditEntries=${entries.length}`,
    );
}

const entry = process.argv[1] ? path.resolve(process.argv[1]) : '';
if (entry === path.resolve(fileURLToPath(import.meta.url))) {
    main();
}
