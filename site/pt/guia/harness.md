# O agente de código

Chat é chat: o modelo responde. Quando você quer trabalho feito — arquivos
lidos e editados, comandos rodados, um projeto levado de um estado a outro —
isso é o **DeepSeek Harness**, e ele tem item próprio na barra lateral, logo
abaixo do Chat.

Não é um atalho para outro lugar. O OpenWeights instala, supervisiona,
configura contra os seus modelos e roda ele embutido no aplicativo.

## A primeira abertura

Abrir pela primeira vez baixa um Node portátil e cerca de 190 pacotes npm numa
pasta do próprio app — nunca instalação global, nunca o Node do seu sistema.
Leva **de dez a trinta minutos**, a maior parte resolvendo a árvore de
dependências, e a tela mostra o log ao vivo o tempo todo. Você pode sair da
tela: a instalação continua e o progresso está lá quando você voltar.

Depois disso, é um clique.

::: tip Tudo dentro do aplicativo
O harness roda de uma pasta do app, escuta só em loopback e é encerrado junto
com o OpenWeights. Remover é um botão na mesma tela — com a escolha de apagar
também as sessões e credenciais criadas lá dentro.
:::

## O que ele já sabe sobre os seus modelos

Você não configura provedor, não cola endereço nem copia chave. A cada vez que
o harness sobe, o app escreve a configuração dele com tudo que conhece:

- **Servidor Local** — todos os modelos que o seu roteador llama.cpp atende,
  cada um com a janela de contexto real.
- **OpenRouter** — os seus favoritos, quando o provedor está ligado e tem
  chave.
- **9Router** — o catálogo dele, quando está instalado e no ar.

Chave de API nunca entra nesse arquivo. Ele nomeia variáveis de ambiente, e os
valores vão só para o processo do harness — então um arquivo que alguém leia
depois não tem segredo nenhum.

## Esforço de raciocínio

Modelos que pensam antes de responder ganham um seletor de esforço ao lado do
nome, e **os níveis vêm do chat template do próprio modelo**, não de uma lista
que inventamos. O template declara quais valores aceita e recusa qualquer
outro; o app lê essa linha e oferece exatamente aqueles.

Isso pesa mais do que parece. Diante de um pedido aberto, um modelo de
raciocínio no nível mais alto consegue gastar o **orçamento inteiro de saída
pensando** e parar antes de escrever o primeiro arquivo. Num Qwen3.8 27B,
medido na mesma pergunta: o nível baixo produz cerca de 600 caracteres de
raciocínio, o mais alto quase 6 000 — dez vezes mais. Baixar o esforço é, com
frequência, a diferença entre uma resposta e um rascunho cortado.

**Desligado** desliga o raciocínio de verdade, não o reduz.

O esforço padrão é da **rota inteira**, não de cada modelo — o harness só
aceita um valor para todos. Como a rota local costuma misturar modelos que
raciocinam com modelos que não raciocinam (um Coder, por exemplo), quase nunca
existe um nível que sirva a todos: aí a rota vai sem padrão e o raciocínio
começa desligado, à espera do seletor. É de propósito — um padrão que algum
modelo da rota não aceita faz esse modelo recusar toda mensagem antes de
enviá-la.

## Teto de saída

Cada modelo local também declara quanto pode escrever numa resposta — metade
da sua própria janela de contexto. Sem isso, o harness assume 32k fixos para
todo mundo, número que um modelo pequeno não tem como honrar e ao qual um
grande não precisa ficar preso.

Se uma resposta parar com *Output token limit reached*, é esse teto, e o que
já foi escrito fica: mandar `continue` retoma de onde parou.

## Velocidade, medida em vez de prometida

A decodificação especulativa — o modelo adivinhando vários tokens à frente e
conferindo todos numa passada — fica na tela **Servidor Local**, não aqui,
porque é propriedade do servidor. Vale saber que o app a mede na sua máquina e
aplica o que vence, e que ele confere que a resposta não mudou antes de aplicar
qualquer coisa. Veja
[especulação medida](/pt/integracoes/api-local#especulacao-medida).

## Os outros agentes

Claude Code, Aider e OpenCode não são gerenciados pelo app, mas ganham o
comando pronto apontado para a sua API local, com a chave mascarada na prévia.
Eles moram em **Servidor Local → Abrir em um harness**. Veja
[o servidor de API local](/pt/integracoes/api-local#abrir-em-um-harness).
