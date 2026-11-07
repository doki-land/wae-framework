import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    NativeLoadError,
    NativeUnsupportedError,
    capabilityMismatch,
    inspectNative,
    loadNative,
    nativeCandidates,
    nativeFileName,
    platformPackageName,
    resolveNativePath,
    supportedPlatforms,
} from '../src/native.ts';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WIN32_NODE = path.join('node_modules', '@wae', 'wae-win32-x64', 'wae.win32-x64.node');

function capture<T>(type: new (...args: never[]) => T, run: () => unknown): T {
    try {
        run();
    } catch (error) {
        if (error instanceof type) {
            return error;
        }
        throw error;
    }
    throw new Error('expected the call to throw');
}

test('only the published native platforms are supported', () => {
    assert.deepEqual(supportedPlatforms(), ['win32-x64', 'darwin-arm64', 'linux-x64']);
    assert.equal(platformPackageName('win32-x64'), '@wae/wae-win32-x64');
    assert.equal(platformPackageName('darwin-x64'), undefined);
    assert.equal(nativeFileName('linux-x64'), 'wae.linux-x64.node');
});

test('resolveNativePath rejects an unpublished platform with capability and platform', () => {
    const error = capture(NativeUnsupportedError, () => resolveNativePath({ platform: 'sunos', arch: 'x64', here: HERE }));
    assert.equal(error.code, 'unsupported_platform');
    assert.equal(error.capability, 'native-addon');
    assert.equal(error.platform, 'sunos-x64');
    assert.deepEqual(error.attempts, []);
    assert.deepEqual(error.toJSON(), {
        code: 'unsupported_platform',
        capability: 'native-addon',
        platform: 'sunos-x64',
        attempts: [],
        message: error.message,
    });
});

test('resolveNativePath reports every attempted specifier when the artifact is missing', () => {
    const error = capture(NativeUnsupportedError, () =>
        resolveNativePath({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => {
                throw new Error('not installed');
            },
            exists: () => false,
        }),
    );
    assert.equal(error.platform, 'win32-x64');
    assert.equal(error.attempts.length, 2);
    assert.equal(error.attempts[0], '@wae/wae-win32-x64');
    assert.match(error.attempts[1], /platforms[\\/]wae-win32-x64$/);
    assert.match(error.message, /@wae\/wae-win32-x64/);
});

test('resolveNativePath prefers the package specifier and falls back to the checkout directory', () => {
    const candidates = nativeCandidates({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
    });
    assert.equal(candidates.length, 2);
    assert.equal(candidates[0], '@wae/wae-win32-x64');
    assert.ok(candidates[1].includes(path.join('platforms', 'wae-win32-x64')));

    const resolved = resolveNativePath({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
        resolve: (specifier) => {
            if (specifier === '@wae/wae-win32-x64') return WIN32_NODE;
            throw new Error(`unexpected specifier ${specifier}`);
        },
        exists: (file) => file === WIN32_NODE,
    });
    assert.equal(resolved, WIN32_NODE);
});

test('loadNative returns the binding and wraps load failures structurally', () => {
    const binding = { version: () => '0.0.1' };
    const addon = path.join(HERE, 'wae.win32-x64.node');
    assert.equal(
        loadNative({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => addon,
            exists: () => true,
            load: () => binding,
        }),
        binding,
    );

    const error = capture(NativeLoadError, () =>
        loadNative({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => addon,
            exists: () => true,
            load: () => {
                throw new Error('bad image');
            },
        }),
    );
    assert.equal(error.code, 'native_load_failed');
    assert.equal(error.capability, 'native-addon');
    assert.equal(error.platform, 'win32-x64');
    assert.equal(error.path, addon);
});

test('native load failure keeps the original error as cause', () => {
    const original = new Error('bad image');
    const addon = path.join(HERE, 'wae.win32-x64.node');
    const error = capture(NativeLoadError, () =>
        loadNative({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => addon,
            exists: () => true,
            load: () => {
                throw original;
            },
        }),
    );
    assert.equal(error.cause, original);
});

test('inspectNative reports the resolved path, version, capabilities and audit wiring', () => {
    const capabilities = { platform: 'windows', arch: 'x64', audit: true, version: '0.0.2' };
    const addon = path.join(HERE, 'wae.win32-x64.node');
    const report = inspectNative({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
        resolve: () => addon,
        exists: () => true,
        load: () => ({
            version: () => '0.0.2',
            capabilities: () => capabilities,
            probe: () => ({ ok: true, platform: 'windows', arch: 'x64' }),
            auditQuery: () => JSON.stringify([{ capability: 'wae.native.probe' }]),
        }),
    });
    assert.equal(report.platformKey, 'win32-x64');
    assert.equal(report.path, addon);
    assert.equal(report.version, '0.0.2');
    assert.deepEqual(report.capabilities, capabilities);
    assert.deepEqual(report.probe, { ok: true, platform: 'windows', arch: 'x64' });
    assert.equal(report.auditEntries, 1);
});

test('inspectNative keeps unsupported platforms structured', () => {
    const error = capture(NativeUnsupportedError, () => inspectNative({ platform: 'darwin', arch: 'x64', here: HERE }));
    assert.equal(error.code, 'unsupported_platform');
    assert.equal(error.platform, 'darwin-x64');
    assert.deepEqual(error.attempts, []);
});

test('capabilityMismatch accepts partial identity reports', () => {
    const expected = { platform: 'windows', arch: 'x86_64', version: '0.0.3' };
    assert.equal(capabilityMismatch({}, expected), undefined);
    assert.equal(capabilityMismatch({ platform: 'windows' }, expected), undefined);
    assert.equal(capabilityMismatch({ arch: 'x86_64' }, expected), undefined);
    assert.equal(capabilityMismatch({ platform: 'windows', arch: 'x86_64', version: '0.0.3' }, expected), undefined);
});

test('capabilityMismatch rejects every present field that disagrees with the target', () => {
    const expected = { platform: 'windows', arch: 'x86_64', version: '0.0.3' };
    assert.match(String(capabilityMismatch({ platform: 'linux' }, expected)), /platform linux, expected windows/);
    assert.match(String(capabilityMismatch({ arch: 'aarch64' }, expected)), /arch aarch64, expected x86_64/);
    assert.match(
        String(capabilityMismatch({ version: '0.0.2' }, expected)),
        /version 0\.0\.2 does not match version\(\) 0\.0\.3/,
    );
});
