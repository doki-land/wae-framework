# AGENTS

Rules for this WAE repository.

## Product shape

```text
@wae/commander     i18n-first TypeScript CLI framework
@wae/wae           meta CLI bin (new / build / dist / publish)
projects/wae.rs/   Rust facade `wae` + `wae-macros` / `wae-napi` / `wae-webview-win32` / `wae-dist`
projects/wae.ts/templates/   scaffolds for downstream apps (`wae new`)
```

There is **no** Rust product bin. Downstream app CLIs use `@wae/commander`.

## Language (mandatory)

### English — npm publish surface only

| Surface | Language |
|---------|----------|
| npm `@wae/*` package `README` / `readme.md` | English |
| Documentation comments in `@wae/*` TypeScript sources (`/** … */`, file banners) | English |
| Downstream app defaults authored under `projects/wae.ts/templates/**` (README, source comments, default GUI strings) | English |

Identifiers stay English. Proper nouns (WebView2, N-API, pnpm) stay as-is.

### Chinese — Cargo / repo-local Rust

| Surface | Language |
|---------|----------|
| Rust crate `README` under `projects/wae.rs/crates/**` | Chinese |
| Rust `//!` / `///` documentation comments | Chinese |
| Root repo `readme.md` (not an npm package page) | Chinese |

### Exceptions

- **i18n catalogs** (`locales/<locale>/*.json`) may use any locale. Catalog values are product strings, not package docs.
- **Git commit messages**: gitmoji character prefix; English subject and body.

## Layout notes

- pnpm workspace: `projects/wae.ts/packages/*`
- Do **not** commit `pnpm-lock.yaml` (see `.gitignore`)
- Root and package README: install, build, configure, package/crate list, license only
