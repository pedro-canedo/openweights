# Code Mode — o que foi medido aqui

O Code Mode entrega as ferramentas ao modelo como uma biblioteca e pede um
**programa**; só o que o programa imprime volta para a conversa. A promessa é
gastar menos idas ao modelo e menos janela de contexto. Este documento guarda a
medição feita **nesta máquina**, para não repetir número de terceiro.

## Como reproduzir

```bash
# um servidor OpenAI-compatible com o modelo carregado (aqui: Ollama)
OLLAMA_HOST=127.0.0.1:11435 OLLAMA_CONTEXT_LENGTH=32768 ollama serve &

cd src-tauri
OW_LIVE_URL=http://127.0.0.1:11435 OW_LIVE_MODEL=qwen2.5-coder:14b \
  cargo test -p lr_agent --test live_model -- --ignored --nocapture \
  code_mode_and_native_mode_run_the_same_case
```

O caso é o mesmo do vídeo que motivou o trabalho: 12 arquivos de log, contagem
por nível, percentual de erro em `resumo.csv` ordenado, e um `criticos.md` só
com os arquivos acima de 25% de erro. O fixture tem contagem conhecida, então
as checagens são objetivas (`the_log_fixture_has_the_counts_the_checks_expect`
protege a régua).

## Medição — 2026-08-18, qwen2.5-coder:14b, RTX 5060 Ti

| modo | passos | chamadas de ferramenta | tempo | checagens |
|---|---:|---:|---:|---:|
| nativo | 37 | 34 | 390,1 s | 4/9 |
| **programa** | **5** | 17 | **115,5 s** | **5/9** |

- **3,4× mais rápido** e **7,4× menos idas ao modelo** — cada ida reprocessa a
  conversa inteira, e é daí que vem a maior parte do tempo.
- O Code Mode foi o único dos dois que chegou a produzir o `resumo.csv` com as
  12 linhas.
- Nenhum dos dois completou a tarefa inteira: o percentual do `app-12` e o
  `criticos.md` continuam errados. Isso é o modelo de 14B, não o harness — o
  mesmo limite que o vídeo relatou com o modelo mais barato.

## O caminho até esse número

As quatro primeiras medições falharam, e cada uma apontou um defeito nosso:

1. **`run_code` recusada por "nome desconhecido"** — o modelo escrevia a
   chamada certa em texto, mas a ferramenta não está no cardápio ativo (vive à
   parte, com as assinaturas do run na descrição).
2. **JSON quebrado** — o modelo embrulhava o programa num `{"code": "..."}`,
   e a string com aspas e quebras de linha nunca fecha. A cutucada do Code Mode
   passou a pedir um bloco ` ```js ` puro.
3. **A ferramenta devolvia frase, não dado** — `for (const arquivo of await
   fs_glob(...))` iterava `"12 arquivos casaram com…"` caractere por caractere.
   `ToolOutput` ganhou `data`, e o programa passou a receber array e texto cru.
4. **Guard-rails do modelo aplicados ao programa** — o detector de repetição
   escalava o run quando o segundo programa refazia uma chamada do primeiro.

Nenhum dos quatro aparecia sem rodar contra um modelo de verdade.
