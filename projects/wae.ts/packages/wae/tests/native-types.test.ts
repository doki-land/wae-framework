import assert from 'node:assert/strict';
import test from 'node:test';
import {
    DEFAULT_NATIVE_TYPES_OUT,
    NativeTypegenError,
    parseMetadata,
    renderNativeModule,
    type NativeMetadata,
} from '../src/native-types.ts';

function metadata(): NativeMetadata {
    return {
        generator: 'wae',
        wae_version: '0.0.6',
        exports: [
            {
                js_name: 'render',
                params: [{ name: 'options', ts: 'RenderOptions', optional: false }],
                returns: 'RenderReport',
                untyped: false,
            },
            { js_name: 'version', params: [], returns: 'string', untyped: false },
        ],
        types: [
            { name: 'RenderReport', text: 'export interface RenderReport {\n    bytes: number\n}' },
            { name: 'RenderOptions', text: 'export interface RenderOptions {\n    out: string\n}' },
        ],
    };
}

const targets = [{ platform: 'win32', arch: 'x64', package: '@wae/demo-win32-x64' }];

test('the default output lives in a `native/` directory', () => {
    assert.equal(DEFAULT_NATIVE_TYPES_OUT, 'src/native/index.ts');
});

test('the generated module owns the platform map, types and forwarders', () => {
    const text = renderNativeModule(metadata(), targets);
    assert.match(text, /const ADDONS: Record<string, string> = \{\n    'win32-x64': '@wae\/demo-win32-x64',\n\};/);
    assert.match(text, /export interface RenderOptions \{/);
    assert.match(text, /export interface NativeModule \{/);
    assert.match(text, /    render\(options: RenderOptions\): RenderReport;/);
    assert.match(text, /export function render\(options: RenderOptions\): RenderReport \{\n    return loadNative\(\)\.render\(options\);/);
    assert.match(text, /export function loadNative\(\): NativeModule \{/);
});

test('rendering is deterministic and independent of input order', () => {
    const first = renderNativeModule(metadata(), targets);
    const shuffled = metadata();
    shuffled.exports.reverse();
    shuffled.types.reverse();
    const second = renderNativeModule(shuffled, [...targets]);
    assert.equal(first, second);
});

test('an optional parameter becomes a `?` parameter', () => {
    const value = metadata();
    value.exports = [
        {
            js_name: 'query',
            params: [{ name: 'filter', ts: 'Filter', optional: true }],
            returns: 'string',
            untyped: false,
        },
    ];
    value.types = [];
    const text = renderNativeModule(value, targets);
    assert.match(text, /query\(filter\?: Filter\): string/);
    assert.match(text, /return loadNative\(\)\.query\(filter\);/);
});

test('a legacy export keeps an honest `unknown` surface', () => {
    const value = metadata();
    value.exports = [{ js_name: 'probe', params: [], returns: 'unknown', untyped: true }];
    value.types = [];
    const text = renderNativeModule(value, targets);
    assert.match(text, /probe\(\.\.\.args: Array<unknown>\): unknown;/);
    assert.match(text, /export function probe\(\.\.\.args: Array<unknown>\): unknown \{\n    return loadNative\(\)\.probe\(\.\.\.args\);/);
});

test('an async export returns a Promise', () => {
    const value = metadata();
    value.exports = [{ js_name: 'probeAsync', params: [], returns: 'ProbeReport', untyped: false, async: true }];
    value.types = [{ name: 'ProbeReport', text: 'export interface ProbeReport {\n    ok: boolean\n}' }];
    const text = renderNativeModule(value, targets);
    assert.match(text, /probeAsync\(\): Promise<ProbeReport>;/);
    assert.match(text, /export function probeAsync\(\): Promise<ProbeReport> \{/);
});

test('metadata must carry a JSON payload with exports and types', () => {
    for (const value of [undefined, '', 'not json', '{}', '{"exports":[],"types":{}}']) {
        assert.throws(() => parseMetadata(value), NativeTypegenError, String(value));
    }
    const parsed = parseMetadata(JSON.stringify({ exports: [], types: [] }));
    assert.deepEqual(parsed.exports, []);
    assert.deepEqual(parsed.types, []);
});

test('metadata rejects non-identifier export names and duplicate exports', () => {
    const invalid = JSON.stringify({
        exports: [{ js_name: 'not-a-name', params: [], returns: 'string', untyped: false }],
        types: [],
    });
    assert.throws(() => parseMetadata(invalid), NativeTypegenError);

    const value = metadata();
    value.exports.push({ ...value.exports[0]! });
    assert.throws(() => renderNativeModule(value, targets), NativeTypegenError);
});

test('rendering needs at least one declared target', () => {
    assert.throws(() => renderNativeModule(metadata(), []), NativeTypegenError);
});
