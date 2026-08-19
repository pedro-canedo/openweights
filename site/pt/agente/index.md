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

Há ainda dois relógios em volta do modelo: um generoso para o primeiro token —
processar um prompt de 8k na CPU leva minutos e isso não é travamento — e um
mais apertado entre pedaços, porque silêncio depois que a geração começou é
servidor pendurado.

## Verificação

Quando a execução termina, uma checagem barata roda sobre o que ela afirma ter
feito. Sem modelo envolvido: os arquivos que ela escreveu existem? algum comando
terminou com erro?

O objetivo não é auditar o trabalho — é pegar a mentira fácil, a que modelo
pequeno mais conta: anunciar um arquivo que a ferramenta nunca criou. A trilha
mostra **Resultado verificado** ou **A verificação achou problemas**.

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

## A seguir

- [Autorização e permissões](/pt/agente/autorizacao) — quem decide o que roda.
- [Ferramentas](/pt/agente/ferramentas) — o catálogo, por família.
- [Modos de trabalho e planos](/pt/agente/planos) — como tarefas grandes são
  cortadas.
- [Code Mode](/pt/agente/code-mode) — um passo, muitas chamadas.
