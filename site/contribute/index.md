# Build from source

Issues and pull requests are welcome. This page is what you need to get the app
running from a clone.

## Prerequisites

| Tool | Version | What for |
|---|---|---|
| [Node.js](https://nodejs.org/) | 22+ | frontend (React + Vite) |
| [Rust](https://rustup.rs/) | stable (1.85+) | app core (Tauri) |
| C++ build tools | — | the Rust linker on each OS |

### Windows

```powershell
winget install OpenJS.NodeJS.LTS

# When the Rust installer asks about Visual Studio, ACCEPT the automatic
# "Visual Studio Build Tools" install — it provides the linker.
winget install Rustlang.Rustup
```

::: warning Open a new PowerShell afterwards
PATH only updates in new sessions. Confirm with `cargo --version`.
:::

If the Rust installer did not offer the Build Tools:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

PowerShell usually blocks `npm.ps1` (*"running scripts is disabled"*). Allow
local scripts for your user only, no admin needed:

```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

*(Or use `npm.cmd` instead of `npm` and change nothing.)*

WebView2, the UI engine, already ships with Windows 10/11.

### macOS

```bash
xcode-select --install          # command line tools (linker)
curl https://sh.rustup.rs -sSf | sh
```

### Linux

```bash
curl https://sh.rustup.rs -sSf | sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev build-essential \
  libayatana-appindicator3-dev librsvg2-dev
```

## Run it

```bash
git clone https://github.com/pedro-canedo/openweights.git
cd openweights
npm install
npm run tauri dev
```

::: tip The first build is slow
It compiles ~500 Rust crates: 5 to 15 minutes. The ones after are incremental
(seconds).
:::

## Commands

| Command | What it does |
|---|---|
| `npm run tauri dev` | The app in development mode, hot reload |
| `npm run tauri build` | The production installer |
| `npm run build` | Type check + frontend-only build |
| `npm run dev` | UI in the browser with mocked data, no Rust |
| `cd src-tauri && cargo test --workspace` | Rust tests (~960 of them) |

## Before opening a PR

Run what CI runs:

```bash
npm run build
cd src-tauri
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Conventions

- **Comments and commit messages in Portuguese**, explaining the **why** — what
  the code does is already in the code. Identifiers and test names are in
  English.
- **Test names in English, as sentences**:
  `a_cancelled_run_keeps_what_it_already_said`.
- **Every behaviour change ships with a test.** A test that does not fail
  without the fix proves nothing.
- **The UI is bilingual**: new keys go into `src/i18n/pt-BR.json` **and**
  `en.json`, always both.
- **This site is bilingual too**: a page under `site/` gets its counterpart
  under `site/pt/`.

## The docs site

```bash
cd site
npm install
npm run dev
```

It is a VitePress site; pages are Markdown. `npm run build` renders it, and
pushing to `main` publishes it.
