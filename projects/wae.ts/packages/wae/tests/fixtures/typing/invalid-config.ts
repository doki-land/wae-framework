/**
 * Type-check fixture: `profile` is a closed union and `targets` is required.
 * Compiling this on purpose must fail, which proves the published types reject
 * a config the runtime parser would also reject.
 */
import { defineConfig } from '../../../src/cli.ts';

export default defineConfig({
    native: {
        crate: 'tool-addon',
        profile: 'fast',
    },
});
