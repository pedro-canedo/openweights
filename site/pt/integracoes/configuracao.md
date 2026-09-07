# Referência de configuração

Tudo o que o app consegue dizer ao llama.cpp, e onde cada ajuste vai parar.
Esta página explica o **sistema**; a lista exata de chaves mora no app, em
**Servidor Local → Desempenho → Configurar o llama.cpp**, porque ela é lida do
binário que esta versão fixa e cresce quando o llama.cpp cresce.

## Onde um ajuste pode morar

O motor roda em modo Router: um processo `llama-server`, mais um arquivo INI
que descreve cada modelo. Um ajuste tem, portanto, duas casas possíveis — a
linha de comando do processo e o INI — e qual delas ele usa não é detalhe de
estilo.

| Escopo | Onde cai | Exemplo |
|---|---|---|
| **Global** | Argumento de linha de comando do processo | Porta, host, nível de log |
| **Por modelo** | A seção do próprio modelo no INI do Router | Tamanho de contexto, camadas na GPU, quantização do cache |
| **Ambos** | A seção `[*]` do INI quando definido globalmente, nunca a linha de comando | Chaves que fazem sentido como padrão, mas precisam continuar sobrescrevíveis |
| **Só do Router** | Chave que existe apenas no INI | `load-on-startup` |
| **Gerenciado** | O app decide, e mostra um cadeado | Porta, chave de API, ligação do cluster |

**A precedência é o que torna "Ambos" sutil.** O Router resolve um ajuste nesta
ordem:

```
linha de comando   >   a seção do próprio modelo   >   a seção [*]
```

Uma flag global colocada na linha de comando venceria toda escolha por modelo,
em silêncio. É por isso que os ajustes de escopo "Ambos" vão para o `[*]`: um
padrão que o modelo ainda pode sobrescrever é um padrão; um que não pode é uma
regra se passando por padrão.

## Categorias

O catálogo é agrupado pela pergunta com que você chega, não pela ordem em que o
`--help` imprime:

| Categoria | Do que trata |
|---|---|
| **context** | Janela de contexto, lotes, quanto do prompt é preservado |
| **memory** | Camadas na GPU, travamento de memória, quantização do cache, offload |
| **cpu** | Threads para geração e para processamento de prompt |
| **rope** | Escalonamento RoPE e YARN, para esticar um modelo além da janela treinada |
| **spec** | Decodificação especulativa: o modelo rascunho e o quanto ele adivinha à frente |
| **multimodal** | O projetor (`mmproj`) para modelos que enxergam imagens |
| **adapters** | LoRA e vetores de controle |
| **server** | Host, porta, slots, tempos-limite, nível de log |
| **router** | Chaves que só o Router entende |
| **usage** | Todo o resto que o binário fixado aceita |

## Ajustes que dependem de outra coisa

Algumas chaves só fazem sentido dentro de um contexto, e o app as esconde até
ele valer: um ajuste de modelo rascunho não significa nada sem especulação
ligada, e um caminho de projetor não significa nada num modelo que não enxerga.
Os requisitos que o app confere são a presença de GPU, mais de uma GPU, o
modelo ser Mixture-of-Experts, o modelo aceitar predição de múltiplos tokens, a
presença de um projetor, a especulação estar ligada, o RoPE estar em YARN e o
Flash Attention estar ligado.

É por isso que o mesmo modelo mostra listas diferentes em duas máquinas. Não
está faltando nada — a chave não teria efeito ali.

## Carga contra requisição

Um ajuste que o motor lê ao **carregar** um modelo não muda enquanto aquele
modelo está carregado. O app avisa, e parar e subir o servidor faz parte de
aplicá-lo. Tamanho de contexto, camadas na GPU e quantização do cache são todos
de carga.

Parâmetros de requisição — temperatura, top-p, penalidades, o teto da resposta
— pertencem à conversa, não ao motor, e moram no [Chat](/pt/guia/chat).

## Ver o que vai ser enviado

O card **Configurar o llama.cpp** mostra a linha de comando exata e o INI exato
que o app vai escrever, renderizados pelo mesmo código que os escreve na
inicialização — não é uma reconstrução. Se a prévia e a realidade discordarem
algum dia, isso é um defeito que vale relatar.

## Medir em vez de chutar

Uma configuração que parece melhor no papel é uma hipótese. O
[Desempenho no chat](/pt/guia/desempenho) explica como testar uma contra o
perfil atual, com aquecimento e repetições, e como o app decide quando uma
diferença é real em vez de ruído.
