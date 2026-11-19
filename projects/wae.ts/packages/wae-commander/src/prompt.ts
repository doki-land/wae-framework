/**
 * `ctx.prompt` engine for `@wae/commander` — pure Node (readline + raw-mode
 * TTY), no third-party prompt dependency.
 *
 * Frozen contract (dual-host plan §5/§11.4):
 * - Questions and validation feedback go to stderr; stdout stays data-only.
 * - One active prompt per context (`commander::interaction_busy`).
 * - EOF or Ctrl+C after a prompt started → `commander::prompt_canceled` (exit 4).
 *   Only the readline `SIGINT` event is handled — the host entry owns process signals.
 * - `deadlineMs` is absolute for the whole request; validation retries never reset it.
 * - `select` returns the stable `value`, never an array index.
 * - `password` requires `setRawMode`; without it the prompt fails as
 *   `commander::password_unsupported` (exit 3) and never degrades to plain input.
 *   Echo/raw mode is restored on every exit path (success / EOF / Ctrl+C / timeout).
 * - Non-interactive `awaitAction` throws `CliError.actionRequired` (exit 5) so the
 *   caller resumes in another run; it never hangs and never claims completion.
 */

import { createInterface } from 'node:readline';
import { Writable } from 'node:stream';

import type {
    ActionRequiredDescriptor,
    AwaitActionOutcome,
    AwaitActionRequest,
    ConfirmRequest,
    InputRequest,
    PasswordRequest,
    PromptStream,
    SelectRequest,
} from './types.js';
import { CliError, SecretInput } from './types.js';

/** Everything the engine needs from the owning command context. */
export type PromptEngineOptions = {
    stdin: PromptStream;
    /** Localized human output channel (stderr). Never stdout. */
    writeErr: (text: string) => void;
    t: (id: string, args?: Record<string, string>) => string;
    /** Live eligibility: false under CI / `--json` / `--no-interactive` / no TTY. */
    interactive: () => boolean;
    /** True once the command consumed stdin as a data source (`claimStdin`). */
    stdinClaimed: () => boolean;
};

const CANCELED = () => CliError.canceledError('commander::prompt_canceled');
const TIMEOUT = () => CliError.canceledError('commander::prompt_timeout');
const UNAVAILABLE = () => CliError.capabilityError('commander::interactive_unavailable');
const BUSY = () => CliError.capabilityError('commander::interaction_busy');

/** Resolve a chrome id, treating `{{id}}` (unresolved) as absent. */
function hint(t: PromptEngineOptions['t'], id: string, args?: Record<string, string>): string {
    const value = t(id, args);
    return value === `{{${id}}}` ? '' : value;
}

export class PromptEngine {
    private readonly errStream: Writable;
    private readonly opts: PromptEngineOptions;
    private busy = false;

    constructor(opts: PromptEngineOptions) {
        this.opts = opts;
        this.errStream = new Writable({
            write: (chunk: Buffer, _enc, cb) => {
                this.opts.writeErr(chunk.toString());
                cb();
            },
        });
    }

    /** Re-check eligibility at acquire time and take the single-active-prompt lease. */
    private acquire(): void {
        if (!this.opts.interactive() || this.opts.stdinClaimed()) throw UNAVAILABLE();
        if (this.busy) throw BUSY();
        this.busy = true;
    }

    /** Read one line from stdin with the question on stderr. */
    private askLine(question: string, until: number | undefined): Promise<string> {
        return new Promise<string>((resolve, reject) => {
            const rl = createInterface({ input: this.opts.stdin, output: this.errStream, terminal: true });
            let settled = false;
            let timer: ReturnType<typeof setTimeout> | undefined;
            const cleanup = (): void => {
                if (timer !== undefined) clearTimeout(timer);
                rl.removeAllListeners();
                rl.close();
            };
            const fail = (err: CliError): void => {
                if (settled) return;
                settled = true;
                cleanup();
                reject(err);
            };
            const done = (line: string): void => {
                if (settled) return;
                settled = true;
                cleanup();
                resolve(line);
            };
            rl.once('line', done);
            rl.once('close', () => fail(CANCELED()));
            rl.on('SIGINT', () => fail(CANCELED()));
            if (until !== undefined) {
                const remaining = until - Date.now();
                if (remaining <= 0) {
                    fail(TIMEOUT());
                    return;
                }
                timer = setTimeout(() => fail(TIMEOUT()), remaining);
            }
            this.opts.writeErr(question);
        });
    }

    /** Read one secret line with echo off. Raw mode is restored on every path. */
    private readSecretLine(until: number | undefined): Promise<string> {
        const stdin = this.opts.stdin;
        const setRaw = stdin.setRawMode;
        if (typeof setRaw !== 'function') {
            return Promise.reject(CliError.capabilityError('commander::password_unsupported'));
        }
        return new Promise<string>((resolve, reject) => {
            let buffer = '';
            let settled = false;
            let timer: ReturnType<typeof setTimeout> | undefined;
            const cleanup = (): void => {
                if (timer !== undefined) clearTimeout(timer);
                stdin.off('data', onData);
                stdin.off('end', onEnd);
                stdin.off('close', onEnd);
                try {
                    setRaw.call(stdin, false);
                } catch {
                    // Terminal already gone — nothing left to restore.
                }
                stdin.pause();
            };
            const fail = (err: CliError): void => {
                if (settled) return;
                settled = true;
                cleanup();
                reject(err);
            };
            const done = (): void => {
                if (settled) return;
                settled = true;
                cleanup();
                this.opts.writeErr('\n');
                resolve(buffer);
            };
            const onData = (chunk: Buffer | string): void => {
                for (const ch of chunk.toString('utf8')) {
                    if (ch === '\u0003' || ch === '\u0004') return fail(CANCELED());
                    if (ch === '\r' || ch === '\n') return done();
                    if (ch === '\b' || ch === '\u007f') {
                        buffer = buffer.slice(0, -1);
                        continue;
                    }
                    if (ch === '\u001b') continue;
                    buffer += ch;
                }
            };
            const onEnd = (): void => fail(CANCELED());
            try {
                setRaw.call(stdin, true);
            } catch (err) {
                reject(CliError.capabilityError('commander::password_unsupported', { detail: err instanceof Error ? err.message : String(err) }));
                return;
            }
            stdin.on('data', onData);
            stdin.on('end', onEnd);
            stdin.on('close', onEnd);
            stdin.resume();
            if (until !== undefined) {
                const remaining = until - Date.now();
                if (remaining <= 0) {
                    fail(TIMEOUT());
                    return;
                }
                timer = setTimeout(() => fail(TIMEOUT()), remaining);
            }
        });
    }

    /** Absolute deadline anchor for one request (retries share it). */
    private static deadline(deadlineMs: number | undefined): number | undefined {
        return deadlineMs === undefined ? undefined : Date.now() + deadlineMs;
    }

    async confirm(req: ConfirmRequest): Promise<boolean> {
        this.acquire();
        try {
            const until = PromptEngine.deadline(req.deadlineMs);
            const choices = hint(this.opts.t, 'commander.prompt.confirm_choices');
            const question = choices ? `${this.opts.t(req.key)} ${choices}` : this.opts.t(req.key);
            for (;;) {
                const line = (await this.askLine(question, until)).trim().toLowerCase();
                if (line === 'y' || line === 'yes') return true;
                if (line === 'n' || line === 'no') return false;
                if (line === '' && req.default !== undefined) return req.default;
                // Anything else (including empty without a declared default) re-prompts.
            }
        } finally {
            this.busy = false;
        }
    }

    async input(req: InputRequest): Promise<string> {
        this.acquire();
        try {
            const until = PromptEngine.deadline(req.deadlineMs);
            const question = this.opts.t(req.key);
            for (;;) {
                const value = await this.askLine(question, until);
                const problem = req.validate ? req.validate(value) : null;
                if (!problem) return value;
                const message = hint(this.opts.t, problem.code, problem.args) || problem.code;
                this.opts.writeErr(message);
            }
        } finally {
            this.busy = false;
        }
    }

    async select<T = string>(req: SelectRequest<T>): Promise<T> {
        this.acquire();
        try {
            const until = PromptEngine.deadline(req.deadlineMs);
            const lines = [this.opts.t(req.key)];
            req.choices.forEach((choice, index) => {
                lines.push(`  ${index + 1}) ${this.opts.t(choice.labelKey)}`);
            });
            const question = `${lines.join('\n')}\n`;
            for (;;) {
                const line = (await this.askLine(question, until)).trim();
                const picked = Number(line);
                if (Number.isInteger(picked) && picked >= 1 && picked <= req.choices.length) {
                    return req.choices[picked - 1]!.value;
                }
                const message = hint(this.opts.t, 'commander.prompt.select_invalid', { value: line }) || line;
                this.opts.writeErr(message);
            }
        } finally {
            this.busy = false;
        }
    }

    async password(req: PasswordRequest): Promise<SecretInput> {
        this.acquire();
        try {
            const until = PromptEngine.deadline(req.deadlineMs);
            this.opts.writeErr(this.opts.t(req.key));
            const value = await this.readSecretLine(until);
            return new SecretInput(value);
        } finally {
            this.busy = false;
        }
    }

    async awaitAction(req: AwaitActionRequest): Promise<AwaitActionOutcome> {
        if (!this.opts.interactive()) {
            const descriptor: ActionRequiredDescriptor = { actionId: req.actionId };
            if (req.statusRef !== undefined) descriptor.statusRef = req.statusRef;
            if (req.expiresAt !== undefined) descriptor.expiresAt = req.expiresAt;
            if (req.nextAction !== undefined) descriptor.nextAction = req.nextAction;
            throw CliError.actionRequired(descriptor);
        }
        this.acquire();
        try {
            const until = PromptEngine.deadline(req.deadlineMs);
            const hintLine = hint(this.opts.t, 'commander.prompt.await_action_hint');
            const body = this.opts.t(req.instructionsKey, req.args);
            const question = hintLine ? `${body}\n${hintLine}\n` : `${body}\n`;
            for (;;) {
                const line = (await this.askLine(question, until)).trim().toLowerCase();
                if (line === '') return 'confirmed';
                if (line === 'd' || line === 'defer') return 'deferred';
            }
        } finally {
            this.busy = false;
        }
    }
}
