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

function cliFromSpec(spec: unknown): string {
    return `export const cli = {
    describe() {
        return ${JSON.stringify(spec)};
    },
};
`;
}

function skillWithInvocations(invocations: string[]): string {
    const lines = invocations.map((invocation) => `Run \`${invocation}\`.`).join('\n');
    return `---
name: run
description: preview commands
---

## Safety

Always ask the user for confirmation（确认） before mutating commands.

${lines}
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

test('skills audit accepts command aliases and nested command paths', async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-skills-tree-'));
    try {
        const spec = {
            name: 'app',
            options: ['--verbose'],
            commands: [
                { names: ['install', 'i'], options: ['--dry-run'], children: [] },
                { names: ['db'], options: [], children: [{ names: ['migrate', 'm'], options: ['--dry-run'], children: [] }] },
            ],
        };
        fs.writeFileSync(path.join(root, 'cli.mjs'), cliFromSpec(spec));
        fs.mkdirSync(path.join(root, 'skills', 'run'), { recursive: true });
        const skillFile = path.join(root, 'skills', 'run', 'SKILL.md');
        fs.writeFileSync(skillFile, skillWithInvocations(['app i --dry-run', 'app db m --dry-run', 'app install --verbose --dry-run']));

        const result = await auditSkills(root, { dir: 'skills', cliEntry: 'cli.mjs', commandName: 'app' });
        assert.equal(result.commands, 3);
        assert.equal(result.invocations, 3);

        // Unknown children must still fail even when the parent command exists.
        fs.writeFileSync(skillFile, skillWithInvocations(['app db drop']));
        await assert.rejects(async () => {
            await auditSkills(root, { dir: 'skills', cliEntry: 'cli.mjs', commandName: 'app' });
        }, /unknown command `db drop`/);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
