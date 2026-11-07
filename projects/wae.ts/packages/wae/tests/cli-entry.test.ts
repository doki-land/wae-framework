import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { entryRelation } from '../src/entry.ts';

const CLI_URL = new URL('../src/cli.ts', import.meta.url);
const SRC_DIR = path.dirname(fileURLToPath(CLI_URL));
const SELF_NAME = (JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { name: string }).name;

test('entry guard follows package-manager links to this module', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-entry-'));
    try {
        const link = path.join(tmp, 'linked');
        fs.symlinkSync(SRC_DIR, link, process.platform === 'win32' ? 'junction' : 'dir');
        assert.equal(entryRelation(CLI_URL.href, path.join(link, 'cli.ts')), 'main');
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('a same-package file that is not this entry is unresolved, never a silent success', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-entry-'));
    try {
        fs.writeFileSync(path.join(tmp, 'package.json'), JSON.stringify({ name: SELF_NAME }));
        const other = path.join(tmp, 'cli.ts');
        fs.writeFileSync(other, '');
        assert.equal(entryRelation(CLI_URL.href, other), 'unresolved');
        assert.equal(entryRelation(CLI_URL.href, path.join(tmp, 'missing', 'cli.ts')), 'unresolved');
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('a same-named CLI from another package is a normal import', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-entry-'));
    try {
        fs.writeFileSync(path.join(tmp, 'package.json'), JSON.stringify({ name: '@other/tool' }));
        const other = path.join(tmp, 'cli.ts');
        fs.writeFileSync(other, '');
        assert.equal(entryRelation(CLI_URL.href, other), 'imported');
        assert.equal(entryRelation(CLI_URL.href, path.join(tmp, 'runner.ts')), 'imported');
        assert.equal(entryRelation(CLI_URL.href, ''), 'imported');
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});
