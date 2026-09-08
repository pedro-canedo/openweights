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

Esse motor é **do aplicativo**: fica numa pasta dele, isolado de qualquer
llama.cpp que você já tenha no sistema. Pouco depois de abrir, o app o verifica
sozinho — executa o binário e lê a build que ele reporta, o que separa "os
arquivos estão lá" de "o motor funciona". Se algo pede providência (build
antiga depois de uma atualização do OpenWeights, variante errada porque a placa
ou o driver mudaram, pacote incompleto), aparece um ponto no item
**Configurações** da barra lateral, e lá o card do motor diz o que fazer em uma
frase. Builds antigas que ficaram para trás aparecem com o tamanho e um botão
para devolver o espaço.

## 3. Seu primeiro modelo

Em **Descobrir**, busque um modelo (`qwen`, `llama`, `gemma`…) e abra. A lista
de quantizações é colorida para a *sua* máquina:

- <span class="ow-verdict ow-verdict--gpu"></span> **verde** — roda inteiro na GPU;
- <span class="ow-verdict ow-verdict--split"></span> **amarelo** — divide entre GPU e CPU, mais devagar;
- <span class="ow-verdict ow-verdict--cpu"></span> **cinza** — só CPU.

Escolha uma, baixe, e ela aparece em **Meus Modelos**. Downloads interrompidos
podem ser retomados, mesmo depois de reiniciar o computador.

::: tip Modelos com licença
Alguns repositórios exigem aceitar uma licença no Hugging Face. O aceite fica
gravado na **sua conta**, não na máquina — o app precisa saber quem é você.

Em **Configurações**, clique em **Entrar com o Hugging Face**: o navegador
abre na página de autorização, você confirma, e a conta fica conectada. Não há
token a criar nem a colar, e o app renova a sessão sozinho enquanto baixa.

Feito isso, clique em **Aceitar a licença no Hugging Face** no aviso do
modelo. Enquanto a aba estiver aberta, o app pergunta ao Hub a cada poucos
segundos se o portão abriu — assim que você aceita, o aviso some e o download
que estava esperando começa sozinho.

Quem preferir continuar colando um token pode fazê-lo em **Ou usar um token
manualmente**. Nesse caminho vale a atenção de sempre: um token *fine-grained*
precisa da permissão "Read access to contents of all public gated repos", ou o
download falha mesmo com a licença aceita — e **Configurações** avisa quando é
esse o caso.
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
- [Desempenho no chat](/pt/guia/desempenho) — por que a resposta demorou.
- [O agente de código](/pt/guia/harness) — quando você quer trabalho feito,
  não só resposta.
