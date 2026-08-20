# Memória e índice do projeto

Duas coisas diferentes, frequentemente confundidas. A memória é o que o agente
*aprendeu*. O índice é o que o seu projeto *contém*.

## Memória

Toda conversa começa do zero. Sem memória, você explica pela quinta vez que aqui
é pnpm, que o teste é `cargo test -p lr_x`, que você quer resposta curta.

A solução preguiçosa seria jogar o histórico inteiro num banco vetorial e
recuperar trechos por similaridade. Isto é, deliberadamente, **o contrário
disso**.

Memória aqui é **pouca, curta, curada e inspecionável**:

- **poucos fatos, não conversas inteiras** — o contexto de um modelo local é
  caro, e o que entra no prompt entra em *toda* execução seguinte;
- **cada fato passa por curadoria antes de existir** — normalizado, limitado em
  tamanho, deduplicado, com escopo definido;
- **tudo é arquivo que você lê**: `.openweights/memory/*.md` no projeto, mais um
  escopo global. Abra a pasta pelo app, edite na mão, versione se quiser;
- **o trabalho pesado acontece em ocioso** — transformar execuções passadas em
  fatos duráveis roda entre execuções, não no meio de uma. **Arrumar memória
  agora** dispara isso.

O agente guarda fatos sozinho com `memory_save`, e você pode acrescentar ou
esquecer um na mão no painel de Memória. Cada fato é **global** ou **deste
projeto**.

Não confunda com `.openweights/progress.md`, o rascunho de *uma* execução — o
que já rodou, os arquivos tocados, o próximo passo. Ele é reescrito o tempo todo
e não passa por curadoria nenhuma; está descrito em
[modos de trabalho e planos](/pt/agente/planos).

## Índice do projeto

O `grep` só acha o que você soube escrever. "Onde a gente valida o token de
sessão?" não tem palavra-chave óbvia — quem responde é a busca semântica. Mas
embedding sozinho erra em nome próprio (`RagHandle`, `AGENT_MAX_STEPS`), onde o
casamento literal é imbatível.

Por isso a busca é **híbrida**: FTS5 (BM25) e vetor rodam em paralelo e o
resultado é fundido por RRF. A lista diz qual lado achou cada trecho — *texto*,
*significado*, ou os dois.

### Construindo

**Indexar projeto**, na coluna do explorador, varre e fatia os arquivos e depois
constrói os vetores. O progresso aparece por arquivo e por pedaço, e dá para
cancelar.

- **Os vetores saem do seu próprio llama-server**, por `/v1/embeddings` — sem
  download, sem serviço externo. Baixe um modelo de embedding
  (`nomic-embed-text`, `bge-m3`) para essa metade funcionar.
- **Sem modelo de embedding o índice ainda funciona**, só com texto. Pior, mas
  funcionando — e o app diz isso em vez de falhar.
- **É incremental.** Um catálogo com hash e mtime por arquivo evita reler o
  projeto inteiro a cada atualização.

Clicar num resultado abre o arquivo no editor do app, no trecho. O agente alcança
o mesmo índice pela ferramenta `workspace_search`.
