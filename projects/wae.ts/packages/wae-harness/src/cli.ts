#!/usr/bin/env node
/**
 * `wae-harness`: drive a WAE app through its loopback control channel.
 *
 * The CLI allocates a loopback port and a one-shot token, starts the app with
 * `WAE_HARNESS_PORT` / `WAE_HARNESS_TOKEN`, then replays a JSON operations file
 * (`ready` / `eval` / `waitFor` / `screenshot` / `close`). It never resolves or
 * executes the script text itself; the app's harness does.
 */
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { createCli, type ParsedOptions } from '@wae/commander';
import { runHarness, WaeHarnessError } from './driver.js';

function localesRoot(): string {
    return path.join(fileURLToPath(new URL('..', import.meta.url)), 'locales');
}

/** Build the harness CLI (tests may call this directly). */
export function buildCli() {
    const cli = createCli('wae-harness').locales(localesRoot()).intro('cli.intro');
    cli.command('run', 'cli.cmd.run')
        .option('--app <path>', 'cli.opt.app', { required: true })
        .option('--arg <value>...', 'cli.opt.arg')
        .option('--script <file>', 'cli.opt.script', { required: true })
        .option('--timeout <ms>', 'cli.opt.timeout', { coerce: (raw: string | boolean | string[]) => positiveInt(String(raw), '--timeout') })
        .option('--token <value>', 'cli.opt.token')
        .actionCtx(async (options) => cmdRun(options));
    return cli;
}

async function cmdRun(options: ParsedOptions): Promise<number> {
    const app = typeof options.app === 'string' ? options.app : '';
    const script = typeof options.script === 'string' ? options.script : '';
    if (app.length === 0 || script.length === 0) {
        console.error('wae-harness requires --app <path> and --script <file>');
        return 2;
    }
    const rawArgs = options.arg;
    const args = Array.isArray(rawArgs) ? rawArgs.filter((value): value is string => typeof value === 'string') : [];
    const timeoutMs = typeof options.timeout === 'number' && options.timeout > 0 ? options.timeout : 15_000;
    const token = typeof options.token === 'string' && options.token.length > 0 ? options.token : randomUUID();
    try {
        const executed = await runHarness({ app, args, script, token, timeoutMs });
        console.log(`harness finished: ${String(executed)} operation(s)`);
        return 0;
    } catch (error) {
        if (error instanceof WaeHarnessError) {
            console.error(`${error.code}: ${error.message}`);
            return 1;
        }
        console.error(`harness failed: ${String(error)}`);
        return 1;
    }
}

function positiveInt(raw: string, flag: string): number {
    const value = Number(raw);
    if (!Number.isInteger(value) || value <= 0) {
        throw new Error(`${flag} must be a positive integer`);
    }
    return value;
}

async function main(): Promise<void> {
    process.exitCode = await buildCli().parse(process.argv);
}

const invoked = process.argv[1] ? path.resolve(process.argv[1]) : '';
if (invoked === fileURLToPath(import.meta.url)) {
    void main();
}
