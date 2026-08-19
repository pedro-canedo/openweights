# What OpenWeights is

OpenWeights is an open-source desktop app that runs large language models on
your own computer. It wraps [llama.cpp](https://github.com/ggml-org/llama.cpp)
in an interface that hides the parts people usually get stuck on: which build
matches your GPU, which quantization fits your VRAM, which flags to pass.

It runs on Windows, macOS and Linux, and the core is Rust + Tauri 2 — an
installer of a few MB, no Electron.

## What it does

- **Detects your hardware** and downloads the right llama.cpp runtime (CUDA,
  Vulkan or CPU-only) on first launch.
- **Searches GGUF models on Hugging Face** and tells you, per quantization,
  whether it fits fully on your GPU, splits with the CPU, or is CPU-only.
- **Chats locally**, with streaming, markdown, code highlighting and history on
  disk.
- **Runs an agent** that reads and edits files, runs commands, uses Git,
  browses the web and analyses data — under the authorization level you pick.
- **Tunes itself for your machine**: it asks llama.cpp how much memory each
  configuration costs on *your* card, applies one, and rolls back on its own if
  the model fails to load.
- **Exposes an OpenAI-compatible API** so other tools can use the same model.

## What it is not

- It is not a cloud service. There is no account, no telemetry sent anywhere,
  no server of ours to be down.
- It is not a training tool. It runs models; it does not fine-tune them.
- It does not run every model format. OpenWeights runs **GGUF** files, which is
  what llama.cpp reads. MLX (Apple's format), safetensors, GPTQ and AWQ do not
  run here — the same model almost always exists as GGUF.

## Where your data lives

| What | Where |
|---|---|
| Conversations, settings, run history | Local SQLite database in the app's data folder |
| Models | The folder you chose when downloading them |
| The llama.cpp runtime | App data folder, downloaded on first launch |
| Agent memory | Markdown files, per project and global — openable from the app |
| API keys (Hugging Face, OpenRouter, web search) | The same SQLite database, in plain text |

::: warning About the API keys
Keys are stored **in plain text** next to the rest of the settings — there is no
OS keyring integration yet. The file never leaves your machine, but a program
running as your user can read it.
:::

## Next

- [Install](/guide/install) — one line, or the installer by hand.
- [First run](/guide/first-run) — hardware detection and the engine download.
- [The agent harness](/agent/) — what makes the agent work on small models.
