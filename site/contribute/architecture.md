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
| `store` | Local SQLite: chats, messages, presets, settings, and the agent state — runs, tools, permissions, MCP, memory |
| `engine` | Inference engines. The main one is llama.cpp's llama-server in **Router mode**: one process that loads, unloads and swaps models according to the `model` field of each request |
| `runtime` | Picks the right llama.cpp build for the machine, downloads it from a pinned GitHub release, verifies and extracts it |
| `hw` | Hardware detection at startup, plus live telemetry at 1–2 Hz |
| `models` | Hugging Face Hub client for GGUF files, and the download manager |
| `advisor` | Estimates the memory each GGUF file needs and grades it against your hardware — the green/yellow/grey verdict |
| `agent` | The run loop: steps, tools, confirmations, guard-rails, the trail |
| `tools` | The tool catalogue: name, description, JSON schema, execution |
| `policy` | For each call: run it, ask, or refuse |
| `checkpoint` | Project snapshots — shadow git or file copy |
| `codemode` | The Code Mode SDK, the bridge and the sandboxed Node execution |
| `mcp` | MCP connectors, with the approval gate against tool changes |
| `memory` | Long-term memory: curated facts in Markdown files |
| `rag` | Project index: hybrid FTS5 + vector search, fused with RRF |
| `webtools`, `codetools`, `gittools`, `datatools`, `desktop` | Tool implementations: internet, build/test, Git, CSV/SQLite, the computer |
| `providers`, `ninerouter`, `gateway`, `nodejs` | External model sources, the local router, the single entry point, the portable Node |
| `proc`, `fetch` | Long-lived child process supervision, and HTTP |

## Router mode

The engine is a **single** llama-server process that loads and unloads models on
demand, driven by the `model` field of each request. That is what lets the chat,
the agent, the project indexing and the local API share one engine instead of
spawning a process per model — and what makes "concurrent models" a real
setting instead of a wish.

## The frontend

React 19 + Vite + Tailwind 4, with i18next for pt-BR/en. `npm run dev` runs the
UI in a browser with mocked data, no Rust involved — that is where most UI work
happens.

The screens map to the crates above: Discover (`models` + `advisor`), My Models,
Chat (`engine` + `agent`), Activity (run history), Local Server (`engine`),
Model sources (`providers`), Settings.
