/**
 * {{nameTitle}} build configuration.
 *
 * Plain literal data: `wae` parses this file without executing it, so a config
 * cannot run commands, spawn processes, or branch on `process.env`. Every
 * relative path resolves against this directory.
 *
 * `wae build` runs `native → ts → ui → skills → staging` in that order and
 * fails with a stable `build_*` code instead of skipping a missing artifact.
 */
import { defineConfig } from '@wae/wae';

export default defineConfig({
    native: {
        crate: '{{nameSnake}}-napi',
        cdylib: '{{nameSnake}}_napi',
        profile: 'release',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@wae/{{name}}-win32-x64',
                nodeFile: '{{name}}.win32-x64.node',
                dir: 'packages/{{name}}-win32-x64',
            },
        ],
    },
    packages: [
        {
            name: '@wae/{{name}}-win32-x64',
            dir: 'packages/{{name}}-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: '{{name}}.win32-x64.node',
        },
    ],
    build: {
        ts: { packages: ['@wae/{{name}}'] },
        ui: { package: '@wae/{{name}}-ui' },
        stage: [
            {
                from: 'packages/{{name}}-ui/dist/static',
                to: 'packages/{{name}}/dist/gui',
                clean: true,
            },
        ],
        outputs: ['packages/{{name}}/dist/gui/index.html'],
    },
    skills: {
        packages: [
            {
                dir: 'packages/{{name}}-skills/skills',
                packageJson: 'packages/{{name}}-skills/package.json',
                appPackageJson: 'packages/{{name}}/package.json',
                cliEntry: 'packages/{{name}}/dist/cli.js',
                commandName: '{{name}}',
            },
        ],
    },
    // `release` stays unset until the platform package drops `private` and the
    // app leaves `0.0.0`. Then list the exact published set here, platform
    // packages first, so `wae build` can compare it with every manifest.
    ci: {
        repo: 'your-org/{{name}}',
        workflows: ['check-rust.yml', 'check-typescript.yml'],
        shaBinding: true,
    },
    install: { shortcutDefault: true, configLocation: 'per-user' },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'env' } },
});
