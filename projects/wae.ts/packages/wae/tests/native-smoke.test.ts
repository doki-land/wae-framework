import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildCli } from '../src/cli.ts';
import { WAE_CONFIG_FILE } from '../src/config.ts';

const PACKAGE_ROOT = fileURLToPath(new URL('..', import.meta.url));
const PLATFORM_KEY = `${process.platform}-${process.arch}`;

/** The staged addon for this host, so the smoke runs against the real Node-API surface. */
function hostAddon(): { dir: string; nodeFile: string } | undefined {
    const dir = path.resolve(PACKAGE_ROOT, '..', '..', 'platforms', `wae-${PLATFORM_KEY}`);
    const manifestPath = path.join(dir, 'package.json');
    if (!fs.existsSync(manifestPath)) {
        return undefined;
    }
    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as { main?: string };
    if (typeof manifest.main !== 'string' || !fs.existsSync(path.join(dir, manifest.main))) {
        return undefined;
    }
    return { dir: dir.replaceAll('\\', '/'), nodeFile: manifest.main };
}

const HOST_ADDON = hostAddon();
const SKIP = HOST_ADDON === undefined ? `no staged native addon for \`${PLATFORM_KEY}\`` : false;

const TMP_VARS = ['TMPDIR', 'TMP', 'TEMP'] as const;

/**
 * Point `os.tmpdir()` at a private directory for this test only.
 *
 * Test files run in parallel processes and share the host temp directory, so a global
 * before/after comparison would be polluted by unrelated runs. Isolating the temp root keeps
 * the audit-directory assertions exact and order independent.
 */
function privateTmp(): { dir: string; restore: () => void } {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-smoke-tmp-'));
    const saved = TMP_VARS.map((key) => [key, process.env[key]] as const);
    for (const key of TMP_VARS) {
        process.env[key] = dir;
    }
    return {
        dir,
        restore: () => {
            for (const [key, value] of saved) {
                if (value === undefined) {
                    delete process.env[key];
                } else {
                    process.env[key] = value;
                }
            }
            fs.rmSync(dir, { recursive: true, force: true });
        },
    };
}

function auditTempDirs(dir: string): string[] {
    return fs.readdirSync(dir)
        .filter((name) => name.startsWith('wae-native-audit-'))
        .sort();
}

function writeConfig(root: string, dir: string, nodeFile: string, manifestVersion?: string): void {
    const packageName = `@scope/tool-${PLATFORM_KEY}`;
    fs.writeFileSync(path.join(root, WAE_CONFIG_FILE), `
export default defineConfig({
    native: {
        crate: 'tool-napi',
        targets: [{
            platform: '${process.platform}',
            arch: '${process.arch}',
            triple: 'x86_64-unknown-linux-gnu',
            package: '${packageName}',
            nodeFile: '${nodeFile}',
            dir: '${dir}',
        }],
    },
    packages: [{
        name: '${packageName}',
        dir: '${dir}',
        os: ['${process.platform}'],
        cpu: ['${process.arch}'],
        main: '${nodeFile}',
    }],
});
`);
    if (manifestVersion === undefined) {
        return;
    }
    const manifestDir = path.resolve(root, dir);
    fs.mkdirSync(manifestDir, { recursive: true });
    fs.writeFileSync(path.join(manifestDir, 'package.json'), JSON.stringify({
        name: packageName,
        version: manifestVersion,
        os: [process.platform],
        cpu: [process.arch],
        main: nodeFile,
    }));
}

/** A throwaway project that points at the host platform package, so the project itself is removable. */
function smokeProject(): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-smoke-'));
    writeConfig(root, HOST_ADDON!.dir, HOST_ADDON!.nodeFile);
    return root;
}

/** Silence the CLI failure report so the test output stays readable. */
async function quietly(run: () => Promise<number>): Promise<number> {
    const original = console.error;
    console.error = () => {};
    try {
        return await run();
    } finally {
        console.error = original;
    }
}

test('native smoke removes its temporary audit directory and leaves the environment unset', { skip: SKIP }, async () => {
    const inherited = process.env.WAE_AUDIT_DIR;
    delete process.env.WAE_AUDIT_DIR;
    const tmp = privateTmp();
    try {
        const root = smokeProject();
        assert.deepEqual(auditTempDirs(tmp.dir), []);
        const code = await buildCli().parse(['native', 'smoke', '--project', root]);
        assert.equal(code, 0);
        assert.equal(process.env.WAE_AUDIT_DIR, undefined);
        assert.deepEqual(auditTempDirs(tmp.dir), []);
    } finally {
        tmp.restore();
        if (inherited !== undefined) {
            process.env.WAE_AUDIT_DIR = inherited;
        }
    }
});

test('native smoke keeps an inherited audit directory untouched', { skip: SKIP }, async () => {
    const inherited = process.env.WAE_AUDIT_DIR;
    const tmp = privateTmp();
    try {
        const auditDir = fs.mkdtempSync(path.join(tmp.dir, 'wae-audit-inherited-'));
        fs.writeFileSync(path.join(auditDir, 'probe.jsonl'), '');
        process.env.WAE_AUDIT_DIR = auditDir;
        const root = smokeProject();
        const code = await buildCli().parse(['native', 'smoke', '--project', root]);
        assert.equal(code, 0);
        assert.equal(process.env.WAE_AUDIT_DIR, auditDir);
        assert.deepEqual(auditTempDirs(tmp.dir), []);
        assert.equal(fs.existsSync(path.join(auditDir, 'probe.jsonl')), true);
    } finally {
        tmp.restore();
        if (inherited === undefined) {
            delete process.env.WAE_AUDIT_DIR;
        } else {
            process.env.WAE_AUDIT_DIR = inherited;
        }
    }
});

test('native smoke cleans up when the run fails midway', { skip: SKIP }, async () => {
    const inherited = process.env.WAE_AUDIT_DIR;
    delete process.env.WAE_AUDIT_DIR;
    const tmp = privateTmp();
    try {
        const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-smoke-'));
        writeConfig(root, 'platforms/tool', 'broken.node', '0.0.3');
        fs.writeFileSync(path.join(root, 'platforms', 'tool', 'broken.node'), 'not an addon');
        assert.deepEqual(auditTempDirs(tmp.dir), []);
        const code = await quietly(() => buildCli().parse(['native', 'smoke', '--project', root]));
        assert.equal(code, 1);
        assert.equal(process.env.WAE_AUDIT_DIR, undefined);
        assert.deepEqual(auditTempDirs(tmp.dir), []);
    } finally {
        tmp.restore();
        if (inherited !== undefined) {
            process.env.WAE_AUDIT_DIR = inherited;
        }
    }
});
