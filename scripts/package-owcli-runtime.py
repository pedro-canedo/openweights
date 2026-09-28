"""Compila, empacota e prova o runtime do OwCLI (fork do OpenAI Codex CLI).

Três subcomandos, na ordem em que o owcli-runtime.yml os chama — e que rodam
iguais fora da CI:

  build    compila a revisão pinada do fork (a pasta de trabalho do cargo é
           `codex-rs`, para valerem o rust-toolchain.toml e o .cargo/config.toml).
           No Linux o `bwrap` sai primeiro: o sha256 dos bytes finais dele vai
           para CODEX_BWRAP_SHA256 antes de o `codex` compilar, porque o codex
           confere o bwrap empacotado contra esse hash antes de usá-lo.
  package  monta o layout canônico do Codex (codex-package.json, bin/,
           codex-resources/, codex-path/) com o executável chamado `owcli` —
           é o nome que liga o modo OwCLI no ow-launch — e o `runtime.json`
           que o app confere; grava openweights-<tag>-<alvo>.tar.gz (.zip no
           Windows).
  smoke    extrai o pacote numa pasta nova e prova o que o app vai usar.

Uso:
  python scripts/package-owcli-runtime.py build   --source owcli --target linux-x64 --revision SHA --out bins
  python scripts/package-owcli-runtime.py package --source owcli --target linux-x64 --revision SHA --tag TAG --bins bins --output output
  python scripts/package-owcli-runtime.py smoke   --archive output/<pacote> --target linux-x64 --revision SHA --work "$RUNNER_TEMP/owcli-smoke" [--build-info bins/build.json] [--sandbox exigir]

Não publica nada e não mexe em runtime instalado.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.request
import zipfile
from dataclasses import dataclass

NOME_DO_RUNTIME = "owcli-runtime"
FORMATO_DO_RUNTIME = 1


@dataclass(frozen=True)
class Alvo:
    rust: str  # triple que o cargo compila
    dotslash: str  # plataforma no manifesto DotSlash do rg do fork
    ext: str

    @property
    def windows(self):
        return "windows" in self.rust

    @property
    def linux(self):
        return "linux" in self.rust

    @property
    def macos(self):
        return "apple-darwin" in self.rust

    @property
    def exe(self):
        return ".exe" if self.windows else ""


# Nomes do app → triple. Linux em musl, como o release do upstream: binário
# estático, sem depender da glibc nem da libssl da máquina.
ALVOS = {
    "linux-x64": Alvo("x86_64-unknown-linux-musl", "linux-x86_64", "tar.gz"),
    "win32-x64": Alvo("x86_64-pc-windows-msvc", "windows-x86_64", "zip"),
    "darwin-arm64": Alvo("aarch64-apple-darwin", "macos-aarch64", "tar.gz"),
    "darwin-x64": Alvo("x86_64-apple-darwin", "macos-x86_64", "tar.gz"),
}

# Os auxiliares do sandbox do Windows, que o codex acha em codex-resources/.
AUXILIARES_WINDOWS = ["codex-command-runner", "codex-windows-sandbox-setup"]


def alvo_de(args):
    alvo = ALVOS[args.target]
    if getattr(args, "rust_target", None):
        # Só para validar fora da CI (ex.: glibc local no lugar do musl).
        alvo = Alvo(args.rust_target, alvo.dotslash, alvo.ext)
    return alvo


def entrada(alvo):
    return f"bin/owcli{alvo.exe}"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for bloco in iter(lambda: f.read(1 << 20), b""):
            h.update(bloco)
    return h.hexdigest()


def rodar(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def cabeca_do_git(fonte):
    return subprocess.check_output(["git", "-C", str(fonte), "rev-parse", "HEAD"], text=True).strip()


def conferir_revisao(fonte, revisao):
    cabeca = cabeca_do_git(fonte)
    if cabeca != revisao:
        raise SystemExit(f"o fork em {fonte} está em {cabeca}, o pin é {revisao}")


def versao_do_workspace(fonte):
    """[workspace.package].version do codex-rs/Cargo.toml (a versão do upstream)."""
    dentro = False
    for linha in (fonte / "codex-rs" / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        s = linha.strip()
        if s == "[workspace.package]":
            dentro = True
        elif dentro and s.startswith("["):
            break
        elif dentro and s.startswith("version") and "=" in s:
            return s.split("=", 1)[1].strip().strip('"')
    raise SystemExit("versão do workspace não encontrada em codex-rs/Cargo.toml")


# ── build ─────────────────────────────────────────────────────────────────


def build(args):
    fonte = args.source.resolve()
    alvo = alvo_de(args)
    conferir_revisao(fonte, args.revision)
    rs = fonte / "codex-rs"
    env = dict(os.environ)
    env.update(
        # O commit que o binário carimba (build-info) e o que o `doctor` mostra.
        STABLE_GIT_COMMIT=args.revision,
        CODEX_BUILD_COMMIT=args.revision,
        # O perfil release do fork guarda line-tables (1,38 GB no Linux): sem
        # símbolos, o binário final é o que vai no pacote.
        CARGO_PROFILE_RELEASE_DEBUG="0",
        CARGO_PROFILE_RELEASE_STRIP="symbols",
        CARGO_NET_GIT_FETCH_WITH_CLI="true",
    )
    env.pop("CODEX_BWRAP_SHA256", None)
    if alvo.windows:
        env["LIBSQLITE3_FLAGS"] = "SQLITE_DISABLE_INTRINSIC"
    destino = Path(env.get("CARGO_TARGET_DIR") or rs / "target")
    if not destino.is_absolute():
        destino = rs / destino
    release = destino / alvo.rust / "release"
    saida = args.out.resolve()
    saida.mkdir(parents=True, exist_ok=True)
    # Sempre com --bin: sem ele o cargo compila o workspace inteiro, V8 junto.
    cargo = ["cargo", "build", "--locked", "--release", "--target", alvo.rust]
    info = {"revision": args.revision, "rustTarget": alvo.rust}

    if alvo.linux:
        rodar(cargo + ["--bin", "bwrap"], cwd=rs, env=env)
        bwrap = saida / "bwrap"
        shutil.copy2(release / "bwrap", bwrap)
        rodar(["strip", "--strip-debug", "--strip-unneeded", bwrap])
        # O hash é destes bytes (os que vão no pacote), antes do codex compilar.
        env["CODEX_BWRAP_SHA256"] = info["bwrapSha256"] = sha256(bwrap)
        print(f"bwrap sha256:{info['bwrapSha256']}", flush=True)

    bins = ["codex"] + (AUXILIARES_WINDOWS if alvo.windows else [])
    rodar(cargo + [a for b in bins for a in ("--bin", b)], cwd=rs, env=env)
    for b in bins:
        shutil.copy2(release / f"{b}{alvo.exe}", saida / f"{b}{alvo.exe}")
    if alvo.macos:
        # Assinatura ad-hoc (a política do app): o arm64 não roda binário sem
        # assinatura, e o strip do link invalida a do linker.
        rodar(["codesign", "--force", "--sign", "-", saida / "codex"])
    (saida / "build.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    for p in sorted(saida.iterdir()):
        print(f"{p.name}: {p.stat().st_size} bytes", flush=True)


# ── package ───────────────────────────────────────────────────────────────


def manifesto_dotslash(caminho):
    texto = caminho.read_text(encoding="utf-8")
    return json.loads("\n".join(l for l in texto.splitlines() if not l.startswith("#!")))


def baixar_rg(fonte, alvo, pasta):
    """O ripgrep que o próprio fork pina (scripts/codex_package/rg), conferido
    por tamanho e sha256. Devolve o executável e as licenças dele."""
    info = manifesto_dotslash(fonte / "scripts" / "codex_package" / "rg")["platforms"][alvo.dotslash]
    url = info["providers"][0]["url"]
    arquivo = pasta / url.rsplit("/", 1)[1]
    for tentativa in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as r, open(arquivo, "wb") as f:
                shutil.copyfileobj(r, f)
            break
        except OSError as e:
            if tentativa == 2:
                raise
            print(f"download do rg falhou ({e}); de novo", flush=True)
            time.sleep(5)
    tamanho = arquivo.stat().st_size
    if tamanho != info["size"] or sha256(arquivo) != info["digest"]:
        raise SystemExit(f"rg baixado não confere com o manifesto do fork ({url})")
    membro = info["path"]
    topo = membro.split("/", 1)[0]
    licencas = {}
    if info["format"] == "zip":
        with zipfile.ZipFile(arquivo) as z:
            exe = z.read(membro)
            for nome in ("COPYING", "LICENSE-MIT", "UNLICENSE"):
                if f"{topo}/{nome}" in z.namelist():
                    licencas[nome] = z.read(f"{topo}/{nome}")
    else:
        with tarfile.open(arquivo, "r:gz") as t:
            exe = t.extractfile(membro).read()
            for nome in ("COPYING", "LICENSE-MIT", "UNLICENSE"):
                try:
                    licencas[nome] = t.extractfile(f"{topo}/{nome}").read()
                except KeyError:
                    pass
    return exe, licencas


def escrever_json(caminho, valor):
    caminho.write_text(json.dumps(valor, indent=2) + "\n", encoding="utf-8")


def copiar_executavel(origem, destino):
    destino.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(origem, destino)
    destino.chmod(0o755)


def identidade(alvo_app, alvo, revisao, versao):
    # Os seis primeiros campos são o contrato que o app confere; o resto é
    # informativo.
    return {
        "name": NOME_DO_RUNTIME,
        "format": FORMATO_DO_RUNTIME,
        "revision": revisao,
        "upstreamTag": f"rust-v{versao}",
        "target": alvo_app,
        "entry": entrada(alvo),
        "rustTarget": alvo.rust,
        "codexVersion": versao,
    }


def package(args):
    fonte = args.source.resolve()
    alvo = alvo_de(args)
    conferir_revisao(fonte, args.revision)
    versao = versao_do_workspace(fonte)
    bins = args.bins.resolve()
    info = json.loads((bins / "build.json").read_text(encoding="utf-8"))
    if info["revision"] != args.revision or info["rustTarget"] != alvo.rust:
        raise SystemExit(f"os binários em {bins} são de {info}, não de {args.revision}/{alvo.rust}")
    saida = args.output.resolve()
    saida.mkdir(parents=True, exist_ok=True)
    nome = f"openweights-{args.tag}-{args.target}.{alvo.ext}"

    with tempfile.TemporaryDirectory(prefix="owcli-runtime-") as tmp:
        raiz = Path(tmp) / "runtime"
        for d in ("bin", "codex-resources", "codex-path", "licenses"):
            (raiz / d).mkdir(parents=True)
        copiar_executavel(bins / f"codex{alvo.exe}", raiz / entrada(alvo))
        if alvo.linux:
            bwrap = raiz / "codex-resources" / "bwrap"
            copiar_executavel(bins / "bwrap", bwrap)
            if sha256(bwrap) != info["bwrapSha256"]:
                raise SystemExit("o bwrap a empacotar não é o que o codex embutiu (CODEX_BWRAP_SHA256)")
            shutil.copyfile(fonte / "codex-rs" / "vendor" / "bubblewrap" / "COPYING",
                            raiz / "licenses" / "bubblewrap-COPYING")
        for aux in AUXILIARES_WINDOWS if alvo.windows else []:
            copiar_executavel(bins / f"{aux}.exe", raiz / "codex-resources" / f"{aux}.exe")
        rg, licencas = baixar_rg(fonte, alvo, Path(tmp))
        destino_rg = raiz / "codex-path" / f"rg{alvo.exe}"
        destino_rg.write_bytes(rg)
        destino_rg.chmod(0o755)
        for n, conteudo in licencas.items():
            (raiz / "licenses" / f"ripgrep-{n}").write_bytes(conteudo)
        for n in ("LICENSE", "NOTICE", "FORK.md"):
            shutil.copyfile(fonte / n, raiz / n)
        # O layout que o install-context do Codex reconhece: com ele o binário
        # acha o bwrap, os auxiliares e põe codex-path/ (o rg) no PATH.
        escrever_json(raiz / "codex-package.json", {
            "layoutVersion": 1,
            "version": versao,
            "target": alvo.rust,
            "variant": "codex",
            "entrypoint": entrada(alvo),
            "resourcesDir": "codex-resources",
            "pathDir": "codex-path",
        })
        escrever_json(raiz / "runtime.json", identidade(args.target, alvo, args.revision, versao))

        arquivo = saida / nome
        arquivos = sorted(p for p in raiz.rglob("*") if p.is_file())
        if alvo.ext == "zip":
            with zipfile.ZipFile(arquivo, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
                for p in arquivos:
                    z.write(p, p.relative_to(raiz).as_posix())
        else:
            def normalizar(ti):
                ti.uid = ti.gid = 0
                ti.uname = ti.gname = ""
                return ti

            with tarfile.open(arquivo, "w:gz", compresslevel=9) as t:
                for p in arquivos:
                    t.add(p, p.relative_to(raiz).as_posix(), recursive=False, filter=normalizar)
        for p in arquivos:
            print(f"  {p.relative_to(raiz).as_posix()}  {p.stat().st_size} bytes", flush=True)
    print(f"{arquivo}  {arquivo.stat().st_size} bytes  sha256:{sha256(arquivo)}", flush=True)


# ── smoke ─────────────────────────────────────────────────────────────────


class Falhou(Exception):
    pass


def exigir(condicao, mensagem, saida=None):
    if not condicao:
        if saida is not None:
            mensagem += f"\n--- código {saida.returncode}\n--- stdout:\n{saida.stdout[-3000:]}\n--- stderr:\n{saida.stderr[-3000:]}"
        raise Falhou(mensagem)


def porta_fechada():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def servidor_responses_falso(marcador):
    """Um /v1/responses mínimo em SSE: responde uma mensagem com o marcador e
    guarda o que recebeu (cabeçalho de autorização, modelo)."""
    recebidos = []

    class Tratador(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):
            pass

        def do_GET(self):
            recebidos.append({"metodo": "GET", "caminho": self.path})
            self._json(404, {"error": {"message": "not found"}})

        def do_POST(self):
            tamanho = int(self.headers.get("Content-Length") or 0)
            corpo = self.rfile.read(tamanho) if tamanho else b""
            try:
                # Sem compressão: o Codex só comprime (zstd) para o provedor
                # da OpenAI.
                pedido = json.loads(corpo or b"{}")
            except ValueError:
                pedido = {}
            recebidos.append({
                "metodo": "POST",
                "caminho": self.path,
                "autorizacao": self.headers.get("Authorization"),
                "modelo": pedido.get("model"),
            })
            if not self.path.rstrip("/").endswith("/responses"):
                self._json(404, {"error": {"message": "not found"}})
                return
            eventos = [
                {"type": "response.created", "response": {"id": "resp_fumaca"}},
                {"type": "response.output_item.done", "item": {
                    "type": "message", "role": "assistant", "id": "msg_fumaca",
                    "content": [{"type": "output_text", "text": marcador}]}},
                {"type": "response.completed", "response": {"id": "resp_fumaca", "usage": {
                    "input_tokens": 1, "input_tokens_details": None, "output_tokens": 1,
                    "output_tokens_details": None, "total_tokens": 2}}},
            ]
            corpo = "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in eventos).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(corpo)))
            self.end_headers()
            self.wfile.write(corpo)

        def _json(self, codigo, valor):
            corpo = json.dumps(valor).encode()
            self.send_response(codigo)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(corpo)))
            self.end_headers()
            self.wfile.write(corpo)

    servidor = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Tratador)
    threading.Thread(target=servidor.serve_forever, daemon=True).start()
    return servidor, recebidos


def conexao_do_app(base_url, token, modelo):
    """O openweights.json que o app escreve na casa do OwCLI (contrato v1)."""
    return {
        "versao": 1,
        "baseUrl": base_url,
        "token": token,
        "modelos": [{"id": modelo, "nome": "Fumaça", "janela": 32768, "esforcos": []}],
        "modeloPadrao": modelo,
    }


def _limpo(caminho):
    # O Codex canoniza caminhos com std::fs::canonicalize, que no Windows
    # devolve a forma verbatim (\\?\D:\...).
    s = str(caminho)
    if s.startswith("\\\\?\\UNC\\"):
        s = "\\\\" + s[8:]
    elif s.startswith("\\\\?\\"):
        s = s[4:]
    return os.path.normcase(os.path.realpath(s))


def dentro_de(caminho, pasta):
    if not caminho:
        return False
    c, p = _limpo(caminho), _limpo(pasta)
    return c == p or c.startswith(p.rstrip(os.sep) + os.sep)


def menciona(textos, pasta):
    p = _limpo(pasta)
    return any(p in os.path.normcase(t.replace("\\\\?\\", "")) for t in textos if isinstance(t, str))


def smoke(args):
    alvo = alvo_de(args)
    trabalho = args.work.resolve()
    # Em release, o Codex recusa criar os aliases do arg0 (codex-linux-sandbox,
    # apply_patch) numa casa dentro da pasta temporária do sistema — a fumaça
    # tem de rodar onde o app roda, fora dela.
    if dentro_de(trabalho, tempfile.gettempdir()):
        raise SystemExit(f"--work {trabalho} está dentro da pasta temporária ({tempfile.gettempdir()})")
    if trabalho.exists():
        shutil.rmtree(trabalho)
    raiz = trabalho / "pacote"
    raiz.mkdir(parents=True)
    arquivo = args.archive.resolve()
    if arquivo.name.endswith(".zip"):
        with zipfile.ZipFile(arquivo) as z:
            z.extractall(raiz)
    else:
        with tarfile.open(arquivo, "r:gz") as t:
            t.extractall(raiz, filter="data")
    ws = trabalho / "ws"
    ws.mkdir()
    exe = raiz / entrada(alvo)
    passos = []

    def passo(nome):
        def deco(f):
            passos.append((nome, f))
            return f
        return deco

    base_env = {k: v for k, v in os.environ.items()
                if k not in ("CODEX_HOME", "OWCLI_HOME", "OWCLI", "OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL")}
    base_env["NO_COLOR"] = "1"

    def owcli(argv, casa, *, programa=None, timeout=120, env_extra=None):
        env = dict(base_env)
        if casa is not None:
            env["OWCLI_HOME"] = str(casa)
        env.update(env_extra or {})
        return subprocess.run([str(programa or exe), *argv], cwd=ws, env=env, stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)

    def casa_nova(nome, conexao=None):
        casa = trabalho / nome
        casa.mkdir()
        if conexao is not None:
            escrever_json(casa / "openweights.json", conexao)
        return casa

    versao = None

    @passo("identidade: runtime.json e codex-package.json")
    def _():
        nonlocal versao
        # A versão vem do codex-package.json (é dele que o Codex lê a própria);
        # o `--version` abaixo confere contra a compilada no binário.
        cp = json.loads((raiz / "codex-package.json").read_text(encoding="utf-8"))
        versao = cp.get("version")
        rt = json.loads((raiz / "runtime.json").read_text(encoding="utf-8"))
        esperado = identidade(args.target, alvo, args.revision, versao)
        for campo in ("name", "format", "revision", "upstreamTag", "target", "entry"):
            exigir(rt.get(campo) == esperado[campo], f"runtime.json {campo}={rt.get(campo)!r}, esperado {esperado[campo]!r}")
        exigir(cp.get("entrypoint") == rt["entry"] and cp.get("layoutVersion") == 1
               and cp.get("resourcesDir") == "codex-resources" and cp.get("pathDir") == "codex-path",
               f"codex-package.json não bate com o runtime.json: {cp}")

    @passo("arquivos do layout")
    def _():
        exigidos = [entrada(alvo), f"codex-path/rg{alvo.exe}", "LICENSE", "NOTICE", "FORK.md"]
        if alvo.linux:
            exigidos.append("codex-resources/bwrap")
        if alvo.windows:
            exigidos += [f"codex-resources/{a}.exe" for a in AUXILIARES_WINDOWS]
        for rel in exigidos:
            p = raiz / rel
            exigir(p.is_file(), f"falta {rel} no pacote")
            if not alvo.windows and rel.startswith(("bin/", "codex-path/", "codex-resources/")):
                exigir(p.stat().st_mode & stat.S_IXUSR, f"{rel} sem permissão de execução")
        if args.build_info and alvo.linux:
            esperado = json.loads(args.build_info.read_text(encoding="utf-8"))["bwrapSha256"]
            exigir(sha256(raiz / "codex-resources" / "bwrap") == esperado,
                   "o bwrap extraído não tem o hash que o codex embutiu (CODEX_BWRAP_SHA256)")

    @passo("--version")
    def _():
        r = owcli(["--version"], casa_nova("casa-versao"))
        exigir(r.returncode == 0 and versao in r.stdout, f"--version não disse {versao}", r)
        print("   ", r.stdout.strip())

    @passo("sem o app (sem openweights.json): recusa do lançador, código 2")
    def _():
        r = owcli(["exec", "--skip-git-repo-check", "diga oi"], casa_nova("casa-sem-app"))
        exigir(r.returncode == 2 and "OwCLI runs on OpenWeights models" in r.stderr,
               "exec sem o app não saiu com a recusa do lançador", r)

    @passo("o nome decide o modo: owcli desliga o daemon, uma cópia `codex` não")
    def _():
        r = owcli(["features", "list"], casa_nova("casa-features"))
        linha = next((l for l in r.stdout.splitlines() if l.startswith("daemon_auto_start")), "")
        exigir(r.returncode == 0 and linha.split()[-1:] == ["false"], "owcli features list: daemon_auto_start não está false", r)
        copia = trabalho / "controle" / f"codex{alvo.exe}"
        copia.parent.mkdir()
        shutil.copy2(exe, copia)
        r = owcli(["features", "list"], None, programa=copia, env_extra={"CODEX_HOME": str(casa_nova("casa-codex"))})
        linha = next((l for l in r.stdout.splitlines() if l.startswith("daemon_auto_start")), "")
        exigir(r.returncode == 0 and linha.split()[-1:] == ["true"], "a cópia `codex` (modo upstream) não mostrou daemon_auto_start true", r)
        copia.unlink()

    token = "tok-fumaca-" + os.urandom(6).hex()
    modelo = "local:Fumaca-1B"

    @passo("app fechado: --ow-token lê o arquivo; exec recusa com código 2")
    def _():
        casa = casa_nova("casa-app-fechado",
                         conexao_do_app(f"http://127.0.0.1:{porta_fechada()}/owcli/v1", token, modelo))
        r = owcli(["--ow-token"], casa)
        exigir(r.returncode == 0 and r.stdout.strip() == token, "--ow-token não devolveu o token do openweights.json", r)
        r = owcli(["exec", "--skip-git-repo-check", "diga oi"], casa)
        exigir(r.returncode == 2 and "OpenWeights is not answering" in r.stderr,
               "exec com o app fechado não saiu com a recusa do lançador", r)

    @passo("doctor: layout do pacote, rg empacotado e commit")
    def _():
        casa = casa_nova("casa-doctor",
                         conexao_do_app(f"http://127.0.0.1:{porta_fechada()}/owcli/v1", token, modelo))
        r = owcli(["doctor", "--json"], casa, timeout=240)
        try:
            checks = json.loads(r.stdout)["checks"]
        except (ValueError, KeyError):
            raise Falhou("doctor --json não devolveu JSON") from None
        rg = checks["runtime.search"]["details"]
        exigir(rg.get("search provider") == "bundled" and dentro_de(rg.get("search command", ""), raiz / "codex-path"),
               f"o rg não é o do pacote: {rg}")
        instalacao = checks["installation"]["details"]
        exigir(menciona(instalacao.values(), raiz), f"o codex não reconheceu o layout do pacote: {instalacao}")
        prov = checks["runtime.provenance"]["details"]
        exigir(prov.get("commit") == args.revision and prov.get("version") == versao,
               f"proveniência inesperada: {prov}")

    @passo("conversa com um /v1/responses falso (token pelo auth.command, modelo do catálogo)")
    def _():
        marcador = "fumaca-ok-" + os.urandom(4).hex()
        servidor, recebidos = servidor_responses_falso(marcador)
        try:
            porta = servidor.server_address[1]
            casa = casa_nova("casa-conversa", conexao_do_app(f"http://127.0.0.1:{porta}/owcli/v1", token, modelo))
            ultima = trabalho / "ultima-mensagem.txt"
            r = owcli(["exec", "--skip-git-repo-check", "--ephemeral", "-o", str(ultima), "diga oi"], casa, timeout=180)
        finally:
            servidor.shutdown()
        exigir(r.returncode == 0, f"exec contra o servidor falso falhou; recebidos={recebidos}", r)
        exigir(ultima.is_file() and marcador in ultima.read_text(encoding="utf-8", errors="replace"),
               f"a resposta do modelo não chegou; recebidos={recebidos}", r)
        posts = [x for x in recebidos if x["metodo"] == "POST" and x["caminho"].rstrip("/").endswith("/owcli/v1/responses")]
        exigir(posts, f"nenhum POST em /owcli/v1/responses: {recebidos}")
        exigir(all(x["autorizacao"] == f"Bearer {token}" for x in posts), f"token errado no pedido: {posts}")
        exigir(all(x["modelo"] == modelo for x in posts), f"modelo errado no pedido: {posts}")
        print(f"    {len(posts)} pedido(s) em /owcli/v1/responses, modelo {posts[0]['modelo']}")

    SANDBOX = "sandbox :workspace — o rg do pacote no PATH, escreve só no workspace"
    if args.sandbox:
        @passo(SANDBOX)
        def _():
            casa = casa_nova("casa-sandbox")
            env_extra = {}
            if alvo.linux:
                # O Codex prefere um bwrap do sistema que esteja no PATH; o do
                # pacote é o que vale para quem não tem um. Sem bwrap no PATH,
                # é o empacotado que roda — e os comandos vão com caminho
                # absoluto, porque /usr/bin sai do PATH junto.
                env_extra["PATH"] = os.pathsep.join(
                    d for d in base_env.get("PATH", "").split(os.pathsep)
                    if d and not (Path(d) / "bwrap").exists())

            def no_sandbox(comando):
                return owcli(["sandbox", "-P", ":workspace", "-C", str(ws), "--", *comando], casa,
                             timeout=180, env_extra=env_extra)

            if alvo.windows:
                achar = ["cmd", "/d", "/c", "where", "rg"]
                escrever = lambda p: ["cmd", "/d", "/c", "type", "nul", ">", str(p)]  # noqa: E731
            else:
                achar = ["/bin/sh", "-c", "command -v rg"]
                escrever = lambda p: ["/usr/bin/touch", str(p)]  # noqa: E731
            r = no_sandbox(achar)
            achado = next((l.strip() for l in r.stdout.splitlines() if l.strip()), "")
            exigir(r.returncode == 0 and dentro_de(achado, raiz / "codex-path"),
                   f"no sandbox, o rg achado foi {achado!r}", r)
            dentro, fora = ws / "dentro.txt", trabalho / "fora.txt"
            r = no_sandbox(escrever(dentro))
            exigir(r.returncode == 0 and dentro.is_file(), "o sandbox não deixou escrever no workspace", r)
            r = no_sandbox(escrever(fora))
            exigir(not fora.exists(), "o sandbox deixou escrever fora do workspace", r)
            if alvo.linux:
                # O hash que o codex embutiu é o do bwrap empacotado: com um
                # byte a mais, ele se recusa a usá-lo (código 8 do linux-sandbox).
                bwrap = raiz / "codex-resources" / "bwrap"
                original = bwrap.read_bytes()
                try:
                    bwrap.write_bytes(original + b"\0")
                    r = no_sandbox(achar)
                finally:
                    bwrap.write_bytes(original)
                exigir(r.returncode != 0 and "bundled bubblewrap digest mismatch" in r.stderr,
                       "o codex aceitou um bwrap adulterado (o hash embutido não é o do pacote?)", r)

    falhas = 0
    for nome, f in passos:
        inicio = time.monotonic()
        try:
            f()
            print(f"ok   {nome} ({time.monotonic() - inicio:.1f}s)", flush=True)
        except (Falhou, subprocess.TimeoutExpired, OSError) as e:
            if nome == SANDBOX and args.sandbox == "tentar":
                # O sandbox do Windows depende do que a conta do runner pode;
                # aqui ele avisa, não barra a publicação.
                print(f"::warning::fumaça do sandbox falhou (não barra): {e}".replace("\n", " | "), flush=True)
                continue
            falhas += 1
            print(f"FALHOU {nome}: {e}", flush=True)
    if falhas:
        raise SystemExit(f"{falhas} passo(s) da fumaça falharam")
    print(f"fumaça ok: {arquivo.name}", flush=True)


def main(argv):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="comando", required=True)

    def comum(s, fonte=True):
        s.add_argument("--target", required=True, choices=sorted(ALVOS))
        s.add_argument("--revision", required=True)
        s.add_argument("--rust-target", help="troca o triple (só para validar fora da CI)")
        if fonte:
            s.add_argument("--source", type=Path, required=True, help="o clone do fork (a raiz, não codex-rs)")

    b = sub.add_parser("build")
    comum(b)
    b.add_argument("--out", type=Path, required=True)
    k = sub.add_parser("package")
    comum(k)
    k.add_argument("--tag", required=True)
    k.add_argument("--bins", type=Path, required=True)
    k.add_argument("--output", type=Path, required=True)
    s = sub.add_parser("smoke")
    comum(s, fonte=False)
    s.add_argument("--archive", type=Path, required=True)
    s.add_argument("--work", type=Path, required=True, help="pasta nova fora da pasta temporária do sistema")
    s.add_argument("--build-info", type=Path, help="o build.json do `build` (o hash do bwrap que o codex embutiu)")
    s.add_argument("--sandbox", choices=["exigir", "tentar"],
                   help="também roda comandos no sandbox do Codex; `tentar` só avisa se falhar")
    args = p.parse_args(argv)
    {"build": build, "package": package, "smoke": smoke}[args.comando](args)


if __name__ == "__main__":
    main(sys.argv[1:])
