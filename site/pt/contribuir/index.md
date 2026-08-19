# Compilar do código-fonte

Issues e pull requests são bem-vindos. Esta página é o que você precisa para
rodar o app a partir de um clone.

## Pré-requisitos

| Ferramenta | Versão | Para quê |
|---|---|---|
| [Node.js](https://nodejs.org/) | 22+ | frontend (React + Vite) |
| [Rust](https://rustup.rs/) | stable (1.85+) | núcleo do app (Tauri) |
| Ferramentas C++ | — | o linker do Rust em cada sistema |

### Windows

```powershell
winget install OpenJS.NodeJS.LTS

# Quando o instalador do Rust perguntar sobre o Visual Studio, ACEITE a
# instalação automática do "Visual Studio Build Tools" — é ela que traz o linker.
winget install Rustlang.Rustup
```

::: warning Abra um PowerShell novo depois
O PATH só atualiza em sessões novas. Confirme com `cargo --version`.
:::

Se o instalador do Rust não ofereceu as Build Tools:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

O PowerShell costuma bloquear o `npm.ps1` (*"a execução de scripts foi
desabilitada"*). Libere scripts locais só para o seu usuário, sem admin:

```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

*(Ou use `npm.cmd` no lugar de `npm` e não mude nada.)*

O WebView2, motor da interface, já vem com o Windows 10/11.

### macOS

```bash
xcode-select --install          # ferramentas de linha de comando (linker)
curl https://sh.rustup.rs -sSf | sh
```

### Linux

```bash
curl https://sh.rustup.rs -sSf | sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev build-essential \
  libayatana-appindicator3-dev librsvg2-dev
```

## Rodando

```bash
git clone https://github.com/pedro-canedo/openweights.git
cd openweights
npm install
npm run tauri dev
```

::: tip A primeira compilação é lenta
Ela compila ~500 crates Rust: de 5 a 15 minutos. As seguintes são incrementais
(segundos).
:::

## Comandos

| Comando | O que faz |
|---|---|
| `npm run tauri dev` | O app em modo de desenvolvimento, com hot reload |
| `npm run tauri build` | O instalador de produção |
| `npm run build` | Checagem de tipos + build só do frontend |
| `npm run dev` | Interface no navegador com dados simulados, sem Rust |
| `cd src-tauri && cargo test --workspace` | Testes Rust (~960 deles) |

## Antes de abrir um PR

Rode o que a CI roda:

```bash
npm run build
cd src-tauri
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Convenções

- **Comentários e mensagens de commit em português**, explicando o **porquê** —
  o que o código faz já está no código. Identificadores e nomes de teste em
  inglês.
- **Nomes de teste em inglês, como frases**:
  `a_cancelled_run_keeps_what_it_already_said`.
- **Toda mudança de comportamento vem com teste.** Um teste que não falha sem a
  correção não prova nada.
- **A interface é bilíngue**: chaves novas entram em `src/i18n/pt-BR.json` **e**
  `en.json`, sempre nos dois.
- **Este site também é bilíngue**: uma página em `site/` ganha a contraparte em
  `site/pt/`.

## O site de documentação

```bash
cd site
npm install
npm run dev
```

É um site VitePress; as páginas são Markdown. `npm run build` renderiza, e um
push na `main` publica.
