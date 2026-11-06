/**
 * Lightweight directory copy and placeholder rewrite for scaffolding.
 */
import fs from 'node:fs';
import path from 'node:path';

const TEXT_EXTS = new Set(['.json', '.ts', '.toml', '.md', '.html', '.vmz', '.yml', '.yaml']);

/**
 * Placeholder values derived from one scaffold name.
 *
 * - `{{name}}` → kebab-case app name (`demo-app`)
 * - `{{nameSnake}}` → Rust crate / cdylib spelling (`demo_app`)
 * - `{{namePascal}}` → product title spelling (`DemoApp`)
 * - `{{nameUpperSnake}}` → env var and constant spelling (`DEMO_APP`)
 */
export function placeholderValues(name: string): Record<string, string> {
    const nameSnake = name.replaceAll('-', '_');
    const namePascal = name
        .split('-')
        .filter(Boolean)
        .map((part) => part[0].toUpperCase() + part.slice(1))
        .join('');
    return {
        name,
        nameSnake,
        namePascal,
        nameUpperSnake: nameSnake.toUpperCase(),
    };
}

/** Kebab-case app name that can become an npm package, crate, and binary. */
export const SCAFFOLD_NAME_PATTERN = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;

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

/** Recursively replace scaffold placeholders in text files. */
export function rewritePlaceholders(dir: string, name: string): void {
    const values = placeholderValues(name);
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
            rewritePlaceholders(full, name);
            continue;
        }
        const ext = path.extname(full).toLowerCase();
        if (!TEXT_EXTS.has(ext)) continue;
        const raw = fs.readFileSync(full, 'utf8');
        let next = raw;
        for (const [token, value] of Object.entries(values)) {
            next = next.replaceAll(`{{${token}}}`, value);
        }
        if (next !== raw) fs.writeFileSync(full, next, 'utf8');
    }
}

/** Remove a path if it exists. */
export function removePath(target: string): void {
    fs.rmSync(target, { recursive: true, force: true });
}
