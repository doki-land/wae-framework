/**
 * Meta CLI path helpers (package root, template root, locales).
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** Scaffold templates shipped with `@wae/wae`. */
export const SCAFFOLD_TEMPLATES = ['open-tools', 'open-app'] as const;

export type ScaffoldTemplate = (typeof SCAFFOLD_TEMPLATES)[number];

/** `@wae/wae` package root (parent of `dist/cli.js`). */
export function packageRoot(from: string = import.meta.url): string {
    return fileURLToPath(new URL('..', from));
}

/**
 * Template directory for a scaffold template.
 *
 * In a checkout the templates live at `../../templates/<template>`. A published tarball has no
 * workspace siblings, so `prepack` vendors them into `templates/<template>` and that copy is used
 * when the checkout path is absent.
 */
export function templateRoot(template: ScaffoldTemplate, from: string = import.meta.url): string {
    const checkout = path.join(packageRoot(from), '..', '..', 'templates', template);
    if (fs.existsSync(checkout)) {
        return checkout;
    }
    return path.join(packageRoot(from), 'templates', template);
}

/** In-repo `open-tools` template directory (minimal runnable tool). */
export function openToolsTemplateRoot(from: string = import.meta.url): string {
    return templateRoot('open-tools', from);
}

/** In-repo `open-app` template directory (full Open-X application skeleton). */
export function openAppTemplateRoot(from: string = import.meta.url): string {
    return templateRoot('open-app', from);
}

/** This package's `locales` directory. */
export function localesRoot(from: string = import.meta.url): string {
    return path.join(packageRoot(from), 'locales');
}
