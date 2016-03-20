import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { findCdylib } from '../src/native-stage.ts';

test('findCdylib resolves dll stem with hyphen to underscore', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-cdylib-'));
    try {
        const artifact = path.join(dir, 'open_cleaner_napi.dll');
        fs.writeFileSync(artifact, 'x');
        assert.equal(findCdylib(dir, 'open-cleaner-napi'), artifact);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
