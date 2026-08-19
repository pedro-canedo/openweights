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
