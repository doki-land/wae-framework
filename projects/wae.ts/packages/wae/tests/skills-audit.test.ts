import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { auditSkills } from '../src/skills-audit.ts';

function cliSource(flag: string): string {
    return `export const cli = {
    describe() {
        return {
            name: 'app',
            options: [],
            commands: [{ names: ['install'], options: ['--dry-run', '${flag}'], children: [] }],
        };
    },
};
`;
}

function skillSource(flag: string): string {
    return `---
name: run
description: preview installs
---

## Safety

Always ask the user for confirmation（确认） before mutating commands.

Run \`app install --dry-run ${flag}\`.
`;
}

test('each skills package reads its own CLI flags in-process', async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-skills-audit-'));
    try {
        fs.writeFileSync(path.join(root, 'cli-a.mjs'), cliSource('--alpha'));
        fs.writeFileSync(path.join(root, 'cli-b.mjs'), cliSource('--beta'));
        for (const [dir, flag] of [['skills-a', '--alpha'], ['skills-b', '--beta']] as const) {
            fs.mkdirSync(path.join(root, dir, 'run'), { recursive: true });
            fs.writeFileSync(path.join(root, dir, 'run', 'SKILL.md'), skillSource(flag));
        }

        const first = await auditSkills(root, { dir: 'skills-a', cliEntry: 'cli-a.mjs', commandName: 'app' });
        assert.equal(first.commands, 1);
        assert.equal(first.skills, 1);
        const second = await auditSkills(root, { dir: 'skills-b', cliEntry: 'cli-b.mjs', commandName: 'app' });
        assert.equal(second.commands, 1);
        assert.equal(second.skills, 1);

        // A swapped flag must fail, proving the two packages do not share a flag set.
        await assert.rejects(async () => {
            await auditSkills(root, { dir: 'skills-b', cliEntry: 'cli-a.mjs', commandName: 'app' });
        }, /--beta/);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
