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
    // `release` is intentionally unset: the platform package is still
    // `private`, so there is no publishable set to compare yet.
    // `wae build` validates this CI contract: the declared workflow files must
    // exist, `requiredJobs` must be real job ids, and `repo` must match
    // `Cargo.toml#repository`. Claim `shaBinding` only when a workflow actually
    // binds `GITHUB_SHA` to the release tag.
    ci: {
        repo: 'your-org/{{name}}',
        workflows: ['check-rust.yml', 'check-typescript.yml'],
        shaBinding: false,
    },
    // `wae build` writes these into the app CLI as `dist/wae-runtime.json`.
    // `shortcutDefault` is the install command's default (a desktop shortcut is
    // created unless `--no-shortcut` is passed), and every capability call
    // resolves the audit actor from the channels declared here.
    install: { shortcutDefault: true },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'env' } },
    // Explicit window policy: one window per process, and a second launch
    // focuses the existing window instead of opening another one. Declaring
    // `independent` or `primary-secondary` requires a host that owns a window
    // registry; the current host fails the build with
    // `window_mode_unsupported` and a reason instead of downgrading silently.
    window: { mode: 'single', secondLaunch: 'focus' },
});
