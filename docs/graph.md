# Grafo de código — o índice que diz *onde olhar*

Plano de trabalho e caderno de medições do `lr_graph`. Mesma regra do resto do
projeto: número medido **nesta máquina**, com o que falhou junto.

## O problema

O público deste app roda modelo local, muitas vezes com 32k — e às vezes com 8k — de
janela. O gargalo do harness não é a inteligência do modelo: é que **descobrir onde
as coisas estão consome a janela antes de o trabalho começar**.

Medido neste repositório em 2026-08-19:

| | tokens |
|---|---:|
| Corrigir a rolagem da interface exigiu ler `App.tsx` + `Chat.tsx` + `WorkspacePanel.tsx` + `RunTimeline.tsx` | **~26.204** |
| Só `run.rs`, para entender o laço do agente | **~28.948** |
| Janela de um 8B típico | 8.192 |
| Vizinhança de `WorkspaceExplorer` num grafo AST (6 arestas, com arquivo e linha) | **~129** |
| Vizinhança de `execute_run` (13 nós, 12 arestas) | **~461** |

Um arquivo do próprio projeto não cabe na janela do modelo que o projeto existe para
servir. Hoje a saída é delegar a um ajudante, que queima a janela dele para devolver
dez linhas.

## Por que um grafo, e por que agora

Rodando o [graphify](https://pypi.org/project/graphifyy/) sobre este repositório
saíram **5.635 nós e 13.818 arestas** de 241 arquivos, com **99% marcados
`_origin: "ast"` e `confidence: EXTRACTED`**. Ou seja: a extração é sintática, e
**construir o grafo não custa token de modelo nenhum**.

É isso que separa a ideia do GraphRAG clássico, caro porque extrai entidades com LLM.
Aqui vale a mesma régua dos guard-rails do laço: determinístico, testável sem rede,
mesma entrada e mesma saída.

## O que o grafo NÃO resolve

Perguntado em linguagem natural — *"onde o roteamento de modelos entre local e
provedor externo é decidido"* — o grafo devolveu `LocalServer.tsx`, `nav.ts` e
schemas do Tauri. Nada de `crates/providers`, que é a resposta certa.

Grafo casa **nome**; a busca semântica casa **significado**. Um não substitui o outro,
e é por isso que o desenho é de três peças:

> o **RAG** acha o ponto de entrada · o **grafo** expande a vizinhança exata ·
> o **`fs_read`** (que já aceita `offset_lines`/`max_lines`) lê só o trecho.

O modelo passa a ler pouco porque **sabe onde olhar**, não porque alguém apertou um
limite.

## Decisões travadas

- **Rust e TypeScript/TSX** na primeira versão. Cobrem 100% deste repositório, o que
  permite medir o ganho no próprio projeto antes de expandir.
- **Teto de +15 MB** no instalador (hoje: 8 MB no Windows).
- **Nada de processo externo.** O `sqlite-vec` já prova que dá para embarcar C no
  binário; o tree-sitter foi feito para isso. Chroma foi avaliado e descartado: o
  crate Rust oficial é cliente HTTP e exige servidor, o que repetiria o custo do
  9router (Node portátil, porta, centenas de MB) para um índice que hoje é uma tabela
  no `.db`.

## Medição 1 — peso das gramáticas (2026-08-19)

Portão de decisão do plano: as gramáticas cabem no orçamento do instalador?

Dois binários com as mesmas flags do projeto (`lto = true`, `codegen-units = 1`,
`strip = true`), um vazio e outro usando de verdade as três gramáticas:

| | tamanho |
|---|---:|
| binário base | 0,29 MB |
| binário com `tree-sitter` + `tree-sitter-rust` + `tree-sitter-typescript` (TS e TSX) | 4,19 MB |
| **delta no binário** | **3,89 MB** |
| **delta comprimido** (aproxima o instalador NSIS) | **0,41 MB** |

**Passou com folga de 11 MB.** A surpresa útil: as tabelas de parser são grandes e
altamente comprimíveis — 3,89 MB de binário viram cerca de 0,4 MB no instalador.
Nenhuma linguagem precisa ser cortada por peso.

### Como reproduzir

```bash
cargo new --bin base && cargo new --bin com
# em ambos os Cargo.toml:  [profile.release] lto = true, codegen-units = 1, strip = true
# em com/Cargo.toml:       tree-sitter, tree-sitter-rust, tree-sitter-typescript
# em com/src/main.rs:      parsear uma linha com cada gramática (senão o linker descarta)
cargo build --release   # nos dois, e comparar os binários
```

O programa de teste **precisa usar** as três gramáticas: com o `lto` ligado, um
`extern crate` não referenciado é descartado e o número mente.

## Próximas medições

- **Medição 2 — o ganho.** Ferramenta provisória lendo o `graph.json` que já existe,
  sem tree-sitter e sem tabelas, e o mesmo caminho do Code Mode:
  `cargo test -p lr_agent --test live_model -- --ignored --nocapture`, mesma tarefa
  com e sem grafo, contando passos, chamadas, tokens e tempo.
  **Critério de corte, definido antes de ver o resultado: ganho abaixo de 20% em
  passos ou tokens encerra o trabalho** — e esta página passa a registrar por quê.
- **Medição 3 — o ganho com a implementação real**, depois das fases abaixo.

## Fases

| fase | entrega |
|---|---|
| 0 | Medições 1 (feita) e 2 — portão de decisão |
| 1 | `lr_graph`: extração por tree-sitter, reusando a varredura de `crates/rag/src/walker.rs` (ignore rules, `MAX_FILE_BYTES`, detecção de binário já testadas) |
| 2 | `graph_nodes` / `graph_edges` no mesmo `.db`, incremental por hash da árvore sintática, como o `rag_catalog` já faz |
| 3 | Três ferramentas: `graph_neighbors`, `graph_path`, `graph_impact` — categoria `read`, família *Índice do projeto* em `menu.rs` |
| 4 | `workspace_search` passa a devolver trecho **+ vizinhança do símbolo**: uma chamada em vez de duas |
| 5 | Painel *Índice do projeto* e comandos Tauri, espelhando `commands_rag.rs` |
| 6 | Medição 3 e, se o resultado justificar, página no site |

### Regras que atravessam todas as fases

- **Confiança explícita em cada aresta** (`EXTRACTED` / `AMBIGUOUS`). Sem
  type-checking, dois símbolos homônimos geram um `calls` falso; marcar é o que
  impede o agente de afirmar cadeia de chamada que não existe.
- **Degrade gracioso.** Projeto sem grafo, ou linguagem sem gramática: as ferramentas
  não entram no cardápio e o resto funciona igual — o mesmo que o RAG já faz sem
  modelo de embedding.
- **Toda resposta traz `arquivo:linha`.** É o que permite ao `fs_read` ler o trecho e
  ao usuário conferir. Trecho sem endereço vira alegação que ninguém verifica.
