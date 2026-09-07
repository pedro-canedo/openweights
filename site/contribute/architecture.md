# Architecture

OpenWeights is a Tauri 2 app: a Rust core, a React frontend, and a Rust
workspace split into one crate per concern.

```
src/                  React frontend (screens, components, i18n pt-BR/en)
src-tauri/src/        The Tauri app itself (commands, state, telemetry)
src-tauri/crates/     The core, one crate per concern
site/                 This documentation site (VitePress)
```

## The crates

| Crate | What it owns |
|---|---|
| `types` | Types shared across crates, serialized to the frontend in `camelCase` |
| `store` | Local SQLite: chats, messages, presets, settings and the configuration comparisons |
| `engine` | Inference engines. The main one is llama.cpp's llama-server in **Router mode**: one process that loads, unloads and swaps models according to the `model` field of each request |
| `runtime` | Picks the right llama.cpp build for the machine, downloads it from a pinned GitHub release, verifies and extracts it |
| `hw` | Hardware detection at startup, plus live telemetry at 1–2 Hz |
| `models` | Hugging Face Hub client for GGUF files, and the download manager |
| `advisor` | Estimates the memory each GGUF file needs and grades it against your hardware — the green/yellow/grey verdict |
| `cluster` | llama.cpp RPC cluster: one host and one worker on the local network |
| `providers` | LLM providers: the remote catalogue (OpenRouter) and endpoint resolution |
| `ninerouter` | Installs, supervises and removes 9router in isolation |
| `dshhost` | Installs, supervises and configures the DeepSeek Harness (dsh) in isolation |
| `gateway` | Single entry point (a local Traefik) for the LLM providers |
| `nodejs` | Portable Node.js runtime, isolated from the system's Node |
| `proc` | Long-lived child process supervision: process tree, Job Object and free port |
| `fetch` | Downloads, verifies by SHA256 and extracts release packages in an atomic install |

## Router mode

The engine is a **single** llama-server process that loads and unloads models on
demand, driven by the `model` field of each request. That is what lets the chat
and the local API share one engine instead of spawning a process per model —
and what makes "concurrent models" a real setting instead of a wish.

## The frontend

React 19 + Vite + Tailwind 4, with i18next for pt-BR/en. `npm run dev` runs the
UI in a browser with mocked data, no Rust involved — that is where most UI work
happens.

The screens map to the crates above: Discover (`models` + `advisor`), My Models,
Chat (`engine`), Local Server (`engine`), Model sources (`providers`), Settings.
