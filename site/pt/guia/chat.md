# Chat

A tela de chat é onde um modelo te responde. Ela transmite em streaming,
renderiza markdown e código, guarda o histórico em disco e deixa você voltar
atrás e mudar o que pediu.

## O básico

- **Modelo** — escolhido no compositor. Trocar no meio da conversa fica salvo
  com ela, então reabrir depois restaura o mesmo modelo.
- **O motor sobe sozinho** na sua primeira mensagem; a primeira carga de um
  modelo na memória demora, e o app diz isso em vez de parecer travado.
- **Regerar, editar e reenviar, apagar** — toda mensagem tem. Editar uma
  mensagem reescreve o histórico a partir dali.
- **Copiar como Markdown** exporta a conversa inteira.
- **Ler em voz alta** narra uma resposta; o botão de microfone dita uma.

## Anexos e `@arquivo`

Arraste arquivos para a conversa, use o menu **+**, ou digite `@` para escolher
um arquivo da pasta do projeto. Imagens exigem um modelo multimodal — o app
avisa quando o modelo atual não tem projetor de visão.

## Janela de contexto

O anel ao lado do compositor é o **medidor de contexto**: quanto da janela do
modelo já está comprometido, dividido em instruções de sistema, conversa,
raciocínio, mensagem atual e anexos.

Isso importa mais do que parece. O raciocínio e a resposta dividem o mesmo
orçamento da conversa, e o cache KV mora na VRAM: uma janela maior não é de
graça. A janela é definida **na carga** do modelo, não por mensagem — mudá-la
pede uma recarga.

## Parâmetros

O painel da direita tem duas metades, e a divisão é o ponto:

**Por mensagem** — valem no próximo envio:

| Parâmetro | O que faz |
|---|---|
| Instruções de sistema | A instrução permanente para o modelo |
| Criatividade (temperatura) | Mais alta divaga mais, mais baixa repete mais |
| Top-P / Top-K | Quão largo é o conjunto de tokens candidatos |
| Limite de tokens da resposta | Teto duro da resposta |
| Esforço | Respostas mais completas, mais lentas e mais pesadas |

Presets salvam um conjunto de parâmetros com um nome.

Em modelo local, o app traduz o **Esforço** para os níveis que o próprio
modelo aceita, lidos do arquivo: Alto, Extra e Máx viram o nível mais alto
dele (`xhigh` no Qwen3.8 e no Bonsai 2, que recusam `high`), e Baixo desliga o
raciocínio.

Com o [Jev](/pt/integracoes/provedores#jev-camada-de-decisao) ligado em Fontes,
**Esforço** vira um teto em vez de um valor fixo: antes de cada envio um modelo
de decisão olha a mensagem e desliga o raciocínio do modelo local, põe no médio
ou deixa no que você escolheu. Os detalhes da execução da resposta dizem o que
foi decidido e com que confiança; quando o seu valor foi mantido, nada aparece.

**Amostragem avançada** (modo Avançado) — uma seção recolhida com `min_p`,
penalidades de repetição, de presença e de frequência, uma semente (mesma
semente e mesmos parâmetros, mesma resposta), sequências de parada e o pedido de
resposta em JSON (com um esquema JSON, se quiser). Campo vazio não é enviado, e
**Usar os padrões do servidor** para de mandar a temperatura, o Top-P e o Top-K.
Logo abaixo, **Copiar requisição** põe na área de transferência a chamada que o
chat faria agora, como cURL, Python ou JavaScript — o mesmo corpo que o chat
manda, sem o streaming. A chave de API nunca vai na cópia: o código a lê da
variável de ambiente `OPENWEIGHTS_API_KEY`.

**Na carga** — janela de contexto, cache KV, flash attention, especulação
(MTP), visão e o resto dos botões do llama.cpp — mudaram de casa: agora moram
em **Servidor Local**, junto do modelo que os usa. O atalho no painel leva
direto para lá, com o modelo da conversa já selecionado. A razão é que carga é
propriedade do modelo, não da conversa: é a mesma configuração para o chat e
qualquer app que consuma a API. Veja
[configurar o llama.cpp](/pt/integracoes/api-local#configurar-o-llama-cpp).

## Quando você quer trabalho feito, não só resposta

Chat é chat: o modelo fala. Para trabalho de agente — ler e editar arquivos,
rodar comandos — existe o **AgenticOw**, um agente de código completo com
**item próprio na barra lateral**, logo abaixo do Chat. Ele é o fork do próprio
OpenWeights do [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness);
o app instala, supervisiona e mostra ele dentro da janela, já apontado para
todos os provedores e modelos que você tem. O botão **Agente** no compositor só leva
até lá. Veja [o agente de código](/pt/guia/harness).
