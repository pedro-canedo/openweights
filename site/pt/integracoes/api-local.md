# Servidor de API local

O **Servidor Local** expõe uma API compatível com a OpenAI para outros apps
usarem o modelo que você já tem carregado.

## Ligando

Escolha a porta, aperte **Iniciar**, e o endereço aparece com um botão de copiar.
Tudo abaixo vale no momento de iniciar — mudar um ajuste exige parar e iniciar o
motor de novo. A tela diz isso.

| Ajuste | O que faz |
|---|---|
| **Porta** | Onde ele escuta |
| **Permitir acesso da rede local** | Outros aparelhos da sua rede alcançam a API |
| **Chave de API** | Opcional; quando definida, as requisições precisam apresentá-la |
| **Modelos simultâneos** | Com 1, trocar de modelo descarrega o anterior — o que a maioria das GPUs aguenta. Acima disso, os modelos ficam carregados juntos e podem não caber na memória de vídeo |
| **Conversas ao mesmo tempo** | Cada conversa simultânea leva uma fatia da janela de contexto. Com 1, a janela que você pediu é a janela que você tem |

A tela também mostra o log do servidor.

## Configurar o llama.cpp

Logo abaixo dos ajustes do servidor fica **Configurar llama.cpp**: a
configuração de carga **por modelo**, no mesmo lugar onde o modelo é carregado.

Escolha um modelo da sua biblioteca e a seção mostra o que o arquivo declara —
se é MoE, se traz **cabeça MTP**, se tem projetor de visão, até que janela foi
treinado. **Recomendar para esta máquina** preenche tudo medindo o seu hardware
de verdade (a memória vem do `llama-fit-params`, do próprio pacote do llama.cpp,
não de uma conta nossa).

A partir daí:

| O que | Onde |
|---|---|
| Janela de contexto, cache KV, flash attention, visão | Controles diretos, com a explicação do preço de cada um |
| **Especulação (MTP)** | Ligar `draft-mtp` abre os tokens por rascunho (2–4 é o equilíbrio) e a confiança mínima (0,75 evita ciclos desperdiçados em contexto longo) |
| Camadas na GPU, experts na CPU, batch, threads, mmap, mlock | Em "mais opções" |
| **Todas as outras flags** | Uma busca sobre o catálogo inteiro da versão instalada do llama.cpp — `rope`, `yarn`, `cache-reuse`, `jinja`, `lora`, `override-kv`, o que existir |

O catálogo não é uma lista escrita à mão que envelhece: as flags mais usadas têm
rótulo e dica em português, e **todo o resto é lido do próprio binário**. Quando
o motor for atualizado, as flags novas aparecem na busca sozinhas.

Flags que o app administra (porta, chave de API, caminho do modelo, cluster)
aparecem com um cadeado apontando para o controle certo, em vez de sumirem.

**Presets** guardam um conjunto com nome; já vêm *Padrão*, *MTP turbo*,
*Economia de VRAM* e *Contexto longo*. Aplicar um preset mescla sobre o que você
já tinha.

A **prévia** mostra o comando e o arquivo INI exatos que o motor vai receber no
próximo boot — gerados pelo mesmo código que escreve o arquivo de verdade, então
o que está na tela é o que vai acontecer.

Como o roteador só lê essa configuração ao subir, mudanças ficam marcadas como
"valem no próximo reinício". O botão **Carregar** reinicia sozinho quando há algo
pendente, e o mesmo botão descarrega o modelo — a memória de vídeo volta na hora.

### Flags globais

Um cartão à parte guarda as flags que valem para **todos** os modelos. As de
processo viram argumentos do `llama-server`; as demais entram na seção `[*]` do
INI, e a configuração própria de um modelo sempre vence a global.

## Abrir em um harness

A seção **Abrir em um harness** entrega os seus modelos a um agente de código
externo apontado para a sua API local — um clique, sem configurar endpoint na
mão. É aqui que o trabalho de agente mora agora: o app não tem um
chat-com-ferramentas próprio.

| Harness | Como |
|---|---|
| [**DeepSeek Harness**](https://github.com/deepseek-ai/deepseek-harness) | Tem tela própria na barra lateral — instalar, subir, parar e remover acontecem ali, e ele roda embutido no app. O app instala o `dsh` numa pasta isolada (Node portátil incluído) e escreve **todos** os seus provedores e modelos numa pasta de configuração própria. O cartão daqui leva até essa tela |
| **Claude Code** | Lançado contra a sua API local, com o modelo já escolhido |
| **Aider** | Sobe apontado para a sua API, com o modelo já escolhido |
| **OpenCode** | Idem, pelas variáveis de ambiente que ele espera |

Cada cartão diz se o programa está instalado, mostra o comando (copiável) e
oferece o botão **Abrir**. Quando não está instalado mas o `npx` existe, o app
usa o `npx` e mostra o comando de instalação definitiva.

A tela do DeepSeek Harness é onde o ciclo de vida dele inteiro mora: o estado
da instalação, o log ao vivo da primeira (Node portátil mais ~190 pacotes npm,
de dez a trinta minutos), o painel embutido quando está no ar, e uma
desinstalação que pode levar junto as sessões e credenciais criadas lá dentro.
O botão **Agente** no compositor do Chat leva para essa mesma tela.

O app também conta ao harness duas coisas sobre cada modelo local que antes
ficavam no chute. O **teto de saída** passa a sair da janela de contexto do
próprio modelo (metade dela), em vez de o harness assumir 32k para todo mundo
— número que um modelo pequeno não tem como honrar. E quando o chat template
do modelo lê `enable_thinking` — o caso do Qwen3 e afins — o app declara o
interruptor de raciocínio, e o harness passa a mostrar um seletor de esforço
com um **Off** que desliga o raciocínio de verdade. Isso pesa mais do que
parece: um modelo de raciocínio diante de um pedido aberto ("faça um site
bonito") gasta o orçamento inteiro de saída pensando, e para antes de escrever
o primeiro arquivo.

::: tip A chave nunca vai no comando
Quando você define uma chave de API, ela viaja por variável de ambiente — nunca
na linha de comando, que qualquer processo da máquina consegue ler. A prévia
mostra a chave mascarada.
:::

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
