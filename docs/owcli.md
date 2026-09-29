# OwCLI — terminais, o fork e o gateway

O **OwCLI** tem duas metades. Uma é a tela de terminais do app: sessões de verdade
(pseudoterminal no Rust, xterm.js na webview) lado a lado, com lista, grade e avisos. A
outra é o agente: um fork do [Codex CLI da OpenAI](https://github.com/openai/codex)
(Apache-2.0) que pensa só com os modelos do OpenWeights. Este documento é o lado do app;
o lado do fork (regras, arquivos do upstream editados, o contrato em detalhe, medições)
está no `FORK.md` do próprio fork.

## Por que um fork do Codex

- **É um agente de terminal.** A TUI do Codex roda dentro de um pseudoterminal como
  qualquer programa, então a mesma tela serve para o agente, para shells e para os
  harnesses de fora (Claude Code, Aider, OpenCode).
- **Fala com o llama.cpp sem tradução.** O Codex só fala Responses API, e o
  llama-server pinado já atende `/v1/responses`. O spike rodou o Codex **sem
  modificação** contra ele e fez 4 de 5 tarefas com zero reescrita de requisição (a
  que falhou foi MCP; ver adiante).
- **O resto é configuração.** Provedor, catálogo, token e o que liga ou desliga entram
  como `-c` pela linha de comando, montados por um crate nosso. O que é do upstream
  quase não muda, e a sincronização é por tag estável.

## Licença e marca

O Codex é Apache-2.0. O fork mantém o `NOTICE` do upstream e acrescenta o nosso, e o
`FORK.md` lista cada arquivo do upstream editado (o aviso de modificação que a licença
pede). "Codex" e "OpenAI" são marcas: o produto se chama OwCLI, a relação aparece como
"baseado no Codex CLI da OpenAI (Apache-2.0)", e a marca troca nas telas só quando o
lançador define `OWCLI=1`, então os testes e snapshots do upstream não mudam.

## Onde as coisas moram no app

| O quê | Onde |
|---|---|
| Pseudoterminais | `src-tauri/crates/pty` (`lr_pty`): sessões, anel, OSC, encerramento |
| Comandos dos terminais | `src-tauri/src/commands_terminal.rs` (evento `terminal`) |
| Gateway | `src-tauri/crates/owgw` (`lr_owgw`) |
| Gateway, `openweights.json`, modelos | `src-tauri/src/commands_owcli.rs` |
| Runtime pinado: pins, instalação, poda | `src-tauri/crates/owcli` (`lr_owcli`), sobre `lr_fetch::pins` |
| Estado e instalação do runtime | `owcli_status`, `owcli_instalar` (evento `owcli-runtime`) |
| Pacote do runtime por sistema | `.github/workflows/owcli-runtime.yml`, pré-release `owcli-runtime-<rev8>-v<n>` |
| Histórico | `src-tauri/src/owcli_historico.rs` |
| Catálogo compartilhado com o AgenticOw | `src-tauri/src/catalogo.rs` |
| Tela | `src/screens/OwCLI.tsx`, estado em `src/lib/terminals.ts` |
| Cartões "escolha o cérebro" | `src/components/agents/CartoesDasFontes.tsx` |
| Casa do OwCLI (conversas, config) | `OWCLI_HOME`, padrão `~/.owcli` |
| Settings | `owcli.ativo` (já foi usado), `owcli.token` (token do gateway) |
| Porta do gateway | preferida 11740, caminho `/owcli/v1` |

## Terminais

Cada sessão é um processo num pseudoterminal (`portable-pty`, ConPTY no Windows):

- **Ambiente do sistema.** O filho parte de `env_clear()` mais
  `lr_proc::host_environment()`, com a pasta de trabalho sempre explícita. No AppImage,
  herdar o ambiente do pacote derruba os programas que o shell chamar. O teste-barreira
  do `lr_proc` procura `CommandBuilder::new(` além de `Command::new(`.
- **Anel e deslocamento.** A saída vai para um anel por sessão, com o deslocamento em
  bytes. Quem se anexa pede "desde o byte N" e recebe o que falta, sem buraco nem
  duplicata, e é assim que a tela volta depois de trocar de aba ou recarregar.
- **Controle de fluxo.** A tela confirma o que desenhou. Acima de 512 KiB sem
  confirmação o leitor para, e o kernel segura o filho: um `cat` de arquivo grande não
  enche a memória do app.
- **Transporte.** Um `Channel` do Tauri com `InvokeResponseBody::Raw`; cada mensagem é
  `[deslocamento u64 LE][truncado u8][dados]`. Nunca `Channel<Vec<u8>>`, que o serde
  transforma num array JSON de números.
- **OSC no Rust.** Título (OSC 0/2), pasta (OSC 7) e "precisa de você" (OSC 9, OSC 777
  e BEL) viram avisos do evento `terminal`, que a lista escuta mesmo com a tela fechada.
  O lançador põe o Codex para notificar por OSC 9 sempre.
- **Encerramento por sessão.** Fechar manda SIGHUP e, passado o prazo, mata tudo o que
  tem o `sid` da sessão (varredura de `/proc` no Linux, `ps` no macOS; Job Object no
  Windows). `kill(-pid)` não pegaria os jobs do shell. Ao sair do app, `shutdown_blocking`
  encerra todas.

Na webview, cada sessão tem uma instância do xterm.js que vive fora do React: o elemento
muda de painel sem recriar o terminal. Três armadilhas custaram caro:

- O Tauri acrescenta hash ao `style-src` e isso desliga o `'unsafe-inline'`: todo
  `<style>` criado em runtime (o xterm cria vários) era bloqueado. A correção é
  `dangerousDisableAssetCspModification: ["style-src"]` no `tauri.conf.json`.
- O fontconfig responde a qualquer família: a fonte monoespaçada é escolhida **medindo**
  no canvas, não confiando no nome.
- A área de transferência usa o `arboard` (só texto). O `tauri-plugin-clipboard-manager`
  exigiria subir o Tauri e o wry.

## O gateway

O OwCLI não conhece chave nem endereço de fonte nenhuma. Ele fala com
`http://127.0.0.1:<porta>/owcli/v1` e prova quem é com um token (comparação em tempo
constante). O gateway lê o prefixo do modelo, o que vem antes do **primeiro** `:`
(`local:`, `openrouter:`, `ninerouter:`), troca pelo id que a fonte conhece, injeta a chave
daquela fonte e repassa a resposta em streaming, byte a byte. `GET /models` lista os ids com
prefixo.

O corpo passa intacto, com uma exceção, só na fonte `local` (`lr_owgw::reescritas`): o Codex
manda as instruções em `instructions` e mensagens `developer` (permissões, ambiente, troca de
modelo) dentro de `input`, o llama.cpp as converte todas em `system`, e o chat template do
Ternary Bonsai 2 e do Qwen3.8 exige uma só, na primeira posição (`raise_exception('System
message must be at the beginning.')`). Isso chega ao Codex como HTTP 500, que ele traduz na
frase "We're currently experiencing high demand" — o spike não pegou porque o Qwen3-Coder e o
Qwen3-8B toleram. O gateway tira as mensagens `developer` e `system` de `input` e põe o texto
delas no fim de `instructions`, na ordem. O OpenRouter e o 9router recebem o pedido como veio.

Uma fonte sem chave não entra no catálogo: o 9router só tem a chave depois do primeiro boot
dele (a chave vem de arquivos que ele escreve), e sem ela todo pedido volta 401 "Missing API
key". Enquanto ela falta, o catálogo é refeito a cada poucos segundos, até oito vezes.

Nada disso nasce para quem nunca usou o OwCLI: o gateway e a casa só aparecem depois da
primeira sessão (setting `owcli.ativo`). Daí em diante o gateway sobe com o app e, a cada
catálogo novo (o mesmo agendamento do AgenticOw), reescreve o `openweights.json` na casa,
de forma atômica e só para o dono.

## O lançador

O `main` do CLI do upstream chama o crate `ow-launch` antes de tudo. O modo vem do nome
do binário: `owcli` é sempre o produto; `codex` (o que o cargo gera) só vira OwCLI com
`OWCLI_HOME` definido, e sem isso se comporta exatamente como o upstream, o que mantém a
suíte do upstream verde. No modo OwCLI o lançador:

- faz da casa do OwCLI o `CODEX_HOME`;
- injeta os `-c` fixos: telemetria, feedback, checagem de versão e dicas desligados,
  notificação por OSC 9, `windows.sandbox=unelevated`;
- desliga os recursos que falam com serviços da OpenAI ou sobem o daemon destacado
  (medido com `strace`: sem isso, o Codex fala com chatgpt.com e com o GitHub);
- registra o provedor `openweights` (Responses) apontando para o gateway, com o token
  pedido ao próprio binário (`auth.command` → `owcli --ow-token`), então o token nunca
  passa por argv nem por variável de ambiente;
- converte os modelos do `openweights.json` para o catálogo do Codex com os tipos Rust
  do upstream: uma mudança de esquema quebra na compilação do merge, não na máquina de
  ninguém.

No modo OwCLI o provedor também declara que não aceita ferramenta em namespace nem as
hospedadas da OpenAI. O Codex manda as ferramentas do MCP como `type: "namespace"`, e o
shim de Responses do llama.cpp só aceita `function` (llama.cpp#24295): descartava as
ferramentas sem erro. O fork as achata em `mcp__servidor__ferramenta` e reconhece esse nome
quando a chamada volta.

Ele se recusa a abrir uma conversa, com código 2 e uma mensagem nos dois idiomas, em três
casos: sem o `openweights.json`, com o gateway sem responder (o app foi fechado) e com o
catálogo vazio. Sem essa última recusa, o Codex cairia no modelo padrão dele, um da OpenAI.

## Abrir uma sessão do agente

`terminal_abrir_owcli` recebe pasta, aprovação, sandbox, modelo e, ao continuar, o id da
conversa. Valores de aprovação e sandbox fora da lista caem no recomendado
(`on-request`, `workspace-write`); id de conversa e modelo são validados antes de ir para a
linha de comando. O PTY leva `OWCLI_HOME`, `TERM_PROGRAM=OpenWeights` e
`CODEX_TUI_DISABLE_KEYBOARD_ENHANCEMENT=1`, porque o xterm.js não responde à sondagem do
protocolo de teclado do kitty.

O diálogo pede os modelos a `owcli_modelos`, que devolve o catálogo e o estado das
fontes. Sem modelo nenhum, mostra os cartões de fonte (os mesmos do AgenticOw).

O "+" da lista, o estado vazio, cada painel vazio da grade e a paleta (Ctrl+K) oferecem
sempre as duas saídas, agente e terminal. O caminho do agente relê `owcli_status` e decide:
instalado, abre o diálogo acima; com pacote para a máquina mas sem instalar, oferece a
instalação com o tamanho (só baixa se a pessoa aceitar); sem pacote, diz que o agente não
está disponível ali. A 0.25.0 saiu sem o agente, porque não havia runtime publicado.

## O runtime e os pins

`binario()` procura, nesta ordem, `OW_OWCLI_BIN` (build de desenvolvimento do fork) e o
runtime pinado instalado em `<data>/runtimes/owcli/<tag>/`, conferido pelo `runtime.json`
(`name`, `format`, `revision`, `target` e `entry` têm de bater com o pin e o alvo). O
executável se chama `bin/owcli` (`bin/owcli.exe` no Windows): com outro nome o lançador do
fork não entra no modo OwCLI.

`owcli_instalar` baixa o pacote do alvo, confere tamanho e sha256 contra o `pins.json`
embutido (o pin herda a assinatura do updater), extrai, confere a identidade e instala de
forma atômica. Depois roda `owcli --version` numa casa descartável dentro da pasta do
runtime (no `/tmp` o Codex não cria os atalhos do arg0); um pacote que não roda na máquina é
removido e a tela oferece tentar de novo. O progresso sai pelo evento `owcli-runtime`
(`phase` = `download`, `verifying`, `extracting`, `checking`, `installed`; `progress`;
`failed`). As versões que não são a pinada saem no boot e depois de instalar: este processo
só abre sessões com o executável do pin atual.

Enquanto o `pins.json` não tem pacote (`assets: {}`), o agente aparece como "não disponível
aqui" em todo lugar, exceto com `OW_OWCLI_BIN`. Para testar a instalação de ponta a ponta
antes de publicar, só em build de desenvolvimento: `OW_OWCLI_PINS=<outro pins.json>` e
`OW_OWCLI_URL_BASE=<http://…>` (um `python3 -m http.server` na pasta do pacote serve).

### Subir um pin novo

1. Trocar `OWCLI_REVISION` e `OWCLI_TAG` no `owcli-runtime.yml` (a tag leva os 8
   primeiros caracteres da revisão).
2. Rodar o workflow com `publish` desligado até os alvos passarem, depois ligado: ele
   publica a pré-release (nunca a latest, que é de onde o updater lê) com o `pins.json`.
3. Copiar esse `pins.json` para `src-tauri/crates/owcli/pins.json`. O teste
   `os_pins_embutidos_sao_validos` confere tag, revisão, nomes, hashes e tamanhos.

## Histórico

Quem lê as conversas gravadas é o próprio OwCLI: `owcli app-server` fala JSON-RPC, uma
mensagem por linha, pelo stdio. Cada consulta sobe o processo, faz `initialize`, o pedido
(`thread/list` ou `thread/name/set`) e fecha o stdin, e o processo sai sozinho. São 0,1 s,
então não há servidor vivo para supervisionar. Continuar uma conversa é
`owcli resume <id>` numa sessão nova, com o modelo dela se ainda estiver no catálogo.

## Testes

- `lr_pty`: sessões com `sh -c` reais (OSC, anel sem buraco, ack segurando o produtor,
  encerramento sem órfão conferido em `/proc`, o ambiente do sistema e o do pedido
  chegando ao filho). O teste-barreira do `lr_proc` garante que nenhum PTY nasce com o
  ambiente do AppImage.
- `lr_owgw`: contra uma fonte falsa em hyper (401 sem token, rota e chave por fonte, SSE
  byte a byte e em pedaços, 404 que diz a fonte que falta, troca de token a quente).
- `lr_owcli` e `lr_fetch::pins`: pin embutido válido, identidade do `runtime.json` campo a
  campo (inclusive o nome `owcli` da entrada), instalado só com entrada e identidade, poda
  só do que não é a versão pinada, e instalação sem pacote que diz por quê.
- `commands_owcli` e `owcli_historico`: o arquivo nunca leva chave de fonte, nasce `0600`,
  ids com prefixo, validação do que vai para a linha de comando, e a conversa JSON-RPC
  contra um `app-server` de mentira em `sh`.
- Playwright (`tests/e2e/owcli.spec.ts`), com o backend simulado de `lib/terminals.ts`:
  abrir, digitar, grade, avisos, o diálogo do agente, o histórico e o painel sem modelos,
  a instalação do agente (tamanho antes, progresso, falha com nova tentativa), o aviso sem
  pacote para a máquina, o agente pelo painel vazio e pela paleta, além do portão de
  acessibilidade.
- App real: `OW_OWCLI_BIN` aponta para um build do fork (sem o runtime pinado, a tela diz
  que o OwCLI não está instalado), e `OWCLI_HOME` para uma pasta descartável, para não
  tocar em `~/.owcli`. No Linux, o Xvfb do box com `XDG_DATA_HOME` isolado.

## O que ficou para depois

- **`owcli` no PATH**, para usar os modelos do app a partir do terminal do sistema.
- TUI em português, visualizador de transcrição, arquivar e apagar conversas.
