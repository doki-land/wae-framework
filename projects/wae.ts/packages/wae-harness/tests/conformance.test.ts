import assert from 'node:assert/strict';
import test from 'node:test';

import { CONFORMANCE_FIXTURES_DIR, assertFixture, loadFixtures, runFixture } from '../src/conformance.ts';

const fixtures = loadFixtures(CONFORMANCE_FIXTURES_DIR);

test('fixture set has at least 20 cases with unique ids', () => {
    assert.ok(fixtures.length >= 20, `only ${String(fixtures.length)} fixtures in the v0 set`);
    const ids = new Set(fixtures.map((fixture) => fixture.id));
    assert.equal(ids.size, fixtures.length, 'duplicate fixture ids');
});

for (const fixture of fixtures) {
    test(`conformance ${fixture.id}`, async () => {
        assertFixture(fixture, await runFixture(fixture));
    });
}
