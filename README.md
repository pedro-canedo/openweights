# OpenWeights

**Models. Your machine. Your rules.**

Rode LLMs no seu PC sem terminal, sem CUDA e sem chute de quantização.

OpenWeights é um app desktop open-source que esconde o [llama.cpp](https://github.com/ggml-org/llama.cpp) atrás de uma interface simples: detecta o hardware, instala o runtime certo e indica quais modelos cabem na sua máquina. Tudo local — nada vai para a nuvem. Windows primeiro; macOS em seguida.

- 🔍 **Hardware no piloto automático** — identifica CPU, RAM, GPU e VRAM e baixa o build do llama.cpp que combina (CUDA, Vulkan ou só CPU).
- 🤗 **Modelos do Hugging Face, já filtrados** — busca GGUF e recomenda a quantização para o *seu* PC: verde roda inteiro na GPU, amarelo divide com a CPU, cinza fica só no processador.
- 💬 **Chat local** — streaming, markdown e histórico no disco.
- 🔌 **API compatível com OpenAI** — outros apps apontam para `localhost` e usam o mesmo modelo.
- 📊 **Uso em tempo real** — CPU, RAM, GPU, VRAM e tokens/s enquanto você conversa.
- 🪶 **Leve de verdade** — núcleo em Rust + Tauri 2. Instalador de poucos MB, sem Electron.

## Rodando o projeto (desenvolvimento)

### 1. Pré-requisitos

| Ferramenta | Versão | Para quê |
|---|---|---|
| [Node.js](https://nodejs.org/) | 22+ | frontend (React + Vite) |
| [Rust](https://rustup.rs/) | estável (1.85+) | núcleo do app (Tauri) |
| Ferramentas de build C++ | — | linker do Rust em cada SO (ver abaixo) |

#### Windows

```powershell
# 1. Node.js (se ainda não tiver)
winget install OpenJS.NodeJS.LTS

# 2. Rust — quando o instalador perguntar sobre o Visual Studio,
#    ACEITE a instalação automática dos "Visual Studio Build Tools"
#    (obrigatório: é ele que fornece o linker). Download de alguns GB.
winget install Rustlang.Rustup
```

> **Importante:** depois de instalar, **feche e abra um PowerShell novo** — o
> PATH só atualiza em sessões novas. Confirme com `cargo --version`.

Se o instalador do Rust não ofereceu os Build Tools, instale manualmente:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

O PowerShell costuma bloquear o `npm.ps1` ("execução de scripts foi
desabilitada"). Libere scripts locais só para o seu usuário (sem admin):

```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

*(Alternativa sem mudar a política: use `npm.cmd` no lugar de `npm`.)*

O WebView2 (motor da interface) já vem instalado no Windows 10/11.

#### macOS

```bash
xcode-select --install          # ferramentas de linha de comando (linker)
curl https://sh.rustup.rs -sSf | sh
```

#### Linux (para quem for contribuir)

```bash
curl https://sh.rustup.rs -sSf | sh
# Dependências do Tauri (Debian/Ubuntu):
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev build-essential \
  libayatana-appindicator3-dev librsvg2-dev
```

### 2. Instalar e rodar

```powershell
cd C:\AI\LammaRunner        # ou onde você clonou
npm install                 # dependências do frontend
npm run tauri dev           # compila e abre o app
```

> ⏳ A **primeira** execução compila ~500 crates Rust e leva de 5 a 15
> minutos. As seguintes são incrementais (segundos).

Na primeira abertura, o app detecta seu hardware e oferece baixar o motor de
IA (build do llama.cpp adequado à sua GPU, ~100–600 MB) — isso acontece uma
única vez.

### 3. Comandos úteis

| Comando | O que faz |
|---|---|
| `npm run tauri dev` | roda o app em modo desenvolvimento (hot reload) |
| `npm run tauri build` | gera o instalador de produção (NSIS no Windows) |
| `npm run build` | checagem de tipos + build só do frontend |
| `npm run dev` | UI no navegador com dados simulados (sem Rust) |
| `cd src-tauri && cargo test --workspace` | testes do backend Rust |

### Estrutura

```
src/                  frontend React (telas, componentes, i18n pt-BR/en)
src-tauri/src/        app Tauri (comandos, estado, telemetria)
src-tauri/crates/     núcleo Rust: hw (hardware), runtime (llama.cpp),
                      models (Hugging Face + downloads), advisor (quantização),
                      engine (llama-server), store (SQLite), types
```

## Licença

MIT
