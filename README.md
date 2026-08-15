# LlamaRunner

**Rode LLMs localmente sem precisar entender de nada técnico.**

LlamaRunner é um app desktop open-source (Windows primeiro, macOS em seguida) construído sobre o [llama.cpp](https://github.com/ggml-org/llama.cpp):

- 🔍 **Detecta seu hardware** (CPU, RAM, GPU, VRAM) e baixa automaticamente o build certo do llama.cpp — CUDA, Vulkan ou CPU.
- 🤗 **Busca modelos direto do Hugging Face**, com recomendação de quantização para o *seu* hardware (verde = roda 100% na GPU, amarelo = parcial, cinza = só CPU).
- 💬 **Chat integrado** com streaming, markdown e histórico local.
- 🔌 **Servidor API local compatível com OpenAI** para outros apps consumirem.
- 📊 **Monitoramento em tempo real** de CPU/RAM/GPU/VRAM e tokens/s.
- 🪶 Núcleo em **Rust + Tauri 2** — instalador de poucos MB, sem Electron.

## Desenvolvimento

Pré-requisitos: Node.js 22+, Rust estável (1.85+).

```bash
npm install
npm run tauri dev
```

Os crates Rust ficam em `src-tauri/crates/`. Testes: `cd src-tauri && cargo test --workspace`.

## Licença

MIT
