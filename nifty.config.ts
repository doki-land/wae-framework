// Nifty project configuration.
// See @doki-land/nifty for defineConfig, format presets, authorMap path, and lint rules.
//
// The published set is explicit and includes the three native platform packages.
// They carry a staged `.node` produced on their own runner before publishing, so a
// release must fail rather than publish an empty platform package.
export default {
    publish: {
        packages: [
            "@wae/wae-darwin-arm64",
            "@wae/wae-linux-x64",
            "@wae/wae-win32-x64",
            "@wae/commander",
            "@wae/wae",
        ],
    },
    trust: {
        npm: {
            repo: "doki-land/wae-framework",
            file: "publish-npm.yml",
            environment: "NPM_PUBLISH",
        },
    },
};
