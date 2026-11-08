import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { runBuild } from '../src/build.ts';
import { BUILD_CODES, WaeBuildError } from '../src/build-error.ts';
import { WAE_CONFIG_FILE } from '../src/config.ts';

const HOST_PLATFORM = process.platform;
const HOST_ARCH = process.arch;

function hostProject(config: string): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-build-'));
    fs.writeFileSync(path.join(root, WAE_CONFIG_FILE), config);
    return root;
}

function writeJson(file: string, value: unknown): void {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(value, null, 2));
}

function baseConfig(): string {
    return `
export default defineConfig({
    native: {
        crate: 'tool-napi',
        stageOnly: true,
        targets: [{
            platform: '${HOST_PLATFORM}',
            arch: '${HOST_ARCH}',
            triple: 'x86_64-unknown-linux-gnu',
            package: '@scope/tool-${HOST_PLATFORM}-${HOST_ARCH}',
            nodeFile: 'tool.node',
            dir: 'platforms/tool',
        }],
    },
    packages: [{
        name: '@scope/tool-${HOST_PLATFORM}-${HOST_ARCH}',
        dir: 'platforms/tool',
        os: ['${HOST_PLATFORM}'],
        cpu: ['${HOST_ARCH}'],
        main: 'tool.node',
    }],
`;
}

function platformManifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        name: `@scope/tool-${HOST_PLATFORM}-${HOST_ARCH}`,
        version: '0.0.3',
        os: [HOST_PLATFORM],
        cpu: [HOST_ARCH],
        main: 'tool.node',
        files: ['tool.node', 'readme.md'],
        ...overrides,
    };
}

function seedArtifact(root: string): void {
    const dir = path.join(root, 'target', 'release');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'tool_napi.node'), 'fake-addon');
}

function seedPlatform(root: string, overrides: Record<string, unknown> = {}): void {
    writeJson(path.join(root, 'platforms', 'tool', 'package.json'), platformManifest(overrides));
    fs.writeFileSync(path.join(root, 'platforms', 'tool', 'tool.node'), 'fake-addon');
}

test('build fails with build_target_missing for an undeclared target', async () => {
    const root = hostProject(`${baseConfig()}});
`);
    try {
        await assert.rejects(
            () => runBuild({ startDir: root, targetOverride: 'plan9-vax', dryRun: true, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.targetMissing
                && /plan9-vax/.test(error.message),
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('build fails with build_artifact_missing when cargo produced nothing', async () => {
    const root = hostProject(`${baseConfig()}});
`);
    try {
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.artifactMissing,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('build fails with build_platform_package_invalid when os/cpu disagree', async () => {
    const root = hostProject(`${baseConfig()}});
`);
    try {
        seedArtifact(root);
        seedPlatform(root, { os: ['plan9'], cpu: ['vax'] });
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.platformPackageInvalid
                && /declares os plan9/.test(error.message),
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('build fails with build_release_set_mismatch when the published set differs', async () => {
    const root = hostProject(`${baseConfig()}    release: {
        registry: 'https://registry.npmjs.org',
        tagPattern: '^v(\\\\d+\\\\.\\\\d+\\\\.\\\\d+)$',
        packages: [{ name: '@scope/app', dir: 'packages/app' }],
    },
});
`);
    try {
        seedArtifact(root);
        seedPlatform(root);
        writeJson(path.join(root, 'packages', 'app', 'package.json'), {
            name: '@scope/app',
            version: '0.0.3',
            main: 'dist/index.js',
        });
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.releaseSetMismatch
                && /missing publishable package/.test(error.message),
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('dry run plans every step in order and writes nothing', async () => {
    const root = hostProject(`${baseConfig()}    build: {
        stage: [{ from: 'apps/ui/dist', to: 'apps/tool/dist/ui' }],
        outputs: ['apps/tool/dist/ui/index.html'],
    },
});
`);
    try {
        const report = await runBuild({ startDir: root, dryRun: true, log: () => {} });
        assert.deepEqual(report.steps.map((step) => step.name), ['native', 'ts', 'ui', 'skills', 'staging']);
        assert.equal(report.dryRun, true);
        assert.equal(report.steps.every((step) => step.status !== 'ok'), true);
        assert.equal(fs.existsSync(path.join(root, 'apps', 'tool', 'dist', 'ui')), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('a real build refuses to skip a missing stage source', async () => {
    const root = hostProject(`${baseConfig()}    build: {
        stage: [{ from: 'apps/ui/dist', to: 'apps/tool/dist/ui' }],
    },
});
`);
    try {
        seedArtifact(root);
        seedPlatform(root);
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.artifactMissing
                && /stage source is missing/.test(error.message),
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('`build.steps` narrows a real build to the declared steps', async () => {
    const root = hostProject(`${baseConfig()}    build: {
        steps: [{ name: 'native' }],
        stage: [{ from: 'apps/ui/dist', to: 'apps/tool/dist/ui' }],
    },
});
`);
    try {
        seedArtifact(root);
        seedPlatform(root);
        const report = await runBuild({ startDir: root, log: () => {} });
        assert.deepEqual(report.steps.map((step) => step.name), ['native']);
        assert.equal(report.steps[0]?.status, 'ok');
        assert.equal(fs.existsSync(path.join(root, 'apps', 'tool', 'dist', 'ui')), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('staging refuses a target that resolves outside the project root through a link', async () => {
    const parent = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-link-'));
    const root = path.join(parent, 'project');
    fs.mkdirSync(root, { recursive: true });
    fs.writeFileSync(path.join(root, WAE_CONFIG_FILE), `${baseConfig()}    build: {
        stage: [{ from: 'apps/ui/dist', to: '.staging/nested', clean: true }],
    },
});
`);
    const outside = path.join(parent, 'outside');
    fs.mkdirSync(outside, { recursive: true });
    fs.writeFileSync(path.join(outside, 'marker.txt'), 'keep');
    fs.symlinkSync(outside, path.join(root, '.staging'), 'junction');
    try {
        seedArtifact(root);
        seedPlatform(root);
        const ui = path.join(root, 'apps', 'ui', 'dist');
        fs.mkdirSync(ui, { recursive: true });
        fs.writeFileSync(path.join(ui, 'index.html'), '<html></html>');
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.stepFailed
                && /outside the project root/.test(error.message),
        );
        assert.equal(fs.existsSync(path.join(outside, 'marker.txt')), true);
        assert.equal(fs.existsSync(path.join(outside, 'nested')), false);
    } finally {
        fs.rmSync(parent, { recursive: true, force: true });
    }
});

/** Project with only the staging step: a CLI package plus a test workflow. */
function configProject(extra: string, workflow?: string): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-contract-'));
    fs.writeFileSync(path.join(root, WAE_CONFIG_FILE), `
export default defineConfig({
    build: { steps: [{ name: 'staging' }], ts: { packages: ['@scope/app'] } },
    ${extra}
});
`);
    writeJson(path.join(root, 'package.json'), {
        name: 'workspace',
        private: true,
        repository: 'https://github.com/scope/tool',
    });
    writeJson(path.join(root, 'packages', 'app', 'package.json'), {
        name: '@scope/app',
        version: '0.0.1',
        bin: { app: './dist/cli.js' },
    });
    fs.mkdirSync(path.join(root, 'packages', 'app', 'dist'), { recursive: true });
    if (workflow !== undefined) {
        fs.mkdirSync(path.join(root, '.github', 'workflows'), { recursive: true });
        fs.writeFileSync(path.join(root, '.github', 'workflows', 'check.yml'), workflow);
    }
    return root;
}

const WORKFLOW = `name: check
on:
  push:
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo "$GITHUB_SHA"
`;

test('`ci` is read by the build and must match the real workflow files', async () => {
    const root = configProject(
        `ci: { repo: 'scope/tool', workflows: ['check.yml'], requiredJobs: ['test'], shaBinding: true },`,
        WORKFLOW,
    );
    try {
        const report = await runBuild({ startDir: root, log: () => {} });
        const staging = report.steps.find((step) => step.name === 'staging');
        assert.match(staging?.detail ?? '', /ci scope\/tool/);
        assert.match(staging?.detail ?? '', /1 workflow\(s\)/);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('`ci` claims fail with build_ci_mismatch when the workflow does not match', async () => {
    const cases: Array<[string, string, RegExp]> = [
        [
            `ci: { repo: 'scope/tool', workflows: ['missing.yml'] },`,
            WORKFLOW,
            /do not exist/,
        ],
        [
            `ci: { repo: 'scope/tool', workflows: ['check.yml'], requiredJobs: ['native'] },`,
            WORKFLOW,
            /no declared workflow defines/,
        ],
        [
            `ci: { repo: 'scope/tool', workflows: ['check.yml'], shaBinding: true },`,
            WORKFLOW.replace('echo "$GITHUB_SHA"', 'echo ok'),
            /references `GITHUB_SHA`/,
        ],
        [
            `ci: { repo: 'other/tool', workflows: ['check.yml'] },`,
            WORKFLOW,
            /project declares/,
        ],
    ];
    for (const [extra, workflow, pattern] of cases) {
        const root = configProject(extra, workflow);
        try {
            await assert.rejects(
                () => runBuild({ startDir: root, log: () => {} }),
                (error: unknown) => error instanceof WaeBuildError
                    && error.code === BUILD_CODES.ciMismatch
                    && pattern.test(error.message),
            );
        } finally {
            fs.rmSync(root, { recursive: true, force: true });
        }
    }
});

test('`install` is written into the app CLI runtime contract', async () => {
    for (const shortcutDefault of [false, true]) {
        const root = configProject(`install: { shortcutDefault: ${shortcutDefault} },`);
        try {
            await runBuild({ startDir: root, log: () => {} });
            const contract = JSON.parse(
                fs.readFileSync(path.join(root, 'packages', 'app', 'dist', 'wae-runtime.json'), 'utf8'),
            ) as { install?: { shortcutDefault?: boolean } };
            assert.equal(contract.install?.shortcutDefault, shortcutDefault);
        } finally {
            fs.rmSync(root, { recursive: true, force: true });
        }
    }
});

test('`audit` actor mapping is written into the app CLI runtime contract', async () => {
    const mapping = { kind: 'env', id: 'flag', source: 'host' };
    const root = configProject(`audit: { actor: ${JSON.stringify(mapping)} },`);
    try {
        await runBuild({ startDir: root, log: () => {} });
        const contract = JSON.parse(
            fs.readFileSync(path.join(root, 'packages', 'app', 'dist', 'wae-runtime.json'), 'utf8'),
        ) as { audit?: { actor?: Record<string, string> } };
        assert.deepEqual(contract.audit?.actor, mapping);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('`window` policy is written into the app CLI runtime contract', async () => {
    const root = configProject(`window: { mode: 'single', secondLaunch: 'reject' },`);
    try {
        await runBuild({ startDir: root, log: () => {} });
        const contract = JSON.parse(
            fs.readFileSync(path.join(root, 'packages', 'app', 'dist', 'wae-runtime.json'), 'utf8'),
        ) as {
            window?: {
                policy?: { mode?: string; secondLaunch?: string; maxWindows?: number };
                host?: { supported?: boolean; code?: string; reason?: string };
            };
        };
        assert.deepEqual(contract.window?.policy, { mode: 'single', secondLaunch: 'reject', maxWindows: 1 });
        assert.deepEqual(contract.window?.host, { mode: 'single', supported: true });
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('an unsupported `window.mode` fails the build with a stable code and reason', async () => {
    const root = configProject(`window: { mode: 'independent' },`);
    try {
        await assert.rejects(
            () => runBuild({ startDir: root, log: () => {} }),
            (error: unknown) => error instanceof WaeBuildError
                && error.code === BUILD_CODES.windowModeUnsupported
                && error.reason === 'window_mode_unsupported'
                && /one window per process/.test(error.message),
        );
        assert.equal(
            fs.existsSync(path.join(root, 'packages', 'app', 'dist', 'wae-runtime.json')),
            false,
            'an unsupported mode must not emit a contract that claims support',
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('staging copies the declared tree and verifies outputs', async () => {
    const root = hostProject(`${baseConfig()}    build: {
        stage: [{ from: 'apps/ui/dist', to: 'apps/tool/dist/ui', clean: true, ignore: ['notes.txt'] }],
        outputs: ['apps/tool/dist/ui/index.html'],
    },
});
`);
    try {
        seedArtifact(root);
        seedPlatform(root);
        const ui = path.join(root, 'apps', 'ui', 'dist');
        fs.mkdirSync(ui, { recursive: true });
        fs.writeFileSync(path.join(ui, 'index.html'), '<html></html>');
        fs.writeFileSync(path.join(ui, 'notes.txt'), 'ignored');
        const target = path.join(root, 'apps', 'tool', 'dist', 'ui');
        fs.mkdirSync(target, { recursive: true });
        fs.writeFileSync(path.join(target, 'stale.txt'), 'stale');
        const report = await runBuild({ startDir: root, log: () => {} });
        const staging = report.steps.find((step) => step.name === 'staging');
        assert.equal(staging?.status, 'ok');
        assert.equal(fs.existsSync(path.join(target, 'stale.txt')), false);
        assert.equal(fs.existsSync(path.join(target, 'index.html')), true);
        assert.equal(fs.existsSync(path.join(target, 'notes.txt')), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
