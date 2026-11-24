/**
 * Shared types for `@wae/commander` (kept separate so locales.ts can import without cycles).
 */

/**
 * Dual-host exit code contract (identical in the Rust `commander.types` crate).
 *
 * - 0 OK — command completed.
 * - 1 FAILURE — execution or protocol failure (includes domain timeouts).
 * - 2 USAGE — parse / validation failure (unknown option, missing machine input).
 * - 3 CAPABILITY — environment lacks a required capability (interactive prompt,
 *   terminal echo control, concurrent prompt lease).
 * - 4 CANCELED — a started prompt was canceled (EOF, Ctrl+C, explicit abort, prompt timeout).
 * - 5 ACTION_REQUIRED — domain is prepared and needs an external action, then a
 *   separate resume run. Never record this as failure or success.
 */
export const EXIT = {
    OK: 0,
    FAILURE: 1,
    USAGE: 2,
    CAPABILITY: 3,
    CANCELED: 4,
    ACTION_REQUIRED: 5,
} as const;

/** One of the frozen exit codes ({@link EXIT}). */
export type ExitCode = (typeof EXIT)[keyof typeof EXIT];

/**
 * Structured error thrown by `@wae/commander` itself.
 *
 * Exit code contract (POSIX CLI conventions):
 * - 2: parse / validation failure (unknown command, unknown/missing option).
 * - 1: config / factory / action-handler failure.
 * - 3: capability failure (interactive unavailable, busy lease, password unsupported).
 * - 4: canceled prompt (EOF / Ctrl+C / timeout).
 * - 5: domain action required (carries an {@link ActionRequiredDescriptor}).
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
    /** Domain resume descriptor; present only on exit code 5. */
    readonly actionRequired?: ActionRequiredDescriptor;

    private constructor(opts: {
        code: string;
        exitCode: number;
        args?: Record<string, string>;
        cause?: unknown;
        actionRequired?: ActionRequiredDescriptor;
    }) {
        // A SecretInput cause must never leak its value into `message`.
        const safeCauseMessage =
            opts.cause instanceof Error && !(opts.cause instanceof SecretInput) ? opts.cause.message : opts.code;
        super(safeCauseMessage);
        this.name = 'CliError';
        this.code = opts.code;
        this.exitCode = opts.exitCode;
        this.args = opts.args ?? {};
        this.cause = opts.cause;
        if (opts.actionRequired) this.actionRequired = opts.actionRequired;
    }

    /** Parse / validation failures (exit code 2). */
    static parseError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: EXIT.USAGE, args });
    }

    /** Factory / configuration failures (exit code 1). */
    static configError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: EXIT.FAILURE, args });
    }

    /** Capability failures (exit code 3) — environment/terminal cannot serve the request. */
    static capabilityError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: EXIT.CAPABILITY, args });
    }

    /** Canceled prompts (exit code 4) — EOF, Ctrl+C, explicit abort, prompt timeout. */
    static canceledError(code: string, args: Record<string, string> = {}): CliError {
        return new CliError({ code, exitCode: EXIT.CANCELED, args });
    }

    /** Domain paused for an external action (exit code 5) — resume happens in another run. */
    static actionRequired(descriptor: ActionRequiredDescriptor): CliError {
        return new CliError({
            code: 'commander::action_required',
            exitCode: EXIT.ACTION_REQUIRED,
            args: { actionId: descriptor.actionId },
            actionRequired: descriptor,
        });
    }

    /** Wrap an arbitrary exception as a CliError (exit code 1). */
    static from(err: unknown, fallbackCode = 'commander::action_failed'): CliError {
        if (err instanceof CliError) return err;
        // Never stringify a SecretInput into args — the mask glyph is the only safe text.
        const message = err instanceof SecretInput ? '▮' : err instanceof Error ? err.message : String(err);
        // Put both message and detail so product catalogs may use either {message} or {detail}.
        return new CliError({ code: fallbackCode, exitCode: EXIT.FAILURE, cause: err, args: { detail: message, message } });
    }
}

/** Interactive declaration policy for a command or option. */
export type InteractivePolicy = 'optional' | 'required' | 'never';

/**
 * Machine-path declaration attached to a command (`.interactive(spec)`) or an
 * option (`OptionConfig.interactive`). Registration validates that every
 * referenced input key exists and every prompt key is a non-empty id.
 */
export type InteractiveSpec = {
    policy: InteractivePolicy;
    /** Slot name → locale key (`cli.prompt.*` / `commander.prompt.*`). */
    promptKeys: Record<string, string>;
    /**
     * Documented non-interactive input forms. Each entry is an existing option
     * key, `flag:<option-key>`, or `config:<name>`.
     */
    nonInteractiveInputs: string[];
    /** Cross-run continuation: inputs the resume run must accept. */
    continuation?: { resumeInputs: string[] };
};

/** Structured resume instruction returned with exit code 5. */
export type ActionRequiredDescriptor = {
    /** Stable action id the caller submits in the resume run. */
    actionId: string;
    /** Reference (never content) into domain-owned persisted state. */
    statusRef?: string;
    /** RFC3339 deadline after which the external action is no longer valid. */
    expiresAt?: string;
    /** Re-runnable command with literal argv (not a shell string). */
    nextAction?: { command: string; argv: string[] };
};

/**
 * Opaque handle for a secret collected by `ctx.prompt.password`.
 *
 * The value lives only in a private field; `toString()` / `JSON.stringify()`
 * return a mask glyph so accidental logging never prints the secret.
 * JavaScript cannot physically zero string memory — hold a revealed value for
 * the shortest possible lifetime and drop every reference when done.
 */
export class SecretInput {
    #value: string;

    constructor(value: string) {
        this.#value = value;
    }

    /** Return the secret. Callers must keep the result short-lived. */
    reveal(): string {
        return this.#value;
    }

    toString(): string {
        return '▮';
    }

    toJSON(): string {
        return '▮';
    }
}

/**
 * Injectable stdin stream. A real `process.stdin` TTY carries `isTTY` and
 * `setRawMode`; test fakes implement `setRawMode` to exercise the password path.
 */
export type PromptStream = NodeJS.ReadableStream & {
    isTTY?: boolean;
    setRawMode?: (mode: boolean) => unknown;
};

/** Ask the operator a yes/no question. `default` applies only to an empty line in an interactive session. */
export type ConfirmRequest = { key: string; default?: boolean; deadlineMs?: number };

/** Validation error returned by {@link InputRequest.validate} (stable code + safe args). */
export type InputValidationError = { code: string; args?: Record<string, string> } | null | undefined;

/** Ask the operator for a non-secret line of text. */
export type InputRequest = { key: string; validate?: (value: string) => InputValidationError; deadlineMs?: number };

/** One select entry: stable `value` (returned) + localized `labelKey` (displayed). */
export type SelectChoice<T = string> = { value: T; labelKey: string };

/** Ask the operator to pick one choice from a numbered list. */
export type SelectRequest<T = string> = { key: string; choices: SelectChoice<T>[]; deadlineMs?: number };

/** Ask the operator for a secret with terminal echo disabled. */
export type PasswordRequest = { key: string; deadlineMs?: number };

/** Wait for the operator to complete an external action, or defer it to another run. */
export type AwaitActionRequest = {
    actionId: string;
    instructionsKey: string;
    args?: Record<string, string>;
    statusRef?: string;
    expiresAt?: string;
    nextAction?: { command: string; argv: string[] };
    deadlineMs?: number;
};

/** awaitAction outcome (`canceled` is delivered as a thrown exit-4 error). */
export type AwaitActionOutcome = 'confirmed' | 'deferred' | 'canceled';

/** Context-scoped interactive prompt API (`ctx.prompt`). */
export type PromptApi = {
    confirm: (req: ConfirmRequest) => Promise<boolean>;
    input: (req: InputRequest) => Promise<string>;
    select: <T = string>(req: SelectRequest<T>) => Promise<T>;
    password: (req: PasswordRequest) => Promise<SecretInput>;
    awaitAction: (req: AwaitActionRequest) => Promise<AwaitActionOutcome>;
};

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
    /** Interactive declaration for this option's slot (validated at registration). */
    interactive?: InteractiveSpec;
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
    /**
     * Context-scoped interactive prompts. Every call re-checks eligibility at
     * acquire time and takes the single-active-prompt lease for this context.
     */
    prompt: PromptApi;
    /** Eligibility snapshot (`stdin TTY && stderr TTY && !json && !ci && !noInteractive`). */
    isInteractive: () => boolean;
    /**
     * Declare that this command consumes stdin as a data source. Afterwards
     * every `ctx.prompt.*` call fails with `commander::interactive_unavailable`.
     */
    claimStdin: () => void;
    /** Whether {@link CommandContext.claimStdin} has been called. */
    stdinClaimed: boolean;
    /**
     * Emit the final result. JSON mode writes exactly one
     * `wae.cli-result/v0` envelope to stdout; human mode writes `human ?? JSON`.
     */
    emitResult: (data: unknown, human?: string) => void;
    /**
     * Emit a typed failure. JSON mode writes one envelope; human mode writes stderr text.
     *
     * `detail` carries the structured, machine-readable parameters of the code (path,
     * capability, platform, ...). JSON mode copies it into `error.detail` so CLI
     * consumers branch on the same typed payload the SDK and GUI receive.
     */
    emitFailure: (error: { code: string; message: string; detail?: Record<string, unknown> }, descriptor?: ActionRequiredDescriptor) => void;
    /** Throw the exit-5 action-required error carrying the resume descriptor. */
    actionRequired: (descriptor: ActionRequiredDescriptor) => never;
};
