# Primeira execução

A primeira abertura faz três coisas que você nunca mais repete: olha a sua
máquina, baixa o motor que combina com ela e te oferece um primeiro modelo.

## 1. Detecção de hardware

O OpenWeights lê CPU, RAM, GPU e VRAM. É sobre isso que toda recomendação
posterior é construída — quanto de um modelo cabe na placa, quantas camadas
mandar para a GPU, qual quantização recebe sinal verde.

Os números continuam visíveis na barra de status, embaixo: CPU, RAM, GPU, VRAM,
disco e rede, ao vivo, ao lado dos tokens/s do que estiver gerando.

## 2. O motor de IA

Em seguida o app baixa a build do llama.cpp para o seu hardware — **algumas
centenas de MB**, uma vez só:

| Sua placa | O que é baixado |
|---|---|
| NVIDIA | Build CUDA (mais o runtime CUDA redistribuído pelo llama.cpp) |
| AMD, Intel, Apple e outras | Build Vulkan ou Metal |
| Sem GPU aproveitável | Build só de CPU |

É por isso que o instalador é pequeno: nenhuma pilha de GPU vai dentro do
pacote. O runtime CUDA vem direto da release do llama.cpp, na sua máquina,
sujeito à [EULA do CUDA da NVIDIA](https://docs.nvidia.com/cuda/eula/) — o
instalador do OpenWeights não redistribui bibliotecas da NVIDIA.

## 3. Seu primeiro modelo

Em **Descobrir**, busque um modelo (`qwen`, `llama`, `gemma`…) e abra. A lista
de quantizações é colorida para a *sua* máquina:

- <span class="ow-verdict ow-verdict--gpu"></span> **verde** — roda inteiro na GPU;
- <span class="ow-verdict ow-verdict--split"></span> **amarelo** — divide entre GPU e CPU, mais devagar;
- <span class="ow-verdict ow-verdict--cpu"></span> **cinza** — só CPU.

Escolha uma, baixe, e ela aparece em **Meus Modelos**. Downloads interrompidos
podem ser retomados, mesmo depois de reiniciar o computador.

::: tip Modelos com licença
Alguns repositórios exigem aceitar uma licença no Hugging Face. Aceite na página
do modelo e cole um token do Hugging Face em **Configurações** — o app usa ele
para baixar.
:::

## Ajuste para esta máquina

Com o modelo baixado, o OpenWeights pode perguntar ao próprio llama.cpp quanta
memória cada configuração custa **na sua placa**, recomendar uma com os números
por trás, aplicar e desfazer sozinho se o modelo não carregar.

Se você quiser, ele então mede os tokens/s reais e troca a estimativa pelo que a
sua máquina realmente entregou. Uma estimativa que você pode conferir vale mais
que uma promessa que você não pode.

## A barra de status

A faixa de baixo não é enfeite. Ela informa, ao vivo: CPU, RAM, GPU e VRAM; o
**consumo da placa contra o limite dela**; disco e rede; **qual modelo está
carregado** (com um ponto pulsando enquanto ele gera); **quanto da janela de
contexto está em uso**; e os **tokens por segundo** do momento.

Esse último número vem do servidor, não do chat — então ele conta o agente de
código e qualquer app externo apontado para a sua API, não só o que você digita
aqui.

## Para onde ir agora

- [Modelos e quantização](/pt/guia/modelos) — como ler as cores.
- [Chat](/pt/guia/chat) — a tela de conversa, parâmetros e anexos.
- [O agente de código](/pt/guia/harness) — quando você quer trabalho feito,
  não só resposta.
