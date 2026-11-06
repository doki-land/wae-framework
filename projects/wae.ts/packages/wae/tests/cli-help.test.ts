import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCli } from '../src/cli.ts';

test('root help exits 0 and mentions commands', async () => {
    const logs: string[] = [];
    const orig = console.log;
    console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
    try {
        const code = await buildCli().parse(['--help']);
        assert.equal(code, 0);
        const text = logs.join('\n');
        assert.match(text, /new/);
        assert.match(text, /build/);
        assert.match(text, /dist/);
        assert.match(text, /publish/);
        assert.match(text, /native/);
    } finally {
        console.log = orig;
    }
});

test('wae native lists info and audit', async () => {
    const logs: string[] = [];
    const orig = console.log;
    console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
    try {
        const code = await buildCli().parse(['native', '--help']);
        assert.equal(code, 0);
        const text = logs.join('\n');
        assert.match(text, /info/);
        assert.match(text, /audit/);
    } finally {
        console.log = orig;
    }
});

test('wae native audit rejects an unknown actor kind with exit 2', async () => {
    const errs: string[] = [];
    const orig = console.error;
    const origLog = console.log;
    console.error = (...a: unknown[]) => errs.push(a.map(String).join(' '));
    console.log = () => {};
    try {
        const code = await buildCli().parse(['native', 'audit', '--kind', 'robot']);
        assert.equal(code, 2);
        assert.match(errs.join('\n'), /kind/);
    } finally {
        console.error = orig;
        console.log = origLog;
    }
});

test('unknown command exits 2', async () => {
    const errs: string[] = [];
    const orig = console.error;
    console.error = (...a: unknown[]) => errs.push(a.map(String).join(' '));
    try {
        const code = await buildCli().parse(['nope']);
        assert.equal(code, 2);
    } finally {
        console.error = orig;
    }
});
