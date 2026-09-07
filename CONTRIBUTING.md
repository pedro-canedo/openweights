# Contributing to OpenWeights

Issues and pull requests are welcome. This file is the short version; the
[Build from source](https://pedro-canedo.github.io/openweights/contribute/)
page has the per-system prerequisites and a tour of the layout.

## Before opening a pull request

Run what CI runs. Everything below has to be green:

```bash
npm run build                                   # i18n + docs parity, types, frontend
npm test                                        # frontend tests
cd src-tauri
cargo test --workspace                          # Rust tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`npm run build` includes two parity checks that fail the build rather than warn:

- `scripts/i18n-parity.mjs` — every UI string exists in `en.json` **and**
  `pt-BR.json`.
- `scripts/docs-parity.mjs` — every documentation page exists in both
  languages, with the same heading structure, and appears in the sidebar.

Run either on its own with `npm run i18n` and `npm run docs`.

## Conventions

- **Comments and commit messages in Portuguese**, explaining the **why** — what
  the code does is already in the code. Code identifiers and test names are in
  English.
- **Test names in English, as sentences**
  (`a_cancelled_run_keeps_what_it_already_said`).
- **Every behaviour change ships with a test.** A test that does not fail
  without the fix proves nothing.
- **The UI is bilingual.** New keys go into `src/i18n/pt-BR.json` **and**
  `en.json`, always both.
- **The documentation is bilingual.** A page under `site/` gets its counterpart
  under `site/pt/`, in the same commit, and both go into the sidebar in
  `site/.vitepress/config.ts` and into the `PARES` map in
  `scripts/docs-parity.mjs`.

## Documentation

Documentation is part of the change, not a follow-up. A behaviour that a user
can see is a behaviour a page describes.

**Where things live:**

| What | Where |
|---|---|
| Guides, integrations, contributing | `site/` (English) and `site/pt/` |
| Release notes for one version | `docs/releases/<version>.md` — the single source |
| The accumulated history | `CHANGELOG.md`, generated from the above |
| The narrative of the current version | `site/guide/whats-new.md` + `novidades.md`, written by hand |

**Release notes are written once.** `docs/releases/<version>.md` is bilingual in
one file and is what the GitHub release body, the `CHANGELOG.md` and the site's
changelog page all come from. After editing one, regenerate:

```bash
node scripts/release-notes.mjs --changelog --write
```

CI checks that `CHANGELOG.md` matches, the same way `cargo fmt --check` does.

**Linking to files in the repository:** a file whose name carries a version
(`docs/performance-0.17.0.md`) must be linked through the tag
(`blob/v0.17.0/...`), never through `blob/main/`. A `main` link answers 200
today and 404 the day the next release renames the file, and nobody is looking
that day. `docs-parity.mjs` enforces this.

**Numbers that age.** Prefer a measured number to live in
`docs/releases/` or `docs/performance-<version>.md`, which are dated by
construction, and have guide pages link to it rather than repeat it. A precise
number that nobody re-measures becomes a lie; a round, vague one stays true.

## Reporting a bug

Use the [issue template](https://github.com/pedro-canedo/openweights/issues/new/choose).
It asks for hardware, model and log because almost every issue in this app
depends on them — without those, investigating turns into guesswork. The
[troubleshooting page](https://pedro-canedo.github.io/openweights/guide/troubleshooting)
covers the problems that turn out not to be bugs.

## About your API keys

Keys you paste into the app (OpenRouter, Hugging Face) are stored in plain text
in the local SQLite database, next to the rest of the settings. There is no OS
keyring integration yet. The file never leaves your machine, but any program
running as your user can read it. See [SECURITY.md](SECURITY.md).

## Building the sysroot on WSL

The Linux GTK packages the Tauri app needs are fetched into a local sysroot;
those downloads are ignored by git. The
[Build from source](https://pedro-canedo.github.io/openweights/contribute/)
page has the current recipe.
