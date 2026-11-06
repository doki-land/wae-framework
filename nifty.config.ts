// Nifty project configuration.
// See @doki-land/nifty for defineConfig, format presets, authorMap path, and lint rules.
export default {
    publish: {
        packages: ["@wae/wae", "@wae/commander"],
    },
    trust: {
        npm: {
            repo: "doki-land/wae-framework",
            file: "publish-npm.yml",
            environment: "NPM_PUBLISH",
        },
    },
};
