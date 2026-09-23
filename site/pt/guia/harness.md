# O agente de código

Chat é chat: o modelo responde. Quando você quer trabalho feito — arquivos
lidos e editados, comandos rodados, um projeto levado de um estado a outro —
isso é o **AgenticOw**, e ele tem item próprio na barra lateral, logo abaixo
do Chat.

O AgenticOw é o fork do próprio OpenWeights do
[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (licença
MIT), mantido em [pedro-canedo/agenticow](https://github.com/pedro-canedo/agenticow)
do jeito que o Cursor mantém o VS Code: o núcleo do projeto original, a nossa
camada por cima — marca, português, os seus modelos, privacidade — e
sincronização periódica com o original. Não é um atalho para outro lugar: o
app instala, supervisiona e mostra ele dentro da própria janela.

## A primeira abertura

O AgenticOw chega como um **runtime pré-compilado**, montado pela nossa CI para
cada sistema (Windows, Linux e macOS) — não há `npm install` na sua máquina. A
primeira abertura baixa o pacote do seu sistema numa pasta do app, uma vez só,
e o confere contra o sha256 e o tamanho **gravados no binário do app** antes de
rodar qualquer coisa. Pacote que não bate é recusado. Ele roda no Node portátil
do app, nunca no do seu sistema.

Depois disso, abre em segundos. Você pode sair da tela enquanto ele se prepara:
o progresso está lá quando você voltar.

::: tip Tudo dentro do aplicativo
O runtime mora numa pasta do app, escuta só em loopback e é encerrado junto com
o OpenWeights. Remover é um botão na mesma tela — com a escolha de manter ou
apagar também as sessões e configurações criadas lá dentro.
:::

## Dentro da janela principal

A interface do AgenticOw aparece **dentro da janela principal**, sobre a área
de conteúdo, não numa janela separada. Ir para o Chat ou para os Modelos e
voltar não a recarrega: a sessão aberta continua rodando, inclusive no meio de
uma tarefa. Os diálogos do app aparecem por cima dela.

Ele fala o idioma do app — português ou inglês —, e trocar o idioma nas
Configurações troca o do AgenticOw também, sem reiniciar.

## Atualização

A versão do AgenticOw vem fixada em cada versão do OpenWeights, e ele se
atualiza **junto com o app**: quando o OpenWeights atualiza, a próxima abertura
baixa o runtime novo, confere do mesmo jeito e sobe. A versão anterior só é
apagada depois que a nova subiu bem, então um download ruim nunca deixa você
sem agente.

É por isso que o fork existe. O harness que o app usava antes vinha do npm numa
versão fixada no app, e as versões mais novas mudaram o jeito de subir e de
autenticar de formas que o app não conseguia acompanhar — então ele ficava para
trás. Compilar o runtime nós mesmos faz a atualização dele ser parte da
atualização do app.

## O que ele já sabe sobre os seus modelos

Você não configura provedor, não cola endereço nem copia chave. O app entrega
ao AgenticOw tudo que conhece:

- **Servidor Local** — todos os modelos que o seu roteador llama.cpp atende,
  cada um com a janela de contexto real.
- **OpenRouter** — os seus favoritos, quando o provedor está ligado e tem
  chave.
- **9router** — o catálogo dele, quando está instalado e no ar.

E mantém essa lista em dia **enquanto ele roda**: subir ou parar o motor,
baixar ou apagar um modelo, ligar ou desligar o Jev, trocar a chave ou os
favoritos do OpenRouter, subir ou parar o 9router — cada um atualiza o seletor
de modelos sem reiniciar nada.

As chaves de API vão só para a memória do processo do AgenticOw. Nunca são
gravadas em arquivo, então uma pasta que alguém leia depois não tem segredo
nenhum.

## Privacidade

A telemetria do projeto original está desligada: a telemetria de sessão, o
feedback de mensagens e de comandos e o inventário de plugins anexado às
requisições à API da DeepSeek vêm desativados no nosso build, e um teste no
fork confere, a cada sincronização com o original, que todos continuam
desligados. Nada vai para a DeepSeek a menos que você mesmo escolha a DeepSeek
como provedor.

## Vindo do DeepSeek Harness

Se você usou o DeepSeek Harness numa versão anterior do app, a primeira
abertura **copia** as sessões e configurações dele para o AgenticOw. A pasta
original fica intacta, então voltar para uma versão anterior ainda encontra
tudo onde estava.

## Esforço de raciocínio

Modelos que pensam antes de responder ganham um seletor de esforço ao lado do
nome, e **os níveis vêm do chat template do próprio modelo**, não de uma lista
que inventamos. O template declara quais valores aceita e recusa qualquer
outro; o app lê essa linha e oferece exatamente aqueles.

Isso pesa mais do que parece. Diante de um pedido aberto, um modelo de
raciocínio no nível mais alto consegue gastar o **orçamento inteiro de saída
pensando** e parar antes de escrever o primeiro arquivo. Medido num modelo de
raciocínio com esse template, na mesma pergunta: o nível baixo produz cerca de
600 caracteres de raciocínio, o mais alto quase 6 000 — dez vezes mais. Baixar o esforço é, com
frequência, a diferença entre uma resposta e um rascunho cortado.

**Desligado** desliga o raciocínio de verdade, não o reduz.

Com o [Jev](/pt/integracoes/provedores#jev-camada-de-decisao) ligado para
agentes de código, o seletor continua sendo o teto, mas cada requisição que o
AgenticOw manda passa por um proxy local que decide, por mensagem, se o modelo
deve pensar e em qual dos níveis do template. Continuações da mesma tarefa
(resultados de ferramenta voltando) reaproveitam a decisão em vez de pagá-la de
novo, e o que o proxy não consegue decidir atravessa exatamente como o
AgenticOw mandou.

O esforço padrão é da **rota inteira**, não de cada modelo — o AgenticOw só
aceita um valor para todos. Como a rota local costuma misturar modelos que
raciocinam com modelos que não raciocinam (um Coder, por exemplo), quase nunca
existe um nível que sirva a todos: aí a rota vai sem padrão e o raciocínio
começa desligado, à espera do seletor. É de propósito — um padrão que algum
modelo da rota não aceita faz esse modelo recusar toda mensagem antes de
enviá-la.

## Teto de saída

Cada modelo local também declara quanto pode escrever numa resposta — metade
da sua própria janela de contexto. Sem isso, o AgenticOw assume 32k fixos para
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
