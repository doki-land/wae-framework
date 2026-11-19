/**
 * Interactive prompt engine parity: CI/TTY/--json eligibility, confirm/input/
 * select/password behavior (happy path, EOF, timeout), the busy lease, the
 * stdin-claim conflict, and the envelope exit mapping for prompt failures.
 *
 * The engine attaches its readline interface synchronously while `parse()`
 * runs, so answers can be written right after the `parse()` call; between
 * consecutive prompts a short sleep lets the next prompt attach first.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough } from 'node:stream';

import { createCli } from '../src/index.ts';

const missing = { t: (id: string) => `{{${id}}}` };

type FakeStdin = PassThrough & { isTTY: boolean; setRawMode?: (mode: boolean) => FakeStdin };

function makeStdin(): FakeStdin {
    const stdin = new PassThrough() as FakeStdin;
    stdin.isTTY = true;
    return stdin;
}

function rawStdin(calls: boolean[]): FakeStdin {
    const stdin = makeStdin();
    stdin.setRawMode = (mode: boolean) => {
        calls.push(mode);
        return stdin;
    };
    return stdin;
}

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

function interactiveCli(stdin: FakeStdin, out: string[], err: string[], extra: { isTty?: boolean; env?: NodeJS.ProcessEnv } = {}) {
    return createCli('tester', {
        stdin,
        stdout: (text) => out.push(text),
        stderr: (text) => err.push(text),
        isTty: extra.isTty ?? true,
        env: extra.env ?? {},
    }).use(missing);
}

function envelope(out: string[]): Record<string, any> {
    assert.equal(out.length, 1, 'JSON mode must write exactly one envelope');
    return JSON.parse(out[0]!);
}

async function interactiveFlag(isTty: boolean, env: NodeJS.ProcessEnv): Promise<boolean> {
    const out: string[] = [];
    const cli = createCli('tester', {
        stdin: new PassThrough(),
        stdout: (text) => out.push(text),
        stderr: () => {},
        isTty,
        env,
    }).use(missing);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.emitResult({ interactive: ctx.isInteractive() });
        return 0;
    });
    await cli.parse(['go']);
    // Human mode emits the bare JSON text of the result, not the envelope.
    return JSON.parse(out[0]!).interactive === true;
}

test('CI rule: unset, empty, "false" and "0" keep prompts eligible', async () => {
    assert.equal(await interactiveFlag(true, {}), true);
    assert.equal(await interactiveFlag(true, { CI: '' }), true);
    assert.equal(await interactiveFlag(true, { CI: 'false' }), true);
    assert.equal(await interactiveFlag(true, { CI: '0' }), true);
});

test('CI rule: any other CI value disables prompts', async () => {
    assert.equal(await interactiveFlag(true, { CI: 'true' }), false);
    assert.equal(await interactiveFlag(true, { CI: '1' }), false);
    assert.equal(await interactiveFlag(true, { CI: 'GitHub Actions' }), false);
});

test('a non-TTY stream pair disables prompts', async () => {
    assert.equal(await interactiveFlag(false, {}), false);
});

test('confirm reads y and n lines', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        const yes = await ctx.prompt.confirm({ key: 'cli.prompt.go' });
        const no = await ctx.prompt.confirm({ key: 'cli.prompt.go' });
        ctx.emitResult({ yes, no });
        return 0;
    });
    const run = cli.parse(['go']);
    stdin.write('y\n');
    await sleep(10);
    stdin.write('no\n');
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { yes: true, no: false });
});

/** Run one prompt in interactive mode and report the failure code + exit mapping. */
function failingPrompt(stdin: FakeStdin, out: string[], err: string[], prompt: (ctx: import('../src/index.ts').CommandContext) => Promise<unknown>): Promise<number> {
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        try {
            await prompt(ctx);
            ctx.emitResult({ leaked: true });
        } catch (thrown) {
            const e = thrown as { code?: string; exitCode?: number };
            ctx.emitResult({ code: e.code, exit: e.exitCode });
        }
        return 0;
    });
    return cli.parse(['go']);
}

test('confirm EOF cancels with the canceled code and exit 4', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.confirm({ key: 'cli.prompt.go' }));
    stdin.end();
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_canceled', exit: 4 });
});

test('confirm timeout reports the timeout code and exit 4', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.confirm({ key: 'cli.prompt.go', deadlineMs: 30 }));
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_timeout', exit: 4 });
    void stdin;
});

test('--json disables prompts even on a TTY, failing as capability error 3', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        await ctx.prompt.confirm({ key: 'cli.prompt.go' });
        return 0;
    });
    assert.equal(await cli.parse(['--json', 'go']), 3);
    assert.equal(envelope(out).error.code, 'commander::interactive_unavailable');
});

test('input re-asks until validate passes', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        const value = await ctx.prompt.input({
            key: 'cli.prompt.go',
            validate: (s) => (s.length >= 3 ? null : { code: 'cli.err.short' }),
        });
        ctx.emitResult({ value });
        return 0;
    });
    const run = cli.parse(['go']);
    stdin.write('ab\n');
    await sleep(10);
    stdin.write('abc\n');
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { value: 'abc' });
});

test('input EOF cancels with the canceled code and exit 4', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.input({ key: 'cli.prompt.go' }));
    stdin.end();
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_canceled', exit: 4 });
});

test('select returns the choice value, not the index', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        const picked = await ctx.prompt.select({
            key: 'cli.prompt.go',
            choices: [
                { value: 'alpha', labelKey: 'cli.choice.alpha' },
                { value: 'beta', labelKey: 'cli.choice.beta' },
            ],
        });
        ctx.emitResult({ picked });
        return 0;
    });
    const run = cli.parse(['go']);
    stdin.write('9\n');
    await sleep(10);
    stdin.write('2\n');
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { picked: 'beta' });
});

test('select timeout reports the timeout code and exit 4', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.select({
        key: 'cli.prompt.go',
        choices: [{ value: 'alpha', labelKey: 'cli.choice.alpha' }],
        deadlineMs: 30,
    }));
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_timeout', exit: 4 });
    void stdin;
});

test('password reads a secret with echo off and never writes it anywhere', async () => {
    const calls: boolean[] = [];
    const stdin = rawStdin(calls);
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        const secret = await ctx.prompt.password({ key: 'cli.prompt.go' });
        ctx.emitResult({ shown: String(secret) });
        return 0;
    });
    const run = cli.parse(['go']);
    stdin.write('s3cr3t\n');
    assert.equal(await run, 0);
    assert.deepEqual(calls, [true, false]);
    assert.doesNotMatch(out.join('') + err.join(''), /s3cr3t/);
    assert.equal(out.length, 1);
    assert.equal(JSON.parse(out[0]!).shown, '▮');
});

test('password restores echo when the read is canceled', async () => {
    const calls: boolean[] = [];
    const stdin = rawStdin(calls);
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.password({ key: 'cli.prompt.go' }));
    stdin.write('\u0003');
    assert.equal(await run, 0);
    assert.deepEqual(calls, [true, false]);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_canceled', exit: 4 });
});

test('password restores echo on timeout', async () => {
    const calls: boolean[] = [];
    const stdin = rawStdin(calls);
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.password({ key: 'cli.prompt.go', deadlineMs: 30 }));
    assert.equal(await run, 0);
    assert.deepEqual(calls, [true, false]);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::prompt_timeout', exit: 4 });
});

test('password without a raw-mode terminal fails as capability error 3', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const run = failingPrompt(stdin, out, err, (ctx) => ctx.prompt.password({ key: 'cli.prompt.go' }));
    assert.equal(await run, 0);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::password_unsupported', exit: 3 });
});

test('a second concurrent prompt fails with interaction_busy', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        const settled = await Promise.allSettled([
            ctx.prompt.confirm({ key: 'cli.prompt.go' }),
            ctx.prompt.confirm({ key: 'cli.prompt.go' }),
        ]);
        const second = settled[1] as PromiseRejectedResult;
        ctx.emitResult({
            second: second.status,
            code: (second.reason as { code?: string }).code,
        });
        return 0;
    });
    const run = cli.parse(['go']);
    setTimeout(() => stdin.write('y\n'), 5);
    assert.equal(await run, 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { second: 'rejected', code: 'commander::interaction_busy' });
});

test('claiming stdin conflicts with prompts', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        ctx.claimStdin();
        try {
            await ctx.prompt.confirm({ key: 'cli.prompt.go' });
            ctx.emitResult({ leaked: true });
        } catch (err2) {
            ctx.emitResult({ code: (err2 as { code?: string }).code, exit: (err2 as { exitCode?: number }).exitCode });
        }
        return 0;
    });
    assert.equal(await cli.parse(['go']), 0);
    assert.equal(out.length, 1);
    assert.deepEqual(JSON.parse(out[0]!), { code: 'commander::interactive_unavailable', exit: 3 });
});

test('awaitAction under --json fails directly as action-required exit 5', async () => {
    const stdin = makeStdin();
    const out: string[] = [];
    const err: string[] = [];
    const cli = interactiveCli(stdin, out, err);
    cli.command('go', 'cmd.go').actionCtx(async (_options, _args, ctx) => {
        await ctx.prompt.awaitAction({ actionId: 'ext-step', key: 'cli.prompt.wait' });
        return 0;
    });
    assert.equal(await cli.parse(['--json', 'go']), 5);
    const result = envelope(out);
    assert.equal(result.error.code, 'commander::action_required');
    assert.equal(result.actionRequired.actionId, 'ext-step');
});
