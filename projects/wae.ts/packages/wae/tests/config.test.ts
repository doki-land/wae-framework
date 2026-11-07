import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
    discoverConfig,
    loadConfig,
    parseConfigSource,
    parseStaticObject,
    validateConfig,
    WaeConfigError,
    WAE_CONFIG_FILE,
} from '../src/config.ts';
import { runBuild } from '../src/build.ts';
import { BUILD_CODES, WaeBuildError } from '../src/build-error.ts';

function tempDir(prefix = 'wae-config-'): string {
    return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

function writeFile(file: string, content: string): void {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, content);
}

function json(file: string, value: unknown): void {
    writeFile(file, JSON.stringify(value, null, 2));
}

const FULL_CONFIG = `
export default defineConfig({
    native: {
        crate: 'tool-napi',
        profile: 'release',
        targetEnv: 'WAE_NATIVE_TARGET',
        crossCompile: 'reject',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@scope/tool-win32-x64',
                nodeFile: 'tool.win32-x64.node',
                dir: 'platforms/tool-win32-x64',
            },
        ],
    },
    packages: [
        {
            name: '@scope/tool-win32-x64',
            dir: 'platforms/tool-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: 'tool.win32-x64.node',
        },
    ],
    build: {
        ts: { packages: ['@scope/tool'] },
        ui: { package: '@scope/tool-ui' },
        stage: [{ from: 'apps/tool-ui/dist', to: 'apps/tool/dist/ui', clean: true }],
        outputs: ['apps/tool/dist/ui/index.html'],
    },
    skills: { packages: [{ dir: 'packages/tool-skills/skills' }] },
    release: {
        registry: 'https://registry.npmjs.org',
        tagPattern: '^v(\\\\d+\\\\.\\\\d+\\\\.\\\\d+)$',
        packages: [
            { name: '@scope/tool-win32-x64', dir: 'platforms/tool-win32-x64' },
            { name: '@scope/tool', dir: 'packages/tool' },
            { name: '@scope/tool-ui', dir: 'packages/tool-ui' },
        ],
    },
    ci: { repo: 'scope/tool', workflows: ['check-rust.yml'], shaBinding: true },
    install: { shortcutDefault: true, configLocation: 'per-user' },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'env' } },
});
`;

test('static config parser accepts every documented key', () => {
    const config = parseConfigSource(FULL_CONFIG);
    assert.equal(config.native?.crate, 'tool-napi');
    assert.equal(config.native?.targets.length, 1);
    assert.equal(config.packages?.length, 1);
    assert.deepEqual(config.build?.ts?.packages, ['@scope/tool']);
    assert.equal(config.build?.ui?.package, '@scope/tool-ui');
    assert.equal(config.skills?.packages[0]?.dir, 'packages/tool-skills/skills');
    assert.equal(config.release?.packages.length, 3);
    assert.equal(config.ci?.shaBinding, true);
    assert.equal(config.install?.configLocation, 'per-user');
    assert.equal(config.audit?.actor.kind, 'flag');
});

test('static parser rejects executable values', () => {
    const cases: Array<[string, RegExp]> = [
        ['export default defineConfig({ build: { ts: { packages: readPackages() } } });', /non-static/],
        ['export default defineConfig({ build: { outputs: [exec("ls")] } });', /non-static/],
        ['export default defineConfig({ native: { crate: `a${"x"}` } });', /template interpolation/],
        ['export default defineConfig({ ...base });', /object key/],
        ['const name = "x"; export default defineConfig({ native: { crate: name } });', /expected `export`/],
    ];
    for (const [source, pattern] of cases) {
        assert.throws(
            () => parseConfigSource(source),
            (error: unknown) => error instanceof WaeConfigError
                && error.code === 'config_not_static'
                && pattern.test(error.message),
            source,
        );
    }
});

test('static parser accepts plain export default and satisfies', () => {
    assert.deepEqual(parseStaticObject(`export default { publish: { packages: ["a"] } };`).publish, {
        packages: ['a'],
    });
    const config = parseConfigSource(`export default { install: { shortcutDefault: false, configLocation: 'per-user' } } satisfies WaeConfig;`);
    assert.equal(config.install?.shortcutDefault, false);
});

test('unknown keys fail as config_invalid', () => {
    assert.throws(
        () => parseConfigSource(`export default defineConfig({ buildNative: {} });`),
        (error: unknown) => error instanceof WaeConfigError && /unknown key `buildNative`/.test(error.message),
    );
    assert.throws(
        () => parseConfigSource(`export default defineConfig({ build: { uiBuild: {} } });`),
        (error: unknown) => error instanceof WaeConfigError && /unknown key `uiBuild`/.test(error.message),
    );
});

test('a narrowed `build.steps` list must include the steps it depends on', () => {
    assert.throws(
        () => parseConfigSource(`export default defineConfig({ build: { steps: [{ name: 'ts', dependsOn: ['native'] }] } });`),
        (error: unknown) => error instanceof WaeConfigError && /does not run/.test(error.message),
    );
    const config = parseConfigSource(`export default defineConfig({ build: { steps: [{ name: 'native' }, { name: 'ts', dependsOn: ['native'] }] } });`);
    assert.deepEqual(config.build?.steps?.map((step) => step.name), ['native', 'ts']);
});

test('staging targets must stay inside the project root', async () => {
    const cases: Array<[string, RegExp]> = [
        ['.', /not the project root/],
        ['..', /stay inside the project root/],
        ['../outside', /stay inside the project root/],
        ['/outside', /not an absolute path/],
        ['C:/outside', /not an absolute path/],
    ];
    for (const [to, pattern] of cases) {
        assert.throws(
            () => parseConfigSource(`export default defineConfig({ build: { stage: [{ from: 'dist', to: '${to}' }] } });`),
            (error: unknown) => error instanceof WaeConfigError && error.code === 'config_invalid' && pattern.test(error.message),
            to,
        );
    }
    const accepted = parseConfigSource(`export default defineConfig({ build: { stage: [{ from: 'dist', to: 'dist/gui', clean: true }] } });`);
    assert.equal(accepted.build?.stage?.[0]?.to, 'dist/gui');

    // A real build must refuse the config before `clean` can touch the project root.
    const root = tempDir();
    try {
        writeFile(path.join(root, 'marker.txt'), 'keep');
        writeFile(
            path.join(root, WAE_CONFIG_FILE),
            `export default defineConfig({ build: { stage: [{ from: 'marker.txt', to: '.', clean: true }] } });`,
        );
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeConfigError && /not the project root/.test(error.message),
        );
        assert.equal(fs.existsSync(path.join(root, 'marker.txt')), true);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('native targets must line up with the declared platform packages', () => {
    const source = `
export default defineConfig({
    native: {
        crate: 'tool-napi',
        targets: [{ platform: 'win32', arch: 'x64', triple: 'x86_64-pc-windows-msvc', package: '@scope/tool-win32-x64', nodeFile: 'tool.win32-x64.node', dir: 'platforms/tool-win32-x64' }],
    },
    packages: [{ name: '@scope/tool-win32-x64', dir: 'platforms/tool-win32-x64', os: ['linux'], cpu: ['x64'], main: 'tool.win32-x64.node' }],
});
`;
    assert.throws(
        () => parseConfigSource(source),
        (error: unknown) => error instanceof WaeConfigError && /does not match package os\/cpu/.test(error.message),
    );
});

test('artifact file names must stay inside the platform package', () => {
    const withNodeFile = (nodeFile: string): unknown => ({
        native: {
            crate: 'tool-napi',
            targets: [{
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@scope/tool',
                nodeFile,
                dir: 'platforms/tool',
            }],
        },
        packages: [{ name: '@scope/tool', dir: 'platforms/tool', os: ['win32'], cpu: ['x64'], main: nodeFile }],
    });
    const rejects = (raw: unknown, message: RegExp): void => {
        assert.throws(
            () => validateConfig(raw),
            (error: unknown) => error instanceof WaeConfigError && error.code === 'config_invalid' && message.test(error.message),
        );
    };
    for (const bad of ['../escape.node', 'nested/tool.node', 'nested\\tool.node']) {
        rejects(withNodeFile(bad), /bare `.node` file name without path separators/);
    }
    rejects({ packages: [{ name: '@scope/tool', dir: 'platforms/tool', os: ['win32'], cpu: ['x64'], main: 'nested/tool.node' }] },
        /bare `.node` file name without path separators/);
    assert.doesNotThrow(() => validateConfig(withNodeFile('tool.win32-x64.node')));
});

test('config discovery walks up to the nearest wae.config.ts', () => {
    const root = tempDir();
    try {
        writeFile(path.join(root, WAE_CONFIG_FILE), 'export default {};');
        writeFile(path.join(root, 'packages', 'tool', 'src', 'index.ts'), '');
        assert.equal(discoverConfig(path.join(root, 'packages', 'tool', 'src')), path.join(root, WAE_CONFIG_FILE));
        const loaded = loadConfig(path.join(root, 'packages', 'tool', 'src'));
        assert.equal(loaded.source, 'file');
        assert.equal(loaded.root, root);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('loadConfig falls back to the default convention without a file', () => {
    const root = tempDir();
    try {
        const loaded = loadConfig(root);
        assert.equal(loaded.source, 'default');
        assert.equal(loaded.root, path.resolve(root));
        assert.equal(loaded.config, undefined);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('--package override changes only the native crate', async () => {
    const root = tempDir();
    try {
        // Pin the target and allow cross compilation so this stays host-independent:
        // on Linux and macOS CI the fixture's only target is not the host.
        writeFile(path.join(root, WAE_CONFIG_FILE), FULL_CONFIG.replace("crossCompile: 'reject'", "crossCompile: 'allow'"));
        const report = await runBuild({
            startDir: root,
            packageOverride: 'other-napi',
            targetOverride: 'win32-x64',
            dryRun: true,
            log: () => {},
        });
        const native = report.steps.find((step) => step.name === 'native');
        assert.equal(native?.status, 'planned');
        assert.match(native?.detail ?? '', /other-napi/);
        assert.match(native?.detail ?? '', /win32-x64/);
        const ts = report.steps.find((step) => step.name === 'ts');
        assert.match(ts?.detail ?? '', /@scope\/tool/);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('--target override selects exactly one declared target', async () => {
    const root = tempDir();
    try {
        const host = `${process.platform}-${process.arch}`;
        writeFile(
            path.join(root, WAE_CONFIG_FILE),
            `
export default defineConfig({
    native: {
        crate: 'tool-napi',
        crossCompile: 'allow',
        targets: [
            { platform: 'win32', arch: 'x64', triple: 'x86_64-pc-windows-msvc', package: '@scope/a', nodeFile: 'a.node', dir: 'p/a' },
            { platform: 'linux', arch: 'x64', triple: 'x86_64-unknown-linux-gnu', package: '@scope/b', nodeFile: 'b.node', dir: 'p/b' },
        ],
    },
    packages: [
        { name: '@scope/a', dir: 'p/a', os: ['win32'], cpu: ['x64'], main: 'a.node' },
        { name: '@scope/b', dir: 'p/b', os: ['linux'], cpu: ['x64'], main: 'b.node' },
    ],
});
`,
        );
        const report = await runBuild({ startDir: root, targetOverride: 'linux-x64', dryRun: true, log: () => {} });
        const native = report.steps.find((step) => step.name === 'native');
        assert.match(native?.detail ?? '', /linux-x64/);
        assert.doesNotMatch(native?.detail ?? '', /win32-x64/);
        assert.equal(report.steps.length, 5, `host is ${host}`);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('default convention builds a pnpm workspace without a config file', async () => {
    const root = tempDir();
    try {
        writeFile(path.join(root, 'pnpm-workspace.yaml'), 'packages: []\n');
        json(path.join(root, 'package.json'), { name: 'workspace', private: true });
        const report = await runBuild({ startDir: root, dryRun: true, log: () => {} });
        assert.equal(report.source, 'default');
        assert.equal(report.steps[0]?.status, 'planned');
        assert.match(report.steps[0]?.detail ?? '', /workspace package/);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('default convention without a target fails with build_target_missing', async () => {
    const root = tempDir();
    try {
        json(path.join(root, 'package.json'), { name: 'empty', private: true });
        await assert.rejects(
            () => runBuild({ startDir: root, dryRun: true, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError && error.code === BUILD_CODES.targetMissing,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
