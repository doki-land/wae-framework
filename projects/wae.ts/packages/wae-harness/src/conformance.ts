/**
 * `commander.conformance/v0` fixture driver (TS engine side).
 *
 * Loads the shared fixture directory and runs each case against the
 * `@wae/commander` engine through `examples/conformance-cli.ts`, either
 * in-process (injected streams) or as a spawned pipe process. The Rust twin
 * consumes the same fixture files, so every assertion here is contract.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { Readable } from 'node:stream';
import { fileURLToPath, pathToFileURL } from 'node:url';

import type { CreateCliOptions } from '@wae/commander';

/** Frozen fixture schema id (dual-host conformance contract). */
export const CONFORMANCE_SCHEMA = 'commander.conformance/v0';

/** Default fixture directory, resolved from this module (cwd-independent). */
export const CONFORMANCE_FIXTURES_DIR = fileURLToPath(new URL('../fixtures/commander.conformance/v0', import.meta.url));

/** Ctrl+C delivered as the ETX byte the readline engine observes. */
const ETX = String.fromCharCode(3);

/** Assertions a fixture may make about one run. */
export type FixtureExpect = {
    exitCode?: number;
    /** Subset deep-match against the `wae.cli-result/v0` envelope on stdout. */
    envelope?: Record<string, unknown>;
    stderrContains?: string[];
    stdoutContains?: string[];
    /** Must be absent from stdout (the envelope masking guarantee). */
    stdoutNotContains?: string[];
    /** Must be absent from stdout AND stderr (secrets never echo). */
    secretAbsent?: string[];
    /** Some `setRawMode(true)` happened and the last recorded call was `false`. */
    rawModeRestored?: boolean;
    /** stdout parses as exactly one JSON document with no trailing text. */
    stdoutSingleJson?: boolean;
};

/** One `commander.conformance/v0` case. */
export type ConformanceFixture = {
    schema: string;
    id: string;
    mode: 'inproc' | 'pipe';
    tty?: boolean;
    argv: string[];
    stdin?: string[];
    env?: Record<string, string>;
    /** Keep stdin open after the chunks (timeout and Ctrl+C cases). */
    holdStdin?: boolean;
    /** Deliver a Ctrl+C (ETX) keypress to the held stream mid-prompt. */
    sigint?: boolean;
    expect: FixtureExpect;
};

/** Observed outcome of one fixture run. */
export type FixtureResult = {
    exitCode: number;
    stdout: string;
    stderr: string;
    rawModeCalls?: boolean[];
};

type TtyFake = Readable & { isTTY: boolean; rawCalls: boolean[]; setRawMode(mode: boolean): TtyFake };

/** Fake stdin TTY recording every `setRawMode` call (echo-restore assertions). */
function mkTtyStdin(): TtyFake {
    const stream = new Readable({ read() {} }) as TtyFake;
    stream.isTTY = true;
    stream.rawCalls = [];
    stream.setRawMode = (mode: boolean) => {
        stream.rawCalls.push(mode);
        return stream;
    };
    return stream;
}

/**
 * Hermetic environment: the fixture set asserts English chrome and explicit
 * CI behavior, so inherited locale vars and `CI` never steer a run.
 */
function hermeticEnv(extra: Record<string, string> | undefined): NodeJS.ProcessEnv {
    const env: NodeJS.ProcessEnv = { ...process.env };
    delete env.LOCALE;
    delete env.LANG;
    delete env.LC_ALL;
    delete env.CI;
    return { ...env, ...extra };
}

/** Read every `*.json` fixture in `dir`, validate the schema id, sort by id. */
export function loadFixtures(dir: string): ConformanceFixture[] {
    const fixtures = fs
        .readdirSync(dir)
        .filter((name) => name.endsWith('.json'))
        .sort()
        .map((name) => {
            const fixture = JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8')) as ConformanceFixture;
            if (fixture.schema !== CONFORMANCE_SCHEMA) {
                throw new Error(`fixture ${name}: schema ${String(fixture.schema)} is not ${CONFORMANCE_SCHEMA}`);
            }
            if (typeof fixture.id !== 'string' || fixture.id === '') {
                throw new Error(`fixture ${name}: missing id`);
            }
            if (fixture.mode !== 'inproc' && fixture.mode !== 'pipe') {
                throw new Error(`fixture ${name}: mode must be inproc or pipe`);
            }
            if (!Array.isArray(fixture.argv)) {
                throw new Error(`fixture ${fixture.id}: argv must be an array`);
            }
            return fixture;
        });
    return fixtures.sort((a, b) => a.id.localeCompare(b.id));
}

/** Run one fixture against the TS engine (in-process or spawned pipe). */
export async function runFixture(fixture: ConformanceFixture): Promise<FixtureResult> {
    return fixture.mode === 'pipe' ? runPipe(fixture) : runInproc(fixture);
}

async function runInproc(fixture: ConformanceFixture): Promise<FixtureResult> {
    const cliModuleUrl = new URL('../examples/conformance-cli.ts', import.meta.url).href;
    const { buildConformanceCli } = (await import(cliModuleUrl)) as {
        buildConformanceCli: (runtime: CreateCliOptions) => { parse: (argv: string[]) => Promise<number> };
    };
    const stdin = mkTtyStdin();
    for (const chunk of fixture.stdin ?? []) {
        stdin.push(chunk);
    }
    if (!fixture.holdStdin) {
        stdin.push(null);
    }
    const out: string[] = [];
    const err: string[] = [];
    const cli = buildConformanceCli({
        stdin,
        stdout: (text) => {
            out.push(text);
        },
        stderr: (text) => {
            err.push(text);
        },
        isTty: fixture.tty !== false,
        env: hermeticEnv(fixture.env),
    });
    const pending = cli.parse(fixture.argv);
    if (fixture.sigint) {
        setTimeout(() => {
            try {
                stdin.push(ETX);
            } catch {
                // Stream already ended — the prompt will resolve through EOF.
            }
        }, 50);
    }
    // Watchdog: never let a hung prompt block the suite; EOF cancels cleanly.
    const watchdog = setTimeout(() => {
        try {
            stdin.push(null);
        } catch {
            // Already ended.
        }
    }, 5_000);
    try {
        const exitCode = await pending;
        return { exitCode, stdout: out.join('\n'), stderr: err.join('\n'), rawModeCalls: stdin.rawCalls };
    } finally {
        clearTimeout(watchdog);
    }
}

function runPipe(fixture: ConformanceFixture): Promise<FixtureResult> {
    return new Promise<FixtureResult>((resolve, reject) => {
        const cliPath = fileURLToPath(new URL('../examples/conformance-cli.ts', import.meta.url));
        const resolverPath = fileURLToPath(new URL('../../wae-commander/scripts/resolve-ts-from-js.mjs', import.meta.url));
        const child = spawn(
            process.execPath,
            ['--import', pathToFileURL(resolverPath).href, '--experimental-strip-types', cliPath, ...fixture.argv],
            { env: hermeticEnv(fixture.env), stdio: ['pipe', 'pipe', 'pipe'] },
        );
        const out: Buffer[] = [];
        const err: Buffer[] = [];
        child.stdout.on('data', (chunk: Buffer) => {
            out.push(chunk);
        });
        child.stderr.on('data', (chunk: Buffer) => {
            err.push(chunk);
        });
        const killer = setTimeout(() => {
            child.kill();
        }, 15_000);
        child.on('error', (error) => {
            clearTimeout(killer);
            reject(error);
        });
        child.on('close', (code) => {
            clearTimeout(killer);
            resolve({
                exitCode: code ?? -1,
                stdout: Buffer.concat(out).toString('utf8'),
                stderr: Buffer.concat(err).toString('utf8'),
            });
        });
        for (const chunk of fixture.stdin ?? []) {
            child.stdin.write(chunk);
        }
        child.stdin.end();
    });
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Compare only the keys present in `expected` (contract subset matching). */
function matchSubset(expected: unknown, actual: unknown, where: string): void {
    if (isRecord(expected)) {
        assert.ok(isRecord(actual), `${where}: expected an object, got ${JSON.stringify(actual)}`);
        for (const [key, value] of Object.entries(expected)) {
            assert.ok(Object.prototype.hasOwnProperty.call(actual, key), `${where}.${key}: missing from ${JSON.stringify(actual)}`);
            matchSubset(value, actual[key], `${where}.${key}`);
        }
        return;
    }
    if (Array.isArray(expected)) {
        assert.ok(Array.isArray(actual), `${where}: expected an array`);
        assert.equal(actual.length, expected.length, `${where}: array length`);
        expected.forEach((value, index) => {
            matchSubset(value, actual[index], `${where}[${index}]`);
        });
        return;
    }
    assert.deepEqual(actual, expected, `${where}: value mismatch`);
}

/** Check one fixture run against its declared expectations. */
export function assertFixture(fixture: ConformanceFixture, result: FixtureResult): void {
    const expect = fixture.expect ?? {};
    const note = (label: string): string =>
        `${fixture.id}: ${label}\nexit=${String(result.exitCode)}\nstdout=${JSON.stringify(result.stdout)}\nstderr=${JSON.stringify(result.stderr)}`;
    if (expect.exitCode !== undefined) {
        assert.equal(result.exitCode, expect.exitCode, note('exit code'));
    }
    if (expect.envelope !== undefined) {
        let doc: unknown;
        try {
            doc = JSON.parse(result.stdout.trim());
        } catch {
            assert.fail(note('stdout is not a JSON envelope'));
        }
        matchSubset(expect.envelope, doc, `${fixture.id} envelope`);
    }
    for (const needle of expect.stderrContains ?? []) {
        assert.ok(result.stderr.includes(needle), note(`stderr missing ${JSON.stringify(needle)}`));
    }
    for (const needle of expect.stdoutContains ?? []) {
        assert.ok(result.stdout.includes(needle), note(`stdout missing ${JSON.stringify(needle)}`));
    }
    for (const needle of expect.stdoutNotContains ?? []) {
        assert.ok(!result.stdout.includes(needle), note(`stdout leaked ${JSON.stringify(needle)}`));
    }
    for (const needle of expect.secretAbsent ?? []) {
        assert.ok(!result.stdout.includes(needle) && !result.stderr.includes(needle), note(`secret ${JSON.stringify(needle)} appeared in output`));
    }
    if (expect.rawModeRestored === true) {
        const calls = result.rawModeCalls ?? [];
        assert.ok(calls.length > 0, note('no setRawMode calls recorded'));
        assert.ok(calls.includes(true), note('raw mode was never entered'));
        assert.equal(calls[calls.length - 1], false, note('raw mode was not restored'));
    }
    if (expect.stdoutSingleJson === true) {
        const text = result.stdout.trim();
        let parsed: unknown;
        try {
            parsed = JSON.parse(text);
        } catch {
            assert.fail(note('stdout is not exactly one JSON document'));
        }
        assert.ok(isRecord(parsed) && parsed.schema === 'wae.cli-result/v0', note('stdout JSON is not a single result envelope'));
    }
}