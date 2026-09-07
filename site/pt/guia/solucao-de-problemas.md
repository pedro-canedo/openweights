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

## Relatar outra coisa

Abra uma [issue](https://github.com/pedro-canedo/openweights/issues) com o seu
hardware, o modelo, e o log de **Servidor Local → Avançado**. Quase toda
pergunta aqui depende desses três, e sem eles investigar vira adivinhação.
