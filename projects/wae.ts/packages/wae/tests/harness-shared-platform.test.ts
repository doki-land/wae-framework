import test from 'node:test';
import assert from 'node:assert/strict';
import { platformPackageName, supportedPlatforms } from '../src/native.ts';
import { sharedPlatformPackages } from '../../wae-harness/src/native.ts';

test('harness and wae share one platform addon set', () => {
    const shared = sharedPlatformPackages();
    assert.deepEqual(supportedPlatforms().sort(), Object.keys(shared).sort());
    for (const [key, packageName] of Object.entries(shared)) {
        assert.equal(platformPackageName(key), packageName, `${key} must resolve to ${packageName}`);
    }
});
