/**
 * Copy an already built addon into its platform package. Reuses the target and artifact
 * resolution from `build-napi.ts` so the candidate lookup is implemented once.
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { resolveTarget, stageNative } from './build-napi.ts';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export function main(root = ROOT): string {
    const target = resolveTarget();
    const staged = stageNative(root, target, false);
    console.log(`[wae] native addon staged at ${staged}`);
    return staged;
}

const entry = process.argv[1] ? path.resolve(process.argv[1]) : '';
if (entry === path.resolve(fileURLToPath(import.meta.url))) {
    main();
}
