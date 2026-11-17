/**
 * `wae harness`: drive a WAE app through its loopback control channel.
 *
 * The CLI allocates a loopback port and a one-shot token, starts the app with
 * `WAE_HARNESS_PORT` / `WAE_HARNESS_TOKEN`, then replays a JSON operations file
 * (`ready` / `eval` / `waitFor` / `screenshot` / `close`). It never resolves or
 * executes the script text itself; the app's harness does.
 */
import fs from 'node:fs';
import net from 'node:net';
import { spawn, type ChildProcess } from 'node:child_process';

/** One operation from the script file. */
export interface HarnessOp {
    op: string;
    script?: string;
    path?: string;
    timeoutMs?: number;
}

/** CLI inputs for `wae harness`. */
export interface HarnessRunOptions {
    app: string;
    args: string[];
    script: string;
    token: string;
    timeoutMs: number;
}

/** Structured harness failure with a stable code. */
export class WaeHarnessError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = 'WaeHarnessError';
        this.code = code;
    }
}

const CONNECT_RETRY_MS = 50;

/** Read and validate the operations file. */
export function loadOps(file: string): HarnessOp[] {
    let raw: string;
    try {
        raw = fs.readFileSync(file, 'utf8');
    } catch (error) {
        throw new WaeHarnessError('harness_script_unreadable', `cannot read ops file ${file}: ${String(error)}`);
    }
    let parsed: unknown;
    try {
        parsed = JSON.parse(raw);
    } catch (error) {
        throw new WaeHarnessError('harness_script_invalid', `ops file is not JSON: ${String(error)}`);
    }
    if (!Array.isArray(parsed) || parsed.length === 0) {
        throw new WaeHarnessError('harness_script_invalid', 'ops file must be a non-empty array');
    }
    return parsed.map((entry, index) => {
        if (typeof entry !== 'object' || entry === null || typeof (entry as { op?: unknown }).op !== 'string') {
            throw new WaeHarnessError('harness_script_invalid', `op #${String(index)} must carry a string \`op\``);
        }
        const op = entry as HarnessOp;
        if (op.op === 'close' || op.op === 'ready') {
            return op;
        }
        if (op.op === 'screenshot') {
            if (typeof op.path !== 'string' || op.path.length === 0) {
                throw new WaeHarnessError('harness_script_invalid', `op #${String(index)} (screenshot) needs \`path\``);
            }
            return op;
        }
        if (typeof op.script !== 'string' || op.script.length === 0) {
            throw new WaeHarnessError('harness_script_invalid', `op #${String(index)} (${op.op}) needs \`script\``);
        }
        return op;
    });
}

/** Bind a loopback port, read it, and release it for the app to claim. */
export async function allocatePort(): Promise<number> {
    return await new Promise<number>((resolve, reject) => {
        const server = net.createServer();
        server.once('error', reject);
        server.listen(0, '127.0.0.1', () => {
            const address = server.address();
            if (address === null || typeof address === 'string') {
                server.close();
                reject(new WaeHarnessError('harness_port_failed', 'cannot resolve the allocated loopback port'));
                return;
            }
            const port = address.port;
            server.close(() => resolve(port));
        });
    });
}

interface Pending {
    resolve: (value: unknown) => void;
    reject: (error: WaeHarnessError) => void;
}

/** Minimal JSON-lines client for the app control channel. */
class ControlClient {
    private readonly socket: net.Socket;
    private readonly token: string;
    private buffer = '';
    private nextId = 1;
    private readonly pending = new Map<number, Pending>();

    constructor(socket: net.Socket, token: string) {
        this.socket = socket;
        this.token = token;
        socket.setEncoding('utf8');
        socket.on('data', (chunk: string) => this.onData(chunk));
        socket.on('error', (error) => this.failAll(new WaeHarnessError('harness_transport', String(error))));
        socket.on('close', () => this.failAll(new WaeHarnessError('harness_closed', 'control channel closed')));
    }

    static async connect(port: number, token: string, timeoutMs: number): Promise<ControlClient> {
        const deadline = Date.now() + timeoutMs;
        let last: unknown = null;
        while (Date.now() < deadline) {
            try {
                const socket = await new Promise<net.Socket>((resolve, reject) => {
                    const candidate = net.connect({ host: '127.0.0.1', port });
                    candidate.once('connect', () => resolve(candidate));
                    candidate.once('error', (error) => {
                        candidate.destroy();
                        reject(error);
                    });
                });
                return new ControlClient(socket, token);
            } catch (error) {
                last = error;
                await delay(CONNECT_RETRY_MS);
            }
        }
        throw new WaeHarnessError('harness_connect_timeout', `cannot connect to harness on 127.0.0.1:${String(port)}: ${String(last)}`);
    }

    request(op: HarnessOp): Promise<unknown> {
        const id = this.nextId++;
        const payload = { id, token: this.token, ...op };
        return new Promise<unknown>((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.socket.write(`${JSON.stringify(payload)}\n`);
        });
    }

    end(): void {
        this.socket.end();
    }

    private onData(chunk: string): void {
        this.buffer += chunk;
        let newline = this.buffer.indexOf('\n');
        while (newline >= 0) {
            const line = this.buffer.slice(0, newline).trim();
            this.buffer = this.buffer.slice(newline + 1);
            if (line.length > 0) this.onLine(line);
            newline = this.buffer.indexOf('\n');
        }
    }

    private onLine(line: string): void {
        let body: { id?: number; ok?: boolean; value?: unknown; code?: string; message?: string };
        try {
            body = JSON.parse(line) as typeof body;
        } catch (error) {
            this.failAll(new WaeHarnessError('harness_bad_response', `invalid response line: ${String(error)}`));
            return;
        }
        const id = typeof body.id === 'number' ? body.id : 0;
        const pending = this.pending.get(id);
        if (pending === undefined) return;
        this.pending.delete(id);
        if (body.ok === true) {
            pending.resolve(body.value);
        } else {
            pending.reject(new WaeHarnessError(body.code ?? 'harness_failed', body.message ?? 'harness operation failed'));
        }
    }

    private failAll(error: WaeHarnessError): void {
        for (const pending of this.pending.values()) pending.reject(error);
        this.pending.clear();
    }
}

/** Run one harness session and return the number of executed operations. */
export async function runHarness(options: HarnessRunOptions): Promise<number> {
    const ops = loadOps(options.script);
    const port = await allocatePort();
    const child = spawn(options.app, options.args, {
        env: { ...process.env, WAE_HARNESS_PORT: String(port), WAE_HARNESS_TOKEN: options.token },
        stdio: 'inherit',
    });
    const exited = waitForExit(child, options.timeoutMs);
    let executed = 0;
    try {
        const client = await ControlClient.connect(port, options.token, options.timeoutMs);
        try {
            for (const op of ops) {
                const value = await client.request({ ...op, timeoutMs: op.timeoutMs ?? options.timeoutMs });
                executed += 1;
                console.log(`${String(executed)}:${op.op} ${JSON.stringify(value ?? null)}`);
            }
        } finally {
            client.end();
        }
        if (!ops.some((op) => op.op === 'close')) {
            child.kill();
        }
        const clean = await exited;
        if (!clean) {
            throw new WaeHarnessError('harness_exit_timeout', `app did not exit within ${String(options.timeoutMs)}ms`);
        }
        return executed;
    } catch (error) {
        if (child.exitCode === null) child.kill();
        throw error;
    } finally {
        exited.catch(() => undefined);
    }
}

function waitForExit(child: ChildProcess, timeoutMs: number): Promise<boolean> {
    return new Promise<boolean>((resolve) => {
        if (child.exitCode !== null) {
            resolve(true);
            return;
        }
        const timer = setTimeout(() => {
            child.kill();
            resolve(false);
        }, timeoutMs);
        child.once('exit', () => {
            clearTimeout(timer);
            resolve(true);
        });
    });
}

function delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}
