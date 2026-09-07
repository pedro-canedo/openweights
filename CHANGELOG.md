# Changelog

Every released version of OpenWeights, newest first. Installers live on the
[releases page](https://github.com/pedro-canedo/openweights/releases); the
narrative of the version you would download today is in
[What's new](https://pedro-canedo.github.io/openweights/guide/whats-new).

Release notes for 0.16.0 onwards are written in `docs/releases/<version>.md`,
which is the single source this file, the GitHub release body and the site all
come from. Entries below the marker further down were recovered from the
published releases and the version commits, and are kept as history.

## [0.17.0] — 2026-09-07

- Smoother chat with virtualized history, 50 ms streaming updates and deferred
  syntax highlighting. Cancellation preserves partial responses.
- Explicit waiting phases, first-token/first-answer timings and persistent
  run metrics. Missing engine metrics are shown as unavailable.
- Capacity-aware local scheduling, bounded retries and lower-priority title
  generation. Remote providers remain independent of the local queue.
- **Test a better configuration** under Local Server → Performance: warmup,
  three runs per configuration, median/range, apply and restore. Context,
  quantization and reasoning effort are preserved.

The synthetic development fixture reduced median React render CPU by 91.0%
with 500 history messages. This does not claim faster inference. A confirmation
with nine inference samples per client on Windows met the 5% median
non-regression threshold; first-token jitter remains documented.
The comparator requires an idle server and temporarily stops the local API;
avoid external clients and other GPU workloads while testing.

## [0.16.1] — 2026-09-06

### Fix — please update if you are on 0.16.0

The engine card in 0.16.0 said **"the engine is installed but won't run"** on
every machine, even while llama.cpp was serving a model. Nothing was wrong with
the engine: the check read the wrong number.

`llama-server --version` prints
`version: 0.1.0-dev (build 10441, commit 0177dcc73)` — and the check read the
first number after `version:`, which is the zero in `0.1.0-dev`. Since the
folder is named `b10441` and the binary "answered" build 0, the verdict was a
mismatch between folder and contents. The build number now comes from `build`,
where it always was, and the test that should have caught this uses the
binary's real output instead of a format that was assumed.

Everything else in [0.16.0](https://github.com/pedro-canedo/openweights/releases/tag/v0.16.0)
is unchanged.

## [0.16.0] — 2026-09-06

**The configuration screens stopped being piles of cards.** Local Server now
has a pinned status bar — running or stopped, the address with a copy button,
the loaded model and generation speed — and four tabs organised by the question
you arrive with: Overview, Performance, Network, Advanced. On a fresh install,
Overview opens with three numbered steps that disappear once the server answers
its first request. Settings and Sources were reordered by what matters, and the
OpenRouter catalogue finally says how many models you have pinned (only pinned
ones reach the chat picker).

**The AI engine is now verified, not assumed.** The old card read "b10441 ·
cuda13 · installed" from the tag the app *would* install plus the presence of a
file — a CUDA package missing its cudart DLLs passes that check and fails on the
first model load, and anyone with an old build read the new version number and
concluded they were up to date. The app now runs `llama-server --version`,
reads the build the binary reports, and tells you which of five situations you
are in, each with its own fix. It checks on its own shortly after startup and
marks the Settings item in the sidebar when something needs attention. Old
builds left on disk are listed with their size and a button to reclaim the
space.

**The harness reports the version that is actually installed** (read from the
package's own `package.json`) and compares it with the one this release ships —
a mismatch becomes "update pending", with the button that resolves it.

**Model cards render as text again.** Hugging Face READMEs are Markdown mixed
with HTML; the app used to show that HTML literally. It is now translated to
plain Markdown — links, images, tables and code blocks included — before it
reaches the screen.

**The brand in the app is the same geometry as the icons**, generated from a
single source instead of being redrawn by hand in two places.

<!-- gerado-ate-aqui: acima desta linha o conteúdo vem de docs/releases/;
     abaixo é histórico recuperado, escrito à mão, que o gerador preserva -->

## [0.15.2] — 2026-08-31

The Coder answers again, and the Windows installer leaves a desktop shortcut.

## [0.15.1] — 2026-08-31

The model listings show the Hub author's picture, in Discover and in My Models.

## [0.15.0] — 2026-08-31

### Reasoning effort becomes what the model actually accepts — and the status bar starts reporting

Three things the app knew and never showed, and one it offered wrong.

**The reasoning effort was undersized.** The harness got two levels, off and
on — and "on" landed on `xhigh`, the most expensive one there is. That is what
blew the output budget on an open-ended request: the model spent the whole
allowance thinking and stopped before writing its first file.

The chat template inside the GGUF states which levels it accepts, and refuses
anything else with an exception the server returns as a 500. The app now reads
that line and declares exactly those names. Measured against the real server,
same question: `low` produces 583 characters of reasoning, `xhigh` produces
5,742 — ten times more. A template without that validation keeps the plain
on/off switch instead of being handed invented names.

**The status bar reports live.** Which model is up, how much of the window is
already used, and how fast it is answering right now. Context comes from
`/slots`, which belongs to the server rather than to a model — asking does not
wake a sleeping one. Speed comes from the difference of the cumulative decoded
token counter between two reads, because llama.cpp's rate gauges reset on every
scrape and two readers would spoil each other. Since the source is the server,
the number counts the harness and any external app, not just the chat here.

**Watts join the arithmetic.** The bar shows draw and limit next to the GPU,
and a new card switches between the card's stock limit and an efficient target
— both from the driver, neither invented. Generating tokens is bound by memory
bandwidth, not by how much the card may burn, so lowering the limit usually
costs almost nothing in speed. *Usually* is the word: the card tells you to
measure rather than repeating the claim, and for that the performance history
now records the limit in force on every run. Without that column, measuring the
same configuration at 370 W and at 250 W produced two identical rows, and the
delta credited the configuration with a difference that came from watts.

Two honesties the cards state rather than hide: applying the limit needs
administrator rights, and it does not survive a reboot — that is NVIDIA's
design, and no application works around it.

## [0.14.0] — 2026-08-31

A big model fits a small card, and the app says where the ceiling is.

## [0.13.0] — 2026-08-31

### Discovery becomes a list and a panel — and answers before you download

The Discover screen stacked full-width cards with six statistics each: seven
results per screen, comparing two meant scrolling, and the only way to see the
quantizations was a drawer that covered the list — closing it lost your place.
Worse, finding out what a model *is* meant leaving the app for the Hub.

Now it is two columns. The narrow list is for choosing: name, author, size,
when it changed, and what the model can do. The panel on the right is for
deciding, and it answers the questions in the order they come up: **does it fit
my machine** (the quantizations with their hardware verdict, the recommended
one highlighted, a download button on every row), **what is this** (parameters,
architecture, context, licence, capabilities) and **what does the author say**
(the repository's README, rendered right here). The first row comes selected.

Capabilities — vision, tools, reasoning — do not come from the model's name,
which is wrong in both directions. Vision is the `pipeline_tag` its own author
chose; tools and reasoning come from the chat template llama.cpp executes: if
the branch is there, the capability is real. It is the same `enable_thinking`
marker the app already looks for in a downloaded GGUF to offer the harness its
reasoning switch.

The README arrives through a command rather than a fetch from the interface, so
it reuses the Hub token the app already holds — without it, a repository whose
licence you must accept answers 401 and the screen would claim "no description"
for a model that has one. The card's YAML header is stripped, and embedded HTML
is not rendered: model cards carry other people's `<div>`s and images, and a
desktop app has no business executing a stranger's markup.

## [0.12.0] — 2026-08-30

### Thinking gets a switch, output gets a cap, and the sidebar folds away

An open-ended request to the harness — *"let's build a beautiful site with
three.js effects"* — came back cut off, with **Output token limit reached** and
not a single file written. The session record tells the rest: 32,768 output
tokens (the cap the harness assumes when nobody declares one), of which 66,000
*characters* were reasoning and 331 were text. The model wrote the whole site
inside its own thinking — `import * as THREE` appears four times in one block —
and ran out of budget before calling its first tool.

None of that was a context problem. What was missing were two things the app
knows about your models and never told the harness.

- **A reasoning switch.** When a model's chat template reads `enable_thinking`
  (Qwen3 and friends), the route now declares the effort levels and the wire
  format, and the harness shows an effort selector whose **Off** really turns
  thinking off. Detection reads the template inside the GGUF rather than
  guessing from the model's name — the template is what llama.cpp executes, so
  if it reads the variable, the button works. Measured against a real server:
  the same question answers in 2 tokens with thinking off, 30 with it on. The
  route default stays high, because gaining a button must not change how the
  model behaves for someone who never touched it.
- **A declared output cap.** Each local model now declares `maxTokens` — half
  its own context window, between 2k and 64k — instead of the harness assuming
  32k for every model, a promise a 4k-context model cannot keep. Remote routes
  (OpenRouter, 9router) keep the harness defaults: their cap belongs to the
  provider, and asking for more than it accepts is a 400 mid-conversation.
- **A sidebar that folds.** One click turns it into a 56px column of icons,
  giving ~180px back to the stage — which matters now that the harness runs
  embedded, a whole application inside our frame. The choice survives restarts.

## [0.11.0] — 2026-08-30

### The harness gets a screen of its own — and finally installs

The DeepSeek Harness never opened: the button said *Loading…* forever and
nothing was ever installed. This release fixes that and gives the harness the
place it deserves in the app.

- **The install actually finishes.** `npm` writes its whole progress log to
  stderr, and the app was reading stdout only. The other pipe filled up (64 KB
  on Windows), npm blocked on a write, and the app waited for an EOF that could
  never arrive — with the ~190 packages of the dsh monorepo, every single time.
  Both pipes are drained now, so the install runs to the end and its log
  reaches the screen while it does.
- **A DeepSeek Harness item in the sidebar**, right below Chat. Install, start,
  stop and uninstall live there, with the real state of the install, a live log
  and a timer — and once it is up, the harness runs **embedded in the app**,
  not in a separate window. The window is still one button away.
- **You can uninstall it from the app**, and choose whether to also delete the
  sessions and credentials created inside the harness.
- **Honest waiting.** Resolving the dsh dependency tree takes minutes and gigabytes;
  the app now gives npm a large heap, allows up to 45 minutes, and says so on
  screen instead of pretending it is about to finish. You can leave the screen —
  the install keeps going.
- **It opens without a local server too.** If llama.cpp is not up, the harness
  still starts with your remote providers instead of failing the whole launch.
- The **Agent** button in the chat composer and the DeepSeek Harness card in
  Local Server now take you to that screen rather than starting minutes of work
  behind a small button.

## [0.10.0] — 2026-08-30

### The agent moves out — and takes the keys to every provider

**Breaking change**: the built-in agent mode (agent loop, Code Mode, automations, MCP, memory, RAG, Activity) has been discontinued and removed. OpenWeights now focuses on being the best local provider a harness can point at — and opens the harness for you.

- **DeepSeek Harness, one click from the chat.** The Agent button installs dsh into an isolated folder (portable Node, pinned package, nothing touches your system), starts its web mode and opens the panel in an app window. No terminal, no configuration.
- **Every model, every provider.** The app writes your whole catalogue into dsh's settings: local llama.cpp models (with each profile's context window), your OpenRouter favourites and the 9router catalogue — keys travel by environment variable, never in a file.
- **Surgical config writes.** dsh's settings file also holds the choices you make in its own UI; OpenWeights follows dsh's own lock protocol and touches only the provider section.
- Simple chat stays (streaming, markdown, @file, history), and everything else — Local Server with API key/serving stats/benchmarks, Discover, providers, cluster — is untouched.

If you rely on the old built-in agent, stay on v0.9.0.

## [0.9.0] — 2026-08-29

### The server counts what it serves — and speaks Claude

- **Serving stats.** A new card shows total tokens processed, tokens reused from the KV cache, cache efficiency and average prompt/generation speeds — for **all** traffic the server handles, external apps included, straight from the engine's own counters. Session and all-time views (persisted across restarts), per-model filter, one-click clear.
- **The Claude door.** The bundled llama.cpp serves Anthropic's Messages API natively; the Connect card now shows the Claude API URL next to the OpenAI one, with a ready environment block — and **Claude Code** joins the one-click harness list, every tier pointed at your local model.
- **Addresses that work from where you are.** With LAN access on, the Connect card lists your machine's real addresses (WSL and containers can't reach the Windows `127.0.0.1`); with it off, it tells you why.
- Plus: prompt tok/s column in the benchmark history, and the chat now reads the cache fields the server always sent.

## [0.8.0] — 2026-08-29

### The local server becomes a real provider

Point any OpenAI-compatible tool at OpenWeights — one click, no guesswork.

- **API key, end to end.** Generate a `sk-local-…` key in one click. The new **Connect** card hands you everything an external app asks for: base URL (`/v1`), key, loaded model id, plus a copy-paste environment block. A **Test connection** button tells the truth (401 means the running server still has the old key — restart applies it). The key travels via environment variable, never argv, and the internal chat keeps working with auth on.
- **Everything enumerable is now a button or a dropdown.** Port presets (11711, 8080, 1234 and 11434 — apps configured for LM Studio or Ollama connect without changing a thing), selects for concurrent models and parallel requests, chips for LAN, batch/ubatch and speculation settings. Legacy values are never silently overwritten.
- **Benchmark history, per GPU.** Measurements the tuner already recorded now reach the screen: a history card shows the series for your current machine (GPU + driver keyed), with **Δ% vs the previous comparable run** — refusing to compare across engine builds or thermally-suspect runs. Rows record the GPU name and the readable config pairs (`ngl=99 · ctx=16k`), and the chat's real-usage tokens/s shows up aggregated per configuration.

## [0.7.0] — 2026-08-23

Every llama.cpp switch becomes interface: the full flag catalogue moves out of the chat and into Local Server.

## [0.6.1] — 2026-08-21

"Good morning" stops turning into a work plan.

## [0.6.0] — 2026-08-21

The app tunes itself on any machine.

## [0.5.1] — 2026-08-21

The official llama.cpp package already shipped RPC — the overlay build is gone.

## [0.5.0] — 2026-08-20

The other PC's card joins the arithmetic.

## [0.4.1] — 2026-08-20

The agent loop stops giving up on a long spec.

## [0.4.0] — 2026-08-20

The harness stops taking the model's word for it.

## [0.3.0] — 2026-08-19

Session terminal: command output in real time.

## [0.2.1] — 2026-08-19

Primeira versão deste ciclo com instaladores de verdade: a `v0.2.0` foi
publicada sem binário nenhum (o rascunho com os arquivos se perdeu), e este
release a substitui.

**Code Mode** — o modelo escreve um programa que usa as ferramentas de uma vez,
em vez de pedir uma por passo. Medido nesta máquina: 3,4× mais rápido e 7,4×
menos idas ao modelo. O programa roda isolado, e toda chamada continua passando
pela política, pela confirmação e pelo checkpoint.

**Fontes de modelo** — tela própria para provedores externos: OpenRouter com
catálogo público (preço, contexto, suporte a ferramentas) e o 9router
instalado, supervisionado e removido pelo app numa pasta isolada, com painel em
janela própria. Os modelos e combos que ele publica aparecem no seletor do chat.

**Ponto de entrada único (opcional)** — um Traefik local que encaminha um
endereço só para o motor local e para o 9router, por prefixo. Não é túnel: nada
fica alcançável pela internet.

**Code Mode no macOS e no Windows** — o programa roda sob o modo de permissões
do Node, e o caminho entregue a ele precisava de normalização por sistema: no
macOS o link de `/var` derrubava o programa antes da primeira linha; no
Windows, o prefixo verbatim do caminho canônico fazia o mesmo. Nas duas
plataformas o Code Mode não subia.

**Correção de layout** — a interface inteira rolava quando uma coluna alta (a
trilha da execução, o explorador de arquivos) passava da altura da janela, e o
compositor ia junto. Agora quem rola é a tela; o palco fica parado.

**Documentação** — o projeto ganhou site: instalação, modelos e quantização, o
harness agêntico explicado por partes (autorização, ferramentas, planos, Code
Mode, checkpoints, memória, MCP, automações) e as integrações, em inglês e
português: <https://pedro-canedo.github.io/openweights/>

## [0.2.0] — 2026-08-18

API key handling for the 9router configuration.

## [0.1.1] — 2026-08-18

A `.deb` install no longer offers an update button that would fail, and macOS joins automatic updates.

## [0.1.0] — 2026-08-18

First public release of **OpenWeights** — run local GGUF models with an agent that actually does the work: reads and edits files, runs commands, uses Git, browses the web, and checkpoints the project before it touches anything.

| System | File |
|---|---|
| Windows 10/11 (x64) | `OpenWeights_0.1.0_x64-setup.exe` |
| macOS 11+ (Apple Silicon and Intel) | `OpenWeights_0.1.0_universal.dmg` |

**No binary is signed** — signing needs a paid yearly certificate the project doesn't have yet, so your system will warn you:

- **Windows**: on "Windows protected your PC" → *More info* → *Run anyway*.
- **macOS**: right-click the app → *Open* on first launch, or run `xattr -cr /Applications/OpenWeights.app`.

On first launch the app downloads the llama.cpp runtime that matches your GPU (CUDA, Vulkan or CPU — a few hundred MB). That is why the installer is small: no GPU stack ships inside the package.

#### What's in it

- Hardware detection and a matching llama.cpp build, installed for you.
- Hugging Face GGUF search with a per-quantization verdict for *your* machine.
- Agent mode by default, with per-action approval and a project snapshot before the first change.
- "Tune for this machine": asks llama.cpp what each configuration costs on your card, applies one, and rolls back on its own if the model fails to load.
- 30+ tools — files, terminal, Git, web, data, MCP connectors — curated to fit the model's context window.
- Live CPU, RAM, GPU, VRAM, disk, network and tokens/s while you talk.

Known limits: models are only as good as they are — small ones still need the harness to nudge them, and the eval suite in `crates/agent/tests/live_model.rs` is how we measure that. Bug reports welcome.

---

[Comparar versões](https://github.com/pedro-canedo/openweights/compare) ·
[Todas as releases](https://github.com/pedro-canedo/openweights/releases)
