/**
 * wae project configuration.
 *
 * Plain literal data only: `wae` parses this file without executing it, so a
 * config cannot run commands, spawn processes or branch on `process.env`.
 * Everything relative is resolved against this directory.
 */
import { defineConfig } from '@wae/wae';

export default defineConfig({
    native: {
        crate: 'wae-addon',
        cdylib: 'wae_addon',
        profile: 'release',
        targetEnv: 'WAE_NATIVE_TARGET',
        crossCompile: 'reject',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@wae/wae-win32-x64',
                nodeFile: 'wae.win32-x64.node',
                dir: 'projects/wae.ts/platforms/wae-win32-x64',
            },
            {
                platform: 'darwin',
                arch: 'arm64',
                triple: 'aarch64-apple-darwin',
                package: '@wae/wae-darwin-arm64',
                nodeFile: 'wae.darwin-arm64.node',
                dir: 'projects/wae.ts/platforms/wae-darwin-arm64',
            },
            {
                platform: 'linux',
                arch: 'x64',
                triple: 'x86_64-unknown-linux-gnu',
                package: '@wae/wae-linux-x64',
                nodeFile: 'wae.linux-x64.node',
                dir: 'projects/wae.ts/platforms/wae-linux-x64',
            },
        ],
    },
    packages: [
        {
            name: '@wae/wae-win32-x64',
            dir: 'projects/wae.ts/platforms/wae-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: 'wae.win32-x64.node',
        },
        {
            name: '@wae/wae-darwin-arm64',
            dir: 'projects/wae.ts/platforms/wae-darwin-arm64',
            os: ['darwin'],
            cpu: ['arm64'],
            main: 'wae.darwin-arm64.node',
        },
        {
            name: '@wae/wae-linux-x64',
            dir: 'projects/wae.ts/platforms/wae-linux-x64',
            os: ['linux'],
            cpu: ['x64'],
            main: 'wae.linux-x64.node',
        },
    ],
    build: {
        ts: { packages: ['@wae/commander', '@wae/wae'] },
        outputs: [
            'projects/wae.ts/packages/wae-commander/dist/index.js',
            'projects/wae.ts/packages/wae/dist/cli.js',
        ],
    },
    release: {
        registry: 'https://registry.npmjs.org',
        tagPattern: '^v(\\d+\\.\\d+\\.\\d+)$',
        packages: [
            { name: '@wae/wae-win32-x64', dir: 'projects/wae.ts/platforms/wae-win32-x64' },
            { name: '@wae/wae-darwin-arm64', dir: 'projects/wae.ts/platforms/wae-darwin-arm64' },
            { name: '@wae/wae-linux-x64', dir: 'projects/wae.ts/platforms/wae-linux-x64' },
            { name: '@wae/commander', dir: 'projects/wae.ts/packages/wae-commander' },
            { name: '@wae/wae', dir: 'projects/wae.ts/packages/wae' },
        ],
    },
    // `check-rust.yml` defines the `test` and `native` jobs. No workflow binds
    // `GITHUB_SHA` to the release tag yet, so `shaBinding` must stay false until
    // the publish path actually does that.
    ci: {
        repo: 'doki-land/wae-framework',
        workflows: ['check-rust.yml', 'publish-npm.yml'],
        requiredJobs: ['test', 'native'],
        shaBinding: false,
    },
    // `wae build` writes these into the CLI package as `dist/wae-runtime.json`.
    install: { shortcutDefault: false },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'env' } },
});
