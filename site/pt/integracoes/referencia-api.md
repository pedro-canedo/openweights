# Referência da API

O servidor local é o `llama-server` em modo Router, então a superfície é a que
o llama.cpp expõe — compatível com OpenAI no que importa. Esta página lista o
que dá para chamar e como o modo Router muda as respostas. Para ligar o
servidor, o endereço e a chave de API, veja o
[servidor de API local](/pt/integracoes/api-local).

Todo caminho abaixo é relativo ao endereço mostrado no topo do **Servidor
Local** — `http://127.0.0.1:<porta>` por padrão.

## Autenticação

Se há uma chave de API definida, toda requisição precisa levá-la:

```
Authorization: Bearer <sua chave>
```

Sem chave definida, as requisições são aceitas a partir da própria máquina.
Ligar o acesso pela rede local remove essa fronteira — veja o aviso em
[servidor de API local](/pt/integracoes/api-local).

## Chat completions

```
POST /v1/chat/completions
```

O endpoint que um cliente OpenAI espera. O campo `model` não é opcional aqui
como seria contra um servidor de modelo único: **é ele que diz ao modo Router
qual modelo carregar**.

| Campo | Observações |
|---|---|
| `model` | O id de `GET /v1/models`, ou o nome mostrado em **Meus Modelos**. Carrega o modelo se ele não estiver residente |
| `messages` | Papéis padrão: `system`, `user`, `assistant` |
| `stream` | `true` transmite eventos SSE, que é o que a tela de chat usa |
| `temperature`, `top_p`, `top_k` | Amostragem; os parâmetros de chat do app mapeiam para estes |
| `max_tokens` | Teto da resposta. O app também aplica o dele — veja [Chat](/pt/guia/chat) |
| `tools` | Repassado ao chat template do modelo quando ele aceita |

Um modelo que precisa ser carregado antes faz a requisição demorar mais, não
falhar. Essa espera é a fase de "carregamento" descrita em
[Desempenho no chat](/pt/guia/desempenho).

## Text completions

```
POST /v1/completions
```

O formato antigo, sem chat. Disponível para clientes anteriores ao chat
completions; integração nova deve usar `/v1/chat/completions`, porque é ele que
aplica o chat template do modelo.

## Embeddings

```
POST /v1/embeddings
```

Funciona quando o modelo carregado produz embeddings. Um modelo de chat a quem
se pede embedding responde com erro, em vez de um vetor sem sentido.

## Listar modelos

```
GET /v1/models
```

Devolve os ids que o Router está pronto para servir. São esses os ids para pôr
em `model`, e eles batem com os nomes em **Meus Modelos**. Perguntar isso
**não** acorda um modelo adormecido.

## Estado do servidor

```
GET /health
GET /props
GET /slots
```

O `/health` responde assim que o processo sobe, e é o que o app espera quando
você aperta Iniciar.

O `/props` informa as capacidades do servidor, incluindo o que o chat template
do modelo carregado aceita — é dali que o app lê quais níveis de esforço de
raciocínio um modelo realmente suporta, em vez de oferecer nomes inventados.

::: warning /props no modo Router
Pedir `/props` **sem** `?model=` devolve as propriedades do *roteador*, não as
de modelo nenhum. É uma resposta verdadeira para outra pergunta, e é um jeito
fácil de concluir que um modelo não tem uma capacidade que ele tem.
:::

O `/slots` informa os slots de contexto em uso. O app consulta para o medidor
de contexto e para os tokens por segundo — os números ali contam todos os
clientes, incluindo o agente de código e qualquer coisa externa que você
apontou para este endereço, não só o chat.

## O que o modo Router muda

Um processo serve todos os modelos. É isso que permite ao chat e a esta API
dividirem um motor, em vez de subir um processo por modelo, e tem três
consequências que vale conhecer:

- **`model` seleciona, e pode carregar.** Uma requisição para um modelo não
  residente o carrega. Com **Modelos simultâneos** em 1, isso descarrega o
  anterior.
- **Leituras do servidor são do servidor.** O `/slots` e os números de
  velocidade incluem todos os clientes.
- **Alguns ajustes são de carga.** Tamanho de contexto e camadas na GPU valem
  quando um modelo carrega; mudá-los significa recarregar o modelo. Veja a
  [referência de configuração](/pt/integracoes/configuracao).

## Erros

Os erros voltam no formato que clientes OpenAI esperam, com status HTTP e corpo
JSON. Os dois que você vai encontrar de verdade:

| Status | Costuma significar |
|---|---|
| `401` | Há chave de API definida e a requisição não apresentou |
| `404` | O id em `model` não existe — confira `GET /v1/models` |
| `500` | O motor recusou a requisição. Causa comum é um parâmetro que o chat template do modelo não aceita, como um nível de esforço de raciocínio que ele nunca declarou |
| `503` | O servidor está no ar e o modelo ainda está carregando |

O log completo fica em **Servidor Local → Avançado**, e é o jeito mais rápido
de distinguir uma requisição recusada de um modelo que falhou ao carregar.
