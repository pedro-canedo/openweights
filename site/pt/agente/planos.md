# Modos de trabalho e planos

Uma tarefa grande entregue de uma vez a um modelo local enche a janela, o modelo
perde o fio e passa a inventar. A resposta do harness é sempre a mesma:
**quebrar o objetivo em entregas pequenas e executar uma de cada vez, com
contexto novo.**

## Os três modos

| Modo | O que faz |
|---|---|
| **Chat** | Só responde, sem ferramentas. |
| **Planejar** | Investiga e propõe o plano. Não muda nada até você aprovar. |
| **Executar** | Divide o pedido e executa entrega por entrega, provando cada uma. |

O modo Planejar é o honesto para trabalho desconhecido: você vê o que ele
pretende fazer — e os arquivos que espera tocar — antes de qualquer escrita.

Havia um quarto modo, **Laço**, que fazia o que Executar faz hoje. A separação
não se sustentava: o modo Agente criava um plano e o ignorava — executava solto
e só conferia no fim —, então "com plano" e "sem plano" eram duas qualidades de
execução, e a pior era a padrão. Agora há um caminho só.

## Como o plano é montado

O plano é pedido ao modelo com **JSON Schema forçado**. O llama-server converte
o schema em gramática, e é isso que faz um modelo de 8B devolver algo parseável.

O schema pede três coisas de cada entrega: os **arquivos** que ela vai produzir,
um **comando de aceitação** que sai com código 0 quando ela está pronta, e o
critério de *pronto quando* em texto. Nenhum é enfeite — são as três camadas de
prova da seção seguinte. O `plan_create`, ferramenta por onde o modo Planejar
registra o plano, pede o mesmo: um plano aprovado ali já chega dizendo como cada
etapa será conferida.

O schema também pede **até quatro perguntas**, para as decisões que mudam o
plano e que só você pode tomar. Quando elas aparecem, nada executa: veja
[perguntas](#perguntas-antes-de-trabalhar).

Mesmo assim o resultado é validado com desconfiança: um plano que não sobrevive
à validação vira um plano de uma entrega só. A decomposição nunca derruba a
execução. Caminhos que nunca poderiam ser encontrados sob a pasta do projeto são
descartados na leitura do plano — caminho absoluto, `..`, unidade do Windows.
Manter um criaria uma pendência impossível de satisfazer, cobrada para sempre.

Tetos: **12 entregas** por plano, **8 passos** por entrega, **2 tentativas**
antes de considerar uma entrega travada. O teto global de passos da execução
continua valendo por cima.

## Como uma entrega prova que terminou

"Concluído" deixou de ser palavra do modelo. Cada etapa passa por três camadas,
nesta ordem — e as duas primeiras são mecânicas, sem modelo nenhum julgando:

1. **O comando de aceitação.** Ele roda *antes* da etapa (onde se espera que
   falhe — é o teste vermelho) e *depois* dela. Sair com código diferente de
   zero no fim reprova a entrega. É a mesma política de autorização de qualquer
   comando: se você desligou a família de terminal, ele não roda.
2. **Os arquivos.** O que a etapa disse que ia escrever precisa estar no disco.
3. **O juiz do critério.** Só entra quando as duas primeiras não decidiram nada
   — etapa sem arquivo e sem comando conferível — e **nunca** reverte uma
   reprovação mecânica: é desempate sobre o vazio, não veto sobre o disco.

Etapa reprovada volta para a fila com o motivo escrito, e a tentativa seguinte
o recebe. Na segunda reprovação ela trava, e a execução segue para o que não
depende dela. Quando o orçamento de passos acaba com trabalho pendente, o
resultado **nomeia** as entregas que ficaram — acabar o orçamento não é desculpa
para não dizer o que faltou.

## Perguntas antes de trabalhar

Se a divisão do pedido esbarrar numa decisão que muda o plano — qual stack, até
onde vai o escopo, onde salvar —, a execução **para antes da primeira etapa** e
pergunta. Até quatro perguntas, com opções clicáveis quando elas são poucas e
conhecidas.

A pausa é durável: a pergunta fica gravada com o plano e sobrevive a fechar o
app. Ela vale em todo modo, inclusive no automático e nas automações — que não
têm conversa onde responder, então aparecem na fila **Aguardando resposta** da
tela de Atividade, com aviso do sistema.

Responder retoma o plano de onde parou. Quando a pergunta veio antes de qualquer
trabalho, o plano é refeito com a resposta à vista — ela mudava o plano, que era
o motivo de perguntar.

## O que atravessa entre etapas

Não o histórico — um **handoff** de até três linhas.

Esse é o truque inteiro. Como só um resumo atravessa, a janela não cresce com o
tamanho do pedido; cresce com o tamanho de *uma* etapa. O quadro mostra
`Contexto novo para esta etapa` quando isso acontece.

## O quadro

Com um plano rodando, o chat mostra: objetivo, entregas com status (na fila, em
andamento, concluída, travada, falhou, pulada), do que cada uma depende, os
arquivos que espera tocar e a condição de *pronto quando*.

No modo Planejar o quadro tem **Aprovar plano** e **Refazer plano** — nada roda
antes de você aprovar. Uma entrega que trava é marcada como bloqueada com o
motivo, e a execução segue para o que não depende dela em vez de morrer.

Cada entrega mostra também o **tempo**: quanto durou quando já acabou, quanto
deve durar quando ainda não. A previsão sai da velocidade medida da sua máquina
com o modelo carregado — sem medição, não há previsão, porque um número
inventado é pior que nenhum.

## Orçamento de janela

O quadro também mostra a conta com a qual está trabalhando: *janela do modelo: N
tokens — entregas até M*. As entregas são dimensionadas contra a janela que você
carregou de verdade, não contra uma hipotética.
