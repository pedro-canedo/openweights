# Como uma execução funciona

Ligue o botão **Agente** no compositor e uma mensagem deixa de ser pergunta:
vira uma *execução*. O modelo recebe um cardápio de ferramentas, trabalha passo
a passo, e toda ação passa pelo nível de autorização que você escolheu.

Esta seção documenta o harness — a parte do OpenWeights que transforma um modelo
local em algo que entrega trabalho.

## O problema em torno do qual o harness foi construído

O público deste app roda modelo local. Um modelo de 8B com janela de 8k não é um
GPT-4 menor: ele perde o fio, se repete, escolhe a ferramenta errada quando vê
trinta delas e afirma com alegria que escreveu um arquivo que nunca escreveu.

Toda decisão de projeto abaixo existe por causa disso. Nenhuma delas é um
embrulho em cima do bom comportamento de um modelo grande de nuvem.

## Um passo

Um passo é uma ida ao modelo. O modelo responde com texto, com uma ou mais
chamadas de ferramenta, ou com os dois. Para cada chamada, o harness:

1. **pergunta à política** se ela roda, pergunta a você, ou é recusada
   ([autorização](/pt/agente/autorizacao));
2. **tira uma foto do projeto** antes da primeira alteração da execução
   ([checkpoints](/pt/agente/checkpoints));
3. **roda a ferramenta** e grava chamada, argumentos, resultado e duração na
   trilha;
4. **devolve o resultado** ao modelo como entrada do passo seguinte.

A execução termina quando o modelo diz que acabou, quando você para, ou quando
um dos guard-rails abaixo dispara.

## Os guard-rails

Todos determinísticos — nenhum segundo modelo julgando o primeiro, nada que
precise de rede. Em ordem de importância:

| Guard-rail | O que faz |
|---|---|
| **Teto de passos** | Limite duro. A execução sempre termina. Bater no teto avisa e oferece continuar. |
| **Erros seguidos** | Três falhas em sequência e a execução para e devolve a decisão para você, em vez de insistir. |
| **Repetição** | A mesma chamada três vezes é laço, não progresso. |
| **Releitura** | Entregar ao modelo um arquivo que ele já tem só gasta contexto. |
| **Orçamento de contexto** | Em torno de 80% da janela o histórico é resumido para a execução continuar. A trilha mostra *"Contexto resumido para continuar."* |
| **Conferência de entrega** | *(modo Agente)* No fim, os arquivos que o plano prometeu são procurados no disco. Enquanto faltar algum e houver orçamento de passos, a execução é avisada de quais são e continua, em vez de fechar como concluída. |

Há ainda dois relógios em volta do modelo: um generoso para o primeiro token —
processar um prompt de 8k na CPU leva minutos e isso não é travamento — e um
mais apertado entre pedaços, porque silêncio depois que a geração começou é
servidor pendurado.

## Verificação e conferência de entrega

Quando a execução termina, uma checagem barata roda sobre o que ela afirma ter
feito. Sem modelo envolvido: os arquivos que ela escreveu existem? algum comando
terminou com erro?

O objetivo não é auditar o trabalho — é pegar a mentira fácil, a que modelo
pequeno mais conta: anunciar um arquivo que a ferramenta nunca criou. A trilha
mostra **Resultado verificado** ou **A verificação achou problemas**.

Essa checagem tinha um buraco, e era o pior possível: ela não devolve nada
quando nada foi escrito e nada foi executado — ou seja, a única execução que
precisava ser conferida era justamente a que escapava. Uma execução que pensava
por um minuto, escrevia um parágrafo e fechava como *Concluído* passava batido.
Os guard-rails acima também não a pegam: todos pressupõem chamada de
ferramenta, e quem nunca age não dispara nenhum.

As duas checagens respondem a perguntas diferentes, e vale não confundir: a
verificação pergunta *"o que você diz que fez está de pé?"* — e roda nos dois
modos, em qualquer desfecho que teve efeito colateral. A conferência de entrega
pergunta *"entregou tudo?"*, contra o que o **plano** prometeu.

Então o pedido agora é quebrado em entregas antes de o laço começar, **também no
modo Agente** (o modo Laço já fazia), e cada entrega declara os arquivos que vai
produzir. No fim, quem responde *"acabou?"* é o disco: ou os arquivos estão lá,
ou não estão. Enquanto faltar algum, e enquanto houver orçamento de passos, os
nomes que faltam voltam para a conversa e o trabalho continua de onde parou. Se
o orçamento acabar antes, o resultado diz o que ficou faltando, com nome de
arquivo.

A cobrança é **exclusiva do modo Agente**, e as duas exclusões são o ponto. No
modo Laço quem reenfileira a etapa reprovada já é o executor do plano —
empilhar as duas cobraria a mesma entrega em dobro. E o modo Planejamento é
justamente aquele cujo contrato é não mexer em nada antes de você aprovar: ele
termina com o plano escrito e nenhuma entrega feita, que é exatamente o estado
que dispara a cobrança. Sem a restrição, o modo que promete não executar passava
a executar.

Duas recusas deliberadas impedem isto de virar um moedor de tokens: entrega que
não declara arquivo nunca é cobrada — sem evidência, cobrar é chutar, e o preço
do chute é mandar refazer trabalho pronto — e a mesma entrega nunca é cobrada
duas vezes, porque girar em falso queima o orçamento que você paga.

Nada disso substitui a cutucada imediata para o modelo que anuncia trabalho que
não fez — essa é lexical, barata, e age no meio do laço. Ela só pega a
*promessa* ("vou criar os três arquivos"), nunca a afirmação falsa ("pronto,
criei os três arquivos"). A conferência de entrega é a camada factual embaixo
dela, e age no fim, contra o disco.

Quando o pedido foi dividido e há pasta de projeto aberta, o quadro do plano
aparece também no modo Agente — que é o que responde "em que etapa ele está" sem
obrigar ninguém a interpretar *Pensou por 60,6s*. Sem pasta de projeto não há
contra o que conferir, então a conferência de entrega não roda. A mecânica está
em [modos de trabalho e planos](/pt/agente/planos).

## Delegação

Quando o agente precisa *descobrir* algo ("onde o roteamento de modelos é
decidido"), ler seis arquivos enche a janela com conteúdo bruto e não sobra
espaço para pensar.

Então ele pode delegar: um ajudante começa do zero — prompt de sistema próprio,
cardápio próprio, histórico vazio —, investiga e devolve só um resumo. O pai
paga dez linhas em vez de vinte mil tokens.

O ajudante não é uma execução nova. Ele reusa o mesmo laço e o mesmo identificador
de execução, e é isso que faz cancelar e confirmar continuarem funcionando lá
dentro, e o que mantém a trilha em um lugar só. Um ajudante por vez, de
propósito.

## A trilha

Tudo acima é visível. O painel **Execução**, à direita do chat, mostra passos,
chamadas com argumentos e saída, blocos de raciocínio, checkpoints, notas, o
plano quando existe, e o contador final — *N passos · M ferramentas · Xs*.
Execuções antigas são reabertas pela tela de **Atividade**.

## O terminal da sessão

A trilha diz *o que* foi executado. O terminal diz o que está sendo impresso,
enquanto é impresso: um painel no chat onde cada comando que o agente executa
aparece ao vivo — a linha de comando, a pasta em que roda, a saída chegando na
hora e o resultado no fim.

Antes, a saída só aparecia depois que o comando terminava, espalhada pelos
cartões da trilha. Um teste de três minutos ficava mudo até o fim, e não havia
como distinguir compilação lenta de processo pendurado. Essa distinção é o
motivo inteiro de acompanhar a saída ao vivo.

Cada comando mostra a linha executada, a pasta em que rodou, a saída conforme
ela chega, o estado (*Executando*, *Concluído*, *Falhou*), a duração e um aviso
quando a saída foi cortada. Reabrir uma conversa antiga repõe as saídas, que vêm
do banco — então o painel serve também para consulta.

O painel abre sozinho na primeira vez que um comando começa a rodar em cada
tarefa; se você fechar, ele fica fechado até a tarefa seguinte. O botão fica no
canto superior direito do chat, ao lado do da trilha, com o ícone `>_`. Os três
painéis da direita — parâmetros, trilha, terminal — são exclusivos: abrir um
fecha os outros.

Duas coisas a esperar, ambas consequência de não usar terminal emulado: a saída
vem sem cores, e barra de progresso aparece como linhas repetidas em vez de
atualizar no lugar. Não é defeito de exibição — é exatamente o texto que o modelo
lê. E este painel mostra os comandos *do agente*; os logs do llama-server
continuam na tela **Servidor Local**.

## A seguir

- [Autorização e permissões](/pt/agente/autorizacao) — quem decide o que roda.
- [Ferramentas](/pt/agente/ferramentas) — o catálogo, por família.
- [Modos de trabalho e planos](/pt/agente/planos) — como tarefas grandes são
  cortadas.
- [Code Mode](/pt/agente/code-mode) — um passo, muitas chamadas.
