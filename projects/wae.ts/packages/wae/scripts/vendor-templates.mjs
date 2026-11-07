/**
 * Vendor the checkout templates into the package tarball.
 *
 * `wae new` reads templates from the installed package, and npm cannot include files that live
 * outside the package directory. `prepack` copies `templates/` here so `files` can ship it.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(here, '..');
const from = path.resolve(here, '..', '..', '..', 'templates');
const to = path.join(packageRoot, 'templates');

if (!fs.existsSync(from)) {
    console.error(`template source is missing: ${from}`);
    process.exit(1);
}

// The only directory this script replaces is this package's own `templates/`. Resolve it first
// so a link that redirects that name elsewhere is refused instead of written through.
const expected = path.join(fs.realpathSync(packageRoot), 'templates');
if (fs.existsSync(to) && fs.realpathSync(to) !== expected) {
    console.error(`refusing to replace ${to}: it resolves to ${fs.realpathSync(to)}, not ${expected}`);
    process.exit(1);
}

fs.rmSync(to, { recursive: true, force: true });
fs.cpSync(from, to, { recursive: true });
