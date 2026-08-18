# Instalador do OpenWeights para Windows.
#
#   irm https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.ps1 | iex
#
# Baixa o instalador NSIS do último release e o executa. Por padrão em modo
# silencioso — quem chamou um script de uma linha não quer clicar em "Avançar"
# três vezes. Para ver a janela do instalador: `-Interactive`.
#
# O binário NÃO é assinado, então o SmartScreen reclama do arquivo baixado à
# mão. Executado por aqui, o aviso não aparece; o download continua vindo do
# GitHub, e o script mostra qual arquivo está pegando antes de rodar.
[CmdletBinding()]
param(
    # Mostra a janela do instalador em vez de instalar em silêncio.
    [switch]$Interactive
)

$ErrorActionPreference = 'Stop'
$repo = 'pedro-canedo/openweights'

function Falha($mensagem) {
    Write-Host "error: $mensagem" -ForegroundColor Red
    exit 1
}

function Passo($mensagem) {
    Write-Host "==> $mensagem" -ForegroundColor Cyan
}

# O TLS antigo é o padrão no PowerShell 5.1 e a API do GitHub recusa: sem esta
# linha o script morre com um erro de conexão que não explica nada.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Passo "Looking up the latest release of $repo"
try {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" `
        -Headers @{ 'User-Agent' = 'openweights-installer' }
} catch {
    Falha "could not reach the GitHub API: $($_.Exception.Message)"
}

$asset = $release.assets | Where-Object { $_.name -like '*-setup.exe' } | Select-Object -First 1
if (-not $asset) {
    Falha "release $($release.tag_name) has no Windows installer — see https://github.com/$repo/releases"
}

Passo "Latest is $($release.tag_name): $($asset.name)"

$destino = Join-Path $env:TEMP $asset.name
try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $destino -UseBasicParsing
} catch {
    Falha "download failed: $($_.Exception.Message)"
}

# Um instalador truncado falha de um jeito confuso ("arquivo inválido"), então
# vale conferir o tamanho contra o que o release declara.
$baixado = (Get-Item $destino).Length
if ($baixado -ne $asset.size) {
    Remove-Item $destino -Force -ErrorAction SilentlyContinue
    Falha "download is incomplete ($baixado of $($asset.size) bytes) — try again"
}

Passo 'Running the installer'
$argumentos = if ($Interactive) { @() } else { @('/S') }
$processo = Start-Process -FilePath $destino -ArgumentList $argumentos -Wait -PassThru
if ($processo.ExitCode -ne 0) {
    Falha "the installer exited with code $($processo.ExitCode) — run it by hand: $destino"
}

Remove-Item $destino -Force -ErrorAction SilentlyContinue

Passo 'Done. OpenWeights is in the Start menu.'
Write-Host 'On first launch the app downloads the llama.cpp runtime for your GPU (a few hundred MB).'
