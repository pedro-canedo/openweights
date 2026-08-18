#!/usr/bin/env sh
# Instalador do OpenWeights para macOS e Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.sh | sh
#
# Escrito em `sh` puro de propósito: é o único interpretador garantido nas duas
# plataformas, e um instalador que exige instalar algo antes não é instalador.
# Sem `jq` pelo mesmo motivo — a lista de arquivos do release sai do JSON com
# `grep`, que basta para linhas de URL.
#
# O que ele NÃO faz: escolher por você. Se algo der errado, ele diz onde parou
# e qual é o passo manual — falhar em silêncio num script que baixa binário e
# copia para `/Applications` seria a pior combinação possível.
set -eu

REPO="pedro-canedo/openweights"
API="https://api.github.com/repos/$REPO/releases/latest"

erro() {
    echo "error: $*" >&2
    exit 1
}

aviso() { echo "==> $*"; }

# `curl` ou `wget`, o que existir.
baixar() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        erro "neither curl nor wget is available"
    fi
}

texto() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$1"
    else
        erro "neither curl nor wget is available"
    fi
}

# A URL do primeiro arquivo do último release cujo nome termina em $1.
url_do_release() {
    echo "$RELEASE" |
        grep -o '"browser_download_url": *"[^"]*"' |
        sed 's/.*"\(https[^"]*\)"/\1/' |
        grep -- "$1\$" |
        head -n 1
}

instala_macos() {
    url=$(url_do_release ".dmg")
    [ -n "$url" ] || erro "this release has no .dmg — see https://github.com/$REPO/releases"

    dmg="$TMP/openweights.dmg"
    aviso "Downloading $(basename "$url")"
    baixar "$url" "$dmg"

    aviso "Mounting the image"
    ponto="$TMP/mnt"
    mkdir -p "$ponto"
    hdiutil attach -nobrowse -quiet -mountpoint "$ponto" "$dmg" ||
        erro "could not mount the .dmg"

    app=$(find "$ponto" -maxdepth 1 -name '*.app' | head -n 1)
    if [ -z "$app" ]; then
        hdiutil detach -quiet "$ponto" || true
        erro "no .app inside the image"
    fi

    destino="/Applications/$(basename "$app")"
    aviso "Copying to $destino"
    rm -rf "$destino"
    cp -R "$app" "$destino" || {
        hdiutil detach -quiet "$ponto" || true
        erro "could not write to /Applications — run with sudo, or drag the app yourself"
    }
    hdiutil detach -quiet "$ponto" || true

    # O binário não é assinado: sem tirar a quarentena, o Gatekeeper recusa
    # abrir e a mensagem que ele mostra ("damaged") faz pensar em download
    # corrompido, que não é o caso.
    xattr -cr "$destino" 2>/dev/null || true

    aviso "Done. Open it from Launchpad, or run: open '$destino'"
}

instala_linux() {
    arq=$(uname -m)
    [ "$arq" = "x86_64" ] ||
        erro "only x86_64 has a Linux build today (this machine is $arq) — build from source: https://github.com/$REPO"

    # `.deb` quando dá, porque ele resolve as dependências (webkit2gtk) e põe o
    # app no menu; AppImage é o plano B para quem não usa apt.
    if command -v dpkg >/dev/null 2>&1 && command -v sudo >/dev/null 2>&1; then
        url=$(url_do_release ".deb")
        if [ -n "$url" ]; then
            deb="$TMP/openweights.deb"
            aviso "Downloading $(basename "$url")"
            baixar "$url" "$deb"
            aviso "Installing (sudo apt install)"
            sudo apt-get install -y "$deb" ||
                erro "apt refused the package — try: sudo dpkg -i $deb && sudo apt-get -f install"
            aviso "Done. Look for OpenWeights in your app menu, or run: openweights"
            return
        fi
    fi

    url=$(url_do_release ".AppImage")
    [ -n "$url" ] || erro "this release has no Linux build — see https://github.com/$REPO/releases"

    destino="$HOME/.local/bin"
    mkdir -p "$destino"
    alvo="$destino/OpenWeights.AppImage"
    aviso "Downloading $(basename "$url")"
    baixar "$url" "$alvo"
    chmod +x "$alvo"

    # Sem o atalho o AppImage não aparece no menu, e "instalado" viraria "está
    # num arquivo em algum lugar".
    atalhos="$HOME/.local/share/applications"
    mkdir -p "$atalhos"
    cat >"$atalhos/openweights.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=OpenWeights
Comment=Run local LLMs with an agent
Exec=$alvo
Terminal=false
Categories=Development;Utility;
EOF

    aviso "Done: $alvo"
    case ":$PATH:" in
    *":$destino:"*) ;;
    *) aviso "Note: $destino is not in your PATH — add it to run 'OpenWeights.AppImage' by name" ;;
    esac
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

aviso "Looking up the latest release of $REPO"
RELEASE=$(texto "$API") || erro "could not reach the GitHub API"
VERSAO=$(echo "$RELEASE" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
[ -n "$VERSAO" ] || erro "no published release yet — see https://github.com/$REPO/releases"
aviso "Latest is $VERSAO"

case "$(uname -s)" in
Darwin) instala_macos ;;
Linux) instala_linux ;;
*) erro "unsupported system: $(uname -s) — on Windows use scripts/install.ps1" ;;
esac

aviso "On first launch the app downloads the llama.cpp runtime for your GPU (a few hundred MB)."
