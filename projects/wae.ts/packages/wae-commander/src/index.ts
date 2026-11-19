/**
 * `@wae/commander` — i18n-first TypeScript CLI framework.
 *
 * Command / option second args are message ids. Natural language comes from a
 * pluggable Localize plugin or `.locales(root)`. Framework chrome/err ids are
 * `commander.*` with tiny English fallbacks — no product language packs here.
 */

export type {
    ActionRequiredDescriptor,
    AwaitActionOutcome,
    AwaitActionRequest,
    CatalogLoader,
    CommandContext,
    CommandDiagnostic,
    ConfirmRequest,
    DiagnosticContext,
    DiagnosticHandler,
    ExitCode,
    InputRequest,
    InputValidationError,
    InteractivePolicy,
    InteractiveSpec,
    LocaleCatalog,
    LocalizePlugin,
    OptionConfig,
    PasswordRequest,
    PromptApi,
    SelectChoice,
    SelectRequest,
} from './types.js';
export { CliError, EXIT, SecretInput } from './types.js';

export {
    COMMANDER_FALLBACK_EN_US,
    clearLocalesCache,
    createLocalizeFromLocales,
    flattenCatalog,
    loadCatalog,
    loadLocalesManifest,
    resolveLocale,
    translate,
    translateWithFallback,
} from './locales.js';
export type { CreateLocalizeFromLocalesOptions, LocalesManifest } from './locales.js';

import type {
    ActionRequiredDescriptor,
    CatalogLoader,
    CommandContext,
    CommandDiagnostic,
    ConfirmRequest,
    DiagnosticContext,
    DiagnosticHandler,
    InputRequest,
    InteractiveSpec,
    LocaleCatalog,
    LocalizePlugin,
    OptionConfig,
    PasswordRequest,
    PromptApi,
    SelectRequest,
} from './types.js';
import { CliError, SecretInput } from './types.js';
import { COMMANDER_FALLBACK_EN_US, createLocalizeFromLocales, translate, translateWithFallback } from './locales.js';

type Color = 'bold' | 'dim' | 'cyan' | 'blue' | 'green' | 'yellow' | 'red';
const ANSI: Record<Color, string> = {
    bold: '\u001b[1m',
    dim: '\u001b[2m',
    cyan: '\u001b[36m',
    blue: '\u001b[34m',
    green: '\u001b[32m',
    yellow: '\u001b[33m',
    red: '\u001b[31m',
};
const ANSI_RESET = '\u001b[0m';

/** Keep CI and redirected output plain while making interactive help scannable. */
function colorEnabled(): boolean {
    if (process.env.NO_COLOR !== undefined) return false;
    if (process.env.FORCE_COLOR && process.env.FORCE_COLOR !== '0') return true;

    // Node.js reports `isTTY` as `undefined` in many Windows shells (PowerShell,
    // Git Bash) even when the host (Windows Terminal, ConEmu, mintty) does emit
    // ANSI. `tty.isatty(fd)` is reliable on every platform because it asks the
    // OS directly. On top of that we also check well-known Windows-terminal
    // env vars (`WT_SESSION`, `ConEmuANSI`, `ANSICON`) that legacy clients use
    // to declare ANSI support. A `TERM=dumb` redirect still falls through.
    const tty = tryImportTty();
    if (tty && tty.isatty(process.stdout?.fd ?? 0)) return true;
    if (tty && tty.isatty(process.stderr?.fd ?? 2)) return true;
    if (process.env.WT_SESSION) return true;
    if (process.env.ConEmuANSI === 'ON') return true;
    if (process.env.ANSICON) return true;
    if (process.env.TERM && process.env.TERM !== 'dumb') return true;

    return Boolean(process.stdout?.isTTY || process.stderr?.isTTY);
}

/** Dynamically resolve Node's `node:tty` module — safe in non-Node runners. */
function tryImportTty(): typeof import('node:tty') | undefined {
    try {
        // eslint-disable-next-line @typescript-eslint/no-require-imports
        return require('node:tty');
    } catch {
        return undefined;
    }
}

function paint(value: string, ...colors: Color[]): string {
    return colorEnabled() && colors.length ? `${colors.map((color) => ANSI[color]).join('')}${value}${ANSI_RESET}` : value;
}

function errorText(value: string): string {
    return paint(value, 'bold', 'red');
}

/** One help row: the literal flag/command plus an optional localized suffix (example, deprecation). */
type HelpRow = { rawName: string; helpId: string; suffix?: string };

/**
 * Render one help section (header + padded table).
 *
 * Both the root help and the command-node help share the same 3-line pattern:
 * push section header (`paint(header, 'bold', 'blue')`), then a loop of
 * `  ${paint(label.padEnd(colWidth), color)} ${t(helpId)}`.
 *
 * Returns `lines[]` so callers can spread-append without trailing-empty concerns.
 */
function renderHelpSectionWithHeader(
    t: LocalizePlugin['t'],
    header: string,
    rows: ReadonlyArray<HelpRow>,
    color: Color,
    colWidth = 28,
): string[] {
    if (!rows.length) return [];
    const out: string[] = [paint(header, 'bold', 'blue')];
    for (const r of rows) {
        const label = r.suffix ? `${r.rawName} ${r.suffix}` : r.rawName;
        out.push(`  ${paint(label.padEnd(colWidth), color)} ${t(r.helpId)}`);
    }
    return out;
}

function renderHelpSection(
    t: LocalizePlugin['t'],
    headerId: string,
    rows: ReadonlyArray<HelpRow>,
    color: Color,
    colWidth = 28,
): string[] {
    return renderHelpSectionWithHeader(t, t(headerId), rows, color, colWidth);
}

/** Parse/validation diagnostics that get the framework-level "available options" hint. */
const HINT_CODES = new Set<string>([
    'commander::unknown_command',
    'commander::unknown_option',
    'commander::missing_option_value',
    'commander::parse_failed',
    'commander::missing_required_option',
    'commander::option_not_in_enum',
    'commander::option_coerce_failed',
]);

/** Resolve a catalog id, treating `{{id}}` (unresolved) as absent. */
function localizedOrId(t: LocalizePlugin['t'], id: string, args?: Record<string, string>): string {
    const value = t(id, args);
    return value === `{{${id}}}` ? '' : value;
}

/** Localized suffix appended to one option row in help. */
function optionSuffix(t: LocalizePlugin['t'], def: OptionDef): string {
    const config = def.config;
    if (!config) return '';
    const parts: string[] = [];
    if (config.example) parts.push(t('commander.ui.option_example', { example: config.example }));
    if (config.deprecated) {
        const label = typeof config.deprecated === 'string' && config.deprecated
            ? (localizedOrId(t, config.deprecated) || config.deprecated)
            : t('commander.ui.deprecated');
        if (label) parts.push(label);
    }
    return parts.join(' ');
}

/**
 * Render option help, splitting grouped options into their own sections.
 *
 * Options without `group` keep the default `Options:` section. `hidden` options
 * are omitted from help but remain parseable.
 */
function renderOptionSections(t: LocalizePlugin['t'], options: OptionDef[], color: Color): string[] {
    const visible = options.filter((o) => !o.config?.hidden);
    const ungrouped: OptionDef[] = [];
    const grouped = new Map<string, OptionDef[]>();
    for (const option of visible) {
        const group = option.config?.group;
        if (!group) {
            ungrouped.push(option);
            continue;
        }
        const list = grouped.get(group) ?? [];
        list.push(option);
        grouped.set(group, list);
    }
    const rows = (list: OptionDef[]): HelpRow[] =>
        list.map((o) => ({ rawName: o.rawName, helpId: o.helpId, suffix: optionSuffix(t, o) }));
    const out = renderHelpSection(t, 'commander.ui.options', rows(ungrouped), color);
    for (const [group, list] of grouped) {
        const header = localizedOrId(t, group) || group;
        if (out.length) out.push('');
        out.push(...renderHelpSectionWithHeader(t, header, rows(list), color));
    }
    return out;
}

/** Options bag passed to actions (`_` = positionals). */
export type ParsedOptions = Record<string, string | boolean | string[]> & { _: string[] };

/**
 * Injectable stdin stream. A real `process.stdin` TTY carries `isTTY` and
 * `setRawMode`; test fakes implement `setRawMode` to exercise the password path.
 */
export type PromptStream = NodeJS.ReadableStream & {
    isTTY?: boolean;
    setRawMode?: (mode: boolean) => unknown;
};

/**
 * Process-IO seams for `createCli`. Tests and embedding hosts inject streams
 * instead of touching the real console; omitting everything keeps Node defaults.
 */
export type CreateCliOptions = {
    stdin?: PromptStream;
    stdout?: (text: string) => void;
    stderr?: (text: string) => void;
    /** Overrides the stdin+stderr TTY probe used for interactive eligibility. */
    isTty?: boolean;
    /** Env source for CI detection (default `process.env`). */
    env?: NodeJS.ProcessEnv;
};

/** Final stdout contract: human text or the single JSON envelope. */
export type OutputMode = 'human' | 'json';

/** Per-parse routing/eligibility snapshot. */
type RunEnv = { mode: OutputMode; interactive: boolean; env: NodeJS.ProcessEnv };

/**
 * CI rule (frozen): `CI` unset → false, `''` / `'false'` / `'0'` → false,
 * any other value → true. No other `CI_*` marker is matched.
 */
function ciFromEnv(env: NodeJS.ProcessEnv): boolean {
    const v = env.CI;
    if (v === undefined) return false;
    if (v === '' || v === 'false' || v === '0') return false;
    return true;
}

/** Interactive prompts write to stderr, so probe stdin(0) + stderr(2) only. */
function probeTtyPair(): boolean {
    const tty = tryImportTty();
    if (tty) return tty.isatty(0) && tty.isatty(2);
    return Boolean(process.stdin.isTTY && process.stderr.isTTY);
}

/**
 * Validate one interactive declaration against the owning command's option table.
 * Every input reference must resolve to an option key, `flag:<option-key>` or
 * `config:<name>`; every prompt key must be a non-empty id; `required` policy
 * must carry at least one prompt key.
 */
function validateInteractiveSpec(spec: InteractiveSpec, where: string, optionKeys: Set<string>): void {
    const bad = (detail: string): never => {
        throw CliError.configError('commander::interactive_spec_invalid', { command: where, detail });
    };
    if (!spec || typeof spec !== 'object') bad('spec must be an object');
    if (spec.policy !== 'optional' && spec.policy !== 'required' && spec.policy !== 'never') {
        bad(`unknown policy \`${String(spec.policy)}\``);
    }
    const promptKeys = spec.promptKeys ?? {};
    for (const [slot, key] of Object.entries(promptKeys)) {
        if (typeof key !== 'string' || key.length === 0) bad(`prompt key for slot \`${slot}\` must be a non-empty id`);
    }
    if (spec.policy === 'required' && Object.keys(promptKeys).length === 0) {
        bad('policy `required` needs at least one prompt key');
    }
    const refs = [...(spec.nonInteractiveInputs ?? []), ...(spec.continuation?.resumeInputs ?? [])];
    for (const entry of refs) {
        if (typeof entry !== 'string' || entry.length === 0) bad('input reference must be a non-empty string');
        if (entry.startsWith('flag:')) {
            const name = entry.slice('flag:'.length);
            if (!name || !optionKeys.has(name)) bad(`unknown flag reference \`${entry}\``);
        } else if (entry.startsWith('config:')) {
            if (!entry.slice('config:'.length)) bad('empty config reference');
        } else if (!optionKeys.has(entry)) {
            bad(`unknown input reference \`${entry}\``);
        }
    }
}

/** Command action after argv is parsed (legacy API — no context). */
export type ActionHandler = (options: ParsedOptions, ...args: string[]) => void | number | Promise<void | number>;

/** Command action with full framework context (preferred API — additive, no break). */
export type ActionCtxHandler = (options: ParsedOptions, args: string[], ctx: CommandContext) => void | number | Promise<void | number>;

/** Registered option (structure only). */
export type OptionDef = {
    rawName: string;
    helpId: string;
    /** Canonical key in options bag (e.g. `out-dir`). */
    key: string;
    /** Long/short tokens without leading dashes (e.g. `out-dir`, `o`). */
    aliases: string[];
    /** True when the option takes a value (`<x>` / `[x]`). */
    takesValue: boolean;
    /** True when value is optional (`[x]`): bare flag ⇒ `true`. */
    optionalValue: boolean;
    /** True when the option may repeat (`--dirty <path>...` ⇒ `string[]`). */
    repeatable: boolean;
    // --- Phase 3 additive ---
    /** Runtime constraints (enum / required / default / coerce / deprecated). */
    config?: OptionConfig;
};

/** Registered command node. */
export type CommandDef = {
    rawName: string;
    helpId: string;
    /** Match tokens from `rawName` split on `|`. */
    names: string[];
    options: OptionDef[];
    /** Legacy action handler (no ctx). */
    action?: ActionHandler;
    /** Phase 3 action handler (with ctx — preferred). */
    actionCtx?: ActionCtxHandler;
    children: CommandDef[];
    /** If true, do not parse options; remaining argv is positionals / raw rest. */
    passthrough: boolean;
    /** Interactive declaration (validated when the tree is complete, before dispatch). */
    interactive?: InteractiveSpec;
};

export interface Command {
    option(rawName: string, helpId: string, config?: OptionConfig): this;
    /** Declare this command's interactive contract (validated at build time). */
    interactive(spec: InteractiveSpec): this;
    /** Legacy API — handler receives `(options, ...args)` without ctx. */
    action(handler: ActionHandler): this;
    /** Phase 3 API — handler receives `(options, args[], ctx)` with full command context. */
    actionCtx(handler: ActionCtxHandler): this;
    command(rawName: string, helpId: string): Command;
    /** Leave remaining argv unparsed (for nested domain CLIs). */
    passthrough(): this;
}

/** Machine-readable command node for build tooling. */
export type CliSpecCommand = {
    /** Match tokens from `rawName`, first entry is canonical. */
    names: string[];
    /** Option tokens with leading dashes, excluding root-level options. */
    options: string[];
    children: CliSpecCommand[];
};

/** Machine-readable CLI description produced without parsing argv or rendering help. */
export type CliSpec = {
    name: string;
    /** Root-level option tokens with leading dashes. */
    options: string[];
    commands: CliSpecCommand[];
};

export interface Cli {
    use(plugin: LocalizePlugin): this;
    catalog(loader: CatalogLoader): this;
    /**
     * Load catalogs from a locales directory (`locales.json` + `<locale>/*.json`).
     * Later `.use` overrides. Rebuilds at `parse` so `--locale` / env apply.
     */
    locales(root: string, opts?: { envKeys?: string[] }): this;
    /** Global option (parsed before the command; merged into action options). */
    option(rawName: string, helpId: string): this;
    /**
     * Optional short intro line(s) before derived command/option lists.
     * Do **not** paste full usage here — help is derived from the command tree.
     */
    intro(introId: string): this;
    /**
     * @deprecated Use {@link intro}. Kept as alias so older call sites compile.
     */
    help(helpId: string): this;
    command(rawName: string, helpId: string): Command;
    /** Route structured action failures to a product-owned renderer. */
    onDiagnostic(handler: DiagnosticHandler): this;
    /**
     * Describe the registered command tree as plain data.
     *
     * Tooling reads this instead of running `--help` in a subprocess, so the
     * description cannot drift from the real parser and cannot execute actions.
     */
    describe(): CliSpec;
    parse(argv?: string[]): Promise<number>;
}

class CommandBuilder implements Command {
    readonly def: CommandDef;

    constructor(rawName: string, helpId: string) {
        assertHelpId(helpId, 'command');
        this.def = {
            rawName,
            helpId,
            names: splitNames(rawName),
            options: [],
            children: [],
            passthrough: false,
        };
    }

    option(rawName: string, helpId: string, config?: OptionConfig): this {
        assertHelpId(helpId, 'option');
        const def = parseOptionDef(rawName, helpId);
        if (config) def.config = config;
        this.def.options.push(def);
        return this;
    }

    interactive(spec: InteractiveSpec): this {
        this.def.interactive = spec;
        return this;
    }

    action(handler: ActionHandler): this {
        if (this.def.actionCtx) throw CliError.configError('commander::config::action_conflict', { detail: 'Cannot set both .action() and .actionCtx()' });
        this.def.action = handler;
        return this;
    }

    actionCtx(handler: ActionCtxHandler): this {
        if (this.def.action) throw CliError.configError('commander::config::action_conflict', { detail: 'Cannot set both .action() and .actionCtx()' });
        this.def.actionCtx = handler;
        return this;
    }

    passthrough(): this {
        this.def.passthrough = true;
        return this;
    }

    command(rawName: string, helpId: string): Command {
        const child = new CommandBuilder(rawName, helpId);
        this.def.children.push(child.def);
        return child;
    }
}

class CliBuilder implements Cli {
    readonly name: string;
    readonly runtime: CreateCliOptions;
    private localize: LocalizePlugin | null = null;
    /** When set, `parse` rebuilds localize from this root (argv/env/`--locale`). */
    private localesRoot: string | null = null;
    private localesEnvKeys: string[] | undefined;
    private diagnosticHandler: DiagnosticHandler | undefined;
    private introId: string | null = null;
    private readonly roots: CommandBuilder[] = [];
    private readonly rootOptions: OptionDef[] = [];

    constructor(name: string, runtime: CreateCliOptions = {}) {
        this.name = name;
        this.runtime = runtime;
        // Framework globals — parseable everywhere, hidden from product help.
        const json = parseOptionDef('--json', 'commander.opt.json');
        json.config = { hidden: true };
        const noInteractive = parseOptionDef('--no-interactive', 'commander.opt.no_interactive');
        noInteractive.config = { hidden: true };
        this.rootOptions.push(json, noInteractive);
    }

    /** Final stdout writer (envelope / human data only). */
    private writeOut(text: string): void {
        if (this.runtime.stdout) this.runtime.stdout(text);
        else console.log(text);
    }

    /** Final stderr writer (diagnostics / prompts / help-in-json-mode). */
    private writeErr(text: string): void {
        if (this.runtime.stderr) this.runtime.stderr(text);
        else console.error(text);
    }

    /** Help is chrome: stderr in JSON mode (stdout stays envelope-only). */
    private printHelp(text: string, run: RunEnv): void {
        if (run.mode === 'json') this.writeErr(text);
        else this.writeOut(text);
    }

    /** Output mode + interactive eligibility for one parse run. */
    private computeRun(globalOpts: ParsedOptions): RunEnv {
        const json = globalOpts.json === true;
        const noInteractive = globalOpts['no-interactive'] === true;
        const env = this.runtime.env ?? process.env;
        const tty = this.runtime.isTty ?? probeTtyPair();
        const interactive = Boolean(tty) && !json && !ciFromEnv(env) && !noInteractive;
        return { mode: json ? 'json' : 'human', interactive, env };
    }

    use(plugin: LocalizePlugin): this {
        if (!plugin || typeof plugin.t !== 'function') {
            throw CliError.configError('commander::config::invalid_localize_plugin', { detail: 'LocalizePlugin.t is required' });
        }
        this.localesRoot = null;
        this.localesEnvKeys = undefined;
        this.localize = wrapWithCommanderFallback(plugin);
        return this;
    }

    catalog(loader: CatalogLoader): this {
        if (typeof loader !== 'function') {
            throw CliError.configError('commander::config::catalog_loader_not_function');
        }
        return this.use({
            t: (id, args) => {
                const table = loader('en-US');
                if (table && typeof (table as Promise<LocaleCatalog>).then === 'function') {
                    throw CliError.configError('commander::config::async_catalog_unsupported');
                }
                return translateWithFallback(id, args, table as LocaleCatalog);
            },
        });
    }

    locales(root: string, opts: { envKeys?: string[] } = {}): this {
        if (!root || typeof root !== 'string') {
            throw CliError.configError('commander::config::invalid_locales_root');
        }
        this.localesRoot = root;
        this.localesEnvKeys = opts.envKeys;
        if (!this.rootOptions.some((o) => o.key === 'locale')) {
            this.rootOptions.push(parseOptionDef('--locale <id>', 'commander.opt.locale'));
        }
        // Eager plugin so missing-manifest fails early; parse rebuilds with argv.
        this.localize = wrapWithCommanderFallback(createLocalizeFromLocales({ root, envKeys: opts.envKeys }));
        return this;
    }

    option(rawName: string, helpId: string): this {
        assertHelpId(helpId, 'option');
        this.rootOptions.push(parseOptionDef(rawName, helpId));
        return this;
    }

    intro(introId: string): this {
        assertHelpId(introId, 'intro');
        this.introId = introId;
        return this;
    }

    help(helpId: string): this {
        return this.intro(helpId);
    }

    command(rawName: string, helpId: string): Command {
        const cmd = new CommandBuilder(rawName, helpId);
        this.roots.push(cmd);
        return cmd;
    }

    onDiagnostic(handler: DiagnosticHandler): this {
        if (typeof handler !== 'function') throw CliError.configError('commander::config::diagnostic_handler_not_function');
        this.diagnosticHandler = handler;
        return this;
    }

    describe(): CliSpec {
        return {
            name: this.name,
            options: optionTokens(this.rootOptions),
            commands: this.roots.map((root) => commandSpec(root.def)),
        };
    }

    /** Help ids registered on this CLI (for {@link assertCatalogCoverage}). */
    collectHelpIds(): string[] {
        const ids: string[] = [];
        if (this.introId) ids.push(this.introId);
        for (const o of this.rootOptions) ids.push(o.helpId);
        const walk = (def: CommandDef) => {
            ids.push(def.helpId);
            for (const o of def.options) ids.push(o.helpId);
            for (const c of def.children) walk(c);
        };
        for (const r of this.roots) walk(r.def);
        return [...new Set(ids)];
    }

    /**
     * Build-time validation of every interactive declaration in the tree
     * (command level and option level). Runs before dispatch so a broken
     * machine path fails as a config error instead of hanging on a prompt.
     */
    private validateInteractiveSpecs(): void {
        const walk = (defs: CommandDef[], path: string[]): void => {
            for (const d of defs) {
                const token = d.names[0] ?? '?';
                const here = [...path, token].join(' ');
                const keys = new Set(d.options.map((o) => o.key));
                if (d.interactive) validateInteractiveSpec(d.interactive, here, keys);
                for (const o of d.options) {
                    if (o.config?.interactive) validateInteractiveSpec(o.config.interactive, `${here} ${o.rawName}`, keys);
                }
                walk(d.children, [...path, token]);
            }
        };
        walk(this.roots.map((r) => r.def), []);
    }

    async parse(argvInput: string[] = process.argv): Promise<number> {
        this.validateInteractiveSpecs();
        const argv0 = normalizeArgv(argvInput);
        const peeled = peelKnownOptions(argv0, this.rootOptions);
        const globalOpts = peeled.options;
        // A leading separator at program level only marks the end of global
        // options. Keep the following command token dispatchable.
        const argv = peeled.rest[0] === '--' ? peeled.rest.slice(1) : peeled.rest;
        const run = this.computeRun(globalOpts);

        let localize = this.localize;
        if (this.localesRoot) {
            const localeFlag = typeof globalOpts.locale === 'string' && globalOpts.locale ? globalOpts.locale : undefined;
            localize = wrapWithCommanderFallback(
                createLocalizeFromLocales({
                    root: this.localesRoot,
                    locale: localeFlag,
                    argv: argv0,
                    env: run.env,
                    envKeys: this.localesEnvKeys,
                }),
            );
        }
        if (!localize) {
            throw CliError.configError('commander::config::localize_required');
        }
        const t = localize.t.bind(localize);
        const diagnosticContext: DiagnosticContext = {
            locale: localize.resolveLocale?.({ argv: argv0, env: run.env }) ?? 'en-US',
            t,
        };

        // `help <cmd> ...` routing: treat tokens after help as the command path
        if (argv.length && argv[0] === 'help') {
            const pathTokens = argv.slice(1);
            return this.routeHelp(pathTokens, t, run);
        }

        if (argv.length === 0 || argv.every(isHelpToken)) {
            this.printHelp(this.formatRootHelp(t), run);
            return 0;
        }

        const cmdToken = argv[0]!;
        const matched = this.roots.find((c) => c.def.names.includes(cmdToken));
        if (!matched) {
            // Unknown command: attach a suggestion when possible
            const names = this.roots.flatMap((r) => r.def.names);
            const suggest = closestMatch(cmdToken, names);
            this.emitDiagnostic(
                {
                    code: 'commander::unknown_command',
                    severity: 'error',
                    args: suggest ? { cmd: cmdToken, suggest } : { cmd: cmdToken },
                },
                diagnosticContext,
                t,
            );
            return 2;
        }

        return await this.dispatch(matched.def, argv.slice(1), t, [cmdToken], globalOpts, diagnosticContext, run);
    }

    /** Route `help <cmd> [subcmd] ...` to that command's help text. */
    private routeHelp(pathTokens: string[], t: LocalizePlugin['t'], run: RunEnv): number {
        if (!pathTokens.length) {
            this.printHelp(this.formatRootHelp(t), run);
            return 0;
        }
        let pool: CommandDef[] | undefined = this.roots.map((r) => r.def);
        let matched: CommandDef | undefined;
        let consumed = 0;
        for (const tok of pathTokens) {
            const found: CommandDef | undefined = pool?.find((c) => c.names.includes(tok));
            if (!found) break;
            matched = found;
            consumed += 1;
            pool = found.children;
        }
        if (!matched) {
            // No valid command in the path → still show root help
            this.printHelp(this.formatRootHelp(t), run);
            return 0;
        }
        const path = pathTokens.slice(0, consumed);
        this.printHelp(formatCommandHelp(this.name, matched, t, path, this.rootOptions), run);
        return 0;
    }

    /** Emit a diagnostic through `onDiagnostic` when registered, else console.error with a short hint. */
    private emitDiagnostic(
        diag: CommandDiagnostic,
        ctx: DiagnosticContext,
        t: LocalizePlugin['t'],
    ): void {
        if (this.diagnosticHandler) {
            this.diagnosticHandler(diag, ctx);
        } else {
            const code = diag.code ?? '';
            // Catalog template wins so structured args stay interpolated (`--oops`).
            // A business message is only a fallback for codes without a template.
            const fromCatalog = code ? localizedOrId(t, code, diag.args) : '';
            const businessMessage = diag.message && diag.message !== code ? diag.message : '';
            const msg = fromCatalog || businessMessage || code || String(diag);
            this.writeErr(errorText(msg));
        }
        // Hints and suggestions are framework UI; always append even with a handler
        if (diag.code && HINT_CODES.has(diag.code)) {
            // unknown_command: hint points at the root command (command does not exist)
            // otherwise: hint points at the full executable command path
            const cmd = typeof diag.args?.cmd === 'string' ? diag.args.cmd.trim() : '';
            const hintTarget = diag.code === 'commander::unknown_command' || !cmd
                ? this.name
                : `${this.name} ${cmd}`;
            this.writeErr(paint(t('commander.ui.hint.available_options', { command: hintTarget }), 'dim'));
        }
        const suggest = typeof diag.args?.suggest === 'string' ? diag.args.suggest : '';
        if (suggest) {
            this.writeErr(paint(t('commander.ui.hint.did_you_mean', { suggest }), 'dim'));
        }
    }

    /** Derived root help: optional intro + commands/options from the registration tree. */
    formatRootHelp(t: LocalizePlugin['t']): string {
        const lines: string[] = [];
        if (this.introId) {
            lines.push(t(this.introId), '');
        }
        lines.push(paint(t('commander.ui.usage', { name: this.name }), 'bold', 'cyan'));
        lines.push('');
        lines.push(...renderHelpSection(
            t,
            'commander.ui.commands',
            this.roots.map((c) => ({ rawName: c.def.rawName, helpId: c.def.helpId })),
            'green',
        ));
        const opts = [
            ...this.rootOptions,
            ...collectRootOptions(this.roots.map((c) => c.def)).filter((o) => !this.rootOptions.some((r) => r.key === o.key)),
        ];
        const optionSection = renderOptionSections(t, opts, 'yellow');
        if (optionSection.length) {
            lines.push('');
            lines.push(...optionSection);
        }
        return lines.join('\n');
    }

    private async dispatch(
        def: CommandDef,
        rest: string[],
        t: LocalizePlugin['t'],
        path: string[],
        globalOpts: ParsedOptions,
        diagnosticContext: DiagnosticContext,
        run: RunEnv,
    ): Promise<number> {
        // `--` ends option parsing. Keep head (options + command tokens) separate
        // from tail (raw positionals) before help scanning or option parsing.
        const separatorAt = rest.indexOf('--');
        const head = separatorAt === -1 ? rest : rest.slice(0, separatorAt);
        const tail = separatorAt === -1 ? [] : rest.slice(separatorAt + 1);

        // Resolve the deepest command node before interpreting help markers, so
        // `demo document check --help` binds to `check` instead of `document`.
        let target = def;
        const targetPath = path.slice();
        const remaining = head.slice();
        while (target.children.length && remaining.length) {
            const token = remaining[0]!;
            if (token.startsWith('-') || isHelpToken(token)) break;
            const child = target.children.find((c) => c.names.includes(token));
            if (!child) break;
            target = child;
            targetPath.push(token);
            remaining.shift();
        }

        // Scan --help / -h (only head after the command path; tail after `--` is ignored)
        if (hasHelpToken(remaining)) {
            this.printHelp(formatCommandHelp(this.name, target, t, targetPath, this.rootOptions), run);
            return 0;
        }

        if (!target.action && !target.actionCtx) {
            if (remaining.length || tail.length) {
                const childNames = target.children.flatMap((c) => c.names);
                const unknownToken = remaining[0];
                const suggest = unknownToken && !unknownToken.startsWith('-')
                    ? closestMatch(unknownToken, childNames)
                    : '';
                const cmd = [...targetPath, ...remaining, ...tail].join(' ');
                this.emitDiagnostic(
                    {
                        code: 'commander::unknown_command',
                        severity: 'error',
                        args: suggest ? { cmd, suggest } : { cmd },
                    },
                    diagnosticContext,
                    t,
                );
                return 2;
            }
            this.printHelp(formatCommandHelp(this.name, target, t, targetPath, this.rootOptions), run);
            return 0;
        }

        let options: ParsedOptions;
        let usedOptions: Set<OptionDef> = new Set();
        try {
            if (target.passthrough) {
                options = { _: [...remaining, ...tail] };
            } else {
                ({ options, used: usedOptions } = parseOptionsDetailed(remaining, target.options));
                options._.push(...tail);
            }
        } catch (err) {
            const cliErr = CliError.from(err, 'commander::parse_failed');
            const args = { ...(cliErr.args ?? {}), cmd: targetPath.join(' ') };
            this.emitDiagnostic(
                { code: cliErr.code, severity: 'error', args, message: cliErr.message },
                diagnosticContext,
                t,
            );
            return cliErr.exitCode;
        }

        for (const [k, v] of Object.entries(globalOpts)) {
            if (k === '_') continue;
            if (options[k] === undefined) options[k] = v as string | boolean | string[];
        }

        for (const option of target.options) {
            const deprecated = option.config?.deprecated;
            if (!deprecated || !usedOptions.has(option)) continue;
            const args: Record<string, string> = { option: option.key };
            if (typeof deprecated === 'string' && deprecated) {
                args.detail = localizedOrId(t, deprecated) || deprecated;
            }
            this.emitDiagnostic(
                { code: 'commander::option_deprecated', severity: 'warning', args },
                diagnosticContext,
                t,
            );
        }

        try {
            let result: void | number;
            if (target.actionCtx) {
                const ctx = this.buildCommandContext(target, targetPath, diagnosticContext.locale, t, run);
                result = await target.actionCtx(options, options._, ctx);
            } else if (target.action) {
                result = await target.action(options, ...options._);
            } else {
                this.printHelp(formatCommandHelp(this.name, target, t, targetPath, this.rootOptions), run);
                return 0;
            }
            if (typeof result === 'number') return result;
            return 0;
        } catch (err) {
            const diagnostic = normalizeDiagnostic(err);
            this.emitDiagnostic(diagnostic, diagnosticContext, t);
            // CliError.exitCode is POSIX-correct (2 for parse/validation, 1 for config/action)
            if (err instanceof CliError) return err.exitCode;
            return 1;
        }
    }

    /** Build a CommandContext for .actionCtx() handlers. */
    private buildCommandContext(def: CommandDef, path: string[], locale: string, t: LocalizePlugin['t'], run: RunEnv): CommandContext {
        const self = this;
        const name = path.join(' ');
        const state = { stdinClaimed: false };
        const unavailable = () => CliError.capabilityError('commander::interactive_unavailable');
        return {
            commandPath: [this.name, ...path],
            commandName: name,
            locale,
            t,
            help: () => this.printHelp(formatCommandHelp(this.name, def, t, path, this.rootOptions), run),
            stderr: (...msgs) => { for (const m of msgs) this.writeErr(m); },
            stdout: (...msgs) => { for (const m of msgs) this.writeOut(m); },
            fail: (code, args) => {
                self.emitDiagnostic(
                    { code, severity: 'error', args: args ?? {}, message: args ? (args.message ?? '') : '' },
                    { locale, t },
                    t,
                );
                throw CliError.configError(code, args ?? {});
            },
            warn: (code, args) => {
                self.emitDiagnostic(
                    { code, severity: 'warning', args: args ?? {}, message: args ? (args.message ?? '') : '' },
                    { locale, t },
                    t,
                );
            },
            note: (code, args) => {
                self.emitDiagnostic(
                    { code, severity: 'advice', args: args ?? {}, message: args ? (args.message ?? '') : '' },
                    { locale, t },
                    t,
                );
            },
            // Wiring placeholders — the real engines land with the eligibility,
            // prompt and envelope blocks. The type surface is frozen now.
            prompt: {
                confirm: async () => { throw unavailable(); },
                input: async () => { throw unavailable(); },
                select: async () => { throw unavailable(); },
                password: async () => { throw unavailable(); },
                awaitAction: async () => { throw unavailable(); },
            },
            isInteractive: () => run.interactive,
            claimStdin: () => { state.stdinClaimed = true; },
            get stdinClaimed() { return state.stdinClaimed; },
            emitResult: (data) => { console.log(JSON.stringify(data, null, 2)); },
            emitFailure: (error) => { console.error(`${error.code}: ${error.message}`); },
            actionRequired: (descriptor) => { throw CliError.actionRequired(descriptor); },
        };
    }
}

/** Derive command help from one node (children + options). */
export function formatCommandHelp(
    cliName: string,
    def: CommandDef,
    t: LocalizePlugin['t'],
    path: string[],
    rootOptions: OptionDef[] = [],
): string {
    const lines: string[] = [paint(`${cliName} ${path.join(' ')}`, 'bold', 'cyan') + ` — ${t(def.helpId)}`];
    if (def.children.length) {
        lines.push('');
        lines.push(...renderHelpSection(
            t,
            'commander.ui.commands',
            def.children.map((c) => ({ rawName: c.rawName, helpId: c.helpId })),
            'green',
        ));
    }
    const opts = [...rootOptions, ...def.options.filter((o) => !rootOptions.some((r) => r.key === o.key))];
    const optionSection = renderOptionSections(t, opts, 'yellow');
    if (optionSection.length) {
        lines.push('');
        lines.push(...optionSection);
    }
    return lines.join('\n').trimEnd();
}

function commandSpec(def: CommandDef): CliSpecCommand {
    return {
        names: [...def.names],
        options: optionTokens(def.options),
        children: def.children.map(commandSpec),
    };
}

/** Render option aliases as flag tokens (`-o` for a short alias, `--out-dir` otherwise). */
function optionTokens(options: OptionDef[]): string[] {
    const seen = new Set<string>();
    const tokens: string[] = [];
    for (const option of options) {
        for (const alias of option.aliases) {
            const token = alias.length === 1 ? `-${alias}` : `--${alias}`;
            if (seen.has(token)) continue;
            seen.add(token);
            tokens.push(token);
        }
    }
    return tokens;
}

/** Union options declared on root commands (dedupe by key, stable first-seen order). */
function collectRootOptions(roots: CommandDef[]): OptionDef[] {
    const seen = new Set<string>();
    const out: OptionDef[] = [];
    for (const root of roots) {
        for (const o of root.options) {
            if (seen.has(o.key)) continue;
            seen.add(o.key);
            out.push(o);
        }
    }
    return out;
}

/**
 * Create a CLI named `name` (shown in usage).
 *
 * `opts` injects process-IO seams for tests and embedding hosts; Node defaults apply when omitted.
 */
export function createCli(name: string, opts: CreateCliOptions = {}): Cli {
    if (!name || typeof name !== 'string') {
        throw CliError.configError('commander::config::invalid_cli_name', { detail: String(name) });
    }
    return new CliBuilder(name, opts) as unknown as Cli;
}

/**
 * Dev/CI: every registered helpId must exist in `catalog` or commander English fallbacks.
 */
export function assertCatalogCoverage(cli: Cli, catalog: LocaleCatalog): void {
    const ids = typeof (cli as CliBuilder).collectHelpIds === 'function' ? (cli as CliBuilder).collectHelpIds() : [];
    const missing = ids.filter(
        (id) => !Object.prototype.hasOwnProperty.call(catalog, id) && !Object.prototype.hasOwnProperty.call(COMMANDER_FALLBACK_EN_US, id),
    );
    if (missing.length) {
        throw CliError.configError('commander::config::catalog_missing_help_ids', { ids: missing.sort().join(', ') });
    }
}

function wrapWithCommanderFallback(plugin: LocalizePlugin): LocalizePlugin {
    return {
        resolveLocale: plugin.resolveLocale?.bind(plugin),
        t: (id, args) => {
            const fromPlugin = plugin.t(id, args);
            if (fromPlugin !== `{{${id}}}`) return fromPlugin;
            if (Object.prototype.hasOwnProperty.call(COMMANDER_FALLBACK_EN_US, id)) {
                return translate(id, args, COMMANDER_FALLBACK_EN_US);
            }
            return fromPlugin;
        },
    };
}

/**
 * Shared argv → { options, rest } parser used by both strict and permissive modes.
 *
 * - `strict: true` — throws {@link CliError} (exit code 2) on unknown option or
 *   missing required value. Used by {@link parseOptions} before dispatching to an action.
 * - `strict: false` — unknown flags / bare tokens stay in `rest`. Used by
 *   {@link peelKnownOptions} so `.locales(root)` / other early-bound global options
 *   never reject argv that belongs to a later subcommand.
 */
function parseArgvCore(
    argv: string[],
    optionDefs: OptionDef[],
    strict: boolean,
): { options: ParsedOptions; rest: string[]; used: Set<OptionDef> } {
    const byAlias = new Map<string, OptionDef>();
    for (const def of optionDefs) {
        for (const a of def.aliases) byAlias.set(a, def);
    }
    const options: ParsedOptions = { _: [] };
    const rest: string[] = [];
    /** Options explicitly supplied on the command line (defaults are not "used"). */
    const used = new Set<OptionDef>();

    // Bare `-` is a stdout/stdin sentinel, not an option marker: `--out -` equals `--out=-`.
    // `--out --other` / `--out -x` still count as missing value; later flags are not swallowed.
    const isOptionToken = (token: string) => token.length > 1 && token.startsWith('-');

    const handleMissing = (flagName: string) => {
        if (strict) {
            throw CliError.parseError('commander::missing_option_value', { option: flagName });
        }
        rest.push(flagName);
        return true;
    };
    const handleUnknown = (token: string): boolean => {
        if (strict) {
            throw CliError.parseError('commander::unknown_option', { option: token });
        }
        rest.push(token);
        return true;
    };

    for (let i = 0; i < argv.length; i++) {
        const a = argv[i]!;
        if (a === '--') {
            // Strict parsing consumes the separator. Permissive peeling must keep
            // both `--` and its tail so dispatch can split head/tail and never
            // interpret tail tokens as options or help markers.
            if (strict) rest.push(...argv.slice(i + 1));
            else rest.push('--', ...argv.slice(i + 1));
            break;
        }
        // Do not skip help tokens here: leave them in rest for hasHelpToken in dispatch
        if (a.startsWith('--')) {
            const eq = a.indexOf('=');
            const long = eq === -1 ? a.slice(2) : a.slice(2, eq);
            const def = byAlias.get(long);
            if (!def) {
                if (!handleUnknown(a)) continue;
                continue;
            }
            if (def.takesValue) {
                if (eq !== -1) {
                    assignOption(options, def, used, a.slice(eq + 1));
                } else {
                    const next = argv[i + 1];
                    if (next != null && !isOptionToken(next)) {
                        assignOption(options, def, used, next);
                        i += 1;
                    } else if (def.optionalValue) {
                        assignOption(options, def, used, true);
                    } else {
                        if (!handleMissing(`--${long}`)) continue;
                        continue;
                    }
                }
            } else {
                assignOption(options, def, used, true);
            }
            continue;
        }
        // Short flags: -x or -xyz (multi-char); treat as short options when length > 1 and not --
        if (a.startsWith('-') && a.length > 1) {
            const short = a.slice(1);
            const def = byAlias.get(short);
            if (!def) {
                if (!handleUnknown(a)) continue;
                continue;
            }
            if (def.takesValue) {
                const next = argv[i + 1];
                if (next != null && !isOptionToken(next)) {
                    assignOption(options, def, used, next);
                    i += 1;
                } else if (def.optionalValue) {
                    assignOption(options, def, used, true);
                } else {
                    if (!handleMissing(`-${short}`)) continue;
                    continue;
                }
            } else {
                assignOption(options, def, used, true);
            }
            continue;
        }
        rest.push(a);
    }
    return { options, rest, used };
}

/**
 * Consume known root options; leave unknown flags and positionals in `rest`
 * (unlike {@link parseOptions}, which throws on unknown options).
 */
export function peelKnownOptions(argv: string[], optionDefs: OptionDef[]): { options: ParsedOptions; rest: string[] } {
    const { options, rest } = parseArgvCore(argv, optionDefs, false);
    return { options, rest };
}

/** Strip `node` + script when callers pass full `process.argv`. */
export function normalizeArgv(argv: string[]): string[] {
    if (argv.length >= 2 && looksLikeNode(argv[0]!) && looksLikeScript(argv[1]!)) {
        return argv.slice(2);
    }
    return argv.slice();
}

function looksLikeNode(token: string): boolean {
    const base = token.replace(/\\/g, '/').split('/').pop() || '';
    return base === 'node' || base === 'node.exe' || base.startsWith('node');
}

function looksLikeScript(token: string): boolean {
    return /\.(c?js|mjs|ts)$/i.test(token) || token.includes(`${'node_modules'}`);
}

function isHelpToken(token: string): boolean {
    return token === 'help' || token === '-h' || token === '--help';
}

function splitNames(rawName: string): string[] {
    return rawName
        .split('|')
        .map((s) => s.trim())
        .filter(Boolean);
}

function assertHelpId(helpId: string, kind: string): void {
    if (!helpId || typeof helpId !== 'string' || !helpId.trim()) {
        throw CliError.configError('commander::config::help_id_empty', { kind });
    }
    if (/\s/.test(helpId)) {
        throw CliError.configError('commander::config::help_id_has_spaces', { kind, id: helpId });
    }
}

/** Parse `--out-dir <dir>` / `-o, --out-dir <dir>` / `--dirty <path>...` into an OptionDef. */
export function parseOptionDef(rawName: string, helpId: string): OptionDef {
    const repeatable = /\.\.\.\s*$/.test(rawName) || /\.\.\.>/.test(rawName) || /\.\.\.]/.test(rawName);
    const optionalValue = /\[[^\]]+\]/.test(rawName);
    const takesValue = optionalValue || /<[^>]+>/.test(rawName) || repeatable;
    const cleaned = rawName
        .replace(/\.\.\./g, '')
        .replace(/<[^>]+>|\[[^\]]+\]/g, '')
        .trim();
    const parts = cleaned
        .split(/[,\s]+/)
        .map((p) => p.trim())
        .filter(Boolean);
    const aliases: string[] = [];
    for (const p of parts) {
        if (p.startsWith('--')) aliases.push(p.slice(2));
        else if (p.startsWith('-') && p.length > 1) aliases.push(p.slice(1));
    }
    if (!aliases.length) {
        throw CliError.configError('commander::config::invalid_option_raw', { raw: rawName });
    }
    const key = aliases.find((a) => a.length > 1) ?? aliases[0]!;
    return { rawName, helpId, key, aliases, takesValue, optionalValue, repeatable };
}

function normalizeDiagnostic(error: unknown): CommandDiagnostic {
    if (error && typeof error === 'object') {
        const value = error as Record<string, unknown>;
        const candidate = value.diagnostic && typeof value.diagnostic === 'object' ? value.diagnostic as Record<string, unknown>: value;
        if (typeof candidate.code === 'string' || typeof candidate.severity === 'string' || typeof candidate.path === 'string') {
            return {
                code: typeof candidate.code === 'string' ? candidate.code : undefined,
                severity: typeof candidate.severity === 'string' ? candidate.severity : 'error',
                args: candidate.args && typeof candidate.args === 'object' ? candidate.args as Record<string, string>: undefined,
                path: typeof candidate.path === 'string' ? candidate.path : undefined,
                span: candidate.span && typeof candidate.span === 'object' ? candidate.span as { start: number; end: number } : undefined,
                message: typeof candidate.message === 'string' ? candidate.message : undefined,
            };
        }
    }
    const text = error instanceof Error ? error.message : String(error);
    // Put both message and detail so product catalogs may use either {message} or {detail}
    return { severity: 'error', code: 'commander::action_failed', args: { message: text, detail: text }, message: text };
}

/**
 * Parse argv against registered options (strict mode).
 *
 * Throws {@link CliError} with stable code `commander::unknown_option` /
 * `commander::missing_option_value` (exit code 2) on validation failure.
 * Non-option tokens become positionals under `options._`.
 */
export function parseOptions(argv: string[], optionDefs: OptionDef[]): ParsedOptions {
    return parseOptionsDetailed(argv, optionDefs).options;
}

/** Strict parse plus which option defs were actually supplied (for deprecation warnings). */
function parseOptionsDetailed(
    argv: string[],
    optionDefs: OptionDef[],
): { options: ParsedOptions; used: Set<OptionDef> } {
    const { options, rest, used } = parseArgvCore(argv, optionDefs, true);
    options._.push(...rest);
    applyOptionConfig(options, optionDefs, used);
    return { options, used };
}

/**
 * Apply `.option(..., config)` runtime constraints (default / required / enum / coerce).
 *
 * Runs before any action handler executes and throws {@link CliError.parseError}
 * (exit code 2) so callers receive structured `code` + `args` diagnostics.
 */
function applyOptionConfig(
    options: ParsedOptions,
    optionDefs: OptionDef[],
    used: Set<OptionDef>,
): void {
    for (const def of optionDefs) {
        const config = def.config;
        if (!config) continue;
        const provided = used.has(def);
        if (!provided && config.default !== undefined) {
            options[def.key] = Array.isArray(config.default) ? config.default.slice() : config.default;
        }
        const value = options[def.key];
        if (config.required && value === undefined) {
            throw CliError.parseError('commander::missing_required_option', { option: def.key });
        }
        if (value === undefined) continue;
        if (config.enum && provided) {
            const values = Array.isArray(value) ? value : [value];
            for (const item of values) {
                if (typeof item !== 'string') continue;
                if (!config.enum.includes(item)) {
                    throw CliError.parseError('commander::option_not_in_enum', {
                        option: def.key,
                        value: item,
                        allowed: config.enum.join(', '),
                    });
                }
            }
        }
        if (config.coerce && provided) {
            try {
                options[def.key] = config.coerce(value) as string | boolean | string[];
            } catch (err) {
                throw CliError.parseError('commander::option_coerce_failed', {
                    option: def.key,
                    value: Array.isArray(value) ? value.join(', ') : String(value),
                    detail: err instanceof Error ? err.message : String(err),
                });
            }
        }
    }
}

function assignOption(
    out: ParsedOptions,
    def: OptionDef,
    used: Set<OptionDef>,
    value: string | boolean,
): void {
    used.add(def);
    if (def.repeatable) {
        const cur = out[def.key];
        const list = Array.isArray(cur) ? cur : [];
        list.push(String(value));
        out[def.key] = list;
        return;
    }
    out[def.key] = value;
}

/** Whether argv contains a help token (ignore everything after --). */
function hasHelpToken(argv: string[]): boolean {
    for (const tok of argv) {
        if (tok === '--') return false;
        if (isHelpToken(tok)) return true;
    }
    return false;
}

/**
 * Simple Levenshtein nearest-match among candidates.
 * Returns a suggested command name, or an empty string when nothing is close enough.
 */
function closestMatch(input: string, candidates: string[]): string {
    if (!input || !candidates.length) return '';
    const lower = input.toLowerCase();
    let best: { name: string; dist: number }[] = [];
    for (const c of candidates) {
        const d = levenshtein(lower, c.toLowerCase());
        // Accept only distance <= 3 and at most ~60% of input length to avoid unrelated suggestions
        const threshold = Math.max(2, Math.floor(input.length * 0.6) + 1);
        if (d <= threshold) {
            best.push({ name: c, dist: d });
        }
    }
    if (!best.length) return '';
    best.sort((a, b) => a.dist - b.dist);
    // Return only the top-1 match; comma-joining multiple candidates looks like one command
    return best[0]!.name;
}

function levenshtein(a: string, b: string): number {
    const m = a.length;
    const n = b.length;
    if (m === 0) return n;
    if (n === 0) return m;
    // Keep only two rows to save memory
    let prev = new Array<number>(n + 1);
    let curr = new Array<number>(n + 1);
    for (let j = 0; j <= n; j++) prev[j] = j;
    for (let i = 1; i <= m; i++) {
        curr[0] = i;
        for (let j = 1; j <= n; j++) {
            const cost = a[i - 1] === b[j - 1] ? 0 : 1;
            curr[j] = Math.min(curr[j - 1] + 1, prev[j] + 1, prev[j - 1] + cost);
        }
        [prev, curr] = [curr, prev];
    }
    return prev[n]!;
}
