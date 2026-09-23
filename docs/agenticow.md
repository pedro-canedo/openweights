# AgenticOw — o fork, o runtime e o protocolo

O agente de código do OpenWeights é o **AgenticOw**, um fork do
[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) mantido em
[pedro-canedo/agenticow](https://github.com/pedro-canedo/agenticow). Este documento é
o lado do app: como o runtime chega, como o processo é supervisionado, o que passa
pelo canal de controle e como subir uma versão nova. O lado do fork (composição,
plugins, tradução, achados da Fase 0) está no `FORK.md` do próprio fork.

## Por que um fork

Até a 0.23 o app instalava o pacote npm `@deepseek-ai/dsh` numa versão fixada no
código. Isso travou em três pontos:

- **Não atualizava.** A 0.1.5 cai no boot do modo `web` (HMR com `patchReload: "live"`)
  e trocou a autenticação por um cookie `HttpOnly; SameSite=Strict`, que um iframe
  cross-site nunca recebe. O app ficou na 0.1.1-rc.2.
- **Não personalizava.** Marca, persona, provedores e idioma só entravam editando o
  `settings.yaml` de outro projeto por fora — e isso quebrava a cada versão.
- **Custava caro na máquina da pessoa.** Resolver a árvore npm levava de dez a trinta
  minutos na primeira abertura.

O modelo é o do Cursor com o VS Code: núcleo do upstream, camada nossa por cima,
sincronização periódica. Casca e runtime formam uma unidade de release — o AgenticOw
se atualiza junto com o app.

## Licença e marca

- O código do upstream é MIT; `native/system` é BSD-3-Clause. O runtime leva
  `LICENSE` e `THIRD_PARTY_LICENSES.txt` com os avisos da closure inteira.
- "DeepSeek Harness" é marca registrada. O produto se chama AgenticOw; a relação
  aparece só como "baseado no DeepSeek Harness (MIT)", e nenhum material de marca
  oficial é usado de forma que sugira endosso.

## Regras do fork

1. Tudo o que é nosso mora em `apps/openweights-*` (`@openweights/*`, `private: true`,
   JavaScript sem build). A composição é um bundle nosso empilhado **depois** do
   `web-app` — sobrepõe, nunca substitui.
2. Arquivo do upstream só é editado onde não há *seam*, e cada edição entra na tabela
   do `FORK.md` com o motivo e a forma de reaplicar.
3. Nomes internos continuam `@deepseek-ai/*`: nada é publicado no npm, e renomear
   transformaria cada sincronização em conflito. Muda só o que a pessoa vê.
4. Sincronização **só por tag rc**, com merge (sem rebase) num PR
   `sync/upstream-<tag>`. A `pnpm-lock.yaml` é **regenerada**, nunca mesclada à mão.
   Uma rc pode ser pulada.

A tag base fica em `apps/openweights-host/package.json` (`agenticow.upstreamTag`),
porque o checkout raso da CI não carrega as tags do upstream.

## Onde as coisas moram no app

| O quê | Onde |
|---|---|
| Crate | `src-tauri/crates/agenticow` (`lr_agenticow`): `pins`, `install`, `host`, `protocol`, `catalog`, `migrate` |
| Comandos e eventos | `src-tauri/src/commands_agenticow.rs` (canal de eventos `agenticow`) |
| Janela principal | `src-tauri/src/janela.rs` — criada em código para poder receber a webview filha |
| Tela | `src/screens/AgenticOw.tsx`, estado em `src/lib/agenticow.ts` |
| Runtime instalado | `<dados>/runtimes/agenticow/<tag>/` |
| Home (`DSH_HOME`: sessões, configurações, profiles) | `<dados>/agenticow-home` |
| Home da era do DeepSeek Harness | `<dados>/dsh-home` — só lido, na migração |
| Instalação npm antiga | `<dados>/providers/dsh` — não é apagada pelo app |
| Porta estável | setting `agenticow.config` (`{porta}`); preferida 11730 |

## Ciclo de vida

`agenticow_start` (ou o convite do Chat, ou a tela ao montar) faz, nesta ordem:

1. Recusa com `agenticow-unsupported` se `pins.json` não tem pacote para o alvo
   (`linux-x64`, `win32-x64`, `darwin-arm64`, `darwin-x64`).
2. Garante o Node portátil (`lr_nodejs`, o mesmo `PINNED_NODE` com que o runtime foi
   compilado).
3. Se o home novo não existe, **copia** o antigo (`migrate::migrar`).
4. Se o runtime pinado não está instalado, baixa, confere e instala
   (`install::instalar`).
5. Escolhe a porta (preferida, depois qualquer livre — duas tentativas, porque a porta
   pode ser tomada entre a checagem e o bind) e sobe o Host.
6. Espera o `ready` (até 120 s), manda o `locale` e o primeiro `catalog`.
7. Depois de uma subida boa, **poda** as versões antigas do runtime.

Parar manda `shutdown`, fecha o stdin e espera até 7 s antes de matar a árvore de
processos. Fechar o app faz o mesmo em `shutdown_blocking` (só o EOF, que já é um
desligamento gracioso no Host), e o pid guardado em `agenticow_pid` é a rede de
segurança para matar a árvore quando o slot está ocupado nessa hora.

## A interface: webview filha, não iframe

O Host autentica com `?token=` → 303 → cookie `SameSite=Strict`. Num iframe
(`tauri.localhost` → `127.0.0.1`) o cookie é de terceiro e nunca chega — foi medido
no Windows e no macOS. Por isso a interface é uma **webview filha** da janela
principal (`Window::add_child`, feature `unstable` do Tauri), que navega em primeira
parte:

- nasce na primeira vez e depois só é mostrada, escondida e reposicionada
  (`agenticow_show`, `agenticow_hide`, `agenticow_set_bounds`) — trocar de tela não
  recarrega a sessão;
- a tela observa o próprio retângulo (`ResizeObserver`) e esconde a webview enquanto
  há um diálogo do app aberto (`[role=dialog]`, `[aria-modal=true]`), porque uma
  webview nativa ficaria por cima dele;
- **não tem IPC do Tauri**: a URL é remota, e o Tauri recusa comando de origem remota
  sem uma capability `remote` — que não existe;
- **no Linux, a posição é nossa**: o Tauri põe toda webview da janela numa `GtkBox`
  vertical com expand, e o `set_bounds` do wry não age ali — as duas webviews dividiam
  a janela ao meio. O `janela.rs` monta um `GtkOverlay`: a webview do app é o conteúdo
  (acompanha o tamanho sozinha) e a do AgenticOw vira uma camada por cima, posicionada
  por margens. Windows e macOS usam `set_position`/`set_size`.

A URL autenticada carrega o token de lançamento: nunca vai para evento nem para log
(`protocol::redigir` limpa qualquer `token=` das linhas do stderr e das mensagens de
erro antes de irem para a tela ou para o log).

## Protocolo de controle (versão 1)

Uma mensagem JSON por linha, sempre com `"ow": 1`. O stdout do Host é exclusivo do
protocolo — o bootstrap desvia o resto para o stderr antes de importar o dsh — e o app
descarta linha sem a marca. Tipo que o app não conhece é ignorado, então um Host mais
novo não derruba um app mais velho.

| Direção | Tipo | Campos |
|---|---|---|
| Host → app | `hello` | `protocol`, `host`, `revision`, `upstreamTag`, `dsh`, `node`, `nodeAbi`, `pid` |
| Host → app | `ready` | `url` (autenticada), `port` |
| Host → app | `fatal` | `message` |
| Host → app | `catalog-applied` / `catalog-error` | `revision` (+ `message`) |
| app → Host | `shutdown` | — |
| app → Host | `locale` | `locale` (`pt-BR` ou `en`) |
| app → Host | `catalog` | `revision`, `piAi` (a seção `llm-pi-ai` inteira), `env` (chaves `*_API_KEY`) |

EOF no stdin também desliga: se o app morrer sem avisar, o Host não fica órfão. No
Host, o desligamento passa pelo handler de `SIGTERM` do próprio CLI
(`process.emit`), que faz o dispose da árvore — um `kill` no Windows mataria sem
dispose. Comando que chega antes de o plugin dono se registrar fica guardado (o último
de cada tipo).

Os dois lados estão em `src-tauri/crates/agenticow/src/protocol.rs` e, no fork, em
`apps/openweights-host/src/canal.mjs` e `apps/openweights-plugins/src/`.

## O catálogo de modelos

O app monta a seção `llm-pi-ai` (`catalog.rs`) com até três rotas:

- `openweights` — os modelos que o Router atende agora, com janela de contexto, teto de
  saída (metade da janela) e os níveis de raciocínio que o chat template aceita. Com o
  Jev ligado para harnesses, a URL base é a do proxy do Jev.
- `openrouter` — os favoritos, com o provedor ligado e chave.
- `ninerouter` — o catálogo do 9router, instalado e no ar.

O plugin `catalog.js` do fork aplica pela API de configurações, com validação de schema,
e só repara o modelo padrão quando ele está ausente ou quebrado. As chaves ficam na
memória do Host. O adaptador as pede ao serviço de credenciais pelo nome, a cada
requisição, e esse serviço lê o ambiente num retrato congelado na subida — gravar no
`process.env` depois não chegava a ninguém (o primeiro turno falhava com
`MISSING_CREDENTIAL`). O plugin envolve a camada `process` desse retrato para consultar
antes as chaves do app, confere cada uma pelo próprio serviço e responde
`catalog-error` se alguma não chegar. Trocar uma chave não reinicia nada, e nenhuma
toca arquivo.

Com o catálogo vazio (sem servidor local nem provedor), o onboarding do upstream
oferece a conta da DeepSeek; o status do app conta os modelos do último catálogo
(`models`) e a barra da tela avisa que não há modelos do OpenWeights, com o atalho para
o Servidor Local.

`agendar_catalogo` reenvia com debounce de 400 ms (rajadas viram um envio). É chamado
pelo `sincronizar_shim` do Jev — que já roda quando o motor sobe ou desce, a
configuração de provedores muda ou o Jev muda —, pelo 9router ao subir e parar, ao
apagar um modelo e ao terminar um download. Quem acrescentar outra fonte de modelos
chama `agendar_catalogo` no mesmo lugar em que a fonte muda.

## O runtime e os pins

O runtime é a árvore de produção autocontida do Host (`pnpm deploy --prod --legacy`
hoisted), montada pelo `prepare-runtime.mjs` do fork. O
`.github/workflows/agenticow-runtime.yml` deste repositório:

1. faz checkout do fork na revisão `AGENTICOW_REVISION`;
2. compila e empacota nos alvos (`linux-x64`, `win32-x64`, `darwin-arm64`; `darwin-x64`
   quando houver runner Intel);
3. roda a suíte do fork **sobre o pacote extraído**, não sobre a árvore de build;
4. com `publish=true`, publica tudo na release `AGENTICOW_TAG` deste repositório, com
   um `pins.json` (sha256 e tamanho por alvo).

O `pins.json` entra no binário (`include_str!`), então herda a assinatura do updater:
trocar o arquivo na release não muda o que o app aceita. Na instalação, o pacote é
conferido por sha256 e tamanho e o `runtime.json` de dentro dele é conferido contra os
pins (`name`, `format`, `revision`, `target`, `entry`) — qualquer divergência recusa
a instalação, sem deixar nada pela metade.

### Subir um pin novo

1. No fork, a revisão nova na `main`, com a CI verde nos três sistemas.
2. Neste repositório, em `agenticow-runtime.yml`: `AGENTICOW_REVISION` (SHA completo),
   `AGENTICOW_TAG` (`agenticow-runtime-<8 primeiros do SHA>-v1`; `-v2` para reempacotar
   a mesma revisão) e o `concurrency.group`.
3. Commit e push; depois `gh workflow run agenticow-runtime.yml -f publish=true`.
4. Copiar o `pins.json` da release para `src-tauri/crates/agenticow/pins.json` e rodar
   `cargo test -p lr_agenticow` (o teste dos pins confere formato, repositório e
   revisão).
5. O pin novo sai no próximo release do app; quem atualizar baixa o runtime novo na
   próxima abertura.

Se o fork mudar a versão do Node, o `PINNED_NODE` do `lr_nodejs` muda junto: os
módulos nativos (node-pty, fs-ext) são compilados para a ABI do Node do build.

## Migração do DeepSeek Harness

Na primeira subida sem `<dados>/agenticow-home`, o app copia `<dados>/dsh-home` para lá
por um diretório de staging (`agenticow-home.migrando`), renomeado só no fim. Não vêm:
`profiles/` (apontam por links absolutos para a árvore npm antiga; o runtime recria o
dele), arquivos `*.lock` e links simbólicos. O original fica intacto, então um downgrade
encontra tudo. As sessões gravadas pela 0.1.1 estão no formato 0, e o runtime novo traz
a cadeia de migração que as converte ao ler.

## Testes

- `cargo test -p lr_agenticow`: parser do protocolo, redator do token, pins, identidade
  recusada, instalação e poda, catálogo e migração, e um Host falso em Node para
  `ready`, `shutdown`, EOF e `fatal`. Sem `node` no `PATH`, aponte
  `AGENTICOW_TEST_NODE` para um executável; sem nenhum, esses testes pulam.
- No fork, a CI roda o Host de ponta a ponta pelo protocolo, a composição (inclusive o
  teste de que as linhas de telemetria desligadas continuam existindo no upstream), a
  UI em pt-BR e inglês no WebKit e a webview filha com o webview real de cada sistema.

## O que ficou para depois

- **Pontes nativas.** Escolher a pasta do projeto pelo diálogo do sistema (o Host
  pediria ao app) e avisar, por notificação do sistema, que há uma aprovação pendente
  quando a tela do AgenticOw está escondida. Sem as pontes, a pasta é escolhida pelo
  navegador de pastas da própria interface (conferido no Linux, em pt-BR, com caminho
  editável), e a aprovação só aparece com a tela aberta.
- **Limpeza da instalação npm antiga** (`<dados>/providers/dsh`): o app não oferece
  apagar; dá para remover à mão.
- **Sincronização automática com o upstream** (detectar rc nova, fazer o merge,
  regenerar a lockfile e abrir o PR): hoje é manual, seguindo as regras acima.
- Fora do escopo da v1: conta DeepSeek, runtime Python/Office, browser-use e
  computer-use, e instalação de plugins pelo usuário.
