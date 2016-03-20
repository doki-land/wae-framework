import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { buildCli } from '../src/cli.ts';

test('wae new scaffolds without gui by default', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-'));
    const name = 'demo-tool';
    const logs: string[] = [];
    const orig = console.log;
    console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
    try {
        const code = await buildCli().parse(['new', name, '--out', tmp]);
        assert.equal(code, 0);
        const dest = path.join(tmp, name);
        assert.ok(fs.existsSync(path.join(dest, 'package.json')));
        const pkg = JSON.parse(fs.readFileSync(path.join(dest, 'package.json'), 'utf8')) as { name: string };
        assert.equal(pkg.name, name);
        assert.equal(fs.existsSync(path.join(dest, 'gui')), false);
    } finally {
        console.log = orig;
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('wae new --with-gui keeps gui folder', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-gui-'));
    const name = 'demo-gui';
    try {
        const code = await buildCli().parse(['new', name, '--out', tmp, '--with-gui']);
        assert.equal(code, 0);
        assert.ok(fs.existsSync(path.join(tmp, name, 'gui')));
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});
