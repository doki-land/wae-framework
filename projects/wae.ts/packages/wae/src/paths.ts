/**
 * Meta CLI path helpers (package root, template root, locales).
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** `@wae/wae` package root (parent of `dist/cli.js`). */
export function packageRoot(from: string = import.meta.url): string {
    return fileURLToPath(new URL('..', from));
}

/**
 * In-repo `open-tools` template directory.
 *
 * `packages/wae` → `../../templates/open-tools`
 */
export function openToolsTemplateRoot(from: string = import.meta.url): string {
    return path.join(packageRoot(from), '..', '..', 'templates', 'open-tools');
}

/** This package's `locales` directory. */
export function localesRoot(from: string = import.meta.url): string {
    return path.join(packageRoot(from), 'locales');
}
