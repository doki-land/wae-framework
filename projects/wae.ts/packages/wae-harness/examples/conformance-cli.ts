/**
 * `commander.conformance/v0` fixture CLI (TS engine side).
 *
 * One command per prompt primitive plus engine-level cases (stdin conflict,
 * machine paths, deferred actions). The harness driver runs the same fixture
 * directory against this CLI and the Rust twin, so behavior asserted here is
 * contract, not convenience. Secrets never leave the `SecretInput` handle.
 */
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { CliError, createCli, EXIT, type CreateCliOptions, type ParsedOptions } from '@wae/commander';

const here = path.dirname(fileURLToPath(import.meta.url));
const localesRoot = path.join(here, 'locales');

/** Stable action id used by the await-action fixtures. */
const DEMO_ACTION_ID = 'conf-dns01-1';

function str(options: ParsedOptions, key: string): string | undefined {
    const value = options[key];
    return typeof value === 'string' ? value : undefined;
}

function deadlineMs(options: ParsedOptions): number | undefined {
    const raw = str(options, 'deadline');
    return raw === undefined ? undefined : Number(raw);
}

export function buildConformanceCli(runtime: CreateCliOptions) {
    const cli = createCli('conformance-cli', runtime)
        .locales(localesRoot)
        .intro('conf.intro');

    cli.command('confirm', 'conf.cmd.confirm')
        .option('--value <y-or-n>', 'conf.opt.value')
        .option('--default <yes-or-no>', 'conf.opt.default')
        .option('--deadline <ms>', 'conf.opt.deadline')
        .interactive({
            policy: 'optional',
            promptKeys: { confirm: 'conf.prompt.confirm' },
            nonInteractiveInputs: ['value'],
        })
        .actionCtx(async (options, _args, ctx) => {
            const machine = str(options, 'value');
            if (machine !== undefined) {
                ctx.emitResult({ confirmed: machine === 'yes', source: 'machine' }, `confirmed=${String(machine === 'yes')} source=machine`);
                return EXIT.OK;
            }
            const confirmed = await ctx.prompt.confirm({ key: 'conf.prompt.confirm', default: str(options, 'default') === 'yes', deadlineMs: deadlineMs(options) });
            ctx.emitResult({ confirmed }, `confirmed=${String(confirmed)}`);
            return EXIT.OK;
        });

    cli.command('input', 'conf.cmd.input')
        .option('--value <text>', 'conf.opt.value')
        .option('--deadline <ms>', 'conf.opt.deadline')
        .interactive({
            policy: 'optional',
            promptKeys: { input: 'conf.prompt.input' },
            nonInteractiveInputs: ['value'],
        })
        .actionCtx(async (options, _args, ctx) => {
            const machine = str(options, 'value');
            if (machine !== undefined) {
                ctx.emitResult({ value: machine, source: 'machine' }, machine);
                return EXIT.OK;
            }
            const value = await ctx.prompt.input({
                key: 'conf.prompt.input',
                deadlineMs: deadlineMs(options),
                validate: (text) => (text.length === 0 ? { code: 'conf.err.empty' } : null),
            });
            ctx.emitResult({ value }, value);
            return EXIT.OK;
        });

    cli.command('select', 'conf.cmd.select')
        .option('--value <choice>', 'conf.opt.value')
        .option('--deadline <ms>', 'conf.opt.deadline')
        .interactive({
            policy: 'optional',
            promptKeys: { select: 'conf.prompt.select' },
            nonInteractiveInputs: ['value'],
        })
        .actionCtx(async (options, _args, ctx) => {
            const choices = [
                { value: 'alpha', labelKey: 'conf.choice.alpha' },
                { value: 'beta', labelKey: 'conf.choice.beta' },
                { value: 'gamma', labelKey: 'conf.choice.gamma' },
            ];
            const machine = str(options, 'value');
            const picked = choices.find((choice) => choice.value === machine);
            if (picked) {
                ctx.emitResult({ value: picked.value, source: 'machine' }, picked.value);
                return EXIT.OK;
            }
            const value = await ctx.prompt.select({ key: 'conf.prompt.select', choices, deadlineMs: deadlineMs(options) });
            ctx.emitResult({ value }, value);
            return EXIT.OK;
        });

    cli.command('password', 'conf.cmd.password')
        .option('--secret-from-env <name>', 'conf.opt.secret-from-env')
        .option('--deadline <ms>', 'conf.opt.deadline')
        .interactive({
            policy: 'optional',
            promptKeys: { password: 'conf.prompt.password' },
            nonInteractiveInputs: ['secret-from-env'],
        })
        .actionCtx(async (options, _args, ctx) => {
            const envName = str(options, 'secret-from-env');
            if (envName !== undefined) {
                const raw = (runtime.env ?? process.env)[envName] ?? '';
                ctx.emitResult({ length: raw.length }, `length=${String(raw.length)}`);
                return EXIT.OK;
            }
            const secret = await ctx.prompt.password({ key: 'conf.prompt.password', deadlineMs: deadlineMs(options) });
            // Only the length is ever reported; the value stays behind the handle.
            ctx.emitResult({ length: secret.reveal().length }, `length=${String(secret.reveal().length)}`);
            return EXIT.OK;
        });

    cli.command('await-action', 'conf.cmd.await-action')
        .option('--action <id>', 'conf.opt.action')
        .option('--deadline <ms>', 'conf.opt.deadline')
        .interactive({
            policy: 'optional',
            promptKeys: { await: 'conf.prompt.await' },
            nonInteractiveInputs: ['action'],
            continuation: { resumeInputs: ['action'] },
        })
        .actionCtx(async (options, _args, ctx) => {
            const resume = str(options, 'action');
            if (resume !== undefined) {
                ctx.emitResult({ resumed: resume }, `resumed=${resume}`);
                return EXIT.OK;
            }
            const descriptor = {
                actionId: DEMO_ACTION_ID,
                statusRef: 'conf/state/1',
                expiresAt: '2026-12-31T00:00:00Z',
                nextAction: { command: 'conformance-cli', argv: ['await-action', '--action', DEMO_ACTION_ID] },
            };
            if (ctx.isInteractive()) {
                const outcome = await ctx.prompt.awaitAction({
                    ...descriptor,
                    instructionsKey: 'conf.prompt.await',
                    args: { records: 'TXT  _acme-challenge.conf.test  demo-value' },
                    deadlineMs: deadlineMs(options),
                });
                if (outcome === 'confirmed') {
                    ctx.emitResult({ resumed: DEMO_ACTION_ID }, `resumed=${DEMO_ACTION_ID}`);
                    return EXIT.OK;
                }
                return ctx.actionRequired(descriptor);
            }
            return ctx.actionRequired(descriptor);
        });

    cli.command('conflict', 'conf.cmd.conflict')
        .interactive({
            policy: 'never',
            promptKeys: { confirm: 'conf.prompt.confirm' },
            nonInteractiveInputs: [],
        })
        .actionCtx(async (_options, _args, ctx) => {
            ctx.claimStdin();
            const confirmed = await ctx.prompt.confirm({ key: 'conf.prompt.confirm' });
            ctx.emitResult({ confirmed }, `confirmed=${String(confirmed)}`);
            return EXIT.OK;
        });

    cli.command('boom', 'conf.cmd.boom')
        .actionCtx(() => {
            throw new Error('arbitrary failure message must never reach the envelope');
        });

    return cli;
}

/** Guard against `CliError` message leakage in human mode too. */
export function isCliError(error: unknown): error is CliError {
    return error instanceof CliError;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
    const cli = buildConformanceCli({});
    process.exitCode = await cli.parse(process.argv);
}
