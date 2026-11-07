import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { isEntryPoint } from '../src/cli.ts';

const SRC_DIR = path.dirname(fileURLToPath(new URL('../src/cli.ts', import.meta.url)));

test('entry guard follows package-manager links to this module', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-entry-'));
    try {
        const link = path.join(tmp, 'linked');
        fs.symlinkSync(SRC_DIR, link, process.platform === 'win32' ? 'junction' : 'dir');
        assert.equal(isEntryPoint(path.join(link, 'cli.ts')), true);
        assert.equal(isEntryPoint(path.join(tmp, 'absent.ts')), false);
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});
