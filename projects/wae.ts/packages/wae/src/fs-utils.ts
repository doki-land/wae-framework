/**
 * Lightweight directory copy and placeholder rewrite for scaffolding.
 */
import fs from 'node:fs';
import path from 'node:path';

const TEXT_EXTS = new Set(['.json', '.ts', '.toml', '.md', '.html', '.yml', '.yaml']);

/** Recursively copy a directory. */
export function copyDir(from: string, to: string): void {
    fs.mkdirSync(to, { recursive: true });
    for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
        const src = path.join(from, entry.name);
        const dest = path.join(to, entry.name);
        if (entry.isDirectory()) {
            copyDir(src, dest);
        } else {
            fs.copyFileSync(src, dest);
        }
    }
}

/** Recursively replace `{{name}}` in text files. */
export function rewritePlaceholders(dir: string, name: string): void {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
            rewritePlaceholders(full, name);
            continue;
        }
        const ext = path.extname(full).toLowerCase();
        if (!TEXT_EXTS.has(ext)) continue;
        const raw = fs.readFileSync(full, 'utf8');
        const next = raw.replaceAll('{{name}}', name);
        if (next !== raw) fs.writeFileSync(full, next, 'utf8');
    }
}

/** Remove a path if it exists. */
export function removePath(target: string): void {
    fs.rmSync(target, { recursive: true, force: true });
}
