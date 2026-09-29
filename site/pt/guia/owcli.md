# OwCLI: agente e terminais

O **OwCLI** é a tela de terminais do OpenWeights, logo abaixo do AgenticOw na
barra lateral. Cada sessão ali é um terminal de verdade: o agente OwCLI, um
shell comum (bash, zsh, PowerShell) ou um agente de fora, como o Claude Code.
Várias ficam abertas ao mesmo tempo, lado a lado, e continuam rodando quando
você troca de tela.

O agente OwCLI é o fork do OpenWeights do
[Codex CLI da OpenAI](https://github.com/openai/codex) (licença Apache-2.0):
o núcleo do upstream, com a marca, a privacidade e os seus modelos por cima. A
interface dele, por enquanto, é em inglês.

## A tela

À esquerda ficam as sessões abertas. Cada uma mostra o título que o programa
deu ao terminal, a pasta onde está e um ponto de estado: verde enquanto roda,
apagado quando terminou (com o código de saída), amarelo quando precisa de
você. As setas do teclado andam pela lista.

À direita fica a grade. Os botões de **Disposição**, no alto, mostram 1, 2 ou 4
painéis, e o divisor entre eles se arrasta com o mouse ou com as setas. A
escolha fica guardada. Clicar numa sessão da lista a põe no painel com foco.
Uma sessão aparece em um painel só, e diminuir a grade esconde painéis sem
fechar nada.

O **+** no alto da lista (**Nova sessão**) abre as duas saídas: **Agente
OwCLI…** ou **Novo terminal**. Com a tela vazia, os mesmos dois botões ficam no
meio dela, e cada painel vazio da grade também oferece os dois.

Trocar de tela não para nenhuma sessão: o terminal continua rodando e
desenhando, e o que ele escreveu está lá quando você volta. Fechar a sessão (o
**×** na lista ou o menu do botão direito) encerra o programa e tudo o que ele
abriu.

## O agente

**Nova sessão → Agente OwCLI…** (ou **Abrir o agente OwCLI**, com a tela vazia,
ou o mesmo comando na paleta, Ctrl+K) pergunta quatro coisas antes de abrir:

- **Modelo**: os modelos que o OpenWeights sabe servir agora, agrupados por
  fonte. A escolha fica lembrada para a próxima sessão.
- **Pasta de trabalho**: a pasta pessoal, ou a que você escolher.
- **Quando pedir sua aprovação**: *quando precisar* (recomendado), *antes de
  qualquer comando* ou *nunca*.
- **O que ele pode fazer**: *ler e editar a pasta* (recomendado), *só ler* ou
  *tudo, sem limites*.

Lá dentro, o `/model` troca de modelo na mesma conversa, entre todas as fontes.

### Instalar o agente

O agente não vem dentro do app: é um programa à parte, baixado na primeira vez
que você o escolhe. Antes de baixar, o app diz o tamanho no próprio botão, e só
baixa se você aceitar. O pacote é conferido pelo tamanho e pelo sha256 que o
app traz, e testado nesta máquina antes de ficar: um pacote que não roda aqui
não fica instalado, e **Tentar de novo** baixa outra vez. Ele mora na pasta do
app (`runtimes/owcli`). Fechar a janela durante o download não o interrompe, e
instalado, o app segue direto para o diálogo de abrir.

Há pacote para Linux x64, Windows x64 e macOS. Num computador sem pacote, o app
avisa que o agente não está disponível ali, e os terminais funcionam
normalmente.

### De onde vem o cérebro

O OwCLI não tem provedor, chave nem login próprios: ele pensa só com os
modelos do app. São os do Servidor Local, os favoritos do OpenRouter e os do
9router, os mesmos do AgenticOw. Quando não há nenhum (o servidor está parado
e nenhuma fonte remota está ligada), o diálogo vira **Escolha o cérebro do
OwCLI** e mostra o próximo passo de cada fonte. **Iniciar o Servidor Local**
sobe o motor ali mesmo, e o diálogo volta com os modelos.

**Você vê o modelo pensando.** Modelos locais pensam antes de responder, às
vezes por minutos, e o agente só mostrava "Working". As sessões abertas pelo app
agora mostram o raciocínio do modelo à medida que ele o escreve, para você
acompanhar o que ele está pesando e para onde vai. O
[painel de logs](/pt/integracoes/api-local#vendo-o-que-esta-rodando)
(`Ctrl+Shift+L`) mostra o lado do motor: tokens gerados e velocidade.

## Histórico

Abaixo das sessões abertas ficam as conversas que o OwCLI gravou. Cada uma
mostra o nome (ou o começo da primeira mensagem), quando foi e em que pasta.
Clicar continua a conversa numa sessão nova, na pasta dela e com o modelo
dela, se ele ainda estiver disponível. O lápis dá um nome à conversa.

## Quando ele precisa de você

Quando o agente pede aprovação para um comando e você não está olhando para
aquela sessão, ela fica com o ponto amarelo e o aviso **pede sua atenção** na
lista. Com a janela do app sem foco, chega também uma notificação do sistema,
no máximo uma a cada 15 segundos por sessão. Abrir a sessão limpa o aviso.

## Copiar e colar

**Ctrl+Shift+C** copia a seleção e **Ctrl+Shift+V** cola; o menu do botão
direito faz o mesmo e também limpa a tela. Programas que copiam pelo terminal
(a sequência OSC 52, usada pelo tmux e pelo Neovim) chegam à área de
transferência do sistema.

## Outros agentes numa sessão

Claude Code, Aider e OpenCode abrem aqui também, a partir de **Servidor Local
→ Usar em outro app**: o app sobe o programa numa sessão desta tela, apontado
para a sua API local e com o modelo escolhido. No Windows, o cartão oferece
ainda **Terminal externo**, que abre o programa numa janela à parte.

## O comando `owcli` no terminal do sistema

Em **Configurações → OwCLI no terminal do sistema** (modo Avançado), **Ativar o
comando owcli** deixa o agente disponível em qualquer terminal, com os mesmos
modelos da tela OwCLI e a mesma casa (`~/.owcli`), onde ficam as conversas. Nada muda no seu terminal sem esse
clique, e **Desativar** desfaz só o que o app fez.

- **Linux e macOS:** um link em `~/.local/bin/owcli` para a versão instalada. Se
  já existir um arquivo com esse nome que não é do OpenWeights, o app avisa e não
  mexe nele. No macOS, e em alguns Linux, `~/.local/bin` não está no PATH: o
  cartão mostra a linha para pôr no `~/.zshrc` ou `~/.bashrc`.
- **Windows:** a pasta do agente entra no `Path` do seu usuário (o da máquina não
  é tocado). Abra um terminal novo depois de ativar.

Quando o agente é atualizado, o comando passa a apontar para a versão nova, e a
versão antiga só é apagada quando nenhuma sessão roda dela. Como o agente pensa
com os modelos do OpenWeights, **o app precisa estar aberto** quando você usa o
comando; fechado, o `owcli` diz isso e sai.

## Privacidade

- O OwCLI conversa só com o OpenWeights, em `127.0.0.1`, e prova quem é com um
  token do app. As chaves do OpenRouter e do 9router nunca saem do app: quem
  as põe em cada pedido é o próprio OpenWeights.
- Telemetria, envio de feedback, checagem de versão e os recursos que falam
  com serviços da OpenAI vêm desligados.
- As conversas e as configurações do OwCLI moram na casa dele (`~/.owcli`),
  separadas de qualquer Codex instalado na mesma máquina.
