/**
 * `@wae/wae-harness`: in-process automation driver for a WAE WebView.
 *
 * The native work happens in the shared platform addon (`@wae/wae-<platform>-<arch>`),
 * the same `.node` that `@wae/wae` loads. This package only shapes the JS surface.
 */
import { HarnessNativeError, loadHarnessNative, type HarnessNative } from './native.js';

/** Input for opening one harness window. */
export interface HarnessOpenOptions {
    url?: string;
    contentDir?: string;
    title?: string;
    width?: number;
    height?: number;
    hidden?: boolean;
    acceptFileDrop?: boolean;
}

const DEFAULT_TIMEOUT_MS = 10_000;

/** One open harness window. */
export class Harness {
    private readonly native: HarnessNative;
    private readonly handle: number;
    private closed = false;

    private constructor(native: HarnessNative, handle: number) {
        this.native = native;
        this.handle = handle;
    }

    /** Open a hidden or visible WebView window; exactly one may be open per process. */
    static open(options: HarnessOpenOptions): Harness {
        const native = loadHarnessNative();
        return new Harness(native, native.harnessOpen(JSON.stringify(options)));
    }

    /** Evaluate a page script and return its JSON value. */
    eval(script: string): unknown {
        this.ensureOpen();
        return JSON.parse(this.native.harnessEval(this.handle, script)) as unknown;
    }

    /** Poll a script until it evaluates to `true`; throws `harness_timeout` with the last value. */
    waitFor(script: string, timeoutMs: number = DEFAULT_TIMEOUT_MS): unknown {
        this.ensureOpen();
        return JSON.parse(this.native.harnessWaitFor(this.handle, script, timeoutMs)) as unknown;
    }

    /** Wait until the document finishes loading. */
    ready(timeoutMs: number = DEFAULT_TIMEOUT_MS): void {
        this.waitFor("document.readyState === 'complete'", timeoutMs);
    }

    /** Capture the current WebView content as PNG and return the written path. */
    screenshot(path: string): string {
        this.ensureOpen();
        return this.native.harnessScreenshot(this.handle, path);
    }

    /** Drive one message pump; `false` means the window is gone. */
    pump(): boolean {
        this.ensureOpen();
        return this.native.harnessPump(this.handle);
    }

    /** Post one string message to the page. */
    post(text: string): void {
        this.ensureOpen();
        this.native.harnessPost(this.handle, text);
    }

    /** Drain the page-to-host IPC queue. */
    takeIpc(): string[] {
        this.ensureOpen();
        return JSON.parse(this.native.harnessTakeIpc(this.handle)) as string[];
    }

    /** Close the window; repeated calls are a no-op. */
    close(): void {
        if (this.closed) {
            return;
        }
        this.native.harnessClose(this.handle);
        this.closed = true;
    }

    private ensureOpen(): void {
        if (this.closed) {
            throw new HarnessNativeError('window_closed', 'harness window is already closed');
        }
    }
}

export { HarnessNativeError, loadHarnessNative, platformPackageName, sharedPlatformPackages, supportedPlatforms } from './native.js';
export type { HarnessNative } from './native.js';
export { allocatePort, loadOps, runHarness, WaeHarnessError } from './driver.js';
export type { HarnessOp, HarnessRunOptions } from './driver.js';
export * from './conformance.js';
