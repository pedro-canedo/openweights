# OwCLI: terminais

O **OwCLI** é a tela de terminais do OpenWeights, logo abaixo do AgenticOw na
barra lateral. Cada sessão ali é um terminal de verdade: um shell comum (bash,
zsh, PowerShell) ou um agente de código de fora, como o Claude Code. Várias
ficam abertas ao mesmo tempo, lado a lado, e continuam rodando quando você
troca de tela.

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

Trocar de tela não para nenhuma sessão: o terminal continua rodando e
desenhando, e o que ele escreveu está lá quando você volta. Fechar a sessão (o
**×** na lista ou o menu do botão direito) encerra o programa e tudo o que ele
abriu.

## Quando um programa precisa de você

Quando um programa pede a sua atenção (um agente pedindo aprovação para um
comando, por exemplo) e você não está olhando para aquela sessão, ela fica com
o ponto amarelo e o aviso **pede sua atenção** na lista. Com a janela do app sem foco, chega também uma notificação do sistema,
no máximo uma a cada 15 segundos por sessão. Abrir a sessão limpa o aviso.

## Copiar e colar

**Ctrl+Shift+C** copia a seleção e **Ctrl+Shift+V** cola; o menu do botão
direito faz o mesmo e também limpa a tela. Programas que copiam pelo terminal
(a sequência OSC 52, usada pelo tmux e pelo Neovim) chegam à área de
transferência do sistema.

## Agentes de código numa sessão

Claude Code, Aider e OpenCode abrem aqui também, a partir de **Servidor Local
→ Usar em outro app**: o app sobe o programa numa sessão desta tela, apontado
para a sua API local e com o modelo escolhido. No Windows, o cartão oferece
ainda **Terminal externo**, que abre o programa numa janela à parte.
