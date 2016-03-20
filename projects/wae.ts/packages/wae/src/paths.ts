/**
 * 元 CLI 路径解析（模板根、包根）。
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** `@wae/wae` 包根（`dist/cli.js` 的上一级）。 */
export function packageRoot(from: string = import.meta.url): string {
    return fileURLToPath(new URL('..', from));
}

/**
 * monorepo 内 open-tools 模板目录。
 *
 * `packages/wae` → `../../templates/open-tools`
 */
export function openToolsTemplateRoot(from: string = import.meta.url): string {
    return path.join(packageRoot(from), '..', '..', 'templates', 'open-tools');
}

/** 本包 locales 目录。 */
export function localesRoot(from: string = import.meta.url): string {
    return path.join(packageRoot(from), 'locales');
}
