/**
 * Validate agent skills against the real CLI command tree.
 *
 * Ported from the open-app template script so `wae build` owns the check. The
 * rules are unchanged: frontmatter, `## Safety`, dry-run-before-mutation, command
 * and flag existence, no out-of-package references, version and license parity.
 */
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import type { Cli, CliSpec } from '@wae/commander';
import { BUILD_CODES, WaeBuildError } from './build-error.js';
import type { WaeSkillsPackage } from './config.js';

/** Commands that must document a `--dry-run` invocation before execution. */
const MUTATING_COMMANDS = new Set(['install', 'uninstall']);

export type SkillsAuditResult = {
    skills: number;
    invocations: number;
    commands: number;
    log: string[];
};

type AuditContext = {
    root: string;
    pkg: WaeSkillsPackage;
    skillsRoot: string;
    commandName: string;
    cli: string;
    log: string[];
};

export async function auditSkills(root: string, pkg: WaeSkillsPackage, onLine: (line: string) => void = () => {}): Promise<SkillsAuditResult> {
    const log: string[] = [];
    const skillsRoot = path.resolve(root, pkg.dir);
    const appManifestPath = pkg.appPackageJson ? path.resolve(root, pkg.appPackageJson) : undefined;
    const skillsManifestPath = pkg.packageJson ? path.resolve(root, pkg.packageJson) : undefined;
    const appName = appManifestPath && fs.existsSync(appManifestPath) ? readName(appManifestPath) : '';
    const commandName = pkg.commandName ?? (appName ? appName.replace(/^@[^/]+\//, '') : '');
    if (!commandName) {
        fail('skills config needs `commandName` or an app manifest with a package name');
    }
    if (!pkg.cliEntry) {
        fail('skills config needs `cliEntry` pointing at the built CLI');
    }
    const cli = path.resolve(root, pkg.cliEntry);

    const context: AuditContext = { root, pkg, skillsRoot, commandName, cli, log };
    await runAudit(context, appManifestPath, skillsManifestPath);

    const result: SkillsAuditResult = {
        skills: log.filter((line) => line.startsWith('ok ')).length,
        invocations: countInvocations(log),
        commands: Number(log.find((line) => line.startsWith('commands='))?.slice('commands='.length) ?? 0),
        log,
    };
    for (const line of log) onLine(line);
    return result;
}

async function runAudit(context: AuditContext, appManifestPath?: string, skillsManifestPath?: string): Promise<void> {
    if (!fs.existsSync(context.cli)) {
        throw new WaeBuildError(
            BUILD_CODES.artifactMissing,
            `skills CLI build is missing at ${context.cli}; the \`ts\` step must run first`,
            { attempts: [context.cli] },
        );
    }
    if (!fs.existsSync(context.skillsRoot)) {
        fail(`skills directory is missing: ${context.skillsRoot}`);
    }

    const commands = await commandTree(context);
    if (appManifestPath && skillsManifestPath) {
        const app = readJson(appManifestPath);
        const skills = readJson(skillsManifestPath);
        if (skills.license !== 'Apache-2.0') fail('skills package `license` must be `Apache-2.0`');
        if (skills.version !== app.version) {
            fail(`skills package version \`${String(skills.version)}\` must match the app version \`${String(app.version)}\``);
        }
        const files = skills.files;
        if (!Array.isArray(files) || !files.includes('skills') || files.length > (context.pkg.maxFilesEntries ?? 2)) {
            fail(`skills package \`files\` must contain \`skills\` plus at most ${(context.pkg.maxFilesEntries ?? 2) - 1} readme`);
        }
    }

    const skillDirs = fs
        .readdirSync(context.skillsRoot, { withFileTypes: true })
        .filter((entry) => entry.isDirectory())
        .map((entry) => entry.name)
        .sort();
    if (skillDirs.length === 0) fail('no skill directories found');

    let checkedInvocations = 0;
    for (const dir of skillDirs) {
        const file = path.join(context.skillsRoot, dir, 'SKILL.md');
        if (!fs.existsSync(file)) fail(`${dir}: missing SKILL.md`);
        const source = fs.readFileSync(file, 'utf8');

        const { meta, body } = parseFrontmatter(source, `skills/${dir}/SKILL.md`);
        if (!meta.name) fail(`${dir}: frontmatter is missing \`name\``);
        if (!meta.description) fail(`${dir}: frontmatter is missing \`description\``);
        if (!/^[a-z0-9][a-z0-9-]*$/.test(meta.name)) fail(`${dir}: \`name\` must be kebab-case`);
        if (meta.name !== dir) fail(`${dir}: frontmatter \`name\` does not match the directory`);

        for (const section of context.pkg.requiredSections ?? ['## Safety']) {
            const title = section.replace(/^#+\s*/, '');
            const pattern = new RegExp(`^##\\s+${escapeRegExp(title)}\\b`, 'm');
            if (!pattern.test(body)) fail(`${dir}: missing a \`${section}\` section`);
        }
        if (!/confirm|确认/i.test(body)) fail(`${dir}: safety must require explicit confirmation before mutating commands`);
        assertNoOutOfPackageReferences(source, `skills/${dir}/SKILL.md`, path.join(context.skillsRoot, dir), context.skillsRoot);

        const invocationPattern = new RegExp('`(' + escapeRegExp(context.commandName) + ' [^`\\n]+)`', 'g');
        const invocations = [...source.matchAll(invocationPattern)].map((match) => match[1]!.trim());
        if (invocations.length === 0) fail(`${dir}: no backticked \`${context.commandName}\` invocation found`);

        const dryRunCommands = new Set<string>();
        const referencedMutating = new Set<string>();
        for (const invocation of invocations) {
            const tokens = invocation.split(/\s+/);
            const command = tokens[1];
            const allowed = command === undefined ? undefined : commands.get(command);
            if (command === undefined || allowed === undefined) {
                fail(`${dir}: unknown command \`${command ?? '<empty>'}\` in \`${invocation}\``);
            }
            const flags: string[] = [];
            for (const token of tokens.slice(2)) {
                if (token.startsWith('<')) continue;
                if (token.startsWith('--')) {
                    const flag = token.split('=')[0]!;
                    if (!allowed.has(flag)) fail(`${dir}: flag \`${flag}\` is not defined for \`${command}\``);
                    flags.push(flag);
                    continue;
                }
                fail(`${dir}: unexpected positional token \`${token}\` in \`${invocation}\``);
            }
            if (MUTATING_COMMANDS.has(command)) {
                referencedMutating.add(command);
                if (flags.includes('--dry-run')) dryRunCommands.add(command);
            }
            checkedInvocations += 1;
        }

        for (const command of referencedMutating) {
            if (!dryRunCommands.has(command)) {
                fail(`${dir}: \`${command}\` must document a \`--dry-run\` invocation before execution`);
            }
        }

        context.log.push(`ok ${dir}: ${invocations.length} invocation(s)`);
    }

    context.log.push(`commands=${String(commands.size)}`);
    context.log.push(
        `skills=${String(skillDirs.length)} invocations=${String(checkedInvocations)} commands=${String(commands.size)}`,
    );
}

function countInvocations(log: string[]): number {
    let total = 0;
    for (const line of log) {
        const match = /^ok .*: (\d+) invocation\(s\)/.exec(line);
        if (match) total += Number(match[1]);
    }
    return total;
}

/** Command name → allowed flags, derived from the CLI module itself. */
type CommandTree = Map<string, Set<string>>;

async function commandTree(context: AuditContext): Promise<CommandTree> {
    const cli = await loadCli(context);
    let spec: CliSpec;
    try {
        spec = cli.describe();
    } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        fail(`cannot describe \`${context.commandName}\`: ${detail}`);
    }
    const commands: CommandTree = new Map();
    for (const command of spec.commands) {
        const name = command.names[0];
        if (name === undefined) continue;
        commands.set(name, new Set([...spec.options, '--help', ...command.options]));
    }
    if (commands.size === 0) fail(`no commands defined by \`${context.commandName}\``);
    return commands;
}

/** Import the built CLI in-process so no subprocess receives capabilities. */
async function loadCli(context: AuditContext): Promise<Cli> {
    let module: { cli?: Cli };
    try {
        module = (await import(pathToFileURL(context.cli).href)) as { cli?: Cli };
    } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        fail(`cannot load \`${context.cli}\`: ${detail}`);
    }
    const cli = module.cli;
    if (cli === undefined || typeof cli.describe !== 'function') {
        fail(`\`${context.cli}\` must export \`cli\` created with \`@wae/commander\``);
    }
    return cli;
}

function assertNoOutOfPackageReferences(source: string, file: string, dir: string, skillsRoot: string): void {
    for (const match of source.matchAll(/\]\(([^)\s]+)(?:\s+[^)]*)?\)/g)) {
        const target = match[1]!;
        if (/^(?:https?:|mailto:)/i.test(target) || target.startsWith('#')) continue;
        if (/^(?:[a-z]:[\\/]|[\\/]|~[\\/]|file:)/i.test(target) || target.includes('..')) {
            fail(`${file}: link target \`${target}\` must stay inside the skills package`);
        }
        const resolved = path.resolve(dir, target.split('#')[0]!);
        const relative = path.relative(skillsRoot, resolved);
        if (relative.startsWith('..') || path.isAbsolute(relative) || !fs.existsSync(resolved)) {
            fail(`${file}: link target \`${target}\` is not a file inside the skills package`);
        }
    }
    for (const match of source.matchAll(/(?:^|[\s`"'(（])((?:[a-z]:[\\/]|~[\\/]|file:\/\/)[^\s`"'()）]+)/gi)) {
        fail(`${file}: filesystem path \`${match[1]}\` must not appear in a published skill`);
    }
}

type Frontmatter = Record<string, string>;

function parseFrontmatter(source: string, file: string): { meta: Frontmatter; body: string } {
    const lines = source.split(/\r?\n/);
    if (lines[0]?.trim() !== '---') fail(`${file}: frontmatter must start on line 1 with \`---\``);
    const end = lines.findIndex((line, index) => index > 0 && line.trim() === '---');
    if (end < 0) fail(`${file}: frontmatter is not closed with \`---\``);
    const meta: Frontmatter = {};
    for (const line of lines.slice(1, end)) {
        if (!line.trim()) continue;
        const match = line.match(/^([a-zA-Z][a-zA-Z0-9_-]*):\s*(.*)$/);
        if (!match) fail(`${file}: cannot parse frontmatter line \`${line}\``);
        meta[match[1]!] = match[2]!.trim().replace(/^['"]|['"]$/g, '');
    }
    return { meta, body: lines.slice(end + 1).join('\n') };
}

function readJson(file: string): Record<string, unknown> {
    try {
        return JSON.parse(fs.readFileSync(file, 'utf8')) as Record<string, unknown>;
    } catch (error) {
        fail(`cannot read ${file}: ${error instanceof Error ? error.message : String(error)}`);
    }
}

function readName(file: string): string {
    const manifest = readJson(file);
    return typeof manifest.name === 'string' ? manifest.name : '';
}

function escapeRegExp(text: string): string {
    return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function fail(message: string): never {
    throw new WaeBuildError(BUILD_CODES.skillsInvalid, message);
}
