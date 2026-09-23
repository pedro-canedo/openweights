# Servidor de API local

O **Servidor Local** expõe uma API compatível com a OpenAI para outros apps
usarem o modelo que você já tem carregado.

## Como a tela se organiza

No topo, sempre visível enquanto você rola, fica o estado do servidor: ligado ou
parado, o endereço com botão de copiar, o modelo carregado e a velocidade de
geração. O resto vem em quatro abas:

| Aba | O que tem |
|---|---|
| **Visão geral** | Conectar (endereço, chave, teste de conexão), abrir em outro app e o que já foi servido |
| **Desempenho** | Configurar o llama.cpp, especulação medida, histórico de benchmark e energia da GPU |
| **Rede** | Porta, acesso pela rede local, modelos simultâneos, conversas ao mesmo tempo e GPU extra na rede |
| **Avançado** | Flags globais e o log do servidor |

Na primeira visita, a Visão geral abre com três passos — ligar, copiar o
endereço, colar no app que vai usar. Eles somem sozinhos quando o servidor
atende a primeira requisição.

## Ligando

Aperte **Iniciar** no topo e o endereço aparece com um botão de copiar. Os
ajustes de rede ficam na aba **Rede**, e valem no momento de iniciar — mudar um
deles exige parar e iniciar o motor de novo. A tela diz isso.

| Ajuste | O que faz |
|---|---|
| **Porta** | Onde ele escuta |
| **Permitir acesso da rede local** | Outros aparelhos da sua rede alcançam a API |
| **Chave de API** | Opcional; quando definida, as requisições precisam apresentá-la (fica em Conectar, na Visão geral) |
| **Modelos simultâneos** | Com 1, trocar de modelo descarrega o anterior — o que a maioria das GPUs aguenta. Acima disso, os modelos ficam carregados juntos e podem não caber na memória de vídeo |
| **Conversas ao mesmo tempo** | Cada conversa simultânea leva uma fatia da janela de contexto. Com 1, a janela que você pediu é a janela que você tem |

O log do servidor fica na aba **Avançado**.

A build por trás da API pode mudar sozinha: escolher um modelo Bonsai reinicia
o servidor no [motor da PrismML](/pt/guia/modelos#modelos-bonsai-e-o-motor-da-prismml),
e escolher qualquer outro modelo o reinicia na build oficial. A faixa desta
tela diz qual está no ar, e o `/v1/models` reporta essa build.

## Configurar o llama.cpp

Na aba **Desempenho** fica **Configurar llama.cpp**: a configuração de carga
**por modelo**, no mesmo lugar onde o modelo é carregado.

Escolha um modelo da sua biblioteca e a seção mostra o que o arquivo declara —
se é MoE, se traz **cabeça MTP**, se tem projetor de visão, até que janela foi
treinado. **Recomendar para esta máquina** preenche tudo medindo o seu hardware
de verdade (a memória vem do `llama-fit-params`, do próprio pacote do llama.cpp,
não de uma conta nossa).

A partir daí:

| O que | Onde |
|---|---|
| Janela de contexto, cache KV, flash attention, visão | Controles diretos, com a explicação do preço de cada um |
| **Especulação** | MTP e n-grama, e **os dois ao mesmo tempo** — são complementares, veja abaixo |
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

## Especulação medida

Decodificação especulativa é o modelo adivinhar vários tokens à frente e
conferir todos numa passada, ficando com o que ele aceita. Há dois tipos, e
eles são complementares: um **rascunho** (MTP, de camadas que vêm no próprio
arquivo) adivinha texto novo, enquanto um **n-grama** adivinha o que já está
escrito no prompt — que é a maior parte do dia de um agente de código,
reescrevendo um arquivo que acabou de ler. O llama.cpp aceita os dois juntos, e
o app oferece os dois.

Se compensa depende da máquina e do tipo de texto, então o app não decide por
regra — ele **mede**. Uma vez por modelo, máquina e versão do motor, com a
máquina parada, ele testa cada combinação em dois prompts (código e prosa,
reportados separados, porque uma média esconderia exatamente o que interessa) e
aplica o vencedor. O motor reinicia entre os testes; qualquer uso seu
interrompe e adia. O interruptor que desliga isso está no mesmo card.

::: tip Um número de velocidade não diz que a resposta virou lixo
Esta é a parte que importa. A decodificação especulativa é *lossless* — o
modelo grande confere cada rascunho e descarta o que recusa —, então com
temperatura zero a resposta **tem** de ser idêntica à de quem não especula. O
app compara o texto e **recusa** qualquer configuração que o tenha mudado,
mostrando o trecho divergente em vez de apenas afirmar que conferiu.

Ele também se testa antes: se a execução de referência discorda da própria
repetição, o não-determinismo é do kernel da GPU e não da especulação, e
ninguém é desclassificado.
:::

## Energia da GPU

Gerar tokens é limitado pela **banda de memória** da placa, não pelo quanto ela
pode queimar — e banda não sobe com o limite de energia. Então, nesta carga,
baixar o limite costuma custar quase nada em velocidade e tirar bastante calor
e consumo.

"Costuma" é a palavra honesta, então o card lê o estado pelo NVML e deixa você
alternar entre o padrão da placa e um alvo econômico — os dois valores vêm do
driver — e manda medir. O histórico de desempenho registra o limite em vigor em
cada corrida e marca qualquer comparação que atravesse dois limites diferentes,
de modo que o experimento seja legível.

Duas coisas que o card diz em vez de esconder: aplicar exige administrador, e o
limite **não sobrevive a reiniciar o computador** — é assim que a NVIDIA fez,
está documentado no NVML, e nenhum aplicativo contorna.

## Histórico de desempenho

Cada medição fica guardada por máquina, modelo e build do motor, com a variação
contra a corrida anterior. Dois números, não um: **geração** e **processamento
de prompt** são trabalhos diferentes, e cada um tem seu delta com sua condição
— comparar 800 tok/s num prompt de 512 com 300 num de 4096 chamava de piora o
que era simplesmente outro eixo.

Uma corrida fica sem delta quando a versão do motor mudou, quando a placa estava
esquentando durante ela, ou quando não há nada comparável antes. A tela diz qual
é o caso.

## Flags globais

Na aba **Avançado**, um cartão guarda as flags que valem para **todos** os
modelos. As de
processo viram argumentos do `llama-server`; as demais entram na seção `[*]` do
INI, e a configuração própria de um modelo sempre vence a global.

## Abrir em um harness

Na **Visão geral**, o cartão **Usar em outro app** entrega os seus modelos a um
agente de código externo apontado para a sua API local — um clique, sem configurar endpoint na
mão. É aqui que o trabalho de agente mora agora: o app não tem um
chat-com-ferramentas próprio.

| Harness | Como |
|---|---|
| [**AgenticOw**](/pt/guia/harness) | Nosso fork do [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness), com tela própria na barra lateral — subir, parar e remover acontecem ali, e ele aparece dentro da janela do app. O app instala um runtime pré-compilado e verificado numa pasta do app (no Node portátil dele) e entrega **todos** os seus provedores e modelos, mantendo a lista em dia enquanto ele roda. O cartão daqui leva até essa tela |
| **Claude Code** | Lançado contra a sua API local, com o modelo já escolhido |
| **Aider** | Sobe apontado para a sua API, com o modelo já escolhido |
| **OpenCode** | Idem, pelas variáveis de ambiente que ele espera |

Cada cartão diz se o programa está instalado, mostra o comando (copiável) e
oferece o botão **Abrir**. Quando não está instalado mas o `npx` existe, o app
usa o `npx` e mostra o comando de instalação definitiva.

A tela do AgenticOw é onde o ciclo de vida dele inteiro mora: o progresso da
primeira abertura (download, verificação, subida), a interface dele quando está
no ar, e uma desinstalação que pode levar junto as sessões e configurações
criadas lá dentro.
O botão **Agente** no compositor do Chat leva para essa mesma tela.

O app também conta ao AgenticOw duas coisas sobre cada modelo local que, sem
isso, ficariam no chute. O **teto de saída** sai da janela de contexto do
próprio modelo (metade dela), em vez de o AgenticOw assumir 32k para todo mundo
— número que um modelo pequeno não tem como honrar. E quando o chat template
do modelo lê `enable_thinking` — o caso do Qwen3 e afins — o app declara o
interruptor de raciocínio, e o AgenticOw passa a mostrar um seletor de esforço
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
