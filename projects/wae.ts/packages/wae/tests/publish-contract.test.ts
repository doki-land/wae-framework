import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildCli, readWorkspaceVersion, resolvePublishIntent } from '../src/cli.ts';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..');
const workflowsDir = path.join(repoRoot, '.github', 'workflows');
const publishWorkflow = fs.readFileSync(path.join(workflowsDir, 'publish-npm.yml'), 'utf8');
const cliSource = fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/cli.ts'), 'utf8');

/** Run the CLI in-process and capture its output. */
async function runCli(argv: string[]): Promise<{ code: number; out: string; err: string }> {
    const out: string[] = [];
    const err: string[] = [];
    const origLog = console.log;
    const origError = console.error;
    console.log = (...args: unknown[]) => out.push(args.map(String).join(' '));
    console.error = (...args: unknown[]) => err.push(args.map(String).join(' '));
    try {
        const code = await buildCli().parse(argv);
        return { code, out: out.join('\n'), err: err.join('\n') };
    } finally {
        console.log = origLog;
        console.error = origError;
    }
}

/** Temporary npm package plus Cargo workspace fixture. */
function makePublishFixture(
    version: string,
    cargoVersion: string,
    name = '@wae/publish-fixture',
): { dir: string; cargo: string; cleanup: () => void } {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-publish-'));
    const dir = path.join(tmp, 'pkg');
    fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name, version, license: 'MIT' }, null, 4));
    const cargo = path.join(tmp, 'Cargo.toml');
    fs.writeFileSync(cargo, `[workspace]\n\n[workspace.package]\nversion = "${cargoVersion}"\n`);
    return { dir, cargo, cleanup: () => fs.rmSync(tmp, { recursive: true, force: true }) };
}

test('publish workflow follows the Trusted Publisher (OIDC) contract', () => {
    assert.match(publishWorkflow, /on:\s*\n\s*push:\s*\n\s*tags:\s*\['v\*'\]/);
    assert.match(publishWorkflow, /workflow_dispatch:/);
    assert.match(publishWorkflow, /permissions:\s*\n\s*contents:\s*read\s*\n\s*id-token:\s*write/);
    assert.match(publishWorkflow, /environment:\s*NPM_PUBLISH/);
    assert.match(publishWorkflow, /pnpm exec nifty publish --access public --version=/);
    assert.match(publishWorkflow, /pnpm install --frozen-lockfile/);
    assert.doesNotMatch(publishWorkflow, /registry-url|--ci/);
    assert.match(cliSource, /env: process\.env/);
});

test('workflows never reintroduce npm token, OTP, or local publish bypasses', () => {
    const files = fs.readdirSync(workflowsDir).filter((file) => /\.(yml|yaml)$/.test(file));
    for (const file of files) {
        const text = fs.readFileSync(path.join(workflowsDir, file), 'utf8');
        assert.doesNotMatch(text, /NPM_TOKEN|NODE_AUTH_TOKEN/i, `${file} must not use npm token secrets`);
        assert.doesNotMatch(text, /--otp/i, `${file} must not use --otp`);
        assert.doesNotMatch(text, /\bnpm publish\b/, `${file} must publish through nifty publish`);
        assert.doesNotMatch(text, /--execute/, `${file} must not use the local placeholder publish path`);
    }
    assert.doesNotMatch(cliSource, /--otp|NODE_AUTH_TOKEN|--provenance/);
    assert.match(cliSource, /'--access',\s*'public'/);
});

test('publish workflow publishes the configured set and no template placeholder', () => {
    const configSource = fs.readFileSync(path.join(repoRoot, 'nifty.config.ts'), 'utf8');
    const configured = [...configSource.matchAll(/"(@wae\/[a-z0-9-]+)"/g)]
        .map((match) => match[1]!);
    assert.deepEqual(
        [...configured].sort(),
        ['@wae/commander', '@wae/wae', '@wae/wae-darwin-arm64', '@wae/wae-linux-x64', '@wae/wae-win32-x64'],
        'the configured publish set is the two JS packages plus the three native platform packages',
    );

    // The set lives in one place. The workflow must not enumerate packages itself.
    assert.doesNotMatch(publishWorkflow, /--package/);
    assert.doesNotMatch(publishWorkflow, /--dir/);
    assert.match(publishWorkflow, /pnpm exec nifty publish --access public --version=/);
    for (const target of ['win32-x64', 'darwin-arm64', 'linux-x64']) {
        assert.match(publishWorkflow, new RegExp(`wae-${target}`), `the workflow must build wae-${target} on its own runner`);
    }

    const workspaceFile = fs.readFileSync(path.join(repoRoot, 'pnpm-workspace.yaml'), 'utf8');
    assert.doesNotMatch(workspaceFile, /templates/);
    assert.doesNotMatch(publishWorkflow, /templates/);
});

test('platform packages describe their os, cpu, artifact and provenance', () => {
    const version = readWorkspaceVersion(path.join(repoRoot, 'Cargo.toml'));
    assert.ok(version, 'the Cargo workspace must declare a version');

    const platformsDir = path.join(repoRoot, 'projects', 'wae.ts', 'platforms');
    const expected: Record<string, { os: string; cpu: string }> = {
        'wae-win32-x64': { os: 'win32', cpu: 'x64' },
        'wae-darwin-arm64': { os: 'darwin', cpu: 'arm64' },
        'wae-linux-x64': { os: 'linux', cpu: 'x64' },
    };
    const entries = fs.readdirSync(platformsDir).filter((entry) => entry.startsWith('wae-'));
    assert.deepEqual([...entries].sort(), Object.keys(expected).sort());

    const hostManifest = JSON.parse(
        fs.readFileSync(path.join(repoRoot, 'projects', 'wae.ts', 'packages', 'wae', 'package.json'), 'utf8'),
    ) as { optionalDependencies?: Record<string, string> };

    for (const entry of entries) {
        const target = entry.slice('wae-'.length);
        const manifestPath = path.join(platformsDir, entry, 'package.json');
        const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as {
            name: string;
            version: string;
            license: string;
            main: string;
            files: string[];
            os: string[];
            cpu: string[];
            repository: { url: string; directory: string };
        };
        assert.equal(manifest.name, `@wae/${entry}`);
        assert.equal(manifest.version, version, `${manifest.name} version must match the workspace version`);
        assert.deepEqual(manifest.os, [expected[entry]!.os]);
        assert.deepEqual(manifest.cpu, [expected[entry]!.cpu]);

        const artifact = `wae.${target}.node`;
        assert.equal(manifest.main, artifact);
        assert.ok(manifest.files.includes(artifact), `${manifest.name} files must ship ${artifact}`);
        assert.equal(manifest.license, 'Apache-2.0');
        assert.equal(manifest.repository.url, 'git+https://github.com/doki-land/wae-framework.git');
        assert.equal(
            manifest.repository.directory,
            path.relative(repoRoot, path.dirname(manifestPath)).split(path.sep).join('/'),
            `${manifest.name} repository.directory must match its path`,
        );

        const optional = hostManifest.optionalDependencies?.[manifest.name];
        assert.equal(optional, version, `@wae/wae must pin ${manifest.name} to the exact workspace version`);
    }
});

test('published manifests point at the provenance repository', () => {
    const expectedUrl = 'git+https://github.com/doki-land/wae-framework.git';
    const packagesDir = path.join(repoRoot, 'projects', 'wae.ts', 'packages');
    const manifestPaths = fs.readdirSync(packagesDir)
        .map((entry) => path.join(packagesDir, entry, 'package.json'))
        .filter((manifestPath) => fs.existsSync(manifestPath));
    assert.ok(manifestPaths.length > 0);
    for (const manifestPath of manifestPaths) {
        const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as {
            name: string;
            private?: boolean;
            repository?: { url?: string; directory?: string };
        };
        if (manifest.private === true) continue;
        assert.equal(manifest.repository?.url, expectedUrl, `${manifest.name} repository.url must match the provenance repository`);
        const expectedDirectory = path.relative(repoRoot, path.dirname(manifestPath)).split(path.sep).join('/');
        assert.equal(manifest.repository?.directory, expectedDirectory, `${manifest.name} repository.directory must match its path`);
    }

    const rootManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, 'package.json'), 'utf8')) as {
        repository?: string | { url?: string };
    };
    const rootUrl = typeof rootManifest.repository === 'string'
        ? rootManifest.repository
        : rootManifest.repository?.url;
    assert.equal(
        rootUrl?.replace(/^git\+/, '').replace(/\.git$/, ''),
        'https://github.com/doki-land/wae-framework',
        'root repository must match the provenance repository',
    );
});

test('templates never hardcode the real repository', () => {
    const templatesDir = path.join(repoRoot, 'projects', 'wae.ts', 'templates');
    const manifestPaths: string[] = [];
    const walk = (dir: string): void => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) walk(full);
            else if (entry.name === 'package.json') manifestPaths.push(full);
        }
    };
    walk(templatesDir);
    assert.ok(manifestPaths.length > 0, 'expected at least one template manifest');
    for (const manifestPath of manifestPaths) {
        const text = fs.readFileSync(manifestPath, 'utf8');
        assert.doesNotMatch(
            text,
            /doki-land\/wae-framework|oovm\/wae/,
            `${path.relative(repoRoot, manifestPath)} must use a placeholder instead of the real repository`,
        );
    }
});

test('publish verifies tag, package, and Cargo versions without invoking npm', async () => {
    const fixture = makePublishFixture('0.1.0', '0.1.0');
    try {
        const result = await runCli([
            'publish',
            '--dir', fixture.dir,
            '--tag', 'v0.1.0',
            '--cargo', fixture.cargo,
        ]);
        assert.equal(result.code, 0);
        assert.match(result.out, /verified 1 package\(s\) at 0\.1\.0 for v0\.1\.0/);
        assert.match(result.out, /push tag v0\.1\.0/);
        assert.equal(result.err, '');
    } finally {
        fixture.cleanup();
    }
});

test('publish refuses local real-version publishing', async () => {
    const fixture = makePublishFixture('0.1.0', '0.1.0');
    try {
        const result = await runCli([
            'publish',
            '--dir', fixture.dir,
            '--tag', 'v0.1.0',
            '--cargo', fixture.cargo,
            '--execute',
        ]);
        assert.equal(result.code, 2);
        assert.match(result.err, /local publish of 0\.1\.0 is disabled/);
        assert.match(result.err, /push tag v0\.1\.0/);
    } finally {
        fixture.cleanup();
    }
});

test('publish rejects tag and package version mismatch', async () => {
    const fixture = makePublishFixture('0.1.0', '0.1.0');
    try {
        const result = await runCli([
            'publish',
            '--dir', fixture.dir,
            '--tag', 'v0.2.0',
            '--cargo', fixture.cargo,
        ]);
        assert.equal(result.code, 1);
        assert.match(result.err, /tag v0\.2\.0 does not match package version 0\.1\.0/);
    } finally {
        fixture.cleanup();
    }
});

test('publish rejects Cargo and npm version mismatch', async () => {
    const fixture = makePublishFixture('0.1.0', '0.2.0');
    try {
        const result = await runCli([
            'publish',
            '--dir', fixture.dir,
            '--tag', 'v0.1.0',
            '--cargo', fixture.cargo,
        ]);
        assert.equal(result.code, 1);
        assert.match(result.err, /Cargo \[workspace\.package\] version 0\.2\.0 does not match npm package version 0\.1\.0/);
    } finally {
        fixture.cleanup();
    }
});

test('publish rejects private packages', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-publish-private-'));
    try {
        fs.writeFileSync(path.join(tmp, 'package.json'), JSON.stringify({ name: '@wae/guide', version: '0.1.0', private: true }));
        const result = await runCli(['publish', '--dir', tmp]);
        assert.equal(result.code, 1);
        assert.match(result.err, /@wae\/guide is private and cannot be published/);
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('resolvePublishIntent keeps real-version publishing out of the local CLI', () => {
    assert.deepEqual(resolvePublishIntent('0.0.1', { execute: false, env: {} }), { kind: 'verify' });
    assert.equal(resolvePublishIntent('0.0.0', { execute: true, env: {} }).kind, 'placeholder');
    assert.equal(resolvePublishIntent('0.0.1', { execute: true, env: {} }).kind, 'refused');
});

test('readWorkspaceVersion reads only [workspace.package]', () => {
    const text = '[package]\nname = "x"\nversion = "9.9.9"\n\n[workspace.package]\nversion = "0.0.1"\n';
    const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'wae-cargo-')), 'Cargo.toml');
    fs.writeFileSync(file, text);
    try {
        assert.equal(readWorkspaceVersion(file), '0.0.1');
        assert.equal(readWorkspaceVersion(path.join(path.dirname(file), 'missing.toml')), undefined);
    } finally {
        fs.rmSync(path.dirname(file), { recursive: true, force: true });
    }
});
