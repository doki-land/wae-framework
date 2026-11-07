/**
 * Type-check fixture: a valid config must pass `tsc` unchanged.
 * Kept outside the shipped `src` tree and only compiled with `--noEmit`.
 */
import { defineConfig } from '../../../src/cli.ts';

export default defineConfig({
    native: {
        crate: 'tool-addon',
        profile: 'release',
        crossCompile: 'reject',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@scope/tool-win32-x64',
                nodeFile: 'tool.win32-x64.node',
                dir: 'platforms/tool-win32-x64',
            },
        ],
    },
    packages: [
        {
            name: '@scope/tool-win32-x64',
            dir: 'platforms/tool-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: 'tool.win32-x64.node',
        },
    ],
    build: {
        ts: { packages: ['@scope/tool'] },
        ui: { package: '@scope/tool-ui' },
        stage: [{ from: 'apps/ui/dist', to: 'apps/tool/dist/ui', clean: true }],
        outputs: ['apps/tool/dist/ui/index.html'],
    },
    skills: { packages: [{ dir: 'packages/tool-skills/skills' }] },
    release: {
        registry: 'https://registry.npmjs.org',
        tagPattern: '^v(\\d+\\.\\d+\\.\\d+)$',
        packages: [
            { name: '@scope/tool-win32-x64', dir: 'platforms/tool-win32-x64' },
            { name: '@scope/tool', dir: 'packages/tool' },
        ],
    },
    ci: { repo: 'scope/tool', workflows: ['check-rust.yml'], shaBinding: true },
    install: { shortcutDefault: true },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'env' } },
});
