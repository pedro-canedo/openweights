# OpenWeights

**Models. Your machine. Your rules.**

***English** · [Português](README.pt-BR.md)*

[![CI](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml/badge.svg)](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml)
[![Release](https://github.com/pedro-canedo/openweights/actions/workflows/release.yml/badge.svg)](https://github.com/pedro-canedo/openweights/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Windows, macOS and Linux](https://img.shields.io/badge/platforms-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-lightgrey)

📖 **[Documentation](https://pedro-canedo.github.io/openweights/)** — install
guide, models and quantization, and the integrations.

Run LLMs on your own PC — no terminal, no CUDA setup, no guessing which
quantization fits.

OpenWeights is an open-source desktop app that hides [llama.cpp](https://github.com/ggml-org/llama.cpp)
behind a simple interface: it detects your hardware, installs the right runtime
and tells you which models actually fit your machine. Everything runs locally —
nothing is sent to a server of ours, because there is no server of ours.
Windows, macOS and Linux.

- 🔍 **Hardware on autopilot** — detects CPU, RAM, GPU and VRAM, then downloads
  the llama.cpp build that matches (CUDA, Vulkan or CPU-only).
- 🤗 **Hugging Face models, already filtered** — searches GGUF and recommends the
  quantization for *your* PC: green runs fully on the GPU, yellow splits with the
  CPU, grey is CPU-only.
- 💬 **Local chat** — streaming, markdown and history on disk.
- 🤖 **Agent work in an external harness, one click away** — the app hands your
  models to an external coding agent —
  [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness), Claude
  Code, Aider, OpenCode — pre-configured with all your providers and models.
  The DeepSeek Harness is installed and managed by the app itself (isolated
  folder, portable Node included) and opens in a window of its own; your API
  key travels by environment variable, never on the command line.
- 🎛️ **Tunes itself for your machine** — no one has to learn what `-ts`, `-ub`
  or `-ctk` mean. The app asks the engine which devices exist and how much is
  free on each, reads the real layer count from the file, and asks llama.cpp
  itself how much memory each configuration costs — then converges on one, in
  the background, and re-does it when the hardware picture changes. What you
  set by hand is never touched. If you want the numbers, the panel shows every
  candidate and can measure real tokens/s instead of trusting the estimate.
- 🔌 **OpenAI-compatible API** — other apps point at `localhost` and use the same
  model.
- 🎚️ **Every llama.cpp knob, visually** — MTP speculation, RoPE/YaRN, KV cache,
  cache reuse and the rest: the flags that matter carry a label and a hint, and
  **every other flag is read from your installed binary**, so an engine update
  never leaves the interface behind. Named presets (*MTP turbo*, *VRAM saver*),
  a live preview of the exact command and INI, and loading the model from the
  same screen.
- 🖧 **Extra GPU on the network** — two machines on the same network load one
  model together: the file stays on one, the other only lends its card, and a
  12 GB PC next to an 18 GB Mac becomes 30 GB. Off by default, it announces
  nothing until you turn it on, and the machine lending the GPU has to accept
  the request by hand.
- 🌐 **Other model sources** — besides your own machine, conversations can be
  answered by **OpenRouter** (hundreds of models behind one key, with a native
  catalogue showing price and context) or by **9router**, a local router with
  its own dashboard that the app installs, runs and removes in an isolated
  folder — portable Node included, nothing touching your system. The 9router
  dashboard opens embedded in the app.
- 🚪 **Single entry point (optional)** — a local Traefik that forwards one
  address to the local engine and to 9router by prefix, so another tool can
  point at OpenWeights without memorising ports. Not a tunnel: nothing becomes
  reachable from the internet.
- 📊 **Live usage** — CPU, RAM, GPU, VRAM and tokens/s while you talk.
- 🪶 **Genuinely light** — Rust + Tauri 2 core. Installer of a few MB, no Electron.

## Install

One line, latest version:

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.ps1 | iex
```

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.sh | sh
```

Or grab the installer by hand from the **[latest release](https://github.com/pedro-canedo/openweights/releases/latest)**:

| System | File |
|---|---|
| Windows 10/11 (x64) | `OpenWeights_x.y.z_x64-setup.exe` |
| macOS 11+ (Apple Silicon and Intel) | `OpenWeights_x.y.z_universal.dmg` |
| Linux x64 (Debian/Ubuntu) | `OpenWeights_x.y.z_amd64.deb` |
| Linux x64 (any distro) | `OpenWeights_x.y.z_amd64.AppImage` |

Once installed, the app **checks for new versions on its own** and offers a one-click
update — no need to come back here.

**The binaries are not signed** — signing requires a paid, yearly certificate the
project doesn't have yet. Your system will warn you; here is the way around it:

- **Windows**: on "Windows protected your PC", click *More info* → *Run anyway*.
- **macOS**: the simplest path is the command line above — the installer script
  already clears the app. If you downloaded the `.dmg` by hand and got *"Apple
  could not verify this app is free of malware"*:
  - **macOS 15 (Sequoia) or newer**: try to open it once, then go to *System
    Settings → Privacy & Security*, scroll to the notice about OpenWeights and
    click *Open Anyway*.
  - **macOS 14 or older**: right-click the app → *Open*.
  - **On any version**, this settles it from the Terminal:
    `xattr -dr com.apple.quarantine /Applications/OpenWeights.app`

On first launch the app **downloads the llama.cpp runtime** that matches your
card (CUDA, Vulkan or CPU) — a few hundred MB. That's why the installer is
small: no GPU stack ships inside the package.

> Prefer to build it yourself? The section below has the walkthrough.

## Running from source (development)

### 1. Prerequisites

| Tool | Version | What for |
|---|---|---|
| [Node.js](https://nodejs.org/) | 22+ | frontend (React + Vite) |
| [Rust](https://rustup.rs/) | stable (1.85+) | app core (Tauri) |
| C++ build tools | — | Rust linker on each OS (see below) |

#### Windows

```powershell
# 1. Node.js (if you don't have it yet)
winget install OpenJS.NodeJS.LTS

# 2. Rust — when the installer asks about Visual Studio,
#    ACCEPT the automatic "Visual Studio Build Tools" install
#    (required: it provides the linker). A few GB of download.
winget install Rustlang.Rustup
```

> **Important:** after installing, **close and open a new PowerShell** — PATH
> only updates in new sessions. Confirm with `cargo --version`.

If the Rust installer didn't offer the Build Tools, install them manually:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

PowerShell usually blocks `npm.ps1` ("running scripts is disabled"). Allow local
scripts for your user only (no admin needed):

```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

*(Alternative, without changing the policy: use `npm.cmd` instead of `npm`.)*

WebView2 (the UI engine) already ships with Windows 10/11.

#### macOS

```bash
xcode-select --install          # command line tools (linker)
curl https://sh.rustup.rs -sSf | sh
```

#### Linux (for contributors)

```bash
curl https://sh.rustup.rs -sSf | sh
# Tauri dependencies (Debian/Ubuntu):
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev build-essential \
  libayatana-appindicator3-dev librsvg2-dev
```

### 2. Install and run

```bash
git clone https://github.com/pedro-canedo/openweights.git
cd openweights
npm install                 # frontend dependencies
npm run tauri dev           # compiles and opens the app
```

> ⏳ The **first** run compiles ~500 Rust crates and takes 5 to 15 minutes.
> The following ones are incremental (seconds).

On first launch the app detects your hardware and offers to download the AI
engine (the llama.cpp build suited to your GPU, ~100–600 MB) — once only.

### 3. Useful commands

| Command | What it does |
|---|---|
| `npm run tauri dev` | runs the app in development mode (hot reload) |
| `npm run tauri build` | builds the production installer (NSIS on Windows) |
| `npm run build` | type check + frontend-only build |
| `npm run dev` | UI in the browser with mocked data (no Rust) |
| `cd src-tauri && cargo test --workspace` | Rust backend tests |

### Layout

```
src/                  React frontend (screens, components, i18n pt-BR/en)
src-tauri/src/        Tauri app (commands, state, telemetry)
src-tauri/crates/     Rust core, one crate per concern:
                        hw, runtime, models, advisor    hardware and models
                        engine, store, types            llama-server, SQLite, contracts
                        providers, ninerouter, dshhost  external sources, local router,
                        gateway, nodejs                 managed harness, entry point, Node
                        proc, fetch                     process supervision, HTTP
```

## Contributing

Issues and pull requests are welcome. Before opening a PR, run what CI runs:

```bash
npm run build                                   # types + frontend build
cd src-tauri
cargo test --workspace                          # Rust backend tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Project conventions, one line each:

- **Comments and commit messages in Portuguese**, explaining the **why** — what
  the code does is already in the code. (Code identifiers and test names are in
  English.)
- **Test names in English, as sentences** (`a_cancelled_run_keeps_what_it_already_said`).
- **Every behaviour change ships with a test**; a test that doesn't fail without
  the fix proves nothing.
- The UI is bilingual: new keys go into `src/i18n/pt-BR.json` **and** `en.json`,
  always both.

### About your API keys

Keys you paste into the app (OpenRouter, Hugging Face) are stored in
plain text in the local SQLite database, next to the rest of the settings. There
is no OS keyring integration yet — the same is true of every secret the app
already handled, so this is a known limitation rather than something new. The
file never leaves your machine.

## Credits

The engine is [llama.cpp](https://github.com/ggml-org/llama.cpp) (MIT),
downloaded on first launch and used in *Router mode*. Models come from the
[Hugging Face Hub](https://huggingface.co/).

On NVIDIA cards the app also downloads the **CUDA Runtime** redistributed by the
llama.cpp release, subject to the [NVIDIA CUDA EULA](https://docs.nvidia.com/cuda/eula/).
That download happens on your machine, straight from upstream: the OpenWeights
installer does not distribute NVIDIA libraries.

## License

MIT — see [LICENSE](LICENSE).
