import assert from 'node:assert/strict';
import test from 'node:test';
import { Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';

import { CLI_RESULT_SCHEMA, CliError, createCli, EXIT } from '../src/index.ts';

const LOCALES_ROOT = fileURLToPath(new URL('./fixtures/locales/', import.meta.url));

function mkStdin(chunks: Array<string | null>): Readable {
    const s = new Readable({ read() {} });
    for (const c of chunks) s.push(c);
    return s;
}

function makeCli(stdin?: Readable, env: NodeJS.ProcessEnv = {}, isTty = true) {
    const out: string[] = [];
    const err: string[] = [];
    const cli = createCli('tester', { stdin, stdout: (s) => out.push(s), stderr: (s) => err.push(s), isTty, env });
    cli.use({ t: (id) => id });
    return { cli, out, err };
}

// ---------------------------------------------------------------------------
// Envelope emission and exit mapping
// ---------------------------------------------------------------------------

test('`--json` emitResult writes exactly one success envelope to stdout', async () => {
    const { cli, out } = makeCli();
    cli.command('act', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.emitResult({ x: 1 });
        ctx.emitResult({ x: 2 }); // the first envelope owns stdout
    });
    const code = await cli.parse(['--json', 'act']);
    assert.equal(code, EXIT.OK);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { schema: CLI_RESULT_SCHEMA, ok: true, data: { x: 1 } });
});

test('`--json` action-required lands in exit 5 with the resume descriptor', async () => {
    const { cli, out } = makeCli(undefined, { CI: 'true' }, false);
    cli.command('wait', 'k').actionCtx(async (_o, _a, ctx) => {
        await ctx.prompt.awaitAction({ instructionsKey: 'i', actionId: 'dns-txt', statusRef: 'acme/chal-1' });
    });
    const code = await cli.parse(['--json', 'wait']);
    assert.equal(code, EXIT.ACTION_REQUIRED);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!) as Record<string, any>;
    assert.equal(envelope.schema, CLI_RESULT_SCHEMA);
    assert.equal(envelope.ok, false);
    assert.equal(envelope.error.code, 'commander::action_required');
    assert.deepEqual(envelope.actionRequired, { actionId: 'dns-txt', statusRef: 'acme/chal-1' });
});

test('`--json` capability failure maps to exit 3 with the envelope', async () => {
    const { cli, out } = makeCli(mkStdin([]));
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['--json', 'ask']);
    assert.equal(code, EXIT.CAPABILITY);
    const envelope = JSON.parse(out[0]!) as Record<string, any>;
    assert.equal(envelope.ok, false);
    assert.equal(envelope.error.code, 'commander::interactive_unavailable');
});

test('`--json` canceled error maps to exit 4 with the envelope', async () => {
    const { cli, out } = makeCli();
    cli.command('ask', 'k').actionCtx(async () => {
        // JSON mode disables prompts, so cancellation reaches the envelope via
        // the machine path (e.g. a host cancel token surfaced by the action).
        throw CliError.canceledError('commander::prompt_canceled');
    });
    const code = await cli.parse(['--json', 'ask']);
    assert.equal(code, EXIT.CANCELED);
    const envelope = JSON.parse(out[0]!) as Record<string, any>;
    assert.equal(envelope.error.code, 'commander::prompt_canceled');
});

test('an unknown exception never leaks its message into the envelope', async () => {
    const { cli, out } = makeCli();
    cli.command('boom', 'k').actionCtx(async () => {
        throw new Error('token=super-secret-value');
    });
    const code = await cli.parse(['--json', 'boom']);
    assert.equal(code, EXIT.FAILURE);
    const envelope = JSON.parse(out[0]!) as Record<string, any>;
    assert.equal(envelope.error.code, 'commander::action_failed');
    assert.ok(!JSON.stringify(envelope).includes('super-secret-value'));
});

test('emitFailure owns the envelope and dispatch does not write a second one', async () => {
    const { cli, out } = makeCli();
    cli.command('fail', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.emitFailure(CliError.actionRequired({ actionId: 'a1' }), { actionId: 'a1' });
        throw CliError.actionRequired({ actionId: 'a1' });
    });
    const code = await cli.parse(['--json', 'fail']);
    assert.equal(code, EXIT.ACTION_REQUIRED);
    assert.equal(out.length, 1);
});

test('human mode prints human text and never an envelope', async () => {
    const { cli, out } = makeCli();
    cli.command('act', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.emitResult({ x: 1 }, 'one certificate issued');
    });
    const code = await cli.parse(['act']);
    assert.equal(code, EXIT.OK);
    assert.deepEqual(out, ['one certificate issued']);
});

test('usage errors keep exit 2 and gain the envelope in json mode', async () => {
    const { cli, out, err } = makeCli();
    cli.command('act', 'k').action(() => 0);
    const code = await cli.parse(['--json', 'act', '--nope']);
    assert.equal(code, EXIT.USAGE);
    assert.equal(out.length, 1);
    const envelope = JSON.parse(out[0]!) as Record<string, any>;
    assert.equal(envelope.error.code, 'commander::unknown_option');
    assert.ok(err.some((line) => line.includes('commander::unknown_option')));
});

// ---------------------------------------------------------------------------
// Interactive declaration validation
// ---------------------------------------------------------------------------

test('policy `required` without a machine input path is a registration error', async () => {
    const { cli } = makeCli();
    const cmd = cli.command('iss', 'k');
    cmd.interactive({ policy: 'required', promptKeys: { dns: 'cli.prompt.dns' } });
    cmd.action(() => 0);
    await assert.rejects(() => cli.parse(['iss']), (err: unknown) => {
        assert.ok(err instanceof Error);
        assert.match(String((err as { code?: string }).code), /interactive_spec_invalid/);
        return true;
    });
});

test('policy `required` with a declared machine path registers cleanly', async () => {
    const { cli } = makeCli();
    const cmd = cli.command('iss', 'k');
    cmd.option('--provider <name>', 'cli.opt.out-dir');
    cmd.interactive({ policy: 'required', promptKeys: { dns: 'cli.cmd.ping' }, nonInteractiveInputs: ['provider'] });
    cmd.action(() => 0);
    const code = await cli.parse(['iss', '--provider', 'manual']);
    assert.equal(code, EXIT.OK);
});

test('unknown machine input reference is a registration error', async () => {
    const { cli } = makeCli();
    const cmd = cli.command('iss', 'k');
    cmd.interactive({ policy: 'optional', promptKeys: { dns: 'cli.prompt.dns' }, nonInteractiveInputs: ['--ghost'] });
    cmd.action(() => 0);
    await assert.rejects(() => cli.parse(['iss']), (err: unknown) => {
        assert.match(String((err as { code?: string }).code), /interactive_spec_invalid/);
        return true;
    });
});

test('prompt keys must exist in the loaded catalog when locales are configured', async () => {
    const { cli } = makeCli();
    cli.locales(LOCALES_ROOT);
    const cmd = cli.command('iss', 'k');
    cmd.interactive({ policy: 'optional', promptKeys: { dns: 'cli.prompt.missing' } });
    cmd.action(() => 0);
    await assert.rejects(() => cli.parse(['iss']), (err: unknown) => {
        assert.match(String((err as { code?: string }).code), /interactive_spec_invalid/);
        return true;
    });
});

test('prompt keys present in the catalog pass the registration gate', async () => {
    const { cli } = makeCli();
    cli.locales(LOCALES_ROOT);
    const cmd = cli.command('iss', 'k');
    cmd.interactive({ policy: 'optional', promptKeys: { dns: 'cli.cmd.ping' } });
    cmd.action(() => 0);
    const code = await cli.parse(['iss']);
    assert.equal(code, EXIT.OK);
});

test('the catalog gate is skipped without configured locales', async () => {
    const { cli } = makeCli();
    const cmd = cli.command('iss', 'k');
    cmd.interactive({ policy: 'optional', promptKeys: { dns: 'cli.prompt.missing' } });
    cmd.action(() => 0);
    const code = await cli.parse(['iss']);
    assert.equal(code, EXIT.OK);
});
