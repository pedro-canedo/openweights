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

## Decisões — o reflexo na frente dos seus modelos

A aba **Decisões** guarda uma camada pequena e rápida que roda antes de cada
mensagem: um *decisor* olha a mensagem atual mais um trecho curto da conversa e
responde **quanto raciocínio a mensagem pede** — nenhum, médio ou alto. O app
então liga ou desliga o raciocínio do modelo local por mensagem, em vez de usar
o esforço fixo da conversa. Um "oi" deixa de pagar trinta segundos de thinking;
um problema de lógica continua com o orçamento inteiro.

Um decisor não escreve texto. Ele recebe uma pergunta por campo, com os valores
permitidos escritos, e só o primeiro token de cada valor permitido é pontuado —
todos os campos em paralelo, a partir de um prefixo (instruções mais esquema)
que fica em cache. É por isso que a resposta chega em milissegundos e nunca sai
do esquema: o decisor pode errar o julgamento, nunca o formato.

Existem dois decisores, tentados nesta ordem. Tudo é fail-open: quando nenhum
responde, o esforço que você configurou prevalece.

### Decisor local

Um segundo `llama-server`, compilado do
[fork parallel-decision do llama.cpp](https://github.com/thecodacus/llama.cpp/tree/parallel-decision),
com um modelo pequeno só para decidir — por padrão o
`Qwen/Qwen2.5-1.5B-Instruct-GGUF` em Q8_0 (1,9 GB, Apache-2.0), de atenção
pura, que é a arquitetura que esta técnica recompensa. **Instalar** baixa o
motor (cerca de 200 MB, um pacote que o próprio OpenWeights compila e publica) e
o modelo; os dois aparecem no painel de downloads. Depois você pode escolher
qualquer outro GGUF da biblioteca como decisor.

Ele sobe junto do servidor local, em `127.0.0.1:11713`, e para com ele. Enquanto
roda ocupa cerca de **2,6 GB de VRAM** ao lado do modelo do chat, e as medições
de "cabe?" não descontam isso: ele liga sozinho só em GPUs com 12 GB ou mais, e
abaixo disso o cartão mostra o custo e deixa você ligar à mão. Precisa de
Windows ou Linux com GPU NVIDIA em CUDA 13 (driver 580 ou mais novo). Nada sai
da sua máquina.

### Reserva: Jev no OpenRouter

O modelo de decisão **Jev**, da TypeSafe, alcançado pela chave do OpenRouter,
responde quando o decisor local não está instalado, ainda está carregando ou
falha. Quando isso acontece, a mensagem e o trecho saem da sua máquina — o
cartão diz isso ao lado do interruptor — e o custo é fração de centavo por mil
mensagens (só tokens de entrada). Desmarque a reserva para manter as decisões
estritamente locais.

### Onde vale, e o endpoint

Dois interruptores dizem onde a decisão vale. **Chat do app** decide antes de
cada envio e mostra quem decidiu nos detalhes da execução da resposta (*Local*
ou *Jev*). **Agentes de código** sobe um proxy local pequeno em
`127.0.0.1:11712` na frente do motor; o DeepSeek Harness e os outros agentes
que o app abre passam a apontar para ele, e cada `chat/completions` que mandam
recebe a decisão aplicada ao corpo. O cabeçalho de resposta `x-openweights-jev`
conta o que aconteceu: `alto;0.91;local`, `medio;0.80;jev`, `nenhum;cache` ou
`default;<motivo>`. Qualquer outra requisição atravessa intocada, streaming
inclusive, e o proxy só escuta no endereço de loopback.

O proxy também repassa `POST /v1/decision` ao decisor local, então qualquer
coisa que alcance `127.0.0.1:11712` — um agente de código, o gateway, o
[decision playground](https://github.com/thecodacus/decision-playground) — pode
fazer as próprias perguntas. A requisição nomeia os campos com os valores
permitidos e um ou mais contextos; a resposta traz o valor e a probabilidade de
cada campo:

```json
POST /v1/decision
{
  "instructions": "Route this support ticket.",
  "schema": {
    "category": {"type": "enum", "choices": ["billing", "technical", "other"],
                 "description": "What is the ticket about?"},
    "urgent": {"type": "boolean", "description": "Does it need urgent handling?"}
  },
  "contexts": ["I was charged twice and need this fixed today."]
}
```

```json
{"results": [{"decision": {"category": "billing", "urgent": true},
              "fields": {"category": {"value": "billing", "probability": 0.97},
                         "urgent": {"value": true, "probability": 0.88}}}],
 "timings": {"total_ms": 41.0}}
```

Os campos não enxergam as respostas uns dos outros: faça perguntas
independentes. **Só agir acima de** é o piso de confiança, e **Testar decisão**
manda uma pergunta de amostra e conta qual decisor respondeu e quanto tempo
levou.

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
