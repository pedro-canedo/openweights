# Servidor de API local

O **Servidor Local** expõe uma API compatível com a OpenAI para outros apps
usarem o modelo que você já tem carregado.

## Ligando

Escolha a porta, aperte **Iniciar**, e o endereço aparece com um botão de copiar.
Tudo abaixo vale no momento de iniciar — mudar um ajuste exige parar e iniciar o
motor de novo. A tela diz isso, e recusa reiniciar enquanto uma execução do
agente ou a indexação do projeto estiver usando o motor (ela diz qual).

| Ajuste | O que faz |
|---|---|
| **Porta** | Onde ele escuta |
| **Permitir acesso da rede local** | Outros aparelhos da sua rede alcançam a API |
| **Chave de API** | Opcional; quando definida, as requisições precisam apresentá-la |
| **Modelos simultâneos** | Com 1, trocar de modelo descarrega o anterior — o que a maioria das GPUs aguenta. Acima disso, os modelos ficam carregados juntos e podem não caber na memória de vídeo |
| **Conversas ao mesmo tempo** | Cada conversa simultânea leva uma fatia da janela de contexto. Com 1, a janela que você pediu é a janela que você tem |

A tela também lista os modelos em memória e o log do servidor.

## Usando

::: code-group

```bash [curl]
curl http://127.0.0.1:PORTA/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "SEU-MODELO", "messages": [{"role": "user", "content": "Olá!"}]}'
```

```python [Python]
from openai import OpenAI

client = OpenAI(base_url="http://127.0.0.1:PORTA/v1", api_key="local")
resp = client.chat.completions.create(
    model="SEU-MODELO",
    messages=[{"role": "user", "content": "Olá!"}],
)
print(resp.choices[0].message.content)
```

:::

`GET /v1/models` lista o que está carregado. Os ids dos modelos são os mesmos
mostrados em **Meus Modelos**.

::: warning Rede local é rede local
Ligar o acesso pela rede remove a fronteira do "só esta máquina". Qualquer um na
mesma rede alcança seus modelos — defina uma chave de API, e só ligue isso em
rede confiável.
:::

## Requisitos

O motor de IA precisa estar instalado antes — o que acontece na sua primeira
execução. Até lá a tela diz isso em vez de falhar ao iniciar.

## Pegando emprestada a placa de outra máquina

A mesma tela abriga o painel de emparelhamento da
[**GPU extra na rede**](/pt/integracoes/cluster): outro OpenWeights na sua rede
empresta a placa dele para os dois carregarem juntos um modelo que nenhum
segurava sozinho. Vem desligado.
