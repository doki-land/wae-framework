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
    assert.match(publishWorkflow, /registry-url:\s*https:\/\/registry\.npmjs\.org/);
    assert.match(publishWorkflow, /--ci/);
    assert.match(publishWorkflow, /refs\/tags\/\$\{RELEASE_TAG\}/);
});

test('workflows never reintroduce npm token, OTP, or local publish bypasses', () => {
    const files = fs.readdirSync(workflowsDir).filter((file) => /\.(yml|yaml)$/.test(file));
    for (const file of files) {
        const text = fs.readFileSync(path.join(workflowsDir, file), 'utf8');
        assert.doesNotMatch(text, /NPM_TOKEN|NODE_AUTH_TOKEN/i, `${file} must not use npm token secrets`);
        assert.doesNotMatch(text, /--otp/i, `${file} must not use --otp`);
        assert.doesNotMatch(text, /\bnpm publish\b/, `${file} must publish through wae publish --ci`);
        assert.doesNotMatch(text, /--execute/, `${file} must not use the local placeholder publish path`);
    }
    assert.doesNotMatch(cliSource, /--otp|NODE_AUTH_TOKEN/);
    assert.match(cliSource, /'--provenance'/);
    assert.match(cliSource, /'--access',\s*'public'/);
});

test('publish workflow covers every real @wae package and no template placeholder', () => {
    const packagesDir = path.join(repoRoot, 'projects', 'wae.ts', 'packages');
    const platformsDir = path.join(packagesDir, 'platforms');
    const packageDirs = fs.readdirSync(packagesDir)
        .map((entry) => path.join(packagesDir, entry))
        .concat(fs.readdirSync(platformsDir).map((entry) => path.join(platformsDir, entry)));
    const realNames = packageDirs
        .map((dir) => path.join(dir, 'package.json'))
        .filter((manifest) => fs.existsSync(manifest))
        .map((manifest) => JSON.parse(fs.readFileSync(manifest, 'utf8')) as { name: string; private?: boolean })
        .filter((manifest) => manifest.private !== true)
        .map((manifest) => manifest.name);
    assert.ok(realNames.length >= 2, 'expected at least @wae/wae and @wae/commander');
    for (const name of realNames) {
        assert.match(name, /^@wae\//);
    }

    const dirs = new Set([...publishWorkflow.matchAll(/--dir\s+([^\s\\]+)/g)].map((match) => match[1]!));
    const publishedNames = [...dirs].map((dir) => {
        const manifest = JSON.parse(fs.readFileSync(path.join(repoRoot, dir, 'package.json'), 'utf8')) as { name: string };
        return manifest.name;
    });
    assert.deepEqual(new Set(publishedNames), new Set(realNames));
    for (const dir of dirs) {
        assert.ok(!dir.includes('/templates/'), `template placeholder must not be published: ${dir}`);
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

test('publish refuses --ci outside a GitHub Actions OIDC runner', async () => {
    const fixture = makePublishFixture('0.1.0', '0.1.0');
    const saved = {
        actions: process.env.GITHUB_ACTIONS,
        url: process.env.ACTIONS_ID_TOKEN_REQUEST_URL,
        token: process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN,
    };
    delete process.env.ACTIONS_ID_TOKEN_REQUEST_URL;
    delete process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN;
    try {
        const result = await runCli([
            'publish',
            '--dir', fixture.dir,
            '--tag', 'v0.1.0',
            '--cargo', fixture.cargo,
            '--ci',
        ]);
        assert.equal(result.code, 2);
        assert.match(result.err, /Trusted Publisher OIDC/);
    } finally {
        restoreEnv('GITHUB_ACTIONS', saved.actions);
        restoreEnv('ACTIONS_ID_TOKEN_REQUEST_URL', saved.url);
        restoreEnv('ACTIONS_ID_TOKEN_REQUEST_TOKEN', saved.token);
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

test('resolvePublishIntent keeps real-version publishing inside OIDC only', () => {
    const oidcEnv = {
        GITHUB_ACTIONS: 'true',
        ACTIONS_ID_TOKEN_REQUEST_URL: 'https://example.invalid/oidc',
        ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'token',
    } as NodeJS.ProcessEnv;
    assert.deepEqual(resolvePublishIntent('0.0.1', { execute: false, ci: false, env: {} }), { kind: 'verify' });
    assert.equal(resolvePublishIntent('0.0.0', { execute: true, ci: false, env: {} }).kind, 'placeholder');
    assert.equal(resolvePublishIntent('0.0.1', { execute: true, ci: false, env: {} }).kind, 'refused');
    assert.equal(resolvePublishIntent('0.0.1', { execute: false, ci: true, env: {} }).kind, 'refused');
    assert.equal(resolvePublishIntent('0.0.1', { execute: false, ci: true, env: oidcEnv }).kind, 'ci');
    assert.equal(resolvePublishIntent('0.0.1', { execute: true, ci: true, env: oidcEnv }).kind, 'refused');
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

function restoreEnv(key: string, value: string | undefined): void {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
}
