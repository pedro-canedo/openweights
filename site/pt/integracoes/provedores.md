# Fontes externas de modelo

A tela **Fontes de modelo** responde a uma pergunta: onde suas conversas são
respondidas? A sua própria máquina é o padrão e não precisa de nada aqui. O
resto da tela é para quando você quer outra coisa.

No topo, um cartão por fonte diz quais estão prontas e, para as que não estão,
por quê — e clicar no cartão leva direto à aba onde aquilo se resolve.

## OpenRouter

Centenas de modelos atrás de uma chave só.

O **catálogo é público** — você pode navegar por ele, com preço por milhão de
tokens, tamanho de contexto e se o modelo suporta ferramentas, antes de decidir
qualquer coisa. A chave só é necessária para conversar de verdade. Filtre só os
gratuitos ou só os que você já fixou, busque por nome ou id, e fixe os que usa:
**só os fixados aparecem no seletor de modelos do chat**, e o card diz quantos
são e quantos modelos o filtro atual encontrou.

Com a chave definida, a tela mostra quanto você gastou e seu limite de crédito.

## Jev — camada de decisão

Abaixo do cartão do OpenRouter fica o **Jev**, um modelo de decisão da TypeSafe
que o app alcança pela mesma chave do OpenRouter. Ele não gera texto: recebe a
mensagem atual mais um trecho curto da conversa e responde, em bem menos de um
segundo, **quanto raciocínio a mensagem pede** — nenhum, médio ou alto. O app
então liga ou desliga o raciocínio do modelo local por mensagem, em vez de usar
o esforço fixo da conversa. Um "oi" deixa de pagar trinta segundos de thinking;
um problema de lógica continua com o orçamento inteiro.

Vem desligado e só fica disponível depois que a chave do OpenRouter está
configurada. Ligado, essa mensagem e o trecho saem da sua máquina — o cartão diz
isso onde você liga. Todo o resto continua local, e o custo é fração de centavo
por mil mensagens (só tokens de entrada; a resposta é grátis).

Dois interruptores dizem onde vale. **Chat do app** decide antes de cada envio
e mostra a escolha nos detalhes da execução da resposta. **Agentes de código**
sobe um proxy local pequeno em `127.0.0.1:11712` na frente do motor; o DeepSeek
Harness e os outros agentes que o app abre passam a apontar para ele, e cada
`chat/completions` que mandam recebe a mesma decisão aplicada ao corpo. Qualquer
outra requisição atravessa intocada, streaming inclusive. O proxy só escuta no
endereço de loopback: o que chega ao motor pela rede não passa por ele.

**Só agir acima de** é o piso de confiança. Abaixo dele — ou sempre que o Jev
está fora do ar, a chave falta ou a resposta se contradiz — nada muda e o
esforço que você configurou prevalece. **Testar decisão** manda uma pergunta de
amostra para você ver chave e endpoint funcionando antes de uma conversa
depender deles.

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
