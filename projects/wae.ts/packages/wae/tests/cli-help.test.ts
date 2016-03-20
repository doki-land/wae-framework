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
    } finally {
        console.log = orig;
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
