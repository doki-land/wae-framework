import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { loadOps, runHarness, WaeHarnessError } from '../src/driver.ts';

function tempOps(content: unknown): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-harness-'));
    const file = path.join(dir, 'ops.json');
    fs.writeFileSync(file, JSON.stringify(content));
    return file;
}

/** Fake app: a loopback control server that echoes `eval` and exits on `close`. */
const FAKE_APP = `
const net = require('node:net');
const port = Number(process.env.WAE_HARNESS_PORT);
const token = process.env.WAE_HARNESS_TOKEN;
const server = net.createServer((client) => {
    client.setEncoding('utf8');
    let buffer = '';
    client.on('data', (chunk) => {
        buffer += chunk;
        let index = buffer.indexOf('\\n');
        while (index >= 0) {
            const line = buffer.slice(0, index);
            buffer = buffer.slice(index + 1);
            index = buffer.indexOf('\\n');
            if (!line.trim()) continue;
            const request = JSON.parse(line);
            if (request.token !== token) {
                client.write(JSON.stringify({ id: request.id, ok: false, code: 'harness_unauthorized', message: 'bad token' }) + '\\n');
                continue;
            }
            const value = request.op === 'eval' ? request.script : true;
            client.write(JSON.stringify({ id: request.id, ok: true, value }) + '\\n');
            if (request.op === 'close') {
                client.end();
                process.exit(0);
            }
        }
    });
});
server.listen(port, '127.0.0.1');
`;

test('loadOps rejects empty and malformed operation files', () => {
    assert.throws(() => loadOps(tempOps([])), (error: unknown) => error instanceof WaeHarnessError);
    assert.throws(
        () => loadOps(tempOps([{ op: 'eval' }])),
        (error: unknown) => error instanceof WaeHarnessError && error.code === 'harness_script_invalid',
    );
    assert.throws(
        () => loadOps(tempOps([{ op: 'screenshot' }])),
        (error: unknown) => error instanceof WaeHarnessError && error.code === 'harness_script_invalid',
    );
});

test('runHarness drives a control channel end to end', async () => {
    const script = tempOps([
        { op: 'ready' },
        { op: 'eval', script: '1 + 1' },
        { op: 'close' },
    ]);
    const logs: string[] = [];
    const original = console.log;
    console.log = (...args: unknown[]) => logs.push(args.map(String).join(' '));
    try {
        const executed = await runHarness({
            app: process.execPath,
            args: ['-e', FAKE_APP],
            script,
            token: 'test-token',
            timeoutMs: 10_000,
        });
        assert.equal(executed, 3);
        assert.ok(logs.some((line) => line.startsWith('2:eval "1 + 1"')), `unexpected logs: ${logs.join('\n')}`);
    } finally {
        console.log = original;
    }
});

test('runHarness surfaces a control-channel error', async () => {
    const script = tempOps([{ op: 'eval', script: '1' }]);
    const failingApp = `
const net = require('node:net');
const port = Number(process.env.WAE_HARNESS_PORT);
net.createServer((client) => {
    client.setEncoding('utf8');
    client.once('data', (chunk) => {
        const request = JSON.parse(String(chunk).trim());
        client.write(JSON.stringify({ id: request.id, ok: false, code: 'script_error', message: 'boom' }) + '\\n');
    });
}).listen(port, '127.0.0.1');
`;
    await assert.rejects(
        runHarness({ app: process.execPath, args: ['-e', failingApp], script, token: 'test-token', timeoutMs: 2_000 }),
        (error: unknown) => error instanceof WaeHarnessError && error.code === 'script_error',
    );
});

test('runHarness fails fast when the app never opens the channel', async () => {
    const script = tempOps([{ op: 'ready' }]);
    await assert.rejects(
        runHarness({ app: process.execPath, args: ['-e', 'setTimeout(() => {}, 5000)'], script, token: 'x', timeoutMs: 400 }),
        (error: unknown) => error instanceof WaeHarnessError && error.code === 'harness_connect_timeout',
    );
});
