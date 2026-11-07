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
const from = path.join(here, '..', '..', '..', 'templates');
const to = path.join(here, '..', 'templates');

fs.rmSync(to, { recursive: true, force: true });
fs.cpSync(from, to, { recursive: true });
