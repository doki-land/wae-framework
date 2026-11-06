/**
 * Lightweight directory copy and placeholder rewrite for scaffolding.
 */
import fs from 'node:fs';
import path from 'node:path';

const TEXT_EXTS = new Set(['.json', '.ts', '.toml', '.rs', '.md', '.html', '.vmz', '.yml', '.yaml']);

/** Files without an extension that still contain scaffold tokens. */
const TEXT_FILES = new Set(['.gitignore', '.editorconfig']);

/**
 * Placeholder values derived from one scaffold name.
 *
 * - `{{name}}` → kebab-case app id, npm suffix, and CLI binary (`demo-app`)
 * - `{{nameSnake}}` → Rust crate / module spelling (`demo_app`)
 * - `{{namePascal}}` → product directory and manifest spelling (`DemoApp`)
 * - `{{nameTitle}}` → human display title (`Demo App`)
 * - `{{nameUpperSnake}}` → env var and constant spelling (`DEMO_APP`)
 */
export function placeholderValues(name: string): Record<string, string> {
    const parts = name.split('-').filter(Boolean);
    const nameSnake = parts.join('_');
    const namePascal = parts.map((part) => part[0].toUpperCase() + part.slice(1)).join('');
    const nameTitle = parts.map((part) => part[0].toUpperCase() + part.slice(1)).join(' ');
    return {
        name,
        nameSnake,
        namePascal,
        nameTitle,
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

function replaceTokens(text: string, values: Record<string, string>): string {
    let next = text;
    // Longest placeholder first: `{{name}}` must not consume the prefix of
    // `{{nameSnake}}`, `{{namePascal}}`, `{{nameTitle}}`, or `{{nameUpperSnake}}`.
    for (const [token, value] of Object.entries(values).sort((left, right) => right[0].length - left[0].length)) {
        next = next.replaceAll(`{{${token}}}`, value);
    }
    return next;
}

/**
 * Replace scaffold placeholders in both text contents and path names.
 *
 * The `open-app` template uses placeholders in directory names
 * (`packages/{{name}}-ui`), so generation must not leave a manual rename step.
 * Directories are processed bottom-up before their own name changes.
 */
export function rewritePlaceholders(dir: string, name: string): void {
    rewriteTree(dir, placeholderValues(name));
}

function rewriteTree(dir: string, values: Record<string, string>): void {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
            rewriteTree(full, values);
            const renamed = replaceTokens(entry.name, values);
            if (renamed !== entry.name) fs.renameSync(full, path.join(dir, renamed));
            continue;
        }
        const ext = path.extname(full).toLowerCase();
        if (TEXT_EXTS.has(ext) || TEXT_FILES.has(entry.name)) {
            const raw = fs.readFileSync(full, 'utf8');
            const next = replaceTokens(raw, values);
            if (next !== raw) fs.writeFileSync(full, next, 'utf8');
        }
        const renamed = replaceTokens(entry.name, values);
        if (renamed !== entry.name) fs.renameSync(full, path.join(dir, renamed));
    }
}

/** Remove a path if it exists. */
export function removePath(target: string): void {
    fs.rmSync(target, { recursive: true, force: true });
}
