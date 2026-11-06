import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { auditSkills } from '../src/skills-audit.ts';

function cliSource(flagLine: string): string {
    return `const args = process.argv.slice(2);
if (args[0] === 'install') {
    console.log('Options:');
    console.log('  --dry-run  preview');
    console.log('${flagLine}');
} else {
    console.log('Commands:');
    console.log('  install  install the app');
}
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

test('cached CLI flags are scoped to each skills package entry', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wae-skills-audit-'));
    try {
        fs.writeFileSync(path.join(root, 'cli-a.js'), cliSource('  --alpha  alpha only'));
        fs.writeFileSync(path.join(root, 'cli-b.js'), cliSource('  --beta  beta only'));
        for (const [dir, flag] of [['skills-a', '--alpha'], ['skills-b', '--beta']] as const) {
            fs.mkdirSync(path.join(root, dir, 'run'), { recursive: true });
            fs.writeFileSync(path.join(root, dir, 'run', 'SKILL.md'), skillSource(flag));
        }

        const first = auditSkills(root, { dir: 'skills-a', cliEntry: 'cli-a.js', commandName: 'app' });
        assert.equal(first.commands, 1);
        assert.equal(first.skills, 1);
        const second = auditSkills(root, { dir: 'skills-b', cliEntry: 'cli-b.js', commandName: 'app' });
        assert.equal(second.commands, 1);
        assert.equal(second.skills, 1);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
});
