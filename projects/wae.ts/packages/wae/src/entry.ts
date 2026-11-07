import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** How the running process relates to the module that owns a CLI entry. */
export type EntryRelation =
    /** `invoked` resolves to this module, including package-manager symlinks and junctions. */
    | 'main'
    /** Another script imported this module, so the CLI must stay quiet. */
    | 'imported'
    /** `invoked` carries this entry's name from the same package but does not resolve to it. */
    | 'unresolved';

/**
 * Classify `invoked` (normally `process.argv[1]`).
 *
 * `unresolved` exists so a broken entry never exits 0 silently, but it must not
 * fire for a legitimate import: another CLI also named `cli.js` importing this
 * one is a normal `imported` case. Comparing package names keeps those apart.
 */
export function entryRelation(moduleUrl: string, invoked: string): EntryRelation {
    if (invoked.length === 0) return 'imported';
    const self = fileURLToPath(moduleUrl);
    try {
        if (fs.realpathSync(invoked) === fs.realpathSync(self)) return 'main';
    }
    catch {
        // A dangling link or removed file still counts as a launch attempt when
        // it carries this entry's name.
    }
    if (path.basename(invoked) !== path.basename(self)) return 'imported';
    return packageName(invoked) === packageName(self) ? 'unresolved' : 'imported';
}

/** Diagnostic for an entry that was launched but could not be resolved to its module. */
export function unresolvedEntryMessage(moduleUrl: string, invoked: string): string {
    return `cannot resolve entry "${invoked}" to "${fileURLToPath(moduleUrl)}"`;
}

/** Nearest `package.json` name above a file or directory, if any. */
function packageName(target: string): string | undefined {
    let dir = path.dirname(target);
    while (true) {
        const manifest = path.join(dir, 'package.json');
        if (fs.existsSync(manifest)) {
            try {
                const parsed = JSON.parse(fs.readFileSync(manifest, 'utf8')) as { name?: unknown };
                return typeof parsed.name === 'string' ? parsed.name : undefined;
            }
            catch {
                return undefined;
            }
        }
        const parent = path.dirname(dir);
        if (parent === dir) return undefined;
        dir = parent;
    }
}
