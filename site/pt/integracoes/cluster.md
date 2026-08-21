# GPU extra na rede

Duas máquinas com o OpenWeights na mesma rede carregam um modelo juntas. O
arquivo fica em uma delas; a outra só empresta a placa.

É a resposta para um problema específico: o modelo que não cabe em *nenhuma* das
duas placas sozinha. Uma de 12 GB com um Mac de 18 GB viram 30 GB emparelhados —
o suficiente para pesos que, de outro jeito, transbordariam para a RAM do
sistema e arrastariam a geração.

## O que não é

- **Não é mais rápido.** Dividir um modelo por cabo de rede acrescenta espera em
  cada fronteira de camada. Emparelhado, um modelo que já cabia numa placa fica
  *mais lento*. O app sabe disso e só reparte o que não cabe local.
- **Não é nuvem.** Nada sai da sua rede. Não há conta, não há retransmissor, não
  há servidor nosso no meio.
- **Não é mais de um ajudante.** Um host, um worker. O desenho é esse.

## O que precisa

| | |
|---|---|
| **A mesma tag do llama.cpp** | O formato do fio RPC muda entre builds. Tag diferente, e o emparelhamento é recusado com *tag diferente — atualize* |
| **GPU nos dois lados** | Máquina sem placa aparece na lista marcada como *sem GPU* e não pode ser emparelhada |
| **A mesma rede** | A descoberta é mDNS; o canal de controle é uma porta TCP na sua LAN |

Não há nada a mais para baixar. O pacote do llama.cpp que o app já instala na
primeira execução é compilado com suporte a RPC e traz o worker
(`ggml-rpc-server`) ao lado do servidor — então, com o motor de IA instalado e
atualizado, o cluster já tem tudo de que precisa.

## Ligando

O painel fica em **Servidor local**, abaixo das configurações do servidor.

**Oferecer GPU na rede** vem desligado nas duas máquinas. Nada é anunciado e
nenhuma porta é aberta até você ligar — ligar *é* o consentimento. Faça isso nas
duas máquinas, numa rede em que você confia.

Na primeira vez o app busca o motor com RPC (algumas centenas de MB). O painel
mostra *preparando o motor RPC…* enquanto isso acontece.

## Emparelhando

1. Na máquina que tem o modelo, o outro OpenWeights aparece na lista com a placa
   dele e quanta memória oferece.
2. Toque em **Usar como GPU extra**.
3. A outra máquina recebe uma notificação do sistema e mostra o pedido no painel
   dela. Nada acontece até alguém tocar em **Aceitar** ali. Esse é o único
   momento que exige decisão humana.
4. No aceite, o ajudante sobe o processo RPC e o host reinicia o motor apontando
   para ele. A barra de status ganha um chip nos dois lados.

Um par aceito uma vez volta sozinho na próxima vez que os dois apps estiverem
abertos na mesma rede — usando um segredo combinado naquele primeiro aceite, e
não o nome que o app anuncia na rede. **Esquecer** desfaz, e a próxima tentativa
pergunta de novo.

## Como o split é decidido

Cada máquina anuncia um orçamento, não a placa inteira: **75% da VRAM** em
NVIDIA, ou **75% de 75% da RAM** no Apple Silicon (o próprio macOS recusa dar ao
Metal muito mais que três quartos da memória unificada). O resto é cache KV e
buffers de cálculo — anunciar tudo é o jeito clássico de estourar a memória no
primeiro prompt.

Quem tem mais memória fica com as primeiras camadas. Um ajudante de 18 GB ao
lado de uma placa local de 12 GB vira `--device RPC0,CUDA0 --tensor-split 3,2`.
O painel mostra a razão escolhida.

## O que esperar durante o uso

- **O primeiro carregamento é lento.** Os pesos viajam pela rede até a outra
  máquina — uns dois minutos para 16 GB em gigabit. O ajudante guarda em disco,
  então carregar o mesmo modelo de novo é rápido.
- **Wi-Fi funciona mal.** Cada fronteira de camada atravessa o enlace. Cabo, se
  der.
- **O ajudante deve estar parado.** Emprestar a placa e conversar na mesma
  máquina reserva a mesma memória de vídeo duas vezes. O app recusa emprestar
  com o servidor local no ar, e diz isso.
- **Se a outra máquina sumir**, o par cai em cerca de quinze segundos e o host
  reinicia o motor sem a GPU remota.

## Segurança

::: danger Leia antes de ligar
O canal RPC **não tem senha**, e o próprio llama.cpp diz isso sem rodeio: o
backend RPC é uma prova de conceito e não deve rodar em rede aberta. O segredo
do emparelhamento protege de quem apenas conhece o nome que o app anuncia — ele
não é criptografado, então não protege de quem captura o tráfego da rede.

Só aceite máquinas que você reconhece, e nunca encaminhe a porta para a
internet.
:::

A parte que é nossa se comporta com cautela: o recurso fica desligado até você
ligar, o ajudante só abre o processo RPC depois que alguém tocou em Aceitar, um
"aceito" que chega sem ter sido pedido é recusado, e um pedido com o segredo
errado não derruba um par vivo.

## Quando não funciona

| O que aparece | O que significa |
|---|---|
| *Nenhum outro OpenWeights visível nesta rede* | A outra máquina está com o recurso desligado, está em outra rede, ou o mDNS está bloqueado no firewall |
| *tag diferente — atualize* | Os dois apps trazem builds diferentes do llama.cpp. Atualize os dois |
| *O motor instalado não traz o worker RPC* | O motor veio de um pacote antigo. Atualize em **Ajustes → Motor de IA** |
| *o servidor local está rodando nesta máquina* | Pare o servidor local no ajudante antes de emprestar a placa |
