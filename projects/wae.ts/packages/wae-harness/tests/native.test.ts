import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Harness } from '../src/index.ts';
import { HarnessNativeError, platformPackageName, sharedPlatformPackages, supportedPlatforms } from '../src/native.ts';

test('harness resolves the shared WAE platform package set', () => {
    assert.deepEqual(supportedPlatforms().sort(), ['darwin-arm64', 'linux-x64', 'win32-x64']);
    assert.equal(platformPackageName('win32-x64'), '@wae/wae-win32-x64');
    assert.deepEqual(sharedPlatformPackages(), {
        'win32-x64': '@wae/wae-win32-x64',
        'darwin-arm64': '@wae/wae-darwin-arm64',
        'linux-x64': '@wae/wae-linux-x64',
    });
});

test('closed harness rejects further calls with window_closed', () => {
    assert.throws(
        () => {
            throw new HarnessNativeError('window_closed', 'closed');
        },
        (error: unknown) => error instanceof HarnessNativeError && error.code === 'window_closed',
    );
});

test(
    'in-process harness drives the shared native addon',
    { skip: process.env.WAE_HARNESS_NATIVE !== '1' },
    () => {
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-harness-page-'));
        fs.writeFileSync(path.join(dir, 'index.html'), '<!doctype html><body><h1 id="t">hi</h1></body>');
        const harness = Harness.open({ contentDir: dir, hidden: true });
        try {
            harness.ready(10_000);
            assert.equal(harness.eval("document.querySelector('#t').textContent"), 'hi');
            const shot = path.join(dir, 'shot.png');
            assert.ok(fs.statSync(harness.screenshot(shot)).size > 0);
            assert.deepEqual(harness.takeIpc(), []);
        } finally {
            harness.close();
        }
        assert.throws(
            () => harness.eval('1'),
            (error: unknown) => error instanceof HarnessNativeError && error.code === 'window_closed',
        );
    },
);
