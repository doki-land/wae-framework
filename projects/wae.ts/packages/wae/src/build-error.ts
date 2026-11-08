/**
 * Structured build failure.
 *
 * `wae build` never exits 0 without producing the artifact it declared. Every
 * failure carries a stable `code` plus the §3 fields (`capability`, `platform`,
 * `attempts`) so CI can branch without parsing prose.
 */
export class WaeBuildError extends Error {
    readonly code: string;
    readonly capability = 'build';
    readonly platform?: string;
    readonly attempts?: string[];
    /** Stable sub-code for a declared-but-unsupported contract, e.g. `window_mode_unsupported`. */
    readonly reason?: string;

    constructor(
        code: string,
        message: string,
        options: { platform?: string; attempts?: string[]; reason?: string; cause?: unknown } = {},
    ) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'WaeBuildError';
        this.code = code;
        if (options.platform !== undefined) this.platform = options.platform;
        if (options.attempts !== undefined) this.attempts = options.attempts;
        if (options.reason !== undefined) this.reason = options.reason;
    }

    toJSON() {
        return {
            code: this.code,
            capability: this.capability,
            ...(this.reason === undefined ? {} : { reason: this.reason }),
            ...(this.platform === undefined ? {} : { platform: this.platform }),
            ...(this.attempts === undefined ? {} : { attempts: this.attempts }),
            message: this.message,
        };
    }
}

/** Stable failure codes for `wae build` and its native primitives. */
export const BUILD_CODES = {
    targetMissing: 'build_target_missing',
    artifactMissing: 'build_artifact_missing',
    platformPackageInvalid: 'build_platform_package_invalid',
    releaseSetMismatch: 'build_release_set_mismatch',
    ciMismatch: 'build_ci_mismatch',
    skillsInvalid: 'build_skills_invalid',
    windowModeUnsupported: 'build_window_mode_unsupported',
    stepFailed: 'build_step_failed',
} as const;
