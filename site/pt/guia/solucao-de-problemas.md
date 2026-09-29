# Solução de problemas

A maior parte dos problemas aqui vem de um de quatro lugares: o sistema não
confiar num binário sem assinatura, o motor não combinar com a máquina, um
modelo não caber na placa, ou algo já estar usando a porta. Cada seção abaixo
diz o que você vê, por quê, e o que resolve.

Se nada disto cobrir o seu caso, o log em **Servidor Local → Avançado** e uma
[issue](https://github.com/pedro-canedo/openweights/issues) são o passo
seguinte — o modelo de issue pede hardware e modelo porque quase nada aqui se
diagnostica sem eles.

## O sistema recusa abrir o app

Os binários não têm certificado pago de assinatura, então os dois sistemas
avisam. Isso é esperado, e o aviso é sobre procedência, não sobre o app estar
fazendo alguma coisa.

| Sistema | O que você vê | O que fazer |
|---|---|---|
| Windows | *O Windows protegeu o seu PC* | **Mais informações** → **Executar assim mesmo** |
| macOS 15+ | *não pode ser aberto* | Tente uma vez e então **Ajustes do Sistema → Privacidade e Segurança → Abrir Assim Mesmo** |
| macOS 14 ou anterior | *não pode ser aberto* | Clique com o botão direito no app → **Abrir** |

No terminal do macOS, isto resolve em uma linha:

```bash
xattr -dr com.apple.quarantine /Applications/OpenWeights.app
```

O script de instalação já faz isso por você; o aviso aparece quando você baixa
o `.dmg` na mão.

## O card do motor diz que algo está errado

O app roda o llama.cpp dele e pergunta ao binário qual build ele é, em vez de
confiar no nome da pasta. A conclusão é uma de cinco:

| Veredito | O que significa | O que fazer |
|---|---|---|
| **Pronto** | Executou e respondeu com a build que esta versão espera | Nada |
| **Não instalado** | Primeira execução, ou a pasta foi apagada | Instale em **Configurações → Motor de IA** |
| **Atualização disponível** | O disco tem uma build diferente da que esta versão testou | Atualize |
| **Variante errada** | Sua placa ou o driver mudaram desde a instalação | Reinstale — o app escolhe a variante de novo |
| **Não executa** | Os arquivos estão lá e o executável não sobe | Reinstale. Um pacote CUDA sem as DLLs do runtime passa em qualquer checagem de arquivo e só falha aqui |

## O download do motor falha ou trava

São algumas centenas de megabytes, buscados de uma release fixada no GitHub. Um
proxy corporativo ou um antivírus varrendo o arquivo são as causas comuns — numa
máquina com varredura em tempo real, conte com vários minutos em vez de
segundos. A instalação é atômica: um download que falha deixa o motor anterior
funcionando.

## Nenhuma GPU é detectada

A barra de status mostra só CPU, e os modelos recebem nota pior do que você
esperava.

- **NVIDIA**: o driver precisa ser recente o bastante para a build CUDA que o
  app escolheu. Um driver anterior a ela faz o motor cair para trás ou recusar.
- **Gráficos integrados** podem não ser informados. Isso é uma lacuna de
  detecção, não um defeito — o app usa a placa dedicada quando existe uma.
- **Depois de trocar placa ou driver**, reinstale o motor: a variante é
  escolhida no momento da instalação, e a placa embaixo mudou.

## Um modelo não carrega

A cor do veredito em **Descobrir** e **Meus Modelos** é uma estimativa de se o
arquivo cabe na sua placa, feita antes de você baixar. Quando um modelo que
recebeu verde se recusa a carregar mesmo assim:

- **O tamanho do contexto entra na conta.** Um modelo que cabe em 4K pode não
  caber em 32K — o cache KV cresce com a janela. Defina um contexto explícito
  em vez de pedir o máximo.
- **Modelos simultâneos multiplicam isso.** Com o ajuste acima de 1, os modelos
  ficam carregados juntos e dividem a mesma memória de vídeo.
- **Outra coisa está usando a placa.** Um navegador com aceleração por
  hardware, um jogo, ou outra instância do OpenWeights levam a fatia deles.

Baixar as camadas na GPU move parte do modelo para a RAM do sistema: mais
lento, mas roda. Veja a
[referência de configuração](/pt/integracoes/configuracao).

## Um modelo Bonsai diz que precisa do motor da PrismML

Arquivos `PTQ1_0` e `PQ2_0` só carregam no fork da PrismML do llama.cpp. O app
instala esse motor junto com o download; se mesmo assim ele faltar, o chat e o
cartão do modelo mostram um botão **Instalar motor PrismML** com o tamanho do
download para a sua máquina — uns 30 MB na versão Vulkan, cerca de 511 MB no
CUDA 13.3 do Windows, cerca de 727 MB (cerca de 1 GB no disco) no CUDA 12.8 do
Linux com placa NVIDIA —, de uma release fixada no GitHub, conferida
executando o binário. O motor troca
sozinho ao escolher o modelo e volta no seguinte; a faixa do Servidor Local diz
qual está no ar. A otimização e as ferramentas de desempenho medem um modelo
Bonsai **nesse motor**, já que medir é abrir o arquivo com um binário. O que
elas pulam é o braço do MoE-cache, que é outro fork e não abre esses arquivos;
a GPU pela rede (cluster) também continua na build oficial.

No Linux, a versão Vulkan do fork não tem kernels de `PQ2_0` e roda esse
formato na CPU — bem menos de 1 token/s de prompt no 27B. Com placa NVIDIA
(driver 570 ou mais novo, fora a RTX 50), um arquivo `PQ2_0` pede a versão
CUDA 12.8, e o cartão oferece **Instalar a versão CUDA**. Onde o CUDA não é
possível (AMD ou Intel, driver antigo) ou não subiu na sua máquina, o cartão
avisa e sugere o arquivo `PTQ1_0` do mesmo repositório, que roda na GPU pelo
Vulkan. **Tentar a versão CUDA de novo** repete a tentativa depois que você
resolver a causa; atualizar o driver repete sozinho. Se a verificação só disser
que a placa não respondeu *agora* (cheia ou ocupada), os arquivos baixados
ficam guardados e a próxima tentativa só refaz a verificação.

## O servidor local não sobe

| O que você vê | Costuma significar |
|---|---|
| *porta já em uso* | Outro processo a segura — mude a porta em **Rede**, ou pare o outro processo |
| O servidor sobe e cai na hora | O motor falhou ao carregar um modelo. O motivo está em **Avançado** → o log |
| *o motor não está instalado* | Instale em **Configurações → Motor de IA** primeiro |

Um ajuste que vale na inicialização não muda com o servidor rodando. A tela
avisa, e parar e subir faz parte de aplicá-lo.

## Um app externo não alcança a API

- **O endereço é `127.0.0.1` por padrão**, o que significa só esta máquina.
  Alcançar de outro dispositivo exige ligar o acesso pela rede local em
  **Rede**.
- **Chave definida é chave exigida.** A requisição precisa enviar
  `Authorization: Bearer <chave>`.
- **O campo `model` precisa bater** com um id de `GET /v1/models`.

A [referência da API](/pt/integracoes/referencia-api) lista os endpoints e o
que cada status de erro significa.

## GPU extra na rede

| O que você vê | O que significa |
|---|---|
| *Nenhum outro OpenWeights visível nesta rede* | A outra máquina está com o recurso desligado, em outra rede, ou o mDNS está bloqueado pelo firewall |
| *tag diferente — atualize* | Os dois apps trazem builds diferentes do llama.cpp. Atualize os dois |
| *O motor instalado não inclui o worker RPC* | O motor é de um pacote antigo. Atualize em **Configurações → Motor de IA** |
| *o servidor local está rodando nesta máquina* | Pare o servidor local na máquina que empresta antes de ceder a GPU |

## O agente de código não instala

A primeira abertura baixa um Node portátil e os pacotes dele — centenas de
megabytes, e vários minutos numa máquina com antivírus varrendo cada arquivo. O
log ao vivo fica na tela do harness. Ele instala numa pasta própria e nunca
toca num Node que você tenha instalado.

## As respostas estão mais lentas do que deveriam

Antes de mexer em ajustes, descubra qual fase está lenta: os **Detalhes da
execução** depois de uma resposta separam fila, carregamento do modelo, espera
pelo primeiro token e geração. Primeiro token lento em toda mensagem costuma
ser modelo sendo recarregado; geração lenta o tempo todo é questão de
configuração.

O [Desempenho no chat](/pt/guia/desempenho) explica como comparar duas
configurações com honestidade, com aquecimento e repetições, em vez de confiar
numa rodada só.

## A janela do app apaga quando aparece um emoji (Linux)

No AppImage, o navegador embutido foi compilado contra o FreeType do Ubuntu
22.04 e usa o do sistema. Em sistemas com FreeType mais novo e emoji colorido
no formato COLRv1 (o Fedora 44, por exemplo), ele não desenha os emojis com
gradiente, e a janela apagava — de novo a cada recarga. Desde a 0.25.1 o app
esconde essas fontes da própria interface nesses sistemas, e os emojis aparecem
em preto e branco; se não houver outra fonte de emoji instalada, aparecem como
quadradinhos, e uma fonte monocromática como a Noto Emoji resolve. O log do app
diz quando a proteção está ligada (`fontes COLRv1 escondidas da interface`). O
pacote `.deb` usa o navegador do sistema e não tem esse problema.

## O OwCLI responde "We're currently experiencing high demand"

É o jeito do OwCLI mostrar um HTTP 500 do modelo. Antes da 0.26.1 acontecia com
modelos cujo template de chat aceita uma única mensagem de sistema, no começo
(Ternary Bonsai 2, Qwen3.8): o agente manda várias, e o motor recusava o pedido.
Agora o app as reúne antes de chegarem ao motor, então atualize para a 0.26.1 ou
mais nova. Se um modelo do 9router responde `401 Missing API key`, o 9router
tinha acabado de ser instalado e a chave dele ainda não existia; desde a 0.26.1 o
app espera por ela e tenta de novo sozinho, então dê alguns segundos depois de o
9router subir.

A mesma mensagem aparecia também em tarefas longas: se o modelo era parado no
meio de uma chamada de ferramenta (um arquivo inteiro de uma vez), o motor
devolvia a chamada cortada mas marcada como completa, e todo pedido seguinte
falhava. Desde a 0.27.2 o app conserta essa chamada no histórico e o modelo
refaz em partes menores. O painel de logs (`Ctrl+Shift+L`) mostra o conserto.

## Relatar outra coisa

Abra uma [issue](https://github.com/pedro-canedo/openweights/issues) com o seu
hardware, o modelo, e o log de **Servidor Local → Avançado**. Quase toda
pergunta aqui depende desses três, e sem eles investigar vira adivinhação.
