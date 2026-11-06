import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fixtures = path.join(root, 'tests', 'fixtures', 'typing');

test('published types accept a valid config and reject a bad one', () => {
    const result = spawnSync(
        process.execPath,
        [
            path.join(root, 'node_modules', 'typescript', 'bin', 'tsc'),
            '--noEmit',
            '--strict',
            '--skipLibCheck',
            '--target',
            'ES2022',
            '--module',
            'NodeNext',
            '--moduleResolution',
            'NodeNext',
            '--allowImportingTsExtensions',
            path.join(fixtures, 'valid-config.ts'),
            path.join(fixtures, 'invalid-config.ts'),
        ],
        { cwd: root, encoding: 'utf8' },
    );
    const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
    assert.notEqual(result.status, 0, 'invalid-config.ts must fail to type-check');
    assert.doesNotMatch(output, /typing\/valid-config\.ts/);
    assert.match(output, /typing\/invalid-config\.ts/);
    assert.match(output, /"fast"/);
});
