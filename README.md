# OpenWeights

**Models. Your machine. Your rules.**

***English** · [Português](README.pt-BR.md)*

[![CI](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml/badge.svg)](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml)
[![Release](https://github.com/pedro-canedo/openweights/actions/workflows/release.yml/badge.svg)](https://github.com/pedro-canedo/openweights/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Windows, macOS and Linux](https://img.shields.io/badge/platforms-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-lightgrey)

📖 **[Documentation](https://pedro-canedo.github.io/openweights/)** — install
guide, the agent harness explained piece by piece, and the integrations.

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
- 🤖 **Agent mode** — the model reads and edits files, runs commands, uses Git,
  browses the web and analyses data. Every action goes through your approval (or
  not, if you prefer), and a snapshot of the project is taken before the first
  change: you can always go back.
- 🎛️ **Tune for this machine** — the app asks llama.cpp itself how much memory
  each configuration costs on *your* card, recommends one (with the numbers
  behind it), applies it and rolls back on its own if the model fails to load.
  Then, if you want, it measures real tokens/s and replaces the estimate with
  what your machine actually delivered.
- ⚡ **Code Mode** — instead of asking for one tool at a time, the agent writes a
  program that uses them all at once: a whole task becomes a single step, and
  only the result comes back to the conversation. It spends far less context,
  and it gets work out of models that can't emit tool calls at all. The program
  runs sandboxed — no file or command access outside the tools, which still go
  through your approval.
- 🧭 **Built for small models** — the goal becomes short deliverables, each with a
  fresh context, and the tool menu adapts to the model's window: what doesn't fit
  is requested on demand.
- 🧠 **Memory and project index** — the agent remembers what it learned and
  searches your code by meaning.
- 🧩 **MCP connectors** — Model Context Protocol servers become tools, approved
  per server.
- 🔌 **OpenAI-compatible API** — other apps point at `localhost` and use the same
  model.
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
                        agent, tools, policy            agent loop, tools, permissions
                        checkpoint, mcp, memory, rag    undo, connectors, memory, index
                        webtools, codetools,            internet, build/test,
                        gittools, datatools             Git, CSV/SQLite
```

## Contributing

Issues and pull requests are welcome. Before opening a PR, run what CI runs:

```bash
npm run build                                   # types + frontend build
cd src-tauri
cargo test --workspace                          # ~960 tests
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

Keys you paste into the app (OpenRouter, Hugging Face, web search) are stored in
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
