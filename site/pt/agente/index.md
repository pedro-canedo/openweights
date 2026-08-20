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
| **Prova por entrega** | Nenhuma etapa fecha na palavra do modelo: o comando de aceitação precisa sair com código 0, os arquivos prometidos precisam existir, e o que não tem prova mecânica passa por um juiz do critério. Reprovada, a etapa volta para a fila com o motivo. |
| **Passo que só fala** | Anunciar trabalho sem executar nada conta como parado. Três desses ganham a cutucada; no quinto a execução para e devolve a decisão para você. |

Há ainda dois relógios em volta do modelo: um generoso para o primeiro token —
processar um prompt de 8k na CPU leva minutos e isso não é travamento — e um
mais apertado entre pedaços, porque silêncio depois que a geração começou é
servidor pendurado.

## Verificação e prova de entrega

Quando a execução termina, uma checagem barata roda sobre o que ela afirma ter
feito. Sem modelo envolvido: os arquivos que ela escreveu existem? algum comando
terminou com erro?

O objetivo não é auditar o trabalho — é pegar a mentira fácil, a que modelo
pequeno mais conta: anunciar um arquivo que a ferramenta nunca criou. A trilha
mostra **Resultado verificado** ou **A verificação achou problemas**.

Essa checagem sozinha tinha um buraco, e era o pior possível: ela não devolve
nada quando nada foi escrito e nada foi executado — ou seja, a única execução
que precisava ser conferida era justamente a que escapava. Uma execução que
pensava por um minuto, escrevia um parágrafo e fechava como *Concluído* passava
batido. Os guard-rails acima também não a pegavam: quase todos pressupõem
chamada de ferramenta, e quem nunca age não dispara nenhum.

A resposta foi mover a prova para **dentro de cada entrega**, em vez de deixá-la
para o fim. O pedido é quebrado em etapas antes de qualquer trabalho, e nenhuma
delas fecha sem passar por três camadas — comando de aceitação, arquivos em
disco e, só quando as duas primeiras não têm o que olhar, um juiz do critério
escrito. A regra inteira, com o que cada camada pode e não pode decidir, está em
[modos de trabalho e planos](/pt/agente/planos).

Duas consequências que valem dizer em voz alta. A primeira: passo que só fala
agora **conta como parado** — antes, um modelo que pensava três minutos e não
agia não disparava guard-rail nenhum, porque nenhuma ferramenta havia rodado. A
segunda: quando o orçamento de passos acaba com trabalho pendente, o resultado
nomeia as entregas que ficaram, em vez de dizer só *"parei no limite"*.

Nada disso substitui a cutucada imediata para o modelo que anuncia trabalho que
não fez — essa é lexical, barata, e age no meio do laço. Ela só pega a
*promessa* ("vou criar os três arquivos"), nunca a afirmação falsa ("pronto,
criei os três arquivos"). As três camadas são a parte factual embaixo dela, e
agem contra o disco.

O quadro do plano aparece em toda execução, e é o que responde "em que etapa ele
está" sem obrigar ninguém a interpretar *Pensou por 60,6s* — com o tempo que
cada entrega levou, e o previsto para as que faltam.

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
