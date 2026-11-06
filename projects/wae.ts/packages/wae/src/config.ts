/**
 * `wae.config.ts` schema, static parser, discovery, and validation.
 *
 * The config file is pure data: `export default defineConfig({ ... })` with
 * literals only. `wae` parses the source text and never imports or executes it,
 * so a config cannot run commands, spawn processes, read `process.env`, or
 * branch at load time. A non-literal value fails with `config_not_static`.
 */
import fs from 'node:fs';
import path from 'node:path';

/** Config file name discovered from the project root upward. */
export const WAE_CONFIG_FILE = 'wae.config.ts';

/** Build steps in their mandatory order. A config may narrow the list, never reorder it. */
export const WAE_BUILD_ORDER = ['native', 'ts', 'ui', 'skills', 'staging'] as const;

export type WaeBuildStepName = (typeof WAE_BUILD_ORDER)[number];

/** One publishable native target. */
export type WaeNativeTarget = {
    platform: string;
    arch: string;
    triple: string;
    /** npm platform package name, e.g. `@wae/wae-win32-x64`. */
    package: string;
    /** Staged addon file name, e.g. `wae.win32-x64.node`. */
    nodeFile: string;
    /** Platform package directory, relative to the config root. */
    dir: string;
};

export type WaeNativeConfig = {
    crate: string;
    /** cdylib stem when it differs from the crate name with `-` replaced by `_`. */
    cdylib?: string;
    profile?: 'debug' | 'release';
    /** Cargo build directory; defaults to `<root>/target`. */
    buildDir?: string;
    /** Environment variable that pins the target, e.g. `WAE_NATIVE_TARGET`. */
    targetEnv?: string;
    /** `reject` (default) refuses to claim a cross-compiled artifact is loadable. */
    crossCompile?: 'allow' | 'reject';
    /** Stage an already built artifact without running cargo. */
    stageOnly?: boolean;
    targets: WaeNativeTarget[];
};

/** Declared platform package layout; manifests are validated against it. */
export type WaePlatformPackage = {
    name: string;
    dir: string;
    os: string[];
    cpu: string[];
    main: string;
};

export type WaeBuildStep = {
    name: WaeBuildStepName;
    dependsOn?: WaeBuildStepName[];
};

export type WaeBuildConfig = {
    steps?: WaeBuildStep[];
    /** npm packages whose `build` script compiles TypeScript, run in listed order. */
    ts?: { packages: string[] };
    /** npm package that owns the UI build. */
    ui?: { package: string };
    /** Copies applied after all builds; `from` must exist. */
    stage?: { from: string; to: string; clean?: boolean; ignore?: string[] }[];
    /** Files that must exist after staging. */
    outputs?: string[];
};

export type WaeSkillsPackage = {
    /** Skills tree containing `<skill>/SKILL.md` directories. */
    dir: string;
    packageJson?: string;
    appPackageJson?: string;
    /** Built CLI entry that the skills reference. */
    cliEntry?: string;
    /** Invocation prefix used inside backticked commands. */
    commandName?: string;
    requiredSections?: string[];
    maxFilesEntries?: number;
};

export type WaeSkillsConfig = {
    packages: WaeSkillsPackage[];
};

export type WaeReleasePackage = {
    name?: string;
    dir: string;
};

export type WaeReleaseConfig = {
    registry: string;
    /** Release tag shape with one version capture group, e.g. `^v(\d+\.\d+\.\d+)$`. */
    tagPattern: string;
    packages: WaeReleasePackage[];
};

export type WaeCiConfig = {
    repo?: string;
    workflows?: string[];
    requiredJobs?: string[];
    waitTimeoutMs?: number;
    intervalMs?: number;
    shaBinding?: boolean;
};

export type WaeInstallConfig = {
    shortcutDefault: boolean;
    configLocation: 'per-user';
};

export type WaeActorSource = 'flag' | 'env' | 'session' | 'host';

export type WaeAuditConfig = {
    actor: { kind: WaeActorSource; id: WaeActorSource; source: WaeActorSource };
};

export type WaeConfig = {
    native?: WaeNativeConfig;
    packages?: WaePlatformPackage[];
    build?: WaeBuildConfig;
    skills?: WaeSkillsConfig;
    release?: WaeReleaseConfig;
    ci?: WaeCiConfig;
    install?: WaeInstallConfig;
    audit?: WaeAuditConfig;
};

/** Identity helper that gives config files contextual typing. */
export function defineConfig(config: WaeConfig): WaeConfig {
    return config;
}

/** Structured config failure: `config_not_static` for executable syntax, `config_invalid` for the schema. */
export class WaeConfigError extends Error {
    readonly code: 'config_not_static' | 'config_invalid';
    readonly file?: string;

    constructor(code: 'config_not_static' | 'config_invalid', message: string, file?: string) {
        super(message);
        this.name = 'WaeConfigError';
        this.code = code;
        this.file = file;
    }

    toJSON() {
        return {
            code: this.code,
            capability: 'config',
            ...(this.file ? { path: this.file } : {}),
            message: this.message,
        };
    }
}

/**
 * Parse the pure object literal after `export default`.
 *
 * Unlike `parseConfigSource`, this skips schema validation: `nifty.config.ts`
 * uses the same literal shape but a different schema.
 */
export function parseStaticObject(source: string, file = WAE_CONFIG_FILE): Record<string, unknown> {
    const parser = new StaticParser(stripComments(source), file);
    return parser.parse();
}

/** Parse and validate a whole config file. */
export function parseConfigSource(source: string, file = WAE_CONFIG_FILE): WaeConfig {
    return validateConfig(parseStaticObject(source, file), file);
}

/** Read, parse, and validate a config file. */
export function readConfigFile(file: string): WaeConfig {
    const text = fs.readFileSync(file, 'utf8');
    return parseConfigSource(text, file);
}

/** Find `wae.config.ts` from `start` upward; returns `undefined` at the filesystem root. */
export function discoverConfig(start: string): string | undefined {
    let dir = path.resolve(start);
    while (true) {
        const candidate = path.join(dir, WAE_CONFIG_FILE);
        if (fs.existsSync(candidate)) return candidate;
        const parent = path.dirname(dir);
        if (parent === dir) return undefined;
        dir = parent;
    }
}

/** Result of config discovery. The default convention applies when no file exists. */
export type LoadedConfig = {
    source: 'file' | 'default';
    path?: string;
    root: string;
    config?: WaeConfig;
};

/**
 * Discover and load the config. Relative config values are resolved against the
 * config directory, so discovery from a nested directory still uses the project
 * root as the build root.
 */
export function loadConfig(start: string): LoadedConfig {
    const configPath = discoverConfig(start);
    if (!configPath) {
        return { source: 'default', root: path.resolve(start) };
    }
    return {
        source: 'file',
        path: configPath,
        root: path.dirname(configPath),
        config: readConfigFile(configPath),
    };
}

/**
 * Strip `//` and block comments while preserving string literals.
 * Comments are not values, but removing them keeps the literal parser simple.
 */
function stripComments(source: string): string {
    let out = '';
    let index = 0;
    while (index < source.length) {
        const char = source[index]!;
        if (char === '/' && source[index + 1] === '/') {
            while (index < source.length && source[index] !== '\n') index += 1;
            continue;
        }
        if (char === '/' && source[index + 1] === '*') {
            index += 2;
            while (index < source.length && !(source[index] === '*' && source[index + 1] === '/')) index += 1;
            index += 2;
            continue;
        }
        if (char === '"' || char === "'" || char === '`') {
            const end = quotedEnd(source, index);
            out += source.slice(index, end);
            index = end;
            continue;
        }
        out += char;
        index += 1;
    }
    return out;
}

/** End index (exclusive) of a quoted literal, including the closing quote. */
function quotedEnd(source: string, start: number): number {
    const quote = source[start]!;
    let index = start + 1;
    while (index < source.length) {
        if (source[index] === '\\') {
            index += 2;
            continue;
        }
        if (source[index] === quote) return index + 1;
        index += 1;
    }
    return source.length;
}

/**
 * Recursive-descent parser for the static subset:
 * objects, arrays, strings, numbers, booleans, and `null`.
 *
 * Calls, identifiers other than `true`/`false`/`null`, spreads, computed keys,
 * template interpolation, and function values are rejected as `config_not_static`.
 */
class StaticParser {
    private index = 0;
    private readonly source: string;
    private readonly file: string;

    constructor(source: string, file: string) {
        this.source = source;
        this.file = file;
    }

    parse(): Record<string, unknown> {
        this.skipSpace();
        this.skipImports();
        this.expectWord('export');
        this.skipSpace();
        this.expectWord('default');
        this.skipSpace();
        let value: unknown;
        if (this.peekWord() === 'defineConfig') {
            this.readWord();
            this.skipSpace();
            this.expect('(');
            value = this.readValue();
            this.skipSpace();
            this.expect(')');
        } else {
            value = this.readValue();
        }
        this.skipSpace();
        if (this.peekWord() === 'satisfies' || this.peekWord() === 'as') {
            this.readWord();
            this.skipTypeClause();
        }
        this.skipSpace();
        if (this.source[this.index] === ';') {
            this.index += 1;
            this.skipSpace();
        }
        if (this.index < this.source.length) {
            this.fail(`unexpected token ${this.here()} after the default export`);
        }
        if (!isPlainObject(value)) {
            throw new WaeConfigError('config_invalid', `${this.file}: the default export must be an object literal`, this.file);
        }
        return value;
    }

    /**
     * Skip leading `import` statements. They are never evaluated here, so a
     * config may import `defineConfig` for editor and `tsc` typing.
     */
    private skipImports(): void {
        while (this.peekWord() === 'import') {
            this.readWord();
            let depth = 0;
            while (this.index < this.source.length) {
                const char = this.source[this.index]!;
                if (char === '"' || char === "'" || char === '`') {
                    this.index = quotedEnd(this.source, this.index);
                    continue;
                }
                if (char === '{') depth += 1;
                if (char === '}') depth -= 1;
                if (char === ';' || (char === '\n' && depth === 0)) {
                    this.index += 1;
                    break;
                }
                this.index += 1;
            }
            this.skipSpace();
        }
    }

    private readValue(): unknown {
        this.skipSpace();
        const char = this.source[this.index];
        if (char === '{') return this.readObject();
        if (char === '[') return this.readArray();
        if (char === '"' || char === "'" || char === '`') return this.readString();
        if (char === '-' || (char !== undefined && char >= '0' && char <= '9')) return this.readNumber();
        const word = this.peekWord();
        if (word === 'true') {
            this.readWord();
            return true;
        }
        if (word === 'false') {
            this.readWord();
            return false;
        }
        if (word === 'null') {
            this.readWord();
            return null;
        }
        return this.fail(`non-static value ${this.here()} (only literals are allowed)`);
    }

    private readObject(): Record<string, unknown> {
        this.expect('{');
        const result: Record<string, unknown> = {};
        this.skipSpace();
        if (this.source[this.index] === '}') {
            this.index += 1;
            return result;
        }
        while (true) {
            this.skipSpace();
            const key = this.readKey();
            if (Object.prototype.hasOwnProperty.call(result, key)) {
                throw new WaeConfigError('config_invalid', `${this.file}: duplicate key \`${key}\``, this.file);
            }
            this.skipSpace();
            this.expect(':');
            result[key] = this.readValue();
            this.skipSpace();
            const char = this.source[this.index];
            if (char === ',') {
                this.index += 1;
                this.skipSpace();
                if (this.source[this.index] === '}') {
                    this.index += 1;
                    return result;
                }
                continue;
            }
            if (char === '}') {
                this.index += 1;
                return result;
            }
            this.fail(`expected \`,\` or \`}\` after key \`${key}\``);
        }
    }

    private readArray(): unknown[] {
        this.expect('[');
        const result: unknown[] = [];
        this.skipSpace();
        if (this.source[this.index] === ']') {
            this.index += 1;
            return result;
        }
        while (true) {
            result.push(this.readValue());
            this.skipSpace();
            const char = this.source[this.index];
            if (char === ',') {
                this.index += 1;
                this.skipSpace();
                if (this.source[this.index] === ']') {
                    this.index += 1;
                    return result;
                }
                continue;
            }
            if (char === ']') {
                this.index += 1;
                return result;
            }
            this.fail('expected `,` or `]` after an array element');
        }
    }

    private readKey(): string {
        this.skipSpace();
        const char = this.source[this.index];
        if (char === '"' || char === "'" || char === '`') return this.readString();
        const match = /^[A-Za-z_$][A-Za-z0-9_$]*/.exec(this.source.slice(this.index));
        if (!match) this.fail(`expected an object key ${this.here()}`);
        this.index += match[0].length;
        return match[0];
    }

    private readString(): string {
        const quote = this.source[this.index]!;
        const start = this.index;
        this.index += 1;
        let value = '';
        while (this.index < this.source.length) {
            const char = this.source[this.index]!;
            if (char === '\\') {
                value += this.readEscape(start);
                continue;
            }
            if (char === quote) {
                this.index += 1;
                return value;
            }
            if (quote === '`' && char === '$' && this.source[this.index + 1] === '{') {
                throw new WaeConfigError(
                    'config_not_static',
                    `${this.file}: template interpolation is not a static value (near \`${this.here()}\`)`,
                    this.file,
                );
            }
            value += char;
            this.index += 1;
        }
        this.fail(`unterminated string starting at index ${start}`, 'config_not_static');
    }

    private readEscape(start: number): string {
        const next = this.source[this.index + 1];
        if (next === undefined) this.fail(`unterminated escape at index ${start}`, 'config_not_static');
        const simple: Record<string, string> = {
            n: '\n',
            r: '\r',
            t: '\t',
            b: '\b',
            f: '\f',
            v: '\v',
            '0': '\0',
            '\\': '\\',
            "'": "'",
            '"': '"',
            '`': '`',
            $: '$',
        };
        if (next in simple) {
            this.index += 2;
            return simple[next]!;
        }
        if (next === 'u') {
            const hex = this.source.slice(this.index + 2, this.index + 6);
            if (!/^[0-9a-fA-F]{4}$/.test(hex)) this.fail('invalid unicode escape', 'config_not_static');
            this.index += 6;
            return String.fromCharCode(parseInt(hex, 16));
        }
        if (next === 'x') {
            const hex = this.source.slice(this.index + 2, this.index + 4);
            if (!/^[0-9a-fA-F]{2}$/.test(hex)) this.fail('invalid hex escape', 'config_not_static');
            this.index += 4;
            return String.fromCharCode(parseInt(hex, 16));
        }
        this.index += 2;
        return next;
    }

    private readNumber(): number {
        const match = /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(this.source.slice(this.index));
        if (!match) this.fail(`invalid number ${this.here()}`, 'config_not_static');
        this.index += match[0].length;
        return Number(match[0]);
    }

    private skipTypeClause(): void {
        // `satisfies WaeConfig` / `as const` are type-only; consume up to the statement end.
        while (this.index < this.source.length && this.source[this.index] !== ';') {
            this.index += 1;
        }
    }

    private skipSpace(): void {
        while (this.index < this.source.length && /\s/.test(this.source[this.index]!)) this.index += 1;
    }

    private peekWord(): string {
        const match = /^[A-Za-z_$][A-Za-z0-9_$]*/.exec(this.source.slice(this.index));
        return match ? match[0] : '';
    }

    private readWord(): string {
        const word = this.peekWord();
        if (!word) this.fail(`expected an identifier ${this.here()}`);
        this.index += word.length;
        return word;
    }

    private expectWord(word: string): void {
        if (this.peekWord() !== word) this.fail(`expected \`${word}\` ${this.here()}`);
        this.readWord();
    }

    private expect(char: string): void {
        if (this.source[this.index] !== char) this.fail(`expected \`${char}\` ${this.here()}`);
        this.index += 1;
    }

    private here(): string {
        const snippet = this.source.slice(this.index, this.index + 24).replace(/\s+/g, ' ').trim();
        return snippet ? `\`${snippet}\`` : 'at end of file';
    }

    private fail(message: string, code: 'config_not_static' | 'config_invalid' = 'config_not_static'): never {
        throw new WaeConfigError(code, `${this.file}: ${message}`, this.file);
    }
}

/** Validate the parsed object against the `wae.config.ts` schema, rejecting unknown keys. */
export function validateConfig(raw: unknown, file = WAE_CONFIG_FILE): WaeConfig {
    const root = expectObject(raw, file);
    assertKeys(root, ['native', 'packages', 'build', 'skills', 'release', 'ci', 'install', 'audit'], file);

    const config: WaeConfig = {};
    if (root.native !== undefined) config.native = validateNative(root.native, `${file}#native`);
    if (root.packages !== undefined) config.packages = validatePackages(root.packages, `${file}#packages`);
    if (root.build !== undefined) config.build = validateBuild(root.build, `${file}#build`);
    if (root.skills !== undefined) config.skills = validateSkills(root.skills, `${file}#skills`);
    if (root.release !== undefined) config.release = validateRelease(root.release, `${file}#release`);
    if (root.ci !== undefined) config.ci = validateCi(root.ci, `${file}#ci`);
    if (root.install !== undefined) config.install = validateInstall(root.install, `${file}#install`);
    if (root.audit !== undefined) config.audit = validateAudit(root.audit, `${file}#audit`);

    if (config.native && config.packages) {
        for (const target of config.native.targets) {
            const key = `${target.platform}-${target.arch}`;
            const pkg = config.packages.find((entry) => entry.name === target.package);
            if (!pkg) {
                invalid(file, `native target \`${key}\` has no \`packages\` entry named \`${target.package}\``);
            }
            if (pkg.dir !== target.dir) {
                invalid(file, `native target \`${key}\` dir \`${target.dir}\` differs from package dir \`${pkg.dir}\``);
            }
            if (pkg.main !== target.nodeFile) {
                invalid(file, `native target \`${key}\` nodeFile \`${target.nodeFile}\` differs from package main \`${pkg.main}\``);
            }
            if (pkg.os.length !== 1 || pkg.os[0] !== target.platform || pkg.cpu.length !== 1 || pkg.cpu[0] !== target.arch) {
                invalid(file, `native target \`${key}\` does not match package os/cpu ${pkg.os.join(',')}/${pkg.cpu.join(',')}`);
            }
        }
    }

    return config;
}

function validateNative(value: unknown, where: string): WaeNativeConfig {
    const obj = expectObject(value, where);
    assertKeys(
        obj,
        ['crate', 'cdylib', 'profile', 'buildDir', 'targetEnv', 'crossCompile', 'stageOnly', 'targets'],
        where,
    );
    const crate = expectString(obj.crate, `${where}.crate`);
    const config: WaeNativeConfig = { crate, targets: [] };
    if (obj.cdylib !== undefined) config.cdylib = expectString(obj.cdylib, `${where}.cdylib`);
    if (obj.profile !== undefined) config.profile = expectEnum(obj.profile, ['debug', 'release'] as const, `${where}.profile`);
    if (obj.buildDir !== undefined) config.buildDir = expectString(obj.buildDir, `${where}.buildDir`);
    if (obj.targetEnv !== undefined) config.targetEnv = expectString(obj.targetEnv, `${where}.targetEnv`);
    if (obj.crossCompile !== undefined) {
        config.crossCompile = expectEnum(obj.crossCompile, ['allow', 'reject'] as const, `${where}.crossCompile`);
    }
    if (obj.stageOnly !== undefined) config.stageOnly = expectBoolean(obj.stageOnly, `${where}.stageOnly`);

    if (!Array.isArray(obj.targets) || obj.targets.length === 0) {
        invalid(`${where}.targets`, 'expected a non-empty array');
    }
    const seen = new Set<string>();
    for (const [index, item] of obj.targets.entries()) {
        const target = expectObject(item, `${where}.targets[${index}]`);
        assertKeys(target, ['platform', 'arch', 'triple', 'package', 'nodeFile', 'dir'], `${where}.targets[${index}]`);
        const platform = expectString(target.platform, `${where}.targets[${index}].platform`);
        const arch = expectString(target.arch, `${where}.targets[${index}].arch`);
        const key = `${platform}-${arch}`;
        if (seen.has(key)) invalid(`${where}.targets[${index}]`, `duplicate target \`${key}\``);
        seen.add(key);
        const nodeFile = expectString(target.nodeFile, `${where}.targets[${index}].nodeFile`);
        if (!nodeFile.endsWith('.node')) invalid(`${where}.targets[${index}].nodeFile`, 'must end with `.node`');
        config.targets.push({
            platform,
            arch,
            triple: expectString(target.triple, `${where}.targets[${index}].triple`),
            package: expectString(target.package, `${where}.targets[${index}].package`),
            nodeFile,
            dir: expectString(target.dir, `${where}.targets[${index}].dir`),
        });
    }
    return config;
}

function validatePackages(value: unknown, where: string): WaePlatformPackage[] {
    if (!Array.isArray(value) || value.length === 0) invalid(where, 'expected a non-empty array');
    const packages: WaePlatformPackage[] = [];
    const names = new Set<string>();
    for (const [index, item] of value.entries()) {
        const pkg = expectObject(item, `${where}[${index}]`);
        assertKeys(pkg, ['name', 'dir', 'os', 'cpu', 'main'], `${where}[${index}]`);
        const name = expectString(pkg.name, `${where}[${index}].name`);
        if (names.has(name)) invalid(`${where}[${index}]`, `duplicate package \`${name}\``);
        names.add(name);
        const main = expectString(pkg.main, `${where}[${index}].main`);
        if (!main.endsWith('.node')) invalid(`${where}[${index}].main`, 'must end with `.node`');
        packages.push({
            name,
            dir: expectString(pkg.dir, `${where}[${index}].dir`),
            os: expectStringArray(pkg.os, `${where}[${index}].os`),
            cpu: expectStringArray(pkg.cpu, `${where}[${index}].cpu`),
            main,
        });
    }
    return packages;
}

function validateBuild(value: unknown, where: string): WaeBuildConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['steps', 'ts', 'ui', 'stage', 'outputs'], where);
    const build: WaeBuildConfig = {};

    if (obj.steps !== undefined) {
        if (!Array.isArray(obj.steps)) invalid(`${where}.steps`, 'expected an array');
        const steps: WaeBuildStep[] = [];
        let lastOrder = -1;
        for (const [index, item] of obj.steps.entries()) {
            const step = expectObject(item, `${where}.steps[${index}]`);
            assertKeys(step, ['name', 'dependsOn'], `${where}.steps[${index}]`);
            const name = expectEnum(step.name, WAE_BUILD_ORDER, `${where}.steps[${index}].name`);
            const order = WAE_BUILD_ORDER.indexOf(name);
            if (order <= lastOrder) {
                invalid(`${where}.steps[${index}]`, `step \`${name}\` is out of order; build order is ${WAE_BUILD_ORDER.join(' → ')}`);
            }
            lastOrder = order;
            const dependsOn = step.dependsOn === undefined
                ? undefined
                : expectEnumArray(step.dependsOn, WAE_BUILD_ORDER, `${where}.steps[${index}].dependsOn`);
            if (dependsOn?.some((dependency) => WAE_BUILD_ORDER.indexOf(dependency) >= order)) {
                invalid(`${where}.steps[${index}].dependsOn`, 'dependencies must be earlier steps');
            }
            steps.push(dependsOn ? { name, dependsOn } : { name });
        }
        const declared = new Set(steps.map((step) => step.name));
        for (const [index, step] of steps.entries()) {
            for (const dependency of step.dependsOn ?? []) {
                if (!declared.has(dependency)) {
                    invalid(`${where}.steps[${index}].dependsOn`, `step \`${step.name}\` depends on \`${dependency}\`, which the narrowed list does not run`);
                }
            }
        }
        build.steps = steps;
    }

    if (obj.ts !== undefined) {
        const ts = expectObject(obj.ts, `${where}.ts`);
        assertKeys(ts, ['packages'], `${where}.ts`);
        build.ts = { packages: expectStringArray(ts.packages, `${where}.ts.packages`) };
    }

    if (obj.ui !== undefined) {
        const ui = expectObject(obj.ui, `${where}.ui`);
        assertKeys(ui, ['package'], `${where}.ui`);
        build.ui = { package: expectString(ui.package, `${where}.ui.package`) };
    }

    if (obj.stage !== undefined) {
        if (!Array.isArray(obj.stage)) invalid(`${where}.stage`, 'expected an array');
        build.stage = [];
        for (const [index, item] of obj.stage.entries()) {
            const stage = expectObject(item, `${where}.stage[${index}]`);
            assertKeys(stage, ['from', 'to', 'clean', 'ignore'], `${where}.stage[${index}]`);
            build.stage.push({
                from: expectString(stage.from, `${where}.stage[${index}].from`),
                to: expectString(stage.to, `${where}.stage[${index}].to`),
                ...(stage.clean === undefined ? {} : { clean: expectBoolean(stage.clean, `${where}.stage[${index}].clean`) }),
                ...(stage.ignore === undefined
                    ? {}
                    : { ignore: expectStringArray(stage.ignore, `${where}.stage[${index}].ignore`) }),
            });
        }
    }

    if (obj.outputs !== undefined) build.outputs = expectStringArray(obj.outputs, `${where}.outputs`);
    return build;
}

function validateSkills(value: unknown, where: string): WaeSkillsConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['packages'], where);
    if (!Array.isArray(obj.packages) || obj.packages.length === 0) {
        invalid(`${where}.packages`, 'expected a non-empty array');
    }
    const packages: WaeSkillsPackage[] = [];
    for (const [index, item] of obj.packages.entries()) {
        const pkg = expectObject(item, `${where}.packages[${index}]`);
        assertKeys(
            pkg,
            ['dir', 'packageJson', 'appPackageJson', 'cliEntry', 'commandName', 'requiredSections', 'maxFilesEntries'],
            `${where}.packages[${index}]`,
        );
        const entry: WaeSkillsPackage = { dir: expectString(pkg.dir, `${where}.packages[${index}].dir`) };
        if (pkg.packageJson !== undefined) entry.packageJson = expectString(pkg.packageJson, `${where}.packages[${index}].packageJson`);
        if (pkg.appPackageJson !== undefined) entry.appPackageJson = expectString(pkg.appPackageJson, `${where}.packages[${index}].appPackageJson`);
        if (pkg.cliEntry !== undefined) entry.cliEntry = expectString(pkg.cliEntry, `${where}.packages[${index}].cliEntry`);
        if (pkg.commandName !== undefined) entry.commandName = expectString(pkg.commandName, `${where}.packages[${index}].commandName`);
        if (pkg.requiredSections !== undefined) {
            entry.requiredSections = expectStringArray(pkg.requiredSections, `${where}.packages[${index}].requiredSections`);
        }
        if (pkg.maxFilesEntries !== undefined) {
            const max = expectNumber(pkg.maxFilesEntries, `${where}.packages[${index}].maxFilesEntries`);
            if (!Number.isInteger(max) || max < 1) invalid(`${where}.packages[${index}].maxFilesEntries`, 'must be a positive integer');
            entry.maxFilesEntries = max;
        }
        packages.push(entry);
    }
    return { packages };
}

function validateRelease(value: unknown, where: string): WaeReleaseConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['registry', 'tagPattern', 'packages'], where);
    const registry = expectString(obj.registry, `${where}.registry`);
    try {
        const url = new URL(registry);
        if (url.protocol !== 'https:' && url.protocol !== 'http:') invalid(`${where}.registry`, 'must be an http(s) URL');
    } catch {
        invalid(`${where}.registry`, `not a valid URL: \`${registry}\``);
    }
    const tagPattern = expectString(obj.tagPattern, `${where}.tagPattern`);
    try {
        new RegExp(tagPattern);
    } catch {
        invalid(`${where}.tagPattern`, `not a valid regular expression: \`${tagPattern}\``);
    }
    if (!Array.isArray(obj.packages) || obj.packages.length === 0) invalid(`${where}.packages`, 'expected a non-empty array');
    const packages: WaeReleasePackage[] = [];
    const names = new Set<string>();
    const dirs = new Set<string>();
    for (const [index, item] of obj.packages.entries()) {
        const pkg = expectObject(item, `${where}.packages[${index}]`);
        assertKeys(pkg, ['name', 'dir'], `${where}.packages[${index}]`);
        const dir = expectString(pkg.dir, `${where}.packages[${index}].dir`);
        if (dirs.has(dir)) invalid(`${where}.packages[${index}]`, `duplicate package dir \`${dir}\``);
        dirs.add(dir);
        const entry: WaeReleasePackage = { dir };
        if (pkg.name !== undefined) {
            entry.name = expectString(pkg.name, `${where}.packages[${index}].name`);
            if (names.has(entry.name)) invalid(`${where}.packages[${index}]`, `duplicate package \`${entry.name}\``);
            names.add(entry.name);
        }
        packages.push(entry);
    }
    return { registry, tagPattern, packages };
}

function validateCi(value: unknown, where: string): WaeCiConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['repo', 'workflows', 'requiredJobs', 'waitTimeoutMs', 'intervalMs', 'shaBinding'], where);
    const ci: WaeCiConfig = {};
    if (obj.repo !== undefined) ci.repo = expectString(obj.repo, `${where}.repo`);
    if (obj.workflows !== undefined) ci.workflows = expectStringArray(obj.workflows, `${where}.workflows`);
    if (obj.requiredJobs !== undefined) ci.requiredJobs = expectStringArray(obj.requiredJobs, `${where}.requiredJobs`);
    if (obj.waitTimeoutMs !== undefined) ci.waitTimeoutMs = expectNumber(obj.waitTimeoutMs, `${where}.waitTimeoutMs`);
    if (obj.intervalMs !== undefined) ci.intervalMs = expectNumber(obj.intervalMs, `${where}.intervalMs`);
    if (obj.shaBinding !== undefined) ci.shaBinding = expectBoolean(obj.shaBinding, `${where}.shaBinding`);
    return ci;
}

function validateInstall(value: unknown, where: string): WaeInstallConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['shortcutDefault', 'configLocation'], where);
    return {
        shortcutDefault: expectBoolean(obj.shortcutDefault, `${where}.shortcutDefault`),
        configLocation: expectEnum(obj.configLocation, ['per-user'] as const, `${where}.configLocation`),
    };
}

function validateAudit(value: unknown, where: string): WaeAuditConfig {
    const obj = expectObject(value, where);
    assertKeys(obj, ['actor'], where);
    const actor = expectObject(obj.actor, `${where}.actor`);
    assertKeys(actor, ['kind', 'id', 'source'], `${where}.actor`);
    const sources = ['flag', 'env', 'session', 'host'] as const;
    return {
        actor: {
            kind: expectEnum(actor.kind, sources, `${where}.actor.kind`),
            id: expectEnum(actor.id, sources, `${where}.actor.id`),
            source: expectEnum(actor.source, sources, `${where}.actor.source`),
        },
    };
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function expectObject(value: unknown, where: string): Record<string, unknown> {
    if (!isPlainObject(value)) invalid(where, 'expected an object');
    return value;
}

function expectString(value: unknown, where: string): string {
    if (typeof value !== 'string' || !value.trim()) invalid(where, 'expected a non-empty string');
    return value;
}

function expectStringArray(value: unknown, where: string): string[] {
    if (!Array.isArray(value) || value.length === 0) invalid(where, 'expected a non-empty array of strings');
    for (const [index, item] of value.entries()) {
        if (typeof item !== 'string' || !item.trim()) invalid(`${where}[${index}]`, 'expected a non-empty string');
    }
    return value as string[];
}

function expectEnum<T extends string>(value: unknown, allowed: readonly T[], where: string): T {
    if (typeof value !== 'string' || !(allowed as readonly string[]).includes(value)) {
        invalid(where, `expected one of ${allowed.join(', ')}`);
    }
    return value as T;
}

function expectEnumArray<T extends string>(value: unknown, allowed: readonly T[], where: string): T[] {
    if (!Array.isArray(value)) invalid(where, 'expected an array');
    for (const item of value) expectEnum(item, allowed, where);
    return value as T[];
}

function expectBoolean(value: unknown, where: string): boolean {
    if (typeof value !== 'boolean') invalid(where, 'expected a boolean');
    return value;
}

function expectNumber(value: unknown, where: string): number {
    if (typeof value !== 'number' || !Number.isFinite(value)) invalid(where, 'expected a finite number');
    return value;
}

function assertKeys(obj: Record<string, unknown>, allowed: readonly string[], where: string): void {
    for (const key of Object.keys(obj)) {
        if (!allowed.includes(key)) invalid(where, `unknown key \`${key}\``);
    }
}

function invalid(where: string, message: string): never {
    throw new WaeConfigError('config_invalid', `${where}: ${message}`, where.split('#')[0]);
}
