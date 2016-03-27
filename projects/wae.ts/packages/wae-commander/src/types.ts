/**
 * Shared types for `@wae/commander` (kept separate so locales.ts can import without cycles).
 */

/**
 * Structured error thrown by `@wae/commander` itself.
 *
 * Exit code contract (POSIX CLI conventions):
 * - 2: parse / validation failure (unknown command, unknown/missing option).
 * - 1: config / factory / action-handler failure.
 */
export class CliError extends Error {
    /** Stable code (`commander::*`) consumed by callers; also doubles as catalog id. */
    readonly code: string;
    /** Exit code a framework user should return for this failure. */
    readonly exitCode: number;
    /** Structured catalog args (`{ name: value }`) for message interpolation. */
    readonly args: Record<string, string>;
    /** Optional root cause. */
    readonly cause?: unknown;

    private constructor(opts: { code: string; exitCode: number; args?: Record<string, string>; cause?: unknown }) {
        super(opts.cause instanceof Error ? opts.cause.message : opts.code);
        this.name = 'CliError';
        this.code = opts.code;
        this.exitCode = opts.exitCode;
        this.args = opts.args ?? {};
        this.cause = opts.cause;
    }

    /** Parse / validation failures (exit code 2). */
    static parseError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: 2, args });
    }

    /** Factory / configuration failures (exit code 1). */
    static configError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: 1, args });
    }

    /** Wrap an arbitrary exception as a CliError (exit code 1). */
    static from(err: unknown, fallbackCode = 'commander::action_failed'): CliError {
        if (err instanceof CliError) return err;
        const message = err instanceof Error ? err.message : String(err);
        // Put both message and detail so product catalogs may use either {message} or {detail}.
        return new CliError({ code: fallbackCode, exitCode: 1, cause: err, args: { detail: message, message } });
    }
}

/** Message id → template. Owned by the localize plugin / product, not this package. */
export type LocaleCatalog = Record<string, string>;

/** Load messages for one locale (sugar for building a {@link LocalizePlugin}). */
export type CatalogLoader = (locale: string) => LocaleCatalog | Promise<LocaleCatalog>;

/**
 * Pluggable localization. Products and end users supply their own `t` / locale policy.
 * This package never ships official product language packs.
 */
export type LocalizePlugin = {
    resolveLocale?: (ctx: { argv: string[]; env: NodeJS.ProcessEnv }) => string;
    t: (id: string, args?: Record<string, string>) => string;
};

/** Language-neutral diagnostic row accepted at a command boundary. */
export type CommandDiagnostic = {
    severity?: 'error' | 'warning' | 'advice' | string;
    code?: string;
    args?: Record<string, string>;
    path?: string;
    span?: { start: number; end: number };
    message?: string;
};

/** Product-owned sink. Formatting remains in the product diagnostic package. */
export type DiagnosticContext = { locale: string; t: LocalizePlugin['t'] };

/** Product-owned sink. Formatting remains outside commander. */
export type DiagnosticHandler = (diagnostic: CommandDiagnostic, context: DiagnosticContext) => void;

/**
 * Option registration config (additive — `.option(rawName, helpId, config?)`).
 *
 * Existing `parseOptionDef(rawName)` already extracts `<required>` / `[optional]` / `...repeatable`
 * from the raw name string. This config adds runtime constraints that cannot be expressed in the DSL.
 */
export type OptionConfig = {
    /** Default value when the flag is omitted (`options[key] = default`). */
    default?: string | boolean | string[];
    /** Allowed values; strict check at parse time (failure → exit code 2). */
    enum?: string[];
    /** Required; missing at parse time → CliError, exit code 2. */
    required?: boolean;
    /** Custom coercion after parse; failure → CliError.exitCode 2. */
    coerce?: (raw: string | boolean | string[]) => unknown;
    /** Short example shown after the flag in help (e.g. after `--profile <name>`). */
    example?: string;
    /** Option group (help section header). */
    group?: string;
    /** Deprecated — annotated in help; emits a warning when used. */
    deprecated?: boolean | string;
    /** Hidden — omitted from help but still parseable. */
    hidden?: boolean;
};

/**
 * Command action context — constructed by the framework before dispatching to `.actionCtx()`.
 *
 * Provides everything a command handler needs without re-parsing argv or re-discovering
 * product internals. Error paths (`fail` / `warn` / `note`) route through the configured
 * DiagnosticHandler (or stderr fallback) so downstream code never needs ad-hoc `console.error`.
 */
export type CommandContext = {
    /** Full command path (root at `[0]`) — e.g. `['wae', 'new']`. */
    commandPath: string[];
    /** Human-readable command path — e.g. `'wae new'`. */
    commandName: string;
    /** Active locale id — e.g. `'en-US'`. */
    locale: string;
    /** Product `LocalizePlugin.t` — actions can render helpIds / error ids. */
    t: LocalizePlugin['t'];
    /** Print help for the current command (derived from the command tree). */
    help: () => void;
    /** Write to stderr. */
    stderr: (...msgs: string[]) => void;
    /** Write to stdout. */
    stdout: (...msgs: string[]) => void;
    /** Framework fail: routes through onDiagnostic and throws (exit path). */
    fail: (code: string, args?: Record<string, string>) => never;
    /** Framework warn: routes through onDiagnostic (warning, does not exit). */
    warn: (code: string, args?: Record<string, string>) => void;
    /** Framework note: routes through onDiagnostic (advice, does not exit). */
    note: (code: string, args?: Record<string, string>) => void;
};
