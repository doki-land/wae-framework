/**
 * Stable CLI exit codes and the machine-readable error shape.
 *
 * The N-API boundary throws a real `Error` instance with `code` and `detail`
 * properties. This module normalizes that into `AppError` without inventing a
 * second schema and without parsing message prefixes.
 */

/** Process exit codes. `USAGE` is produced by `@wae/commander` itself. */
export const EXIT = {
    /** Success. */
    OK: 0,
    /** Runtime failure that is not a missing capability. */
    FAILURE: 1,
    /** Parse / validation failure. */
    USAGE: 2,
    /** Structured capability failure (`unsupported_platform`, `not_implemented`, `dependency_blocked`). */
    CAPABILITY: 3,
} as const;

/** Error payload emitted by the native boundary. */
export interface ErrorPayload {
    code: string;
    capability?: string;
    platform?: string;
    message: string;
    /** File or directory related to an I/O failure. */
    path?: string;
}

/** Error thrown by the typed library and the CLI. */
export class AppError extends Error {
    readonly code: string;
    readonly capability?: string;
    readonly platform?: string;
    readonly path?: string;

    constructor(payload: ErrorPayload) {
        super(payload.message);
        this.name = 'AppError';
        this.code = payload.code;
        this.capability = payload.capability;
        this.platform = payload.platform;
        this.path = payload.path;
    }

    /** Exit code derived from the structured payload. */
    get exitCode(): number {
        return this.capability ? EXIT.CAPABILITY : EXIT.FAILURE;
    }

    /** Machine-readable form. */
    toJSON(): ErrorPayload {
        return {
            code: this.code,
            ...(this.capability ? { capability: this.capability } : {}),
            ...(this.platform ? { platform: this.platform } : {}),
            message: this.message,
            ...(this.path ? { path: this.path } : {}),
        };
    }
}

/** Whether the value is already an `AppError`. */
export function isAppError(value: unknown): value is AppError {
    return value instanceof AppError;
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return value !== null && typeof value === 'object';
}

/**
 * Normalize an exception from the native boundary or the filesystem.
 *
 * The native boundary is authoritative: a thrown `Error` carrying a string
 * `code` becomes an `AppError` with the same code, and an object `detail`
 * supplies `capability`, `platform`, `path`, and the canonical `message`.
 * Everything else becomes `native_unavailable` or `internal_error` so agents
 * never have to parse prose.
 */
export function fromNativeError(value: unknown): AppError {
    if (value instanceof AppError) return value;
    const message = value instanceof Error ? value.message : String(value);
    if (value instanceof Error) {
        const native = value as Error & { code?: unknown; detail?: unknown };
        if (typeof native.code === 'string' && native.code !== '') {
            const detail = isRecord(native.detail) ? native.detail : undefined;
            return new AppError({
                code: native.code,
                ...(typeof detail?.capability === 'string' ? { capability: detail.capability } : {}),
                ...(typeof detail?.platform === 'string' ? { platform: detail.platform } : {}),
                ...(typeof detail?.path === 'string' ? { path: detail.path } : {}),
                message: typeof detail?.message === 'string' ? detail.message : message,
            });
        }
    }
    const lower = message.toLowerCase();
    if (lower.includes('cannot find module') || lower.includes('no native binding')) {
        return new AppError({
            code: 'native_unavailable',
            message:
                `${message}\nRun \`pnpm run build:napi\` from the repository root, ` +
                'or install the matching platform package.',
        });
    }
    return new AppError({ code: 'internal_error', message });
}
