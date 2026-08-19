# Checkpoints

Antes de o agente alterar qualquer coisa, o harness tira uma foto do projeto para
você poder voltar atrás com um clique.

## Quando um é tirado

Antes da **primeira alteração da execução** — o primeiro `fs_write`, `fs_edit`
ou qualquer coisa que toque o disco. Leitura nunca dispara um. No modo automático
(YOLO) isso não é opcional: o checkpoint é parte do que torna aquele modo
defensável.

A trilha mostra *Checkpoint criado* na linha do tempo, com a hora e qual mecanismo
o tirou. A lista de **Checkpoints** fica na coluna do explorador, ao lado dos
arquivos que ela protege.

## Dois mecanismos

| Mecanismo | Como funciona |
|---|---|
| **Git sombra** | Um repositório git **paralelo**, na pasta de dados do app, que versiona a pasta do projeto sem NUNCA tocar no seu `.git` — nada de renomear, mover ou commitar no seu repositório. Funciona apontando `GIT_DIR`/`GIT_WORK_TREE` para outro lugar. |
| **Cópia de arquivos** | Uma cópia dos arquivos que serão alterados. Sempre disponível (não exige git instalado) e barata quando a execução mexe em poucos arquivos. |

A escolha é automática: git quando existe e o projeto não é gigante; cópia caso
contrário. A lista diz qual foi usado.

::: tip Seu repositório nunca é tocado
Esta é a parte que vale repetir. Um checkpoint não commita, não faz stage, não
faz stash e não reescreve nada do seu histórico git. Se você tinha trabalho não
commitado quando o agente começou, ele continua exatamente como estava.
:::

::: tip Sem janelas de console no Windows
Cada checkpoint chama o `git` algumas vezes, e até a v0.2.2 cada uma dessas
chamadas piscava uma janela preta por cima do que você estava fazendo. O app não
tem console próprio, então o Windows dava um console novo a cada processo filho.
Agora todo processo que o OpenWeights inicia nasce sem console: nada aparece por
cima da sua tela, e nenhuma janela solta pode ser fechada por engano no meio de
uma tarefa.
:::

## Restaurar

Clique em **Restaurar** em qualquer checkpoint. Ele pergunta antes —
*"Restaurar os arquivos para este ponto? As alterações posteriores serão
perdidas."* — porque é a única ação do fluxo do agente que joga trabalho fora.

Restaurar volta os *arquivos*. A conversa fica: você mantém o registro do que foi
tentado, que costuma ser justamente o que você precisa para escrever a próxima
instrução.
