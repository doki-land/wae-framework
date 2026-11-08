/**
 * Window policy contract for `wae.config.ts#window`.
 *
 * A desktop app declares how many windows it may own and what a second open
 * request does. The declaration is data; the host capability table below says
 * which models the current WebView host can actually honour. A declared model
 * the host cannot honour fails with a stable code and reason — it is never
 * silently downgraded to one window.
 */

/** Window models a project may declare. */
export const WAE_WINDOW_MODES = ['single', 'independent', 'primary-secondary'] as const;

/** Window model name. */
export type WaeWindowMode = (typeof WAE_WINDOW_MODES)[number];

/** What a second open request does in `single` mode. */
export const WAE_SECOND_LAUNCH = ['focus', 'reject'] as const;

/** Second-launch behaviour name. */
export type WaeSecondLaunch = (typeof WAE_SECOND_LAUNCH)[number];

/** Whether child windows close with the primary window. */
export const WAE_CHILD_WINDOWS = ['owned', 'detached'] as const;

/** Child-window ownership name. */
export type WaeChildWindows = (typeof WAE_CHILD_WINDOWS)[number];

/** Declared `window` block. `mode` is required; the rest depend on the mode. */
export type WaeWindowConfig = {
    mode: WaeWindowMode;
    /** `single` only, default `focus`: focus the existing window or reject. */
    secondLaunch?: WaeSecondLaunch;
    /** `independent` / `primary-secondary` only, default `8`, at least `2`. */
    maxWindows?: number;
    /** `primary-secondary` only, default `owned`. */
    childWindows?: WaeChildWindows;
};

/** Normalized policy with every default materialized. */
export type WaeWindowPolicy =
    | { mode: 'single'; secondLaunch: WaeSecondLaunch; maxWindows: 1 }
    | { mode: 'independent'; maxWindows: number }
    | { mode: 'primary-secondary'; maxWindows: number; childWindows: WaeChildWindows };

/** Stable codes returned by config validation and the host contract. */
export const WAE_WINDOW_CODES = {
    modeUnsupported: 'window_mode_unsupported',
    modeConflict: 'window_mode_conflict',
    invalid: 'window_config_invalid',
    secondLaunchRejected: 'window_second_launch_rejected',
    limitReached: 'window_limit_reached',
} as const;

/** Default policy when `window` is omitted: one reused window, focused on relaunch. */
export const DEFAULT_WAE_WINDOW_POLICY: WaeWindowPolicy = {
    mode: 'single',
    secondLaunch: 'focus',
    maxWindows: 1,
};

/** Default cap for the multi-window models. */
export const DEFAULT_WAE_MAX_WINDOWS = 8;

/** Host support for one window model. */
export type WaeWindowHostSupport = {
    mode: WaeWindowMode;
    supported: boolean;
    code?: string;
    reason?: string;
};

/**
 * Capability table of the current WebView host.
 *
 * The host keeps one window per process: `openGui` owns a single host slot, and
 * every platform backend creates one native window. Multi-window needs a
 * per-window host registry and window ids across N-API and the app bridge, so
 * `independent` and `primary-secondary` are declared unsupported instead of
 * pretending a second window exists.
 */
export const WAE_WINDOW_HOST_SUPPORT: readonly WaeWindowHostSupport[] = [
    { mode: 'single', supported: true },
    {
        mode: 'independent',
        supported: false,
        code: WAE_WINDOW_CODES.modeUnsupported,
        reason: 'the WebView host owns one window per process, so a second independent window has no host slot',
    },
    {
        mode: 'primary-secondary',
        supported: false,
        code: WAE_WINDOW_CODES.modeUnsupported,
        reason: 'the WebView host owns one window per process, so a primary window cannot own child windows yet',
    },
];

/** Materialize defaults for a validated `window` block. */
export function resolveWindowPolicy(config: WaeWindowConfig): WaeWindowPolicy {
    if (config.mode === 'single') {
        return { mode: 'single', secondLaunch: config.secondLaunch ?? 'focus', maxWindows: 1 };
    }
    if (config.mode === 'independent') {
        return { mode: 'independent', maxWindows: config.maxWindows ?? DEFAULT_WAE_MAX_WINDOWS };
    }
    return {
        mode: 'primary-secondary',
        maxWindows: config.maxWindows ?? DEFAULT_WAE_MAX_WINDOWS,
        childWindows: config.childWindows ?? 'owned',
    };
}

/** Host support for one model; `supported` is always present. */
export function windowModeSupport(mode: WaeWindowMode): WaeWindowHostSupport {
    const entry = WAE_WINDOW_HOST_SUPPORT.find((support) => support.mode === mode);
    return entry ?? {
        mode,
        supported: false,
        code: WAE_WINDOW_CODES.modeUnsupported,
        reason: `no host support entry for window mode \`${mode}\``,
    };
}
