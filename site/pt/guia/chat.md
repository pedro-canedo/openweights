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

**Na carga** — precisam do modelo recarregado:

| Ajuste | O que faz |
|---|---|
| Janela de contexto | Quanto o modelo lembra nesta carga |
| Cache KV | Memória da conversa na GPU; comprimir cabe uma janela maior na mesma VRAM |
| Flash attention | Geração mais rápida com a mesma memória |
| Especulação | Prevê tokens à frente — MTP quando o arquivo traz, n-grama ajuda em código |
| Visão | Se o projetor carrega sempre, sob demanda ou nunca |
| Camadas na GPU, experts na CPU, batch, threads, mmap, mlock | Os botões do llama.cpp, com explicação em português |

Presets salvam um conjunto de parâmetros com um nome.

## Chat ou agente

O botão no compositor alterna entre **Chat** — o modelo só fala — e **Agente**,
onde ele pode usar ferramentas para fazer o trabalho de verdade. Isso é uma
seção inteira à parte: [o harness agêntico](/pt/agente/).
