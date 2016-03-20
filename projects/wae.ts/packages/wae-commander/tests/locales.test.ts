/**
 * `@wae/commander` locales + parse/help coverage.
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import {
    assertCatalogCoverage,
    clearLocalesCache,
    createCli,
    createLocalizeFromLocales,
    flattenCatalog,
    loadCatalog,
    loadLocalesManifest,
    translateWithFallback,
} from '../src/index.ts';

const fixturesRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), 'fixtures', 'locales');

describe('@wae/commander locales + parse', () => {
    let _prevEnv: NodeJS.ProcessEnv;
    before(() => {
        clearLocalesCache();
        _prevEnv = { ...process.env };
        delete process.env.LOCALE;
        delete process.env.LANG;
        delete process.env.LC_ALL;
    });
    after(() => {
        clearLocalesCache();
        Object.assign(process.env, _prevEnv);
    });

    it('loadLocalesManifest + flattenCatalog + loadCatalog fallback chain', () => {
        const manifest = loadLocalesManifest(fixturesRoot);
        assert.equal(manifest.defaultLocale, 'en-US');
        assert.deepEqual(flattenCatalog({ a: { b: 'x' } }), { 'a.b': 'x' });
        assert.deepEqual(flattenCatalog({ 'a.b': 'x' }), { 'a.b': 'x' });

        const en = loadCatalog('en-US', fixturesRoot);
        assert.equal(en['cli.cmd.build'], 'Build the project');
        assert.equal(en['commander.ui.usage'], 'USAGE {name}');

        const zh = loadCatalog('zh-CN', fixturesRoot);
        assert.equal(zh['cli.cmd.build'], '构建项目');
        assert.equal(zh['cli.cmd.ping'], 'Ping');
    });

    it('translateWithFallback uses commander English when catalog omits id', () => {
        const s = translateWithFallback('commander.err.unknown_command', { cmd: 'nope' }, {});
        assert.equal(s, 'unknown command `nope`');
        assert.equal(translateWithFallback('missing.id', undefined, {}), '{{missing.id}}');
    });

    it('.locales(root) derives help and uses commander.err for unknown command', async () => {
        const logs: string[] = [];
        const errs: string[] = [];
        const origLog = console.log;
        const origErr = console.error;
        console.log = (...a: unknown[]) => {
            logs.push(a.map(String).join(' '));
        };
        console.error = (...a: unknown[]) => {
            errs.push(a.map(String).join(' '));
        };
        try {
            const cli = createCli('my-cli').locales(fixturesRoot).intro('cli.intro');
            cli.command('build', 'cli.cmd.build')
                .option('--out-dir <dir>', 'cli.opt.out-dir')
                .action(() => 0);

            const helpCode = await cli.parse(['help']);
            assert.equal(helpCode, 0);
            const help = logs.join('\n');
            assert.match(help, /my-cli demo/);
            assert.match(help, /USAGE my-cli/);
            assert.match(help, /Build the project/);
            assert.match(help, /--locale/);

            logs.length = 0;
            errs.length = 0;
            const bad = await cli.parse(['nope']);
            assert.equal(bad, 2);
            assert.match(errs.join('\n'), /unknown command `nope`/);
        } finally {
            console.log = origLog;
            console.error = origErr;
        }
    });

    it('uses ANSI theme for interactive help when color is forced', async () => {
        const logs: string[] = [];
        const origLog = console.log;
        const priorForceColor = process.env.FORCE_COLOR;
        const priorNoColor = process.env.NO_COLOR;
        console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
        process.env.FORCE_COLOR = '1';
        delete process.env.NO_COLOR;
        try {
            const cli = createCli('color-cli').locales(fixturesRoot).intro('cli.intro');
            cli.command('build', 'cli.cmd.build').option('--out-dir <dir>', 'cli.opt.out-dir').action(() => 0);
            assert.equal(await cli.parse(['help']), 0);
            assert.match(logs.join('\n'), /\u001b\[1m/);
            assert.match(logs.join('\n'), /\u001b\[32m/);
            assert.match(logs.join('\n'), /\u001b\[33m/);
        } finally {
            console.log = origLog;
            if (priorForceColor === undefined) delete process.env.FORCE_COLOR;
            else process.env.FORCE_COLOR = priorForceColor;
            if (priorNoColor === undefined) delete process.env.NO_COLOR;
            else process.env.NO_COLOR = priorNoColor;
        }
    });

    it('honors NO_COLOR even when color is forced', async () => {
        const logs: string[] = [];
        const origLog = console.log;
        const priorForceColor = process.env.FORCE_COLOR;
        const priorNoColor = process.env.NO_COLOR;
        console.log = (...a: unknown[]) => logs.push(a.map(String).join(' '));
        process.env.FORCE_COLOR = '1';
        process.env.NO_COLOR = '1';
        try {
            const cli = createCli('plain-cli').locales(fixturesRoot).intro('cli.intro');
            cli.command('build', 'cli.cmd.build').action(() => 0);
            assert.equal(await cli.parse(['help']), 0);
            assert.equal(logs.join('\n').includes('\u001b['), false);
        } finally {
            console.log = origLog;
            if (priorForceColor === undefined) delete process.env.FORCE_COLOR;
            else process.env.FORCE_COLOR = priorForceColor;
            if (priorNoColor === undefined) delete process.env.NO_COLOR;
            else process.env.NO_COLOR = priorNoColor;
        }
    });

    it('later .use overrides .locales', async () => {
        const logs: string[] = [];
        const origLog = console.log;
        console.log = (...a: unknown[]) => {
            logs.push(a.map(String).join(' '));
        };
        try {
            const cli = createCli('x')
                .locales(fixturesRoot)
                .use({
                    t: (id) => (id === 'cli.intro' ? 'OVERRIDE' : `{{${id}}}`),
                })
                .intro('cli.intro');
            cli.command('ping', 'cli.cmd.ping').action(() => 0);
            await cli.parse(['help']);
            assert.match(logs.join('\n'), /OVERRIDE/);
            assert.match(logs.join('\n'), /Usage:/);
        } finally {
            console.log = origLog;
        }
    });

    it('routes structured action failures to the product diagnostic handler', async () => {
        const prevEnv = { ...process.env };
        delete process.env.LOCALE;
        delete process.env.LANG;
        delete process.env.LC_ALL;
        const received: Array<{ diagnostic: unknown; locale: string }> = [];
        try {
            const cli = createCli('diagnostic')
                .locales(fixturesRoot)
                .onDiagnostic((diagnostic, context) => received.push({ diagnostic, locale: context.locale }));
            cli.command('build', 'cli.cmd.build').action(() => {
                throw {
                    code: 'tool::template::parse_failed',
                    severity: 'error',
                    path: 'src/App.ts',
                    args: { detail: 'unexpected token' },
                    span: { start: 4, end: 5 },
                };
            });

            assert.equal(await cli.parse(['build']), 1);
            assert.deepEqual(received, [{
                locale: 'en-US',
                diagnostic: {
                    code: 'tool::template::parse_failed',
                    severity: 'error',
                    path: 'src/App.ts',
                    args: { detail: 'unexpected token' },
                    span: { start: 4, end: 5 },
                    message: undefined,
                },
            }]);
        } finally {
            Object.assign(process.env, prevEnv);
        }
    });

    it('wraps legacy action exceptions as a language-neutral diagnostic', async () => {
        const received: unknown[] = [];
        const cli = createCli('diagnostic')
            .locales(fixturesRoot)
            .onDiagnostic((diagnostic) => received.push(diagnostic));
        cli.command('build', 'cli.cmd.build').action(() => {
            throw new Error('old parser prose');
        });

        assert.equal(await cli.parse(['build']), 1);
        assert.deepEqual(received, [{
            severity: 'error',
            code: 'commander::action_failed',
            args: { message: 'old parser prose', detail: 'old parser prose' },
            message: 'old parser prose',
        }]);
    });

    it('createLocalizeFromLocales + assertCatalogCoverage', () => {
        const plugin = createLocalizeFromLocales({ root: fixturesRoot, locale: 'en-US' });
        assert.equal(plugin.t('cli.cmd.build'), 'Build the project');
        assert.equal(plugin.t('commander.err.unknown_option', { option: '--x' }), 'unknown option `--x`');

        const cli = createCli('cov').locales(fixturesRoot).intro('cli.intro');
        cli.command('build', 'cli.cmd.build')
            .option('--out-dir <dir>', 'cli.opt.out-dir')
            .action(() => 0);
        assertCatalogCoverage(cli, loadCatalog('en-US', fixturesRoot));

        assert.throws(() => assertCatalogCoverage(cli, { 'cli.intro': 'x', 'cli.cmd.build': 'y' }));
    });
});
