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

## Antes de tudo: isto precisa mesmo de plano?

Nem tudo que se diz ao agente é um projeto. Antes de existir plano, o modelo
recebe uma pergunta curta: este pedido exige um **plano de trabalho** — uma
sequência de entregas executadas por algo que mexe em arquivos e roda comandos —
ou se resolve numa resposta?

Uma saudação, um agradecimento, uma pergunta sobre o que acabou de ser feito, um
pedido de opinião: esses são respondidos. Construir, alterar, investigar ou
corrigir algo no computador ganha plano.

A triagem é de propósito barata — prompt curto e booleano forçado — e economiza
a decomposição inteira quando a resposta é "conversa". **Na dúvida, ela
planeja.** Um plano desnecessário é chato; recusar plano a um pedido real
devolveria você ao laço solto, sem gate por entrega — que é justamente o que o
modo agente existe para substituir.

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

O planejador nunca recebe o spec inteiro: ele recebe um **recorte** — as fases
em uma linha cada, quando o pedido já vem escrito assim, ou os primeiros 2.500
caracteres. Mandar o texto completo para *dividir* gasta a janela justamente no
passo que mais precisa dela.

Mesmo assim o resultado é validado com desconfiança — e o que acontece quando
ele não sobrevive mudou. Antes, qualquer falha virava um plano de uma entrega
só, e um spec de seis fases era executado como se fosse um pedido curto. Agora
a saída é o **próprio pedido**: um texto já escrito em `Fase 1`, `Phase 2`,
`Etapa 3` vira uma entrega por fase, e um pedido longo sem cabeçalho nenhum é
fatiado em pedaços. O plano de uma entrega só ficou para o que ele sempre
deveria ter sido — pedido curto. A decomposição nunca derruba a execução.

A mesma régua vale para o plano que *passou* na validação com uma entrega
gigante. Um modelo pequeno diante de um spec de seis fases responde "implemente
o app", e isso não é um plano: é o pedido de volta. Quando o pedido tem fases
escritas e o plano tem uma entrega só, valem as fases.

Caminhos que nunca poderiam ser encontrados sob a pasta do projeto são
descartados na leitura do plano — caminho absoluto, `..`, unidade do Windows.
Manter um criaria uma pendência impossível de satisfazer, cobrada para sempre.

Tetos: **12 entregas** por plano, **8 passos** por entrega, **2 tentativas**
antes de considerar uma entrega travada. O teto global de passos continua
valendo por cima, mas ele não manda mais no *número de entregas*: com 24 passos
e 8 por entrega, nenhum plano passava de três, e um spec de seis fases perdia
metade antes de começar. Quem dimensiona as entregas é a janela; o teto de
passos dimensiona o trabalho.

## Como uma entrega prova que terminou

"Concluído" deixou de ser palavra do modelo. Cada etapa passa por três camadas,
nesta ordem — e as duas primeiras são mecânicas, sem modelo nenhum julgando:

1. **O comando de aceitação.** Ele roda *antes* da etapa (onde se espera que
   falhe — é o teste vermelho) e *depois* dela. Sair com código diferente de
   zero no fim reprova a entrega. É a mesma política de autorização de qualquer
   comando: se você desligou a família de terminal, ele não roda. Comando que
   passaria em qualquer máquina — `node -v`, `ls`, `pwd`, `echo` — é recusado
   como prova na leitura do plano: era com ele que a conferência ficava verde
   com a pasta vazia.
2. **Os arquivos.** O que a etapa disse que ia escrever precisa estar no disco.
3. **O juiz do critério.** Só entra quando as duas primeiras não decidiram nada
   — etapa sem arquivo e sem comando conferível — e **nunca** reverte uma
   reprovação mecânica: é desempate sobre o vazio, não veto sobre o disco.

Antes das três, para a etapa que promete código, vem uma exigência mais crua:
**algum arquivo escrito nesta etapa**. Uma entrega que declara arquivos, ou cuja
instrução pede implementação, não fecha com prosa — pensar, listar pastas e
rodar `node -v` não é entrega, e agora reprova com essas palavras.

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

O handoff vive na janela, e a janela é apagada. Por isso a execução também
escreve **`.openweights/progress.md`** na pasta do projeto: o que já rodou, os
arquivos tocados, o próximo passo e os bloqueios. Ele é reescrito a cada passo e
sobrevive ao fim da execução — abra no editor, versione se quiser. Não confunda
com a [memória](/pt/agente/memoria): memória é fato curado que vale para sempre,
o progresso é o rascunho *desta* execução.

## Os trilhos de cada fase

Um modelo de 8B não erra por má vontade: ele erra porque ninguém disse, naquele
passo, o que conta como pronto. Dizer tudo o tempo todo também não funciona —
prompt de sistema grande empurra o pedido para fora da janela.

Então os trilhos entram **por fase**, e só a fase corrente:

| Fase | Trilhos |
|---|---|
| Planejar | dividir o spec em uma entrega por fase, instrução curta, comando de aceitação que prova algo |
| Executar | escrever arquivo de verdade neste passo, nomear o que foi criado, conferir com teste ou build |
| As duas | janela curta: a memória da execução é `.openweights/progress.md`, não o histórico |

Eles vêm embutidos no binário — o modelo pequeno não pode depender de você
lembrar de anexar um guia. A seção inteira tem teto de caracteres, porque trilho
comprido custa o mesmo que o pedido.

Para trocar um trilho neste projeto, escreva
`.openweights/skills/<nome>/SKILL.md` com o mesmo `name` do embutido —
`planning`, `implementation`, `verification` ou `context`. Arquivo com nome
desconhecido é ignorado de propósito: a janela é pequena demais para aceitar
qualquer texto que apareça na pasta. No modo Chat nenhum trilho entra.

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

Resumir o histórico perto do teto não basta quando **uma** mensagem é o problema.
Um pedido colado inteiro pode ocupar metade da janela sozinho, e aí não há miolo
para resumir: a compactação roda e não sobra espaço mesmo assim. Então a mensagem
que passa de um quinto da janela é cortada antes disso — o começo e o fim ficam,
o meio vira uma linha dizendo quantos caracteres saíram e onde está o recorte
vivo (`.openweights/progress.md`).
