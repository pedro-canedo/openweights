# Fontes externas de modelo

A tela **Fontes de modelo** responde a uma pergunta: onde suas conversas são
respondidas? A sua própria máquina é o padrão e não precisa de nada aqui. O
resto da tela é para quando você quer outra coisa.

A lista de status no topo sempre diz quais fontes estão prontas e, para as que
não estão, por quê.

## OpenRouter

Centenas de modelos atrás de uma chave só.

O **catálogo é público** — você pode navegar por ele, com preço por milhão de
tokens, tamanho de contexto e se o modelo suporta ferramentas, antes de decidir
qualquer coisa. A chave só é necessária para conversar de verdade. Filtre só os
gratuitos, busque por nome ou id, e fixe os que você usa para eles ficarem no
topo do seletor de modelos.

Com a chave definida, a tela mostra quanto você gastou e seu limite de crédito.

## 9router

Um roteador local com painel próprio: ele põe contas de vários provedores atrás
de um endereço só.

O OpenWeights instala numa **pasta isolada** — Node portátil incluído, sem tocar
no seu sistema —, executa e remove quando você pedir. Os modelos e combos que ele
publica aparecem no seletor de modelos do chat, com a chave de API obtida dele
mesmo. O processo morre junto com o app.

::: warning Instalar demora
Ele baixa o Node.js portátil e o pacote do 9router — algumas centenas de MB em
disco. Com antivírus ativo no Windows pode levar de 2 a 10 minutos.
:::

O painel **abre em janela própria**. Isso não é escolha estética: embutido na
tela do app, o login dele nunca completa. A senha do primeiro boot aparece no
app; depois disso, vale a senha que você definiu dentro do 9router.

Desinstalar pergunta se você quer manter os dados — apagar remove as contas e
provedores configurados dentro do 9router, e isso não tem desfazer.

## Gateway — ponto de entrada único

Um endereço só que encaminha para o motor local e para o 9router **por
prefixo**:

| Prefixo | Vai para |
|---|---|
| `/local` | O motor llama.cpp local |
| `/9router` | O 9router, quando estiver rodando |

Útil para apontar outra ferramenta — um editor, um script — para o OpenWeights
sem decorar duas portas. Ele roda um Traefik local, com versão fixada.

**O que ele não faz**, para ninguém esperar: não cria túnel para a internet
(Traefik é proxy reverso, não túnel); não junta os catálogos num `/v1/models` só,
porque isso seria código nosso e não roteamento; e não acrescenta autenticação
nenhuma.

É **opcional e desligado por padrão** — nada no chat depende dele.

::: warning Expor para a rede local
Com *aceitar conexões da rede local* ligado, qualquer aparelho da sua rede
alcança seus modelos sem senha. Só ligue em rede confiável.
:::
