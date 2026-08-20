# OpenWeights

**Models. Your machine. Your rules.**

*[English](README.md) · **Português***

[![CI](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml/badge.svg)](https://github.com/pedro-canedo/openweights/actions/workflows/ci.yml)
[![Release](https://github.com/pedro-canedo/openweights/actions/workflows/release.yml/badge.svg)](https://github.com/pedro-canedo/openweights/releases)
[![Licença: MIT](https://img.shields.io/badge/licen%C3%A7a-MIT-blue.svg)](LICENSE)
![Windows, macOS e Linux](https://img.shields.io/badge/plataformas-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-lightgrey)

📖 **[Documentação](https://pedro-canedo.github.io/openweights/pt/)** — guia de
instalação, o harness agêntico explicado por partes e as integrações.

Rode LLMs no seu PC sem terminal, sem CUDA e sem chute de quantização.

OpenWeights é um app desktop open-source que esconde o [llama.cpp](https://github.com/ggml-org/llama.cpp) atrás de uma interface simples: detecta o hardware, instala o runtime certo e indica quais modelos cabem na sua máquina. Tudo local — nada vai para a nuvem. Windows, macOS e Linux.

- 🔍 **Hardware no piloto automático** — identifica CPU, RAM, GPU e VRAM e baixa o build do llama.cpp que combina (CUDA, Vulkan ou só CPU).
- 🤗 **Modelos do Hugging Face, já filtrados** — busca GGUF e recomenda a quantização para o *seu* PC: verde roda inteiro na GPU, amarelo divide com a CPU, cinza fica só no processador.
- 💬 **Chat local** — streaming, markdown e histórico no disco.
- 🤖 **Modo agente** — o modelo lê e edita arquivos, roda comandos, usa Git, consulta a internet e analisa dados. Cada ação passa pela sua confirmação (ou não, se você preferir), e uma foto do projeto é tirada antes da primeira alteração: dá para voltar atrás.
- 🎛️ **Ajustar para esta máquina** — o app pergunta ao próprio llama.cpp quanta memória cada configuração custa na *sua* placa, recomenda uma (com o porquê, em números), aplica e volta atrás sozinho se o modelo não carregar. Depois, se você quiser, mede tokens/s de verdade e substitui a estimativa pelo que a máquina deu.
- ⚡ **Code Mode** — em vez de pedir uma ferramenta por vez, o agente escreve um programa que usa todas de uma vez: uma tarefa inteira vira um passo, e só o resultado volta para a conversa. Gasta muito menos contexto, e faz trabalhar até o modelo que não sabe emitir chamada de ferramenta. O programa roda isolado — sem acesso a arquivo nem a comandos por fora das ferramentas, que continuam passando pela sua autorização.
- 🧭 **Feito para modelo pequeno** — o objetivo vira entregas curtas, cada uma com contexto novo, e o cardápio de ferramentas se ajusta à janela do modelo: o que não cabe, ele pede quando precisa.
- 🧠 **Memória e índice do projeto** — o agente lembra do que aprendeu e busca por significado no seu código.
- 🧩 **Conectores MCP** — servidores do padrão Model Context Protocol entram como ferramentas, com aprovação por servidor.
- 🔌 **API compatível com OpenAI** — outros apps apontam para `localhost` e usam o mesmo modelo.
- 🖧 **GPU extra na rede** — duas máquinas na mesma rede carregam um modelo juntas: o arquivo fica em uma, a outra só empresta a placa, e um PC de 12 GB ao lado de um Mac de 18 GB viram 30 GB. Vem desligado, não anuncia nada até você ligar, e quem empresta a placa precisa aceitar o pedido na mão.
- 🌐 **Outras fontes de modelo** — além da sua máquina, as conversas podem ser atendidas pelo **OpenRouter** (centenas de modelos por uma chave só, com catálogo nativo mostrando preço e contexto) ou pelo **9router**, um roteador local com painel próprio que o app instala, roda e remove numa pasta isolada — Node portátil incluído, sem tocar no seu sistema. O painel do 9router abre embutido no app.
- 🚪 **Ponto de entrada único (opcional)** — um Traefik local que encaminha um endereço só para o motor local e para o 9router, por prefixo, para apontar outra ferramenta ao OpenWeights sem decorar portas. Não é túnel: nada fica acessível pela internet.
- 📊 **Uso em tempo real** — CPU, RAM, GPU, VRAM e tokens/s enquanto você conversa.
- 🪶 **Leve de verdade** — núcleo em Rust + Tauri 2. Instalador de poucos MB, sem Electron.

## Instalar

Uma linha, sempre a versão mais recente:

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.ps1 | iex
```

```bash
# macOS e Linux
curl -fsSL https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.sh | sh
```

Ou baixe o instalador à mão, da **[última versão](https://github.com/pedro-canedo/openweights/releases/latest)**:

| Sistema | Arquivo |
|---|---|
| Windows 10/11 (x64) | `OpenWeights_x.y.z_x64-setup.exe` |
| macOS 11+ (Apple Silicon e Intel) | `OpenWeights_x.y.z_universal.dmg` |
| Linux x64 (Debian/Ubuntu) | `OpenWeights_x.y.z_amd64.deb` |
| Linux x64 (qualquer distro) | `OpenWeights_x.y.z_amd64.AppImage` |

Depois de instalado, o app **procura versão nova sozinho** e atualiza com um clique —
não precisa voltar aqui.

**Os binários não são assinados** — assinar exige um certificado pago e anual, que
o projeto ainda não tem. O sistema vai avisar, e o contorno é este:

- **Windows**: em "Windows protegeu o computador", clique em *Mais informações* →
  *Executar assim mesmo*.
- **macOS**: o jeito mais simples é instalar pela linha de comando acima — o
  script já libera o app. Se você baixou o `.dmg` à mão e apareceu *"a Apple não
  conseguiu verificar se este app tem software malicioso"*:
  - **macOS 15 (Sequoia) ou mais novo**: tente abrir uma vez, depois vá em
    *Ajustes do Sistema → Privacidade e Segurança*, role até o aviso sobre o
    OpenWeights e clique em *Abrir assim mesmo*.
  - **macOS 14 ou mais antigo**: clique com o botão direito no app → *Abrir*.
  - **Em qualquer versão**, isto resolve de uma vez pelo Terminal:
    `xattr -dr com.apple.quarantine /Applications/OpenWeights.app`

Na primeira execução o app **baixa o runtime do llama.cpp** que combina com a sua
placa (CUDA, Vulkan ou CPU) — algumas centenas de MB. É por isso que o instalador
é pequeno: nada de GPU vai junto no pacote.

> Prefere compilar você mesmo? A seção abaixo tem o passo a passo.

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

```bash
git clone https://github.com/pedro-canedo/openweights.git
cd openweights
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
src-tauri/crates/     núcleo Rust, um crate por assunto:
                        hw, runtime, models, advisor    hardware e modelos
                        engine, store, types            llama-server, SQLite, contratos
                        agent, tools, policy            laço do agente, ferramentas, permissões
                        checkpoint, mcp, memory, rag    desfazer, conectores, memória, índice
                        webtools, codetools,            internet, build/teste,
                        gittools, datatools             Git, CSV/SQLite
```

## Contribuir

Issues e pull requests são bem-vindos. Antes de abrir um PR, rode o que o CI roda:

```bash
npm run build                                   # tipos + build do frontend
cd src-tauri
cargo test --workspace                          # ~960 testes
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Convenções do projeto, em uma linha cada:

- **Comentários e mensagens de commit em português**, explicando o **porquê** — o
  que o código faz já está no código.
- **Nomes de teste em inglês, em forma de frase** (`a_cancelled_run_keeps_what_it_already_said`).
- **Toda mudança de comportamento vem com teste**; um teste que não falha sem a
  correção não prova nada.
- A interface é bilíngue: chaves novas entram em `src/i18n/pt-BR.json` **e** em
  `en.json`, sempre nas duas.

### Sobre suas chaves de API

As chaves que você cola no app (OpenRouter, Hugging Face, busca na web) ficam em
texto puro no banco SQLite local, ao lado das demais configurações. Ainda não há
integração com o cofre de senhas do sistema — o mesmo já valia para todos os
segredos que o app guardava, então isto é uma limitação conhecida, não uma
novidade. O arquivo nunca sai da sua máquina.

## Créditos

O motor é o [llama.cpp](https://github.com/ggml-org/llama.cpp) (MIT), baixado na
primeira execução e usado em *Router mode*. Os modelos vêm do
[Hugging Face Hub](https://huggingface.co/). Nada é enviado para servidor nosso —
não existe servidor nosso.

Em placas NVIDIA, o app também baixa o **CUDA Runtime** redistribuído pela release
do llama.cpp, sujeito à [NVIDIA CUDA EULA](https://docs.nvidia.com/cuda/eula/).
Esse download acontece na sua máquina, direto do upstream: o instalador do
OpenWeights não distribui bibliotecas da NVIDIA.

## Licença

MIT — veja [LICENSE](LICENSE).
