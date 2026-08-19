# Modos de trabalho e planos

Uma tarefa grande entregue de uma vez a um modelo local enche a janela, o modelo
perde o fio e passa a inventar. A resposta do harness é sempre a mesma:
**quebrar o objetivo em entregas pequenas e executar uma de cada vez, com
contexto novo.**

## Os quatro modos

| Modo | O que faz |
|---|---|
| **Chat** | Só responde, sem ferramentas. |
| **Planejamento** | Investiga e propõe o plano. Não muda nada até você aprovar. |
| **Agente** | Executa a tarefa pedida, um passo por vez. |
| **Laço** | Executa o plano inteiro, verificando cada entrega antes de seguir. |

O modo Planejamento é o honesto para trabalho desconhecido: você vê o que ele
pretende fazer — e os arquivos que espera tocar — antes de qualquer escrita.

## Como o plano é montado

O plano é pedido ao modelo com **JSON Schema forçado**. O llama-server converte
o schema em gramática, e é isso que faz um modelo de 8B devolver algo parseável.

Mesmo assim o resultado é validado com desconfiança: um plano que não sobrevive
à validação vira um plano de uma entrega só. A decomposição nunca derruba a
execução.

Tetos: **12 entregas** por plano, **8 passos** por entrega, **2 tentativas**
antes de considerar uma entrega travada. O teto global de passos da execução
continua valendo por cima.

## O que atravessa entre etapas

Não o histórico — um **handoff** de até três linhas.

Esse é o truque inteiro. Como só um resumo atravessa, a janela não cresce com o
tamanho do pedido; cresce com o tamanho de *uma* etapa. O quadro mostra
`Contexto novo para esta etapa` quando isso acontece.

## O quadro

Com um plano rodando, o chat mostra: objetivo, entregas com status (na fila, em
andamento, concluída, travada, falhou, pulada), do que cada uma depende, os
arquivos que espera tocar e a condição de *pronto quando*.

No modo Planejamento o quadro tem **Aprovar plano** e **Refazer plano** — nada
roda antes de você aprovar. Uma entrega que trava é marcada como bloqueada com o
motivo, e a execução segue para o que não depende dela em vez de morrer.

## Orçamento de janela

O quadro também mostra a conta com a qual está trabalhando: *janela do modelo: N
tokens — entregas até M*. As entregas são dimensionadas contra a janela que você
carregou de verdade, não contra uma hipotética.
