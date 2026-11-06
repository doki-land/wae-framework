import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    NativeLoadError,
    NativeUnsupportedError,
    inspectNative,
    loadNative,
    nativeCandidates,
    nativeFileName,
    platformPackageName,
    resolveNativePath,
    supportedPlatforms,
} from '../src/native.ts';

const HERE = path.dirname(fileURLToPath(import.meta.url));

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

test('resolveNativePath reports every attempted path when the artifact is missing', () => {
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
    assert.equal(error.attempts.length, 1);
    assert.match(error.attempts[0], /platforms[\\/]wae-win32-x64[\\/]wae\.win32-x64\.node/);
    assert.match(error.message, /@wae\/wae-win32-x64/);
});

test('resolveNativePath prefers the resolver result and falls back to the checkout', () => {
    const installed = path.join('node_modules', '@wae', 'wae-win32-x64', 'package.json');
    const candidates = nativeCandidates({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
        resolve: () => installed,
    });
    assert.equal(candidates.length, 2);
    assert.equal(candidates[0], path.join('node_modules', '@wae', 'wae-win32-x64', 'wae.win32-x64.node'));
    assert.ok(candidates[1].includes(path.join('platforms', 'wae-win32-x64', 'wae.win32-x64.node')));

    const resolved = resolveNativePath({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
        resolve: () => installed,
        exists: (file) => file === candidates[0],
    });
    assert.equal(resolved, candidates[0]);
});

test('loadNative returns the binding and wraps load failures structurally', () => {
    const binding = { version: () => '0.0.1' };
    assert.equal(
        loadNative({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => path.join(HERE, 'package.json'),
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
            resolve: () => path.join(HERE, 'package.json'),
            exists: () => true,
            load: () => {
                throw new Error('bad image');
            },
        }),
    );
    assert.equal(error.code, 'native_load_failed');
    assert.equal(error.capability, 'native-addon');
    assert.equal(error.platform, 'win32-x64');
    assert.equal(error.path, path.join(HERE, 'wae.win32-x64.node'));
});

test('native load failure keeps the original error as cause', () => {
    const original = new Error('bad image');
    const error = capture(NativeLoadError, () =>
        loadNative({
            platform: 'win32',
            arch: 'x64',
            here: HERE,
            resolve: () => path.join(HERE, 'package.json'),
            exists: () => true,
            load: () => {
                throw original;
            },
        }),
    );
    assert.equal(error.cause, original);
});

test('inspectNative reports the resolved path, version, capabilities and audit wiring', () => {
    const capabilities = { platform: 'windows', arch: 'x64', audit: true, version: '0.0.2', atomic: ['findArtifact'] };
    const report = inspectNative({
        platform: 'win32',
        arch: 'x64',
        here: HERE,
        resolve: () => path.join(HERE, 'package.json'),
        exists: () => true,
        load: () => ({
            version: () => '0.0.2',
            capabilities: () => capabilities,
            probe: () => ({ ok: true, platform: 'windows', arch: 'x64' }),
            auditQuery: () => JSON.stringify([{ capability: 'wae.native.probe' }]),
        }),
    });
    assert.equal(report.platformKey, 'win32-x64');
    assert.equal(report.path, path.join(HERE, 'wae.win32-x64.node'));
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
