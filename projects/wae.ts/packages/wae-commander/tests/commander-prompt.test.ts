import assert from 'node:assert/strict';
import test from 'node:test';
import { Readable } from 'node:stream';

import { createCli, EXIT } from '../src/index.ts';

/** Buffered fake stdin — data pushed before the prompt attaches stays queued. */
function mkStdin(chunks: Array<string | null>): Readable {
    const s = new Readable({ read() {} });
    for (const c of chunks) s.push(c);
    return s;
}

type TtyFake = Readable & { isTTY: boolean; setRawMode(mode: boolean): TtyFake; rawCalls: boolean[] };

/** Fake stdin TTY that records every setRawMode call (echo-restore assertions). */
function mkTtyStdin(chunks: Array<string | null>): TtyFake {
    const s = mkStdin(chunks) as TtyFake;
    s.isTTY = true;
    s.rawCalls = [];
    s.setRawMode = (mode: boolean) => {
        s.rawCalls.push(mode);
        return s;
    };
    return s;
}

function makeCli(stdin: Readable | undefined, env: NodeJS.ProcessEnv, isTty = true) {
    const out: string[] = [];
    const err: string[] = [];
    const cli = createCli('tester', { stdin, stdout: (s) => out.push(s), stderr: (s) => err.push(s), isTty, env });
    cli.use({ t: (id) => id });
    return { cli, out, err };
}

// ---------------------------------------------------------------------------
// Eligibility (frozen CI rule + flags + TTY probe)
// ---------------------------------------------------------------------------

test('interactive eligibility follows the frozen CI rule', async () => {
    const cases: Array<[NodeJS.ProcessEnv, boolean]> = [
        [{}, true],
        [{ CI: '' }, true],
        [{ CI: 'false' }, true],
        [{ CI: '0' }, true],
        [{ CI: 'true' }, false],
        [{ CI: '1' }, false],
    ];
    for (const [env, expected] of cases) {
        let seen: boolean | undefined;
        const { cli } = makeCli(mkStdin([]), env);
        cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
            seen = ctx.isInteractive();
        });
        const code = await cli.parse(['ask']);
        assert.equal(code, EXIT.OK, JSON.stringify(env));
        assert.equal(seen, expected, `env ${JSON.stringify(env)}`);
    }
});

test('`--json` disables prompts even on a TTY', async () => {
    let seen: boolean | undefined;
    const { cli } = makeCli(mkStdin([]), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = ctx.isInteractive();
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['--json', 'ask']);
    assert.equal(seen, false);
    assert.equal(code, EXIT.CAPABILITY);
});

test('`--no-interactive` disables prompts even on a TTY', async () => {
    const { cli } = makeCli(mkStdin([]), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['--no-interactive', 'ask']);
    assert.equal(code, EXIT.CAPABILITY);
});

test('no TTY means no prompts', async () => {
    let seen: boolean | undefined;
    const { cli } = makeCli(mkStdin([]), {}, false);
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = ctx.isInteractive();
    });
    await cli.parse(['ask']);
    assert.equal(seen, false);
});

// ---------------------------------------------------------------------------
// confirm
// ---------------------------------------------------------------------------

test('confirm parses y/n/yes/no', async () => {
    const answers: Array<[string, boolean]> = [
        ['y\n', true],
        ['yes\n', true],
        ['n\n', false],
        ['NO\n', false],
    ];
    for (const [input, expected] of answers) {
        let seen: boolean | undefined;
        const { cli } = makeCli(mkStdin([input]), {});
        cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
            seen = await ctx.prompt.confirm({ key: 'q' });
        });
        const code = await cli.parse(['ask']);
        assert.equal(code, EXIT.OK);
        assert.equal(seen, expected, input);
    }
});

test('confirm applies the declared default only on an empty line', async () => {
    let seen: boolean | undefined;
    const { cli } = makeCli(mkStdin(['\n']), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = await ctx.prompt.confirm({ key: 'q', default: false });
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.OK);
    assert.equal(seen, false);
});

test('confirm re-prompts on empty line without a default', async () => {
    let seen: boolean | undefined;
    const { cli } = makeCli(mkStdin(['\n', 'y\n']), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = await ctx.prompt.confirm({ key: 'q' });
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.OK);
    assert.equal(seen, true);
});

test('confirm question goes to stderr, never stdout', async () => {
    const { cli, out, err } = makeCli(mkStdin(['y\n']), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        await ctx.prompt.confirm({ key: 'cli.prompt.q' });
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.OK);
    assert.equal(out.length, 0);
    assert.ok(err.some((line) => line.includes('cli.prompt.q')));
});

test('confirm EOF after the prompt started cancels with exit 4', async () => {
    const { cli, err } = makeCli(mkStdin([null]), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.CANCELED);
    assert.ok(err.some((line) => line.includes('commander::prompt_canceled')));
});

test('Ctrl+C during confirm cancels with exit 4', async () => {
    const { cli } = makeCli(mkStdin(['\u0003']), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.CANCELED);
});

test('confirm deadline cancels with exit 4', async () => {
    const { cli, err } = makeCli(mkStdin([]), {});
    cli.command('ask', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q', deadlineMs: 30 })));
    });
    const code = await cli.parse(['ask']);
    assert.equal(code, EXIT.CANCELED);
    assert.ok(err.some((line) => line.includes('commander::prompt_timeout')));
});

// ---------------------------------------------------------------------------
// input / select
// ---------------------------------------------------------------------------

test('input runs validation and retries without resetting the deadline', async () => {
    let seen: string | undefined;
    const { cli, err } = makeCli(mkStdin(['ab\n', 'abcd\n']), {});
    cli.command('in', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = await ctx.prompt.input({
            key: 'q',
            deadlineMs: 2000,
            validate: (v) => (v.length < 4 ? { code: 'test::too_short' } : null),
        });
    });
    const code = await cli.parse(['in']);
    assert.equal(code, EXIT.OK);
    assert.equal(seen, 'abcd');
    assert.ok(err.some((line) => line.includes('test::too_short')));
});

test('input EOF cancels with exit 4', async () => {
    const { cli } = makeCli(mkStdin([null]), {});
    cli.command('in', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(await ctx.prompt.input({ key: 'q' }));
    });
    const code = await cli.parse(['in']);
    assert.equal(code, EXIT.CANCELED);
});

test('select returns the stable value, never the index', async () => {
    let seen: string | undefined;
    const { cli } = makeCli(mkStdin(['2\n']), {});
    cli.command('pick', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = await ctx.prompt.select({
            key: 'q',
            choices: [
                { value: 'manual', labelKey: 'cli.prompt.manual' },
                { value: 'cloudflare', labelKey: 'cli.prompt.cloudflare' },
            ],
        });
    });
    const code = await cli.parse(['pick']);
    assert.equal(code, EXIT.OK);
    assert.equal(seen, 'cloudflare');
});

test('select re-prompts on an out-of-range choice', async () => {
    let seen: string | undefined;
    const { cli, err } = makeCli(mkStdin(['9\n', '1\n']), {});
    cli.command('pick', 'k').actionCtx(async (_o, _a, ctx) => {
        seen = await ctx.prompt.select({
            key: 'q',
            choices: [
                { value: 'manual', labelKey: 'cli.prompt.manual' },
                { value: 'cloudflare', labelKey: 'cli.prompt.cloudflare' },
            ],
        });
    });
    const code = await cli.parse(['pick']);
    assert.equal(code, EXIT.OK);
    assert.equal(seen, 'manual');
    assert.ok(err.some((line) => line.includes('commander.prompt.select_invalid')));
});

// ---------------------------------------------------------------------------
// password
// ---------------------------------------------------------------------------

test('password reads with echo off and restores raw mode on success', async () => {
    const stream = mkTtyStdin(['topsecret\n']);
    const { cli, out, err } = makeCli(stream, {});
    let secret = '';
    cli.command('pw', 'k').actionCtx(async (_o, _a, ctx) => {
        const value = await ctx.prompt.password({ key: 'q' });
        secret = value.reveal();
        ctx.stdout(JSON.stringify({ leaked: value }));
    });
    const code = await cli.parse(['pw']);
    assert.equal(code, EXIT.OK);
    assert.equal(secret, 'topsecret');
    assert.deepEqual(stream.rawCalls, [true, false]);
    assert.ok(!out.join('').includes('topsecret'));
    assert.ok(!err.join('').includes('topsecret'));
    assert.ok(out.join('').includes('"▮"'));
});

test('password restores raw mode on the timeout path', async () => {
    const stream = mkTtyStdin([]);
    const { cli } = makeCli(stream, {});
    cli.command('pw', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.password({ key: 'q', deadlineMs: 30 })));
    });
    const code = await cli.parse(['pw']);
    assert.equal(code, EXIT.CANCELED);
    assert.equal(stream.rawCalls[0], true);
    assert.equal(stream.rawCalls.at(-1), false);
});

test('password restores raw mode on Ctrl+C', async () => {
    const stream = mkTtyStdin(['\u0003']);
    const { cli } = makeCli(stream, {});
    cli.command('pw', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.password({ key: 'q' })));
    });
    const code = await cli.parse(['pw']);
    assert.equal(code, EXIT.CANCELED);
    assert.deepEqual(stream.rawCalls, [true, false]);
});

test('password without setRawMode fails as capability, never plain input', async () => {
    const { cli, err } = makeCli(mkStdin(['x\n']), {});
    cli.command('pw', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(String(await ctx.prompt.password({ key: 'q' })));
    });
    const code = await cli.parse(['pw']);
    assert.equal(code, EXIT.CAPABILITY);
    assert.ok(err.some((line) => line.includes('commander::password_unsupported')));
});

// ---------------------------------------------------------------------------
// Lease, stdin conflict and awaitAction
// ---------------------------------------------------------------------------

test('a second concurrent prompt fails with interaction_busy', async () => {
    const { cli, err } = makeCli(mkStdin(['y\n', 'y\n']), {});
    cli.command('busy', 'k').actionCtx(async (_o, _a, ctx) => {
        await Promise.all([ctx.prompt.confirm({ key: 'q1' }), ctx.prompt.confirm({ key: 'q2' })]);
    });
    const code = await cli.parse(['busy']);
    assert.equal(code, EXIT.CAPABILITY);
    assert.ok(err.some((line) => line.includes('commander::interaction_busy')));
});

test('prompts after claimStdin fail with interactive_unavailable', async () => {
    const { cli, err } = makeCli(mkStdin(['y\n']), {});
    cli.command('claim', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.claimStdin();
        ctx.stdout(String(await ctx.prompt.confirm({ key: 'q' })));
    });
    const code = await cli.parse(['claim']);
    assert.equal(code, EXIT.CAPABILITY);
    assert.ok(err.some((line) => line.includes('commander::interactive_unavailable')));
});

test('awaitAction returns confirmed on Enter and deferred on d', async () => {
    const seen: string[] = [];
    for (const input of ['\n', 'd\n']) {
        const { cli } = makeCli(mkStdin([input]), {});
        cli.command('wait', 'k').actionCtx(async (_o, _a, ctx) => {
            seen.push(await ctx.prompt.awaitAction({ instructionsKey: 'cli.prompt.dns', actionId: 'dns-txt' }));
        });
        const code = await cli.parse(['wait']);
        assert.equal(code, EXIT.OK);
    }
    assert.deepEqual(seen, ['confirmed', 'deferred']);
});

test('awaitAction EOF cancels with exit 4', async () => {
    const { cli } = makeCli(mkStdin([null]), {});
    cli.command('wait', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(await ctx.prompt.awaitAction({ instructionsKey: 'i', actionId: 'a1' }));
    });
    const code = await cli.parse(['wait']);
    assert.equal(code, EXIT.CANCELED);
});

test('awaitAction in a non-interactive run throws action-required (exit 5)', async () => {
    const { cli, err } = makeCli(undefined, { CI: 'true' }, false);
    cli.command('wait', 'k').actionCtx(async (_o, _a, ctx) => {
        ctx.stdout(await ctx.prompt.awaitAction({ instructionsKey: 'i', actionId: 'dns-txt' }));
    });
    const code = await cli.parse(['wait']);
    assert.equal(code, EXIT.ACTION_REQUIRED);
    assert.ok(err.some((line) => line.includes('commander::action_required')));
});
