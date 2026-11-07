import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { buildCli } from '../src/cli.ts';
import { cargoBuildNative } from '../src/native-stage.ts';

function stageProject(config: string): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-native-stage-'));
    fs.writeFileSync(path.join(root, 'wae.config.ts'), config);
    return root;
}

test('`native stage --dry-run` does not require a built artifact', async () => {
    const root = stageProject(`
export default defineConfig({
    native: {
        crate: 'tool-napi',
        targets: [{
            platform: '${process.platform}',
            arch: '${process.arch}',
            triple: 'host-triple',
            package: '@scope/tool',
            nodeFile: 'tool.node',
            dir: 'platforms/tool',
        }],
    },
});
`);
    try {
        assert.equal(fs.existsSync(path.join(root, 'target')), false);
        const code = await buildCli().parse(['native', 'stage', '--project', root, '--dry-run']);
        assert.equal(code, 0);
        assert.equal(fs.existsSync(path.join(root, 'platforms', 'tool', 'tool.node')), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('`native stage` ignores a blank target env var and selects the host', async () => {
    const root = stageProject(`
export default defineConfig({
    native: {
        crate: 'tool-napi',
        targetEnv: 'WAE_STAGE_TARGET',
        targets: [{
            platform: '${process.platform}',
            arch: '${process.arch}',
            triple: 'host-triple',
            package: '@scope/tool',
            nodeFile: 'tool.node',
            dir: 'platforms/tool',
        }],
    },
});
`);
    const previous = process.env.WAE_STAGE_TARGET;
    process.env.WAE_STAGE_TARGET = '   ';
    try {
        const code = await buildCli().parse(['native', 'stage', '--project', root, '--dry-run']);
        assert.equal(code, 0);
    } finally {
        if (previous === undefined) delete process.env.WAE_STAGE_TARGET;
        else process.env.WAE_STAGE_TARGET = previous;
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('cargo spawn failures report the underlying error', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-cargo-spawn-'));
    fs.writeFileSync(path.join(root, 'Cargo.toml'), '[workspace]\n');
    try {
        assert.throws(
            () => cargoBuildNative({ root, crate: 'tool-napi', env: { PATH: '' } }),
            /could not start/,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});

test('`native stage` resolves a cross-compiled artifact under the triple directory', async () => {
    const cross = process.platform === 'linux' ? { platform: 'win32', arch: 'x64' } : { platform: 'linux', arch: 'x64' };
    const triple = 'cross-target-triple';
    const root = stageProject(`
export default defineConfig({
    native: {
        crate: 'tool-napi',
        crossCompile: 'allow',
        targets: [{
            platform: '${cross.platform}',
            arch: '${cross.arch}',
            triple: '${triple}',
            package: '@scope/tool-cross',
            nodeFile: 'tool.node',
            dir: 'platforms/tool',
        }],
    },
});
`);
    try {
        const built = path.join(root, 'target', triple, 'release');
        fs.mkdirSync(built, { recursive: true });
        fs.writeFileSync(path.join(built, 'tool_napi.node'), 'fake-addon');
        const code = await buildCli().parse(['native', 'stage', '--project', root, '--target', `${cross.platform}-${cross.arch}`]);
        assert.equal(code, 0);
        const staged = path.join(root, 'platforms', 'tool', 'tool.node');
        assert.equal(fs.existsSync(staged), true);
        assert.equal(fs.readFileSync(staged, 'utf8'), 'fake-addon');
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
