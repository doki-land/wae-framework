/**
 * Machine-path envelope contract: JSON mode writes exactly one
 * `wae.cli-result/v0` envelope to stdout, and the exit code maps the failure
 * class (usage 2, capability 3, canceled 4, action-required 5). Arbitrary
 * exception text never enters the envelope.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough } from 'node:stream';

import { createCli } from '../src/index.ts';

/** Plugin whose ids all miss so the commander fallback catalog resolves the text. */
const missing = { t: (id: string) => `{{${id}}}` };

function envelopeCli(out: string[], extra: { isTty?: boolean; env?: NodeJS.ProcessEnv } = {}) {
    return createCli('tester', {
        stdin: new PassThrough(),
        stdout: (text) => out.push(text),
        stderr: () => {},
        isTty: extra.isTty ?? false,
        env: extra.env ?? {},
    }).use(missing);
}

test('JSON mode writes exactly one success envelope even if the action emits twice', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.emitResult({ n: 1 });
        ctx.emitResult({ n: 2 });
        return 0;
    });
    assert.equal(await cli.parse(['--json', 'go']), 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { schema: 'wae.cli-result/v0', ok: true, data: { n: 1 } });
});

test('human mode writes the human text, not the envelope', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.emitResult({ n: 1 }, 'one');
        return 0;
    });
    assert.equal(await cli.parse(['go']), 0);
    assert.deepEqual(out, ['one']);
});

test('emitFailure owns the single envelope when it precedes emitResult', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.emitFailure({ code: 'cli::gone', message: 'gone' });
        ctx.emitResult({ n: 1 });
        return 0;
    });
    assert.equal(await cli.parse(['--json', 'go']), 0);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.equal(envelope.ok, false);
    assert.deepEqual(envelope.error, { code: 'cli::gone', message: 'gone' });
});

test('emitFailure copies structured detail into the envelope error', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    const detail = { code: 'io_error', path: 'C:\\locked', platform: 'windows' };
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.emitFailure({ code: 'io_error', message: 'i/o error at C:\\locked', detail });
        return 1;
    });
    assert.equal(await cli.parse(['--json', 'go']), 1);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.deepEqual(envelope.error, { code: 'io_error', message: 'i/o error at C:\\locked', detail });
});

test('action-required exits 5 and the envelope carries the descriptor block', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    const descriptor = {
        actionId: 'dns-01',
        statusRef: 'state/7',
        expiresAt: '2026-11-20T00:00:00Z',
        nextAction: { command: 'tester', argv: ['issue', 'example.test'] },
    };
    cli.command('issue', 'cmd.issue').actionCtx(async (_options, _args, ctx) => {
        ctx.actionRequired(descriptor);
        return 0;
    });
    assert.equal(await cli.parse(['--json', 'issue', 'example.test']), 5);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.equal(envelope.ok, false);
    assert.equal(envelope.error.code, 'commander::action_required');
    assert.match(envelope.error.message, /dns-01/);
    assert.deepEqual(envelope.actionRequired, descriptor);
});

test('human mode writes no envelope for action-required', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('issue', 'cmd.issue').actionCtx(async (_options, _args, ctx) => {
        ctx.actionRequired({ actionId: 'dns-01' });
        return 0;
    });
    assert.equal(await cli.parse(['issue', 'example.test']), 5);
    assert.deepEqual(out, []);
});

test('unknown command exits 2 with a single failure envelope in JSON mode', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(() => 0);
    assert.equal(await cli.parse(['--json', 'nope']), 2);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.equal(envelope.ok, false);
    assert.equal(envelope.error.code, 'commander::unknown_command');
    assert.equal('actionRequired' in envelope, false);
});

test('unknown option exits 2 with a single failure envelope in JSON mode', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(() => 0);
    assert.equal(await cli.parse(['--json', 'go', '--nope']), 2);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.equal(envelope.error.code, 'commander::unknown_option');
});

test('an arbitrary exception never reaches the envelope', async () => {
    const out: string[] = [];
    const cli = envelopeCli(out);
    cli.command('go', 'cmd.go').actionCtx(async () => {
        throw new Error('token=abc123 leaked');
    });
    assert.equal(await cli.parse(['--json', 'go']), 1);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!);
    assert.equal(envelope.error.code, 'commander::action_failed');
    assert.equal(envelope.error.message, 'commander::action_failed');
    assert.doesNotMatch(out[0]!, /abc123/);
});
