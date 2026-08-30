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

**Na carga** — janela de contexto, cache KV, flash attention, especulação
(MTP), visão e o resto dos botões do llama.cpp — mudaram de casa: agora moram
em **Servidor Local**, junto do modelo que os usa. O atalho no painel leva
direto para lá, com o modelo da conversa já selecionado. A razão é que carga é
propriedade do modelo, não da conversa: é a mesma configuração para o chat e
qualquer app que consuma a API. Veja
[configurar o llama.cpp](/pt/integracoes/api-local#configurar-o-llama-cpp).

## Quando você quer trabalho feito, não só resposta

Chat é chat: o modelo fala. Para trabalho de agente — ler e editar arquivos,
rodar comandos — o botão **Agente** no compositor abre o
[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness): um agente
de código completo que o app instala, configura e abre por você, já apontado
para todos os provedores e modelos que você tem. A primeira abertura instala;
depois é um clique. Veja
[Abrir em um harness](/pt/integracoes/api-local#abrir-em-um-harness).
