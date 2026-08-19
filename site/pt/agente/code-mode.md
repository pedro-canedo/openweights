# Code Mode

Em vez de pedir uma ferramenta por passo, o agente escreve um **programa** que
usa todas de uma vez. O harness executa, e só o que o programa imprime volta para
a conversa.

Ligue no compositor, ao lado do botão do agente. Ele precisa de Node na máquina
— o app pode instalar um portátil.

## Um passo, muitas chamadas

No modo nativo cada ferramenta custa um passo: o modelo pede, o servidor
reprocessa a conversa inteira, o resultado empilha na janela. A maior parte do
tempo de relógio de uma execução é esse reprocessamento, não as ferramentas.

No Code Mode o modelo gasta **um** passo escrevendo o programa, e o harness
executa quantas chamadas o programa fizer — nenhuma delas passando pelo modelo.
O que volta é o que o script imprimiu.

```js
// As ferramentas são funções. Todas devolvem texto e todas precisam de `await`.
const arquivos = await fs_glob({ pattern: "logs/*.log" });
let total = 0;
for (const arquivo of arquivos) {
  const texto = await fs_read({ path: arquivo });
  total += (texto.match(/ERROR/g) ?? []).length;
}
say(`${total} erros em ${arquivos.length} arquivos`);
```

A descrição da ferramenta mostrada ao modelo não fala de "Code Mode" nem de
arquitetura: fala do que fazer. Modelo pequeno segue exemplo, não conceito.

## O que não muda

Toda chamada vinda do script atravessa o **mesmo caminho** de uma chamada
normal: política, confirmação, foto do projeto, trilha, contadores. O despacho é
um trait implementado pelo mesmo executor de ferramentas, de propósito — um
atalho aqui apagaria de uma vez as proteções que o harness levou seis fases para
ganhar.

O programa em si roda isolado: sem acesso a arquivo ou comando fora das
ferramentas.

## O que foi medido aqui

Medido nesta máquina, não citado do slide de terceiro. O caso: 12 arquivos de
log, contagem por nível, percentual de erro num `resumo.csv` ordenado, e um
`criticos.md` só com os arquivos acima de 25% de erro.

**2026-08-18 · qwen2.5-coder:14b · RTX 5060 Ti**

| Modo | Passos | Chamadas | Tempo | Checagens |
|---|---:|---:|---:|---:|
| nativo | 37 | 34 | 390,1 s | 4/9 |
| **programa** | **5** | 17 | **115,5 s** | **5/9** |

- **3,4× mais rápido** e **7,4× menos idas ao modelo** — cada ida reprocessa a
  conversa inteira, e é daí que vem a maior parte do tempo.
- O Code Mode foi o único dos dois que chegou a produzir o `resumo.csv` com as
  12 linhas.
- **Nenhum dos dois completou a tarefa inteira.** O percentual do `app-12` e o
  `criticos.md` continuam errados nos dois. Isso é o modelo de 14B, não o
  harness — o mesmo limite que o vídeo que motivou o trabalho relatou com um
  modelo mais barato.

### Como reproduzir

```bash
# um servidor compatível com OpenAI e o modelo carregado (aqui: Ollama)
OLLAMA_HOST=127.0.0.1:11435 OLLAMA_CONTEXT_LENGTH=32768 ollama serve &

cd src-tauri
OW_LIVE_URL=http://127.0.0.1:11435 OW_LIVE_MODEL=qwen2.5-coder:14b \
  cargo test -p lr_agent --test live_model -- --ignored --nocapture \
  code_mode_and_native_mode_run_the_same_case
```

O fixture tem contagem conhecida, então as checagens são objetivas — outro teste
protege a própria régua.

## As quatro falhas até esse número

As quatro primeiras medições falharam, e cada uma apontou um defeito nosso:

1. **`run_code` recusada por "nome desconhecido"** — o modelo escrevia a chamada
   certa em texto, mas a ferramenta não está no cardápio ativo (vive à parte,
   com as assinaturas do run na descrição).
2. **JSON quebrado** — o modelo embrulhava o programa num `{"code": "..."}`, e a
   string com aspas e quebras de linha nunca fecha. A cutucada passou a pedir um
   bloco ```` ```js ```` puro.
3. **A ferramenta devolvia frase, não dado** — `for (const arquivo of await
   fs_glob(...))` iterava `"12 arquivos casaram com…"` caractere por caractere.
   A saída ganhou um campo `data`, e o programa passou a receber array e texto
   cru.
4. **Guard-rails do modelo aplicados ao programa** — o detector de repetição
   escalava a execução quando o segundo programa refazia uma chamada do
   primeiro.

Nenhuma das quatro aparecia sem rodar contra um modelo de verdade.
