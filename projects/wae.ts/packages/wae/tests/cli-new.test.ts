import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { buildCli } from '../src/cli.ts';
import { templateRoot } from '../src/paths.ts';

const PACKAGE_ROOT = fileURLToPath(new URL('..', import.meta.url));
const VENDOR_SCRIPT = path.join(PACKAGE_ROOT, 'scripts', 'vendor-templates.mjs');
const CHECKOUT_TEMPLATES = path.resolve(PACKAGE_ROOT, '..', '..', 'templates');
const VENDORED_TEMPLATES = path.join(PACKAGE_ROOT, 'templates');

test('wae new scaffolds without gui by default', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-'));
    const name = 'demo-tool';
    const logs: string[] = [];
    const orig = console.log;
    console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
    try {
        const code = await buildCli().parse(['new', name, '--out', tmp]);
        assert.equal(code, 0);
        const dest = path.join(tmp, name);
        assert.ok(fs.existsSync(path.join(dest, 'package.json')));
        const pkg = JSON.parse(fs.readFileSync(path.join(dest, 'package.json'), 'utf8')) as { name: string };
        assert.equal(pkg.name, name);
        assert.equal(fs.existsSync(path.join(dest, 'gui')), false);
    } finally {
        console.log = orig;
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('wae new --with-gui keeps gui folder', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-gui-'));
    const name = 'demo-gui';
    try {
        const code = await buildCli().parse(['new', name, '--out', tmp, '--with-gui']);
        assert.equal(code, 0);
        assert.ok(fs.existsSync(path.join(tmp, name, 'gui')));
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

function assertNoPlaceholders(dir: string): void {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        assert.equal(entry.name.includes('{{name'), false, `unreplaced placeholder in path: ${full}`);
        if (entry.isDirectory()) {
            assertNoPlaceholders(full);
            continue;
        }
        const source = fs.readFileSync(full, 'utf8');
        // GitHub Actions uses `${{ ... }}`, so only the scaffold tokens are checked.
        assert.equal(source.includes('{{name'), false, `unreplaced placeholder in file: ${full}`);
    }
}

test('wae new --template open-app scaffolds the full application skeleton', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-app-'));
    const name = 'demo-app';
    try {
        const code = await buildCli().parse(['new', name, '--out', tmp, '--template', 'open-app']);
        assert.equal(code, 0);
        const dest = path.join(tmp, name);
        assert.ok(fs.existsSync(path.join(dest, 'Cargo.toml')));
        assert.ok(fs.existsSync(path.join(dest, 'crates', 'core', 'Cargo.toml')));
        assert.ok(fs.existsSync(path.join(dest, 'crates', 'napi', 'Cargo.toml')));
        const cliFile = path.join(dest, 'packages', name, 'src', 'cli.ts');
        assert.ok(fs.existsSync(cliFile));
        const uiConfig = path.join(dest, 'packages', name + '-ui', 'vmz.config.ts');
        assert.ok(fs.existsSync(uiConfig));
        const skillFile = path.join(dest, 'packages', `${name}-skills`, 'skills', 'workflow', 'SKILL.md');
        assert.ok(fs.existsSync(skillFile));
        const platformPackage = path.join(dest, 'packages', `${name}-win32-x64`, 'package.json');
        assert.ok(fs.existsSync(platformPackage));
        const app = JSON.parse(fs.readFileSync(path.join(dest, 'packages', name, 'package.json'), 'utf8')) as {
            name: string;
            bin: Record<string, string>;
        };
        assert.equal(app.name, `@wae/${name}`);
        assert.equal(app.bin[name], './dist/cli.js');
        const core = fs.readFileSync(path.join(dest, 'crates', 'core', 'Cargo.toml'), 'utf8');
        assert.match(core, /name = "demo_app-core"/);
        assertNoPlaceholders(dest);
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('templateRoot falls back to the templates vendored into the tarball', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-templates-'));
    try {
        const pkgRoot = path.join(tmp, 'package');
        fs.mkdirSync(path.join(pkgRoot, 'templates', 'open-tools'), { recursive: true });
        // `from` points at `<pkgRoot>/dist/paths.js`: the checkout sibling does not exist there,
        // exactly like an installed `@wae/wae`.
        const from = pathToFileURL(path.join(pkgRoot, 'dist', 'paths.js')).href;
        assert.equal(templateRoot('open-tools', from), path.join(pkgRoot, 'templates', 'open-tools'));
        assert.equal(templateRoot('open-app', from), path.join(pkgRoot, 'templates', 'open-app'));
    } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});

test('prepack vendors templates without touching the checkout copy', () => {
    const result = spawnSync(process.execPath, [VENDOR_SCRIPT], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    for (const template of ['open-tools', 'open-app']) {
        assert.ok(fs.existsSync(path.join(VENDORED_TEMPLATES, template, 'package.json')));
        assert.ok(fs.existsSync(path.join(CHECKOUT_TEMPLATES, template, 'package.json')));
    }
});

test('prepack refuses a templates directory that redirects outside the package', () => {
    const outside = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-vendor-outside-'));
    fs.writeFileSync(path.join(outside, 'marker.txt'), 'keep');
    if (fs.existsSync(VENDORED_TEMPLATES)) {
        fs.rmSync(VENDORED_TEMPLATES, { recursive: true, force: true });
    }
    fs.symlinkSync(outside, VENDORED_TEMPLATES, 'junction');
    try {
        const result = spawnSync(process.execPath, [VENDOR_SCRIPT], { encoding: 'utf8' });
        assert.equal(result.status, 1, result.stdout);
        assert.match(result.stderr, /refusing to replace/);
        assert.deepEqual(fs.readdirSync(outside), ['marker.txt']);
    } finally {
        fs.rmSync(VENDORED_TEMPLATES, { recursive: true, force: true });
        fs.rmSync(outside, { recursive: true, force: true });
    }
});

test('wae new rejects unknown templates and invalid names', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-new-bad-'));
    const errors: string[] = [];
    const orig = console.error;
    console.error = (...a: unknown[]) => errors.push(a.map(String).join(' '));
    try {
        assert.equal(await buildCli().parse(['new', 'demo-app', '--out', tmp, '--template', 'nope']), 2);
        assert.equal(await buildCli().parse(['new', 'Demo_App', '--out', tmp]), 2);
        assert.equal(errors.length, 2);
    } finally {
        console.error = orig;
        fs.rmSync(tmp, { recursive: true, force: true });
    }
});
