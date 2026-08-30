# O que é o OpenWeights

O OpenWeights é um app de desktop, de código aberto, que roda modelos de
linguagem no seu próprio computador. Ele embrulha o
[llama.cpp](https://github.com/ggml-org/llama.cpp) numa interface que esconde
justamente as partes onde as pessoas travam: qual build combina com a sua GPU,
qual quantização cabe na sua VRAM, quais parâmetros passar.

Roda em Windows, macOS e Linux, e o núcleo é Rust + Tauri 2 — instalador de
poucos MB, sem Electron.

## O que ele faz

- **Detecta seu hardware** e baixa o runtime do llama.cpp certo (CUDA, Vulkan ou
  só CPU) na primeira execução.
- **Busca modelos GGUF no Hugging Face** e diz, por quantização, se ela cabe
  inteira na GPU, divide com a CPU ou é só CPU.
- **Conversa localmente**, com streaming, markdown, destaque de código e
  histórico em disco.
- **Roda um agente de código próprio** — o DeepSeek Harness tem item na barra
  lateral: instalado, supervisionado e embutido pelo app, pré-configurado com
  todos os seus provedores e modelos. Claude Code, Aider e OpenCode ganham o
  comando pronto, apontado para a API local.
- **Se ajusta para a sua máquina**: pergunta ao llama.cpp quanta memória cada
  configuração custa na *sua* placa, aplica uma e desfaz sozinho se o modelo
  não carregar.
- **Expõe uma API compatível com a OpenAI** para outras ferramentas usarem o
  mesmo modelo.

## O que ele não é

- Não é um serviço em nuvem. Não tem conta, não tem telemetria enviada para
  lugar nenhum, não tem servidor nosso para ficar fora do ar.
- Não é ferramenta de treinamento. Ele roda modelos; não faz fine-tuning.
- Não roda qualquer formato. O OpenWeights roda arquivos **GGUF**, que é o que
  o llama.cpp lê. MLX (o formato da Apple), safetensors, GPTQ e AWQ não rodam
  aqui — e quase sempre o mesmo modelo existe em GGUF.

## Onde ficam seus dados

| O quê | Onde |
|---|---|
| Conversas e ajustes | Banco SQLite local, na pasta de dados do app |
| Modelos | A pasta que você escolheu ao baixar |
| O runtime do llama.cpp | Pasta de dados do app, baixado na primeira execução |
| Chaves de API (Hugging Face, OpenRouter) | O mesmo banco SQLite, em texto puro |

::: warning Sobre as chaves de API
As chaves ficam **em texto puro** junto do resto dos ajustes — ainda não há
integração com o cofre do sistema operacional. O arquivo nunca sai da sua
máquina, mas um programa rodando como seu usuário consegue ler.
:::

## A seguir

- [Instalação](/pt/guia/instalacao) — uma linha, ou o instalador na mão.
- [Primeira execução](/pt/guia/primeira-execucao) — detecção de hardware e o
  download do motor.
- [Abrir em um harness](/pt/integracoes/api-local#abrir-em-um-harness) —
  agentes de código externos pré-configurados com os seus modelos.
