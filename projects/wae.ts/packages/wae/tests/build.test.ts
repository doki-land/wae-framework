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
        const report = await runBuild({ startDir: root, log: () => {} });
        const staging = report.steps.find((step) => step.name === 'staging');
        assert.equal(staging?.status, 'ok');
        const target = path.join(root, 'apps', 'tool', 'dist', 'ui');
        assert.equal(fs.existsSync(path.join(target, 'index.html')), true);
        assert.equal(fs.existsSync(path.join(target, 'notes.txt')), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
