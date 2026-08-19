# Autorização e permissões

Nada que o agente faz escapa desta página. Toda chamada de ferramenta passa pela
mesma decisão, e a decisão é sua para configurar.

## Os quatro níveis

Definidos no compositor, por conversa:

| Nível | O que significa |
|---|---|
| **Sem ferramentas** | O modelo só conversa. Não roda nada. |
| **Sempre perguntar** | Toda ferramenta pede sua confirmação. |
| **Perguntar para alterações** | Leitura roda sozinha; escrita e comandos perguntam antes. |
| **Automático (YOLO)** | Roda sozinho dentro da pasta do projeto. Um checkpoint é criado antes da primeira alteração. |

O modo automático exige uma pasta de projeto escolhida, pede uma confirmação
explícita na primeira vez, e continua com escopo: **ações fora da pasta, acesso
à rede e comandos que não conseguimos analisar continuam perguntando.**

## Como a decisão é tomada

A política roda nesta ordem de precedência, e a primeira regra que casar vence:

1. Um **`nunca`** definido por você sempre vence.
2. **Fora da pasta do projeto** — sempre pergunta.
3. **Comando que não conseguimos analisar por inteiro** — sempre pergunta. A
   confirmação diz isso, em vez de fingir que entendeu.
4. **Acesso à rede** — sempre pergunta, a não ser que você tenha marcado
   *sempre permitir* naquela ferramenta.
5. Só então entram o modo da execução e os atalhos de leitura.

O objetivo é proteger você de um erro do *modelo* — apagar a pasta errada, rodar
um script que ninguém leu — mantendo o fluxo agradável quando a ação é
claramente inofensiva.

## A confirmação

Quando uma chamada precisa de você, o compositor dá lugar à barra de aprovação:
a ferramenta, os argumentos (expansíveis), a pasta onde um comando rodaria, e
avisos quando o alvo está fora do projeto ou o comando não pôde ser analisado.

Você pode **Permitir**, **Sempre permitir** (neste projeto ou em todos),
**Negar**, ou **Negar e explicar** — a última entrega o seu motivo ao modelo,
para ele tentar outra coisa em vez de repetir a mesma.

Teclado: <kbd>Enter</kbd> permite, <kbd>Esc</kbd> nega.

## Permissões por ferramenta

**Configurações → Ferramentas → Permissões** define uma política permanente por
ferramenta: *Sempre permitir*, *Perguntar* ou *Nunca*. É isso que sobrevive
entre conversas, e é onde se define o `nunca` — a regra que nada sobrepõe.

## Famílias de ferramentas

A mesma tela liga e desliga **famílias** inteiras: Arquivos, Terminal, Código,
Git, Dados, Web, Memória, Índice do projeto, Planejamento, Conectores,
Computador. Desligar uma família tira aquelas ferramentas do alcance do agente.

Pelo menos uma família fica ligada. Para impedir o agente de usar ferramentas, o
controle honesto é o nível de autorização — *Sem ferramentas* — e não um
cardápio vazio.

::: info A janela ainda dá a última palavra
Mesmo com todas as famílias ligadas, o agente só recebe as ferramentas que cabem
na janela do modelo. O resto continua alcançável — ele pede pelo nome quando
precisa. Veja [Ferramentas](/pt/agente/ferramentas).
:::
