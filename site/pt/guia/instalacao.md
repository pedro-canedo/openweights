# Instalação

## Uma linha

::: code-group

```powershell [Windows]
irm https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.ps1 | iex
```

```bash [macOS e Linux]
curl -fsSL https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.sh | sh
```

:::

O script escolhe o arquivo certo para o seu sistema, instala, e no macOS ainda
tira a quarentena para o app abrir sem briga.

## Na mão

Pegue o arquivo do seu sistema na
[última release](https://github.com/pedro-canedo/openweights/releases/latest):

| Sistema | Arquivo |
|---|---|
| Windows 10/11 (x64) | `OpenWeights_x.y.z_x64-setup.exe` |
| macOS 11+ (Apple Silicon e Intel) | `OpenWeights_x.y.z_universal.dmg` |
| Linux x64 (Debian/Ubuntu) | `OpenWeights_x.y.z_amd64.deb` |
| Linux x64 (qualquer distro) | `OpenWeights_x.y.z_amd64.AppImage` |

No Windows, a instalação deixa o **atalho na Área de Trabalho** e a entrada no
menu Iniciar — não é preciso caçar o app depois. Quem prefere sem o atalho pode
apagá-lo: as atualizações não o trazem de volta.

Depois de instalado, o app **procura versões novas sozinho** e oferece a
atualização em um clique — você não precisa voltar aqui.

## O aviso de binário não assinado

**Os binários não são assinados.** Assinatura de código exige um certificado
pago e anual que o projeto ainda não tem. Seu sistema vai avisar; dizer isso na
cara é melhor do que fingir que o aviso é bug.

**Windows** — em *"O Windows protegeu o computador"*: clique em **Mais
informações** → **Executar assim mesmo**.

**macOS** — a instalação de uma linha acima já resolve. Se você baixou o `.dmg`
na mão e apareceu *"a Apple não conseguiu verificar se este app está livre de
malware"*:

- **macOS 15 (Sequoia) ou mais novo**: tente abrir uma vez, depois vá em
  *Ajustes do Sistema → Privacidade e Segurança*, role até o aviso sobre o
  OpenWeights e clique em **Abrir assim mesmo**.
- **macOS 14 ou anterior**: clique com o botão direito no app → **Abrir**.
- **Em qualquer versão**, pelo Terminal:

  ```bash
  xattr -dr com.apple.quarantine /Applications/OpenWeights.app
  ```

O que a build de macOS *tem* é uma assinatura **ad-hoc**, feita pela própria
máquina que compilou. Ela não vale como procedência — só evita que o sistema
recuse de saída um `.dmg` universal.

## Espaço em disco

O instalador é pequeno de propósito: nenhuma pilha de GPU vai dentro dele. Na
primeira execução o app baixa o runtime do llama.cpp da sua placa — **algumas
centenas de MB**, uma vez só. Os modelos são baixados à parte e são o grosso do
disco: conte alguns GB por modelo.

## Desinstalar

| Sistema | Como |
|---|---|
| Windows | *Configurações → Aplicativos → OpenWeights → Desinstalar* |
| macOS | Arraste `OpenWeights.app` para o Lixo |
| Linux (`.deb`) | `sudo apt remove openweights` |
| Linux (AppImage) | Apague o arquivo |

Modelos e conversas ficam fora do pacote do app e sobrevivem à desinstalação —
apague a pasta de dados na mão se quiser tudo fora.
