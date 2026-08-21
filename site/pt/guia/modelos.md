# Modelos e quantização

## GGUF, e só GGUF

O OpenWeights roda modelos pelo llama.cpp, que lê arquivos **GGUF**. MLX (o
formato da Apple), safetensors, GPTQ e AWQ não rodam aqui. Quando você cai num
modelo que só publica um desses, o app avisa e oferece procurar a versão GGUF —
para modelos populares, quase sempre alguém já publicou uma.

## O que a quantização realmente custa

Quantização é quantos bits cada peso guarda. Menos bits significa arquivo menor,
menos memória e geração mais rápida, com alguma perda de qualidade:

| Quantização | Tamanho aproximado vs. F16 | Uso típico |
|---|---|---|
| `Q8_0` | ~50% | o mais próximo do original, se couber |
| `Q6_K` | ~40% | bem próximo, comum em placas de 24 GB |
| `Q5_K_M` | ~33% | bom equilíbrio |
| `Q4_K_M` | ~28% | o padrão de fato — mais qualidade por GB |
| `Q3_K_M` | ~22% | quando nada mais cabe |
| `Q2_K` | ~16% | último recurso, degradação visível |

A regra que importa é mais simples que a tabela: **o modelo inteiro na VRAM
ganha de uma quantização melhor dividida com a CPU**. Um `Q4_K_M` inteiro na GPU
costuma ser mais rápido *e* mais agradável que um `Q6_K` transbordando para a
RAM do sistema.

## Lendo as cores

Na gaveta de quantizações cada arquivo é pontuado contra o seu hardware de
verdade:

- <span class="ow-verdict ow-verdict--gpu"></span> **Verde** — cabe inteiro na VRAM, com espaço para a janela de contexto.
- <span class="ow-verdict ow-verdict--split"></span> **Amarelo** — parte das camadas vai para a CPU. Funciona; é mais lento.
- <span class="ow-verdict ow-verdict--cpu"></span> **Cinza** — só CPU. Usável em modelos pequenos, sofrido nos grandes.

A conta inclui a janela de contexto, porque o cache KV mora na mesma memória: um
modelo que cabe com 4k tokens pode não caber com 32k.

## Meus Modelos

Tudo que foi baixado aparece em **Meus Modelos**, com tamanho e quantização,
atalho para conversar e apagar. Modelos importados na mão — jogados na pasta sem
passar pelo Descobrir — aparecem marcados como tal.

Downloads que pararam no meio ficam listados à parte, com **Retomar** e
**Descartar**. Retomar sobrevive a fechar o app e reiniciar a máquina.


## Ajuste, sem você ajustar nada

Cada botão que o llama.cpp expõe — tamanho da janela, tipo do KV cache, flash
attention, quantas camadas vão para a GPU, como a carga se divide entre duas
máquinas — tem uma resposta certa para o *seu* hardware e para aquele modelo. O
app chega nela sozinho.

Ele não chuta. Três coisas são perguntadas em vez de estimadas:

| A pergunta | Quem responde |
|---|---|
| Quais dispositivos existem e quanto sobra em cada um | `llama-server --list-devices`, com a GPU emprestada dentro quando há par |
| Quantas camadas o modelo tem de verdade | o cabeçalho do GGUF (`block_count`) |
| Quanto uma configuração custa de memória | o `llama-fit-params`, em cerca de um segundo e meio, sem carregar o modelo |

Com essas três, uma busca dirigida converge numa configuração: a maior janela
que ainda mantém os pesos na placa, comprimindo o KV cache só quando a janela
exige, flash attention onde ajuda, e a razão do split tirada da memória livre
real.

Ela roda sozinha em segundo plano assim que o motor sobe, e de novo quando o
conjunto de dispositivos muda — parear com outra máquina muda o que cabe tanto
quanto trocar de placa. Cada modelo guarda a impressão digital da situação em
que foi ajustado (máquina, versão do motor, dispositivos), então nada é medido
duas vezes.

**O que você escolheu na mão nunca é tocado.** A passada automática só preenche
o que ela mesma deixou, ou o que nunca teve nada. E como o preset do Router é
lido no boot, uma configuração encontrada com o motor rodando vale no próximo
start — a tela avisa.

### Quando você quiser ver os números

**Meus Modelos → Ajustar para esta máquina** abre o painel: cada candidato com
a memória que custa por dispositivo, qual foi escolhido e por quê, e a opção de
medir tokens/s de verdade com o `llama-bench` em vez de confiar na estimativa.
Está lá para quando você quiser olhar, não porque o app precise de você.

::: tip Uma coisa que não dá para fazer por você
No Apple Silicon, o macOS limita o Metal a cerca de 75% da memória unificada.
Levantar isso exige terminal e a sua senha, então o app mostra o comando em vez
de executá-lo.
:::
