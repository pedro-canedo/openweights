//! Verificação funcional do motor instalado.
//!
//! O app não usa o llama.cpp do sistema: ele instala o seu próprio, numa
//! pasta sob os dados do aplicativo, numa build homologada
//! ([`crate::PINNED_TAG`]). Isso isola o app de uma instalação alheia — e
//! cria a pergunta que esta camada responde: *o motor que está ali serve?*
//!
//! "Serve" tem três partes, e nenhuma delas se deduz da outra:
//!
//! 1. **Está a build que esta versão do app espera?** Quem atualiza o app
//!    ganha uma tag nova; a antiga continua no disco, ocupando gigabytes, e
//!    o app não a usa mais.
//! 2. **É a variante certa para esta máquina, hoje?** Trocar de placa ou
//!    atualizar o driver muda a escolha (Vulkan → CUDA 12 → CUDA 13). O
//!    pacote antigo continua lá, e continua rodando — devagar.
//! 3. **Ele EXECUTA?** Esta é a parte que só um processo de verdade
//!    responde. Um pacote CUDA sem as DLLs do cudart extraídas ao lado do
//!    executável passa em qualquer checagem de arquivo e falha na primeira
//!    carga de modelo. Por isso a verificação roda `llama-server --version`
//!    e lê a build que o próprio binário reporta.
//!
//! A quarta pergunta — "existe build mais nova lá fora?" — é respondida por
//! gentileza, nunca como cobrança: a tag é promovida à mão depois de teste de
//! contrato, então uma release mais nova no GitHub é informação, não pendência.

use crate::{BackendVariant, PINNED_TAG, rpc_exe_name, runtime_dir, server_exe_name};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Teto do `--version`. O binário carrega os backends do ggml antes de
/// imprimir — em placa fria, com CUDA, isso leva alguns segundos.
const PROBE_TIMEOUT: Duration = Duration::from_secs(25);

/// Teto da consulta ao GitHub. É informação secundária: se a rede estiver
/// ruim, a verificação local não pode ficar esperando por ela.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(6);

/// Um pacote do motor encontrado no disco.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    /// A tag da release (`b10441`), lida do nome da pasta.
    pub tag: String,
    /// A variante, quando o nome da pasta é uma que conhecemos.
    pub variant: Option<BackendVariant>,
    /// O nome da pasta da variante, sempre — inclusive o que não
    /// reconhecemos, que é justamente o que precisa aparecer na tela.
    pub variant_dir: String,
    pub dir: PathBuf,
    pub size_bytes: u64,
    pub has_server: bool,
    pub has_rpc: bool,
}

/// O que a verificação concluiu. A tela traduz cada caso; o backend não
/// escreve frase para humano.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// O pacote esperado está instalado e executou.
    Ready,
    /// Não há motor nenhum: primeira execução, ou a pasta foi apagada.
    NotInstalled,
    /// Há um motor de outra build; o desta versão do app ainda não foi
    /// baixado.
    UpdateAvailable,
    /// A build é a certa, mas para outra variante — a máquina mudou
    /// (placa nova, driver novo) desde a instalação.
    VariantChanged,
    /// Os arquivos estão lá e o executável não roda. É o caso que só a
    /// execução revela.
    Broken,
}

/// O relatório inteiro — o que a tela de Ajustes mostra no card do motor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCheck {
    /// A build homologada para esta versão do app.
    pub expected_tag: String,
    /// A variante que o hardware de hoje pede.
    pub expected_variant: BackendVariant,
    pub verdict: Verdict,
    /// A causa técnica, quando existe: a mensagem do sistema operacional, a
    /// saída do binário que não deu para interpretar. Vai para o `title` de
    /// quem quiser ler; a tela não depende dela.
    pub detail: Option<String>,
    /// A build que o próprio executável reportou (`10441`), quando rodou.
    pub reported_build: Option<u64>,
    /// Quanto tempo o `--version` levou. Serve de sinal: 12 s para imprimir
    /// uma linha é uma placa em estado ruim.
    pub probe_ms: Option<u64>,
    /// O pacote que o app usaria agora, se houver.
    pub active: Option<InstalledRuntime>,
    /// O motor da PrismML desta versão do app, quando instalado. Não é
    /// "outro": é o que os modelos Bonsai usam, e a limpeza não o apaga.
    pub prism: Option<InstalledRuntime>,
    /// Os outros pacotes no disco — builds antigas, variantes que sobraram
    /// (inclusive versões antigas do fork).
    pub others: Vec<InstalledRuntime>,
    /// Soma de `others`: o que uma limpeza devolveria ao disco.
    pub reclaimable_bytes: u64,
    /// A última release do llama.cpp, quando a consulta funcionou.
    pub upstream_tag: Option<String>,
    /// Ela é mais nova que a nossa homologada.
    pub upstream_newer: bool,
}

/// `b10441` → `10441`. O que não tem essa forma não é tag de release
/// OFICIAL — a tag do fork da PrismML fica de fora de propósito: quem compara
/// com a homologada e com a release lá fora só quer o oficial.
pub fn tag_number(tag: &str) -> Option<u64> {
    tag.strip_prefix('b')?.parse().ok()
}

/// A build por trás de qualquer tag que o app instala: `b10441` → `10441` e
/// `prism-b10709-9a9394a` → `10709`. É o que o binário reporta em
/// `--version` nos dois casos, porque o fork carrega o número do upstream.
pub fn build_number(tag: &str) -> Option<u64> {
    if let Some(n) = tag_number(tag) {
        return Some(n);
    }
    let resto = tag.strip_prefix("prism-b")?;
    let (numero, _sha) = resto.split_once('-')?;
    numero.parse().ok()
}

/// A pasta é do fork da PrismML.
pub fn is_prism_tag(tag: &str) -> bool {
    tag.starts_with("prism-b")
}

/// Percorre `<data_dir>/runtimes/` e devolve o que está instalado.
///
/// Lê o disco, não um manifesto: manifesto tem como divergir do que existe,
/// e é exatamente aqui que a divergência apareceria.
pub fn scan_installed(data_dir: &Path) -> Vec<InstalledRuntime> {
    let raiz = data_dir.join("runtimes");
    let Ok(tags) = std::fs::read_dir(&raiz) else {
        return Vec::new();
    };
    let mut encontrados = Vec::new();
    for tag_entry in tags.flatten() {
        if !tag_entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let tag = tag_entry.file_name().to_string_lossy().into_owned();
        // Só pastas de release (oficial ou do fork): a sessão de download
        // também mora aqui.
        if build_number(&tag).is_none() {
            continue;
        }
        let Ok(variantes) = std::fs::read_dir(tag_entry.path()) else {
            continue;
        };
        for var_entry in variantes.flatten() {
            if !var_entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let dir = var_entry.path();
            let variant_dir = var_entry.file_name().to_string_lossy().into_owned();
            encontrados.push(InstalledRuntime {
                has_server: dir.join(server_exe_name()).is_file(),
                has_rpc: dir.join(rpc_exe_name()).is_file(),
                size_bytes: tamanho_recursivo(&dir),
                variant: variante_do_nome(&variant_dir),
                tag: tag.clone(),
                variant_dir,
                dir,
            });
        }
    }
    // Mais nova primeiro: é a ordem em que uma lista de versões se lê.
    encontrados.sort_by(|a, b| {
        build_number(&b.tag)
            .cmp(&build_number(&a.tag))
            .then_with(|| a.variant_dir.cmp(&b.variant_dir))
    });
    encontrados
}

/// A variante de um pacote instalado, pela pasta dele — é como se sabe em
/// que variante um processo de pé está rodando (a pasta do executável).
pub fn variant_of_dir(dir: &Path) -> Option<BackendVariant> {
    variante_do_nome(dir.file_name()?.to_str()?)
}

/// O caminho inverso de [`crate::runtime_dir`]: nome da pasta → variante.
fn variante_do_nome(nome: &str) -> Option<BackendVariant> {
    Some(match nome {
        "cuda-13.3" => BackendVariant::Cuda13,
        "cuda-12.4" => BackendVariant::Cuda12,
        "cuda-12.8" => BackendVariant::Cuda128,
        "vulkan" => BackendVariant::Vulkan,
        "cpu" => BackendVariant::Cpu,
        "macos-arm64" => BackendVariant::MacosArm64,
        "macos-x64" => BackendVariant::MacosX64,
        _ => return None,
    })
}

fn tamanho_recursivo(dir: &Path) -> u64 {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return 0;
    };
    entradas
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tamanho_recursivo(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// A build que o binário reporta, extraída da saída do `--version`.
///
/// A saída de verdade (b10441, Windows) é:
///
/// ```text
/// version: 0.1.0-dev (build 10441, commit 0177dcc73)
/// built with Clang 20.1.8 for Windows x86_64
/// ```
///
/// O número que importa é o do `build`, não o que vem depois de `version:` —
/// ali mora um `0.1.0-dev` que o projeto não incrementa. Ler o primeiro
/// número depois de `version:` devolvia **0** para todo mundo, e o card do
/// motor acusava "instalado, mas não executa" em máquina com motor perfeito.
/// Daí a ordem: `build` primeiro; `version:` só vale quando o que vem depois
/// dele é a build inteira (formato antigo, `version: 10441 (hash)`).
///
/// Sai em `stderr` numa build e em `stdout` noutra, daí a busca nas duas.
pub fn parse_reported_build(saida: &str) -> Option<u64> {
    let baixo = saida.to_ascii_lowercase();

    // `build 10441`, `build: 10441`, `build = 10441`.
    for (i, _) in baixo.match_indices("build") {
        let resto = baixo[i + "build".len()..].trim_start_matches([' ', ':', '=']);
        let numero: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = numero.parse::<u64>() {
            return Some(n);
        }
    }

    // Formato antigo: `version: 10441 (hash)`. Um `version: 0.1.0-dev` não
    // conta — número seguido de ponto é versão semântica, não build.
    for (i, _) in baixo.match_indices("version:") {
        let resto = baixo[i + "version:".len()..].trim_start();
        let numero: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !resto[numero.len()..].starts_with('.')
            && let Ok(n) = numero.parse::<u64>()
        {
            return Some(n);
        }
    }
    None
}

/// Por que a execução do motor não provou nada.
///
/// A distinção decide o destino do pacote: o que é do MOMENTO (a sonda não
/// respondeu a tempo, o sistema não criou o processo, a placa ocupada) não
/// pode virar "esta variante não serve nesta máquina" — no motor da PrismML
/// isso grava um marcador que prende a máquina na reserva até o driver
/// mudar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FalhaDaSonda {
    /// Não deu para saber agora. Tentar de novo depois.
    Transitoria(String),
    /// O binário rodou (ou o carregador o recusou) e não provou o que devia.
    Recusou(String),
}

impl std::fmt::Display for FalhaDaSonda {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transitoria(m) | Self::Recusou(m) => f.write_str(m),
        }
    }
}

/// Executa o motor instalado e lê a build que ele reporta.
///
/// O `current_dir` é a pasta do pacote de propósito: as DLLs do CUDA moram ao
/// lado do executável, e é essa vizinhança que o teste precisa exercitar.
pub(crate) async fn probe_build(dir: &Path) -> Result<(u64, u64), FalhaDaSonda> {
    let exe = dir.join(server_exe_name());
    if !exe.is_file() {
        return Err(FalhaDaSonda::Recusou(format!(
            "{} não está em {}",
            server_exe_name(),
            dir.display()
        )));
    }
    let inicio = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(dir)
        .kill_on_drop(true);
    lr_proc::no_window(&mut cmd);
    // O pacote que traz o próprio CUDA runtime é provado com as bibliotecas
    // DELE na frente do `LD_LIBRARY_PATH` (o `host_env` faz isso) — é assim
    // que o motor vai subir.
    lr_proc::host_env(&mut cmd);
    // Estourar o tempo ou não conseguir criar o processo (EAGAIN, ENOMEM)
    // não prova nada sobre o pacote: é o momento, não a máquina.
    let saida = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            FalhaDaSonda::Transitoria(format!(
                "{} não respondeu em {PROBE_TIMEOUT:?}",
                server_exe_name()
            ))
        })?
        .map_err(|e| FalhaDaSonda::Transitoria(e.to_string()))?;
    let ms = inicio.elapsed().as_millis() as u64;

    let texto = format!(
        "{}{}",
        String::from_utf8_lossy(&saida.stdout),
        String::from_utf8_lossy(&saida.stderr)
    );
    // O código de saída não decide: o que prova que o motor roda é ele ter
    // conseguido carregar os backends e imprimir a própria build.
    match parse_reported_build(&texto) {
        Some(build) => Ok((build, ms)),
        None if morto_de_fora(&saida.status) => Err(FalhaDaSonda::Transitoria(format!(
            "{} foi encerrado antes de responder ({})",
            server_exe_name(),
            saida.status
        ))),
        None => Err(FalhaDaSonda::Recusou(
            primeira_linha_util(&texto)
                .unwrap_or_else(|| format!("{} não reportou versão nenhuma", server_exe_name())),
        )),
    }
}

/// O processo morreu por um sinal vindo de FORA (o OOM killer, um `kill`,
/// o desligamento): não disse nada sobre o pacote. Um SIGSEGV, SIGILL ou
/// SIGABRT é o próprio binário falhando — isso conta.
fn morto_de_fora(status: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        const SIGINT: i32 = 2;
        const SIGKILL: i32 = 9;
        const SIGTERM: i32 = 15;
        status
            .signal()
            .is_some_and(|s| matches!(s, SIGINT | SIGKILL | SIGTERM))
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        false
    }
}

/// O que o `llama-server --list-devices` disse sobre a CUDA.
///
/// O `--version` não inicializa a CUDA (0,36 s, sem `ggml_cuda_init`): um
/// pacote CUDA sem placa utilizável passa nele e cai na CPU sem avisar. Só
/// a lista de dispositivos prova que o backend subiu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DispositivosCuda {
    /// Há pelo menos uma linha `CUDA<n>:` — a primeira vem junto.
    Presente(String),
    /// Não deu para saber AGORA (ver [`CUDA_DO_MOMENTO`]): a placa sem
    /// memória para o contexto, ocupada em modo exclusivo, a CUDA num estado
    /// ruim depois de uma suspensão, a sonda estourando o tempo ou o sistema
    /// sem criar o processo. Não diz nada sobre a máquina — tentar de novo.
    Transitoria(String),
    /// A CUDA não subiu por um motivo que não é do momento: sem dispositivo,
    /// biblioteca ausente, driver que não serve, ou algo que não se
    /// reconhece. A tela oferece tentar de novo (ver o marcador em
    /// `prism.rs`), porque "não reconheço" não é prova de nada.
    Ausente(String),
}

/// Os erros da CUDA (`cudaGetErrorString`) que dizem "agora não", não "esta
/// máquina não serve", em minúsculas:
///
/// - `out of memory` — a placa cheia (um jogo, o modelo que o próprio app
///   deixou carregado) não tem espaço nem para o contexto da sonda;
/// - `unknown error` — o `cudaErrorUnknown` de depois de uma suspensão no
///   Linux, até o `nvidia_uvm` ser recarregado ou a máquina reiniciar (o
///   NVML segue respondendo, então a preferida continua sendo o CUDA);
/// - `busy or unavailable` — a placa em `Exclusive_Process` com outro
///   processo CUDA de pé;
/// - `not yet initialized` — o `cudaErrorSystemNotReady` do fabric manager
///   ainda subindo.
const CUDA_DO_MOMENTO: [&str; 4] = [
    "out of memory",
    "unknown error",
    "busy or unavailable",
    "not yet initialized",
];

/// Lê a saída do `--list-devices`. As formas reais (prism-b10709, RTX 3090):
///
/// ```text
/// Available devices:
///   CUDA0: NVIDIA GeForce RTX 3090 (24173 MiB, 22820 MiB free)
/// ```
///
/// e, sem placa (`CUDA_VISIBLE_DEVICES=-1`, código de saída 0):
///
/// ```text
/// E ggml_cuda_init: failed to initialize CUDA: no CUDA-capable device is detected
/// Available devices:
///   (none)
/// ```
pub(crate) fn ler_dispositivos_cuda(saida: &str) -> DispositivosCuda {
    let dispositivo = saida.lines().map(str::trim).find(|l| {
        l.strip_prefix("CUDA").is_some_and(|resto| {
            let digitos = resto.chars().take_while(char::is_ascii_digit).count();
            digitos > 0 && resto[digitos..].starts_with(':')
        })
    });
    if let Some(linha) = dispositivo {
        return DispositivosCuda::Presente(linha.chars().take(300).collect());
    }
    // A causa mais útil é a linha do erro, não o "Available devices:".
    let causa = saida
        .lines()
        .map(str::trim)
        .find(|l| {
            let b = l.to_ascii_lowercase();
            b.contains("error") || b.contains("failed") || b.contains("cannot")
        })
        .map(|l| l.chars().take(300).collect::<String>())
        .or_else(|| primeira_linha_util(saida))
        .unwrap_or_else(|| "o motor não listou nenhum dispositivo CUDA".to_string());
    let baixo = saida.to_ascii_lowercase();
    if CUDA_DO_MOMENTO.iter().any(|m| baixo.contains(m)) {
        DispositivosCuda::Transitoria(causa)
    } else {
        DispositivosCuda::Ausente(causa)
    }
}

/// Roda `llama-server --list-devices` na pasta do pacote, com o mesmo
/// ambiente do motor (sistema + bibliotecas do pacote na frente, pelo
/// `host_env`), e lê o que ele achou. Estourar o tempo, não conseguir criar
/// o processo ou vê-lo morto de fora contam como transitórios: não provam
/// que a máquina não serve.
pub(crate) async fn probe_cuda_devices(dir: &Path) -> DispositivosCuda {
    let exe = dir.join(server_exe_name());
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("--list-devices")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(dir)
        .kill_on_drop(true);
    lr_proc::no_window(&mut cmd);
    lr_proc::host_env(&mut cmd);
    match tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await {
        Err(_) => DispositivosCuda::Transitoria(format!(
            "{} --list-devices não respondeu em {PROBE_TIMEOUT:?}",
            server_exe_name()
        )),
        Ok(Err(e)) => DispositivosCuda::Transitoria(format!(
            "{} --list-devices não abriu: {e}",
            server_exe_name()
        )),
        Ok(Ok(saida)) => {
            let texto = format!(
                "{}{}",
                String::from_utf8_lossy(&saida.stdout),
                String::from_utf8_lossy(&saida.stderr)
            );
            match ler_dispositivos_cuda(&texto) {
                DispositivosCuda::Ausente(_) if morto_de_fora(&saida.status) => {
                    DispositivosCuda::Transitoria(format!(
                        "{} --list-devices foi encerrado antes de responder ({})",
                        server_exe_name(),
                        saida.status
                    ))
                }
                lido => lido,
            }
        }
    }
}

/// A primeira linha com conteúdo, cortada — é o que cabe num `title`.
fn primeira_linha_util(texto: &str) -> Option<String> {
    let linha = texto.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(linha.chars().take(300).collect())
}

/// A última release do llama.cpp no GitHub. `None` em qualquer falha — isto
/// é enfeite, não requisito.
async fn latest_upstream() -> Option<String> {
    // A API do GitHub recusa quem não se identifica; o cliente do `lr_fetch`
    // já nasce com User-Agent.
    let http = lr_fetch::client(concat!("OpenWeights/", env!("CARGO_PKG_VERSION"))).ok()?;
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let resp = http
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(UPSTREAM_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let release: Release = resp.json().await.ok()?;
    tag_number(&release.tag_name).map(|_| release.tag_name)
}

/// A leitura do que foi encontrado — separada da coleta porque é ela que a
/// tela mostra, e porque só assim dá para testar cada caso sem uma placa de
/// vídeo por perto.
fn classifica(
    active: &Option<InstalledRuntime>,
    outros: &[InstalledRuntime],
    probe: Option<Result<(u64, u64), String>>,
) -> (Verdict, Option<String>, Option<u64>, Option<u64>) {
    match (active, probe) {
        // Rodou, mas se identificou como outra build: a pasta diz uma coisa e
        // o binário diz outra. Acontece quando uma extração é interrompida por
        // cima de outra, e é justamente o caso que nenhuma checagem de arquivo
        // pega — a pasta tem o nome certo e o conteúdo errado.
        (Some(r), Some(Ok((build, ms))))
            if tag_number(&r.tag).is_some_and(|esperada| esperada != build) =>
        {
            (
                Verdict::Broken,
                Some(format!(
                    "a pasta {} traz um binário que se diz build {build}",
                    r.tag
                )),
                Some(build),
                Some(ms),
            )
        }
        (Some(_), Some(Ok((build, ms)))) => (Verdict::Ready, None, Some(build), Some(ms)),
        (Some(_), Some(Err(e))) => (Verdict::Broken, Some(e), None, None),
        // A pasta existe sem o executável dentro: extração interrompida.
        (Some(r), None) => (
            Verdict::Broken,
            Some(format!("pacote incompleto em {}", r.dir.display())),
            None,
            None,
        ),
        (None, _) => {
            let mesma_build = outros.iter().any(|r| r.tag == PINNED_TAG && r.has_server);
            // Só builds OFICIAIS contam como "há outra build": um fork antigo
            // no disco não é atualização pendente do motor principal.
            let outra_build = outros
                .iter()
                .any(|r| r.has_server && tag_number(&r.tag).is_some());
            match (mesma_build, outra_build) {
                // A build certa está lá, só que compilada para outra placa.
                (true, _) => (Verdict::VariantChanged, None, None, None),
                (false, true) => (Verdict::UpdateAvailable, None, None, None),
                (false, false) => (Verdict::NotInstalled, None, None, None),
            }
        }
    }
}

/// O que foi achado no disco, em três montes: o oficial que o app usaria, o
/// motor da PrismML em uso (a primeira de `prism_dirs`) e o resto — que é o
/// que uma limpeza devolveria. As outras pastas de `prism_dirs` não entram
/// em monte nenhum: ficam, e não aparecem como recuperáveis.
type Separados = (
    Option<InstalledRuntime>,
    Option<InstalledRuntime>,
    Vec<InstalledRuntime>,
);

fn separa(instalados: Vec<InstalledRuntime>, esperado: &Path, prism_dirs: &[PathBuf]) -> Separados {
    let (ativo, resto): (Vec<_>, Vec<_>) = instalados.into_iter().partition(|r| r.dir == esperado);
    let (prism, outros): (Vec<_>, Vec<_>) =
        resto.into_iter().partition(|r| prism_dirs.contains(&r.dir));
    let prism = prism
        .into_iter()
        .find(|r| prism_dirs.first() == Some(&r.dir))
        .filter(|r| r.has_server);
    (ativo.into_iter().next(), prism, outros)
}

/// A verificação completa: disco, execução e — se der — a release lá fora.
///
/// `prism_dirs` são as pastas do motor da PrismML que ficam. A primeira é a
/// que o app usa AGORA (a variante resolvida — `cuda-12.8` quando instalada,
/// senão a reserva), reportada em `prism`; as demais só saem da conta do que
/// é recuperável: o `cuda-12.8` instalado que esta sessão não pôde confirmar
/// (NVML fora do ar depois de um upgrade do driver), a pasta de onde o
/// servidor de pé está rodando. Não se deduz da variante do oficial: no
/// Linux com NVIDIA as duas diferem.
pub async fn check(
    data_dir: &Path,
    expected_variant: BackendVariant,
    prism_dirs: &[PathBuf],
) -> EngineCheck {
    let esperado = runtime_dir(data_dir, PINNED_TAG, expected_variant);
    let (active, prism, outros) = separa(scan_installed(data_dir), &esperado, prism_dirs);
    let reclaimable_bytes = outros.iter().map(|r| r.size_bytes).sum();

    // A execução só faz sentido no pacote que o app usaria.
    let probe = match &active {
        Some(r) if r.has_server => Some(probe_build(&r.dir).await.map_err(|e| e.to_string())),
        _ => None,
    };

    let (verdict, detail, reported_build, probe_ms) = classifica(&active, &outros, probe);

    let upstream_tag = latest_upstream().await;
    let upstream_newer = match (&upstream_tag, tag_number(PINNED_TAG)) {
        (Some(t), Some(nosso)) => tag_number(t).is_some_and(|deles| deles > nosso),
        _ => false,
    };

    EngineCheck {
        expected_tag: PINNED_TAG.to_string(),
        expected_variant,
        verdict,
        detail,
        reported_build,
        probe_ms,
        active,
        prism,
        others: outros,
        reclaimable_bytes,
        upstream_tag,
        upstream_newer,
    }
}

/// O que a limpeza conseguiu fazer.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneResult {
    pub freed_bytes: u64,
    /// As pastas que resistiram, com o motivo. No Windows, um pacote em uso
    /// não se apaga — e dizer isso é melhor que abortar a limpeza inteira
    /// na primeira pasta presa.
    pub failed: Vec<String>,
}

/// Apaga os pacotes que o app não usa mais.
///
/// Só mexe no que a verificação classificou como `others`: a pasta ativa
/// nunca entra na conta, mesmo que a chamada venha errada.
///
/// `prism_dirs` são as pastas do motor da PrismML que ficam (ver [`check`]).
/// Com o `cuda-12.8` instalado e em uso, o `vulkan` do fork que ficou para
/// trás é lixo como qualquer outro — a não ser que o servidor de pé ainda
/// rode dele; enquanto o CUDA não estiver lá (ou tiver falhado nesta
/// máquina), o `vulkan` é o motor e fica.
///
/// Leva junto as sessões de download que um app fechado no meio deixou em
/// `runtimes/.tmp` (as de processos vivos ficam).
pub fn prune(
    data_dir: &Path,
    expected_variant: BackendVariant,
    prism_dirs: &[PathBuf],
) -> PruneResult {
    let mut manter = vec![runtime_dir(data_dir, PINNED_TAG, expected_variant)];
    // O motor da PrismML em uso também fica: apagá-lo faria o próximo Bonsai
    // baixar o motor de novo sem ninguém ter pedido.
    manter.extend(prism_dirs.iter().cloned());
    let mut r = PruneResult {
        freed_bytes: lr_fetch::sweep_stale_sessions(&data_dir.join("runtimes").join(".tmp")),
        ..PruneResult::default()
    };
    for pacote in scan_installed(data_dir) {
        if manter.contains(&pacote.dir) {
            continue;
        }
        match std::fs::remove_dir_all(&pacote.dir) {
            Ok(()) => {
                r.freed_bytes += pacote.size_bytes;
                // A pasta da tag fica vazia quando era a última variante dela.
                // Num fork antigo pode ter sobrado o marcador de falha do
                // CUDA (`<variante>.failed`), que não é pacote: sai junto.
                if let Some(pai) = pacote.dir.parent()
                    && std::fs::remove_dir(pai).is_err()
                    && pacote.tag != crate::prism::TAG
                {
                    apagar_marcadores(pai);
                    let _ = std::fs::remove_dir(pai);
                }
            }
            Err(e) => r.failed.push(format!("{}: {e}", pacote.dir.display())),
        }
    }
    r
}

/// Apaga os marcadores `*.failed` de uma pasta de tag, e só eles.
fn apagar_marcadores(pasta_da_tag: &Path) {
    let Ok(entradas) = std::fs::read_dir(pasta_da_tag) else {
        return;
    };
    for e in entradas.flatten() {
        let eh_marcador = e.file_type().is_ok_and(|t| t.is_file())
            && e.file_name().to_string_lossy().ends_with(".failed");
        if eh_marcador {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toca(caminho: &Path, bytes: usize) {
        std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
        std::fs::write(caminho, vec![b'x'; bytes]).unwrap();
    }

    /// A saída REAL do `llama-server --version`, copiada de uma execução de
    /// verdade da b10441 no Windows.
    ///
    /// Este teste nasceu com um formato que eu SUPUS (`version: 10441
    /// (hash)`), passou, e mesmo assim o app dizia "instalado, mas não
    /// executa" numa máquina com o motor perfeito: o que vem depois de
    /// `version:` é `0.1.0-dev`, e o parser lia zero.
    #[test]
    fn the_build_number_comes_from_the_binarys_own_output() {
        let real = "version: 0.1.0-dev (build 10441, commit 0177dcc73)\n\
                    built with Clang 20.1.8 for Windows x86_64\n";
        assert_eq!(parse_reported_build(real), Some(10441));

        // O formato antigo, que ainda pode vir de uma build de terceiro.
        assert_eq!(parse_reported_build("version: 9911 (abc)"), Some(9911));
        assert_eq!(parse_reported_build("build: 9911 (abc)"), Some(9911));
        assert_eq!(parse_reported_build("build = 9911"), Some(9911));

        // `0.1.0-dev` sozinho não é build nenhuma.
        assert_eq!(parse_reported_build("version: 0.1.0-dev"), None);

        // Ruído sem número de build não vira versão inventada.
        assert_eq!(
            parse_reported_build("error while loading cudart64_13.dll"),
            None
        );
        assert_eq!(parse_reported_build(""), None);
    }

    #[test]
    fn tags_that_are_not_releases_are_not_versions() {
        assert_eq!(tag_number("b10441"), Some(10441));
        assert_eq!(tag_number("session-abc"), None);
        assert_eq!(tag_number("b"), None);
    }

    /// A tag do fork carrega a build do upstream; `tag_number` a ignora de
    /// propósito (não é release oficial), `build_number` a lê.
    #[test]
    fn the_prism_tag_has_a_build_number_but_is_not_an_official_release() {
        assert_eq!(build_number("prism-b10709-9a9394a"), Some(10709));
        assert_eq!(build_number("b10441"), Some(10441));
        assert_eq!(tag_number("prism-b10709-9a9394a"), None);
        assert_eq!(build_number("prism-b"), None);
        assert_eq!(build_number("prism-b10709"), None);
        assert_eq!(build_number("session-abc"), None);
        assert!(is_prism_tag(crate::prism::TAG));
        assert!(!is_prism_tag(PINNED_TAG));
        assert_eq!(
            build_number(crate::prism::TAG),
            Some(crate::prism::UPSTREAM_BUILD)
        );
    }

    /// A pasta do fork aparece no scan, sai de `others` na verificação e
    /// sobrevive à limpeza; um fork ANTIGO é lixo como qualquer build velha.
    #[test]
    fn the_prism_package_is_reported_kept_and_old_forks_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let prism = runtime_dir(d, crate::prism::TAG, BackendVariant::Cuda13);
        toca(&prism.join(server_exe_name()), 9);
        toca(
            &d.join("runtimes/prism-b10500-abc1234/cuda-13.3")
                .join(server_exe_name()),
            4,
        );
        toca(&d.join("runtimes/b10390/vulkan").join(server_exe_name()), 7);

        let achados = scan_installed(d);
        assert_eq!(achados.len(), 3);
        assert_eq!(
            achados[0].tag,
            crate::prism::TAG,
            "mais nova primeiro, pela build"
        );

        let limpeza = prune(d, BackendVariant::Cuda13, std::slice::from_ref(&prism));
        assert_eq!(limpeza.freed_bytes, 11);
        assert!(prism.join(server_exe_name()).is_file());
        assert!(!d.join("runtimes/prism-b10500-abc1234").exists());
        assert!(!d.join("runtimes/b10390").exists());

        // Um fork antigo no disco não é "atualização disponível" do oficial.
        let so_fork = [pacote("prism-b10500-abc1234", "cuda-13.3", true)];
        assert_eq!(classifica(&None, &so_fork, None).0, Verdict::NotInstalled);
    }

    /// O disco é a fonte: pastas de release viram itens, a sessão de
    /// download não.
    #[test]
    fn the_scan_reads_what_is_really_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        toca(
            &d.join("runtimes/b10441/cuda-13.3").join(server_exe_name()),
            10,
        );
        toca(&d.join("runtimes/b10441/cuda-13.3").join(rpc_exe_name()), 5);
        toca(&d.join("runtimes/b10390/vulkan").join(server_exe_name()), 7);
        // Restos que não são release nenhuma.
        toca(&d.join("runtimes/session-xyz/parte.zip"), 3);

        let achados = scan_installed(d);
        assert_eq!(achados.len(), 2);
        // Mais nova primeiro.
        assert_eq!(achados[0].tag, "b10441");
        assert_eq!(achados[0].variant, Some(BackendVariant::Cuda13));
        assert!(achados[0].has_server && achados[0].has_rpc);
        assert_eq!(achados[0].size_bytes, 15);
        assert_eq!(achados[1].tag, "b10390");
        assert!(!achados[1].has_rpc);
    }

    /// Uma variante que não conhecemos ainda aparece na lista — some da tela
    /// seria pior: são gigabytes que ninguém consegue explicar.
    #[test]
    fn an_unknown_variant_folder_is_still_reported() {
        let tmp = tempfile::tempdir().unwrap();
        toca(
            &tmp.path()
                .join("runtimes/b10441/hip-6.2")
                .join(server_exe_name()),
            4,
        );
        let achados = scan_installed(tmp.path());
        assert_eq!(achados.len(), 1);
        assert_eq!(achados[0].variant, None);
        assert_eq!(achados[0].variant_dir, "hip-6.2");
    }

    /// A limpeza tira as builds antigas e não encosta na que está em uso.
    #[test]
    fn pruning_keeps_the_package_in_use() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let ativo = runtime_dir(d, PINNED_TAG, BackendVariant::Cuda13);
        toca(&ativo.join(server_exe_name()), 10);
        toca(&d.join("runtimes/b10390/vulkan").join(server_exe_name()), 7);
        toca(&d.join("runtimes/b10200/cpu").join(server_exe_name()), 3);

        let prism = runtime_dir(d, crate::prism::TAG, BackendVariant::Cuda13);
        let limpeza = prune(d, BackendVariant::Cuda13, std::slice::from_ref(&prism));
        assert_eq!(limpeza.freed_bytes, 10);
        assert!(limpeza.failed.is_empty());
        assert!(ativo.join(server_exe_name()).is_file());
        assert!(!d.join("runtimes/b10390").exists());
        assert!(!d.join("runtimes/b10200").exists());
    }

    fn pacote(tag: &str, variant_dir: &str, has_server: bool) -> InstalledRuntime {
        InstalledRuntime {
            tag: tag.to_string(),
            variant: variante_do_nome(variant_dir),
            variant_dir: variant_dir.to_string(),
            dir: PathBuf::from(format!("/x/{tag}/{variant_dir}")),
            size_bytes: 1_000,
            has_server,
            has_rpc: has_server,
        }
    }

    /// Cada situação tem um veredito próprio — e "não instalado" não pode
    /// engolir "instalado, mas de outra build": são botões diferentes na
    /// tela e conversas diferentes com quem lê.
    #[test]
    fn each_situation_gets_its_own_verdict() {
        let ativo = Some(pacote(PINNED_TAG, "cuda-13.3", true));

        // Rodou e reportou a build da própria pasta: pronto.
        let build_da_pasta = tag_number(PINNED_TAG).expect("a tag pinada é bNNNNN");
        let (v, d, build, ms) = classifica(&ativo, &[], Some(Ok((build_da_pasta, 820))));
        assert_eq!(v, Verdict::Ready);
        assert_eq!((d, build, ms), (None, Some(build_da_pasta), Some(820)));

        // Rodou, e se disse outra build: a pasta e o binário discordam.
        let (v, d, ..) = classifica(&ativo, &[], Some(Ok((9_001, 500))));
        assert_eq!(v, Verdict::Broken);
        assert!(d.is_some_and(|m| m.contains("9001")));

        // Os arquivos estão lá e o executável não roda.
        let (v, d, ..) = classifica(&ativo, &[], Some(Err("cudart64_13.dll".into())));
        assert_eq!(v, Verdict::Broken);
        assert!(d.is_some_and(|m| m.contains("cudart")));

        // Pasta sem executável dentro: extração interrompida.
        assert_eq!(classifica(&ativo, &[], None).0, Verdict::Broken);

        // Só a build antiga no disco.
        let antigos = [pacote("b10390", "cuda-13.3", true)];
        assert_eq!(
            classifica(&None, &antigos, None).0,
            Verdict::UpdateAvailable
        );

        // A build certa, para outra placa.
        let outra_placa = [pacote(PINNED_TAG, "vulkan", true)];
        assert_eq!(
            classifica(&None, &outra_placa, None).0,
            Verdict::VariantChanged
        );

        // Pastas vazias não contam como instalação.
        let vazio = [pacote("b10390", "cpu", false)];
        assert_eq!(classifica(&None, &vazio, None).0, Verdict::NotInstalled);
        assert_eq!(classifica(&None, &[], None).0, Verdict::NotInstalled);
    }

    /// Sem nenhuma pasta, a verificação não inventa instalação nem quebra.
    #[test]
    fn nothing_installed_is_a_verdict_not_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let achados = scan_installed(tmp.path());
        assert!(achados.is_empty());
        let prism = runtime_dir(tmp.path(), crate::prism::TAG, BackendVariant::Cpu);
        assert_eq!(
            prune(
                tmp.path(),
                BackendVariant::Cpu,
                std::slice::from_ref(&prism)
            )
            .freed_bytes,
            0
        );
    }

    /// Com o CUDA 12.8 do fork instalado e em uso, o `vulkan` do fork que
    /// ficou para trás é recuperável; enquanto o motor em uso for o `vulkan`
    /// (CUDA ausente ou com marcador de falha), ele fica — e o marcador,
    /// que não é pacote, não aparece em lugar nenhum.
    #[test]
    fn the_prism_folder_in_use_decides_what_the_prune_keeps() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let oficial = runtime_dir(d, PINNED_TAG, BackendVariant::Vulkan);
        let prism_vulkan = runtime_dir(d, crate::prism::TAG, BackendVariant::Vulkan);
        let prism_cuda = runtime_dir(d, crate::prism::TAG, BackendVariant::Cuda128);
        toca(&oficial.join(server_exe_name()), 5);
        toca(&prism_vulkan.join(server_exe_name()), 7);
        toca(
            &d.join("runtimes")
                .join(crate::prism::TAG)
                .join("cuda-12.8.failed"),
            3,
        );

        // Marcador presente, CUDA ausente: o vulkan é o motor e fica.
        let achados = scan_installed(d);
        assert_eq!(achados.len(), 2, "o marcador não é pacote: {achados:?}");
        let r = prune(
            d,
            BackendVariant::Vulkan,
            std::slice::from_ref(&prism_vulkan),
        );
        assert_eq!(r.freed_bytes, 0);
        assert!(prism_vulkan.join(server_exe_name()).is_file());

        // CUDA instalado e resolvido: o vulkan do fork vira lixo.
        toca(&prism_cuda.join(server_exe_name()), 11);
        assert_eq!(
            scan_installed(d)
                .iter()
                .find(|r| r.dir == prism_cuda)
                .and_then(|r| r.variant),
            Some(BackendVariant::Cuda128)
        );
        let r = prune(d, BackendVariant::Vulkan, std::slice::from_ref(&prism_cuda));
        assert_eq!(r.freed_bytes, 7);
        assert!(!prism_vulkan.exists());
        assert!(prism_cuda.join(server_exe_name()).is_file());
        assert!(oficial.join(server_exe_name()).is_file());
    }

    /// Mais de uma pasta do fork fica: a resolvida (o CUDA), a de onde o
    /// servidor de pé ainda roda (o Vulkan, logo depois de o CUDA chegar) —
    /// e nenhuma das duas conta como recuperável. A sessão de download que
    /// um app fechado no meio deixou vai embora na limpeza.
    #[test]
    fn every_prism_folder_in_use_is_kept_and_abandoned_sessions_are_freed() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let prism_vulkan = runtime_dir(d, crate::prism::TAG, BackendVariant::Vulkan);
        let prism_cuda = runtime_dir(d, crate::prism::TAG, BackendVariant::Cuda128);
        toca(&prism_vulkan.join(server_exe_name()), 7);
        toca(&prism_cuda.join(server_exe_name()), 11);
        toca(&d.join("runtimes/b10390/vulkan").join(server_exe_name()), 3);
        // `i32::MAX` nunca é PID de processo vivo.
        toca(
            &d.join(format!("runtimes/.tmp/job-{}-1/cublas.whl.part", i32::MAX)),
            5,
        );

        let manter = [prism_cuda.clone(), prism_vulkan.clone()];
        let oficial = runtime_dir(d, PINNED_TAG, BackendVariant::Vulkan);
        let (ativo, prism, outros) = separa(scan_installed(d), &oficial, &manter);
        assert!(ativo.is_none());
        assert_eq!(prism.map(|p| p.dir), Some(prism_cuda.clone()));
        assert_eq!(outros.len(), 1, "{outros:?}");
        assert_eq!(outros[0].tag, "b10390");

        let r = prune(d, BackendVariant::Vulkan, &manter);
        assert_eq!(r.freed_bytes, 3 + 5);
        assert!(prism_vulkan.join(server_exe_name()).is_file());
        assert!(prism_cuda.join(server_exe_name()).is_file());
        assert!(!d.join(format!("runtimes/.tmp/job-{}-1", i32::MAX)).exists());
    }

    #[test]
    fn a_package_folder_names_its_variant() {
        let d = Path::new("/d");
        let cuda = runtime_dir(d, crate::prism::TAG, BackendVariant::Cuda128);
        assert_eq!(variant_of_dir(&cuda), Some(BackendVariant::Cuda128));
        assert_eq!(
            variant_of_dir(&runtime_dir(d, PINNED_TAG, BackendVariant::Vulkan)),
            Some(BackendVariant::Vulkan)
        );
        assert_eq!(variant_of_dir(Path::new("/d/runtimes/b1/hip-6.2")), None);
        assert_eq!(variant_of_dir(Path::new("/")), None);
    }

    /// Um fork antigo com o marcador de falha do CUDA sai inteiro: a pasta
    /// da tag não fica presa por um arquivo que não é pacote.
    #[test]
    fn an_old_fork_leaves_no_failure_marker_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let velho = d.join("runtimes/prism-b10500-abc1234");
        toca(&velho.join("vulkan").join(server_exe_name()), 4);
        toca(&velho.join("cuda-12.8.failed"), 2);
        let prism = runtime_dir(d, crate::prism::TAG, BackendVariant::Vulkan);
        let r = prune(d, BackendVariant::Vulkan, std::slice::from_ref(&prism));
        assert_eq!(r.freed_bytes, 4);
        assert!(!velho.exists());
    }

    /// As saídas REAIS do `llama-server --list-devices` do prism-b10709 com
    /// as bibliotecas CUDA ao lado, na RTX 3090 (host Bluefin, 2026-09-27).
    #[test]
    fn the_cuda_device_list_is_read_from_the_real_output() {
        let passa =
            "Available devices:\n  CUDA0: NVIDIA GeForce RTX 3090 (24173 MiB, 22820 MiB free)\n";
        assert_eq!(
            ler_dispositivos_cuda(passa),
            DispositivosCuda::Presente(
                "CUDA0: NVIDIA GeForce RTX 3090 (24173 MiB, 22820 MiB free)".into()
            )
        );

        // `CUDA_VISIBLE_DEVICES=-1`: sai com 0 e sem nenhum dispositivo.
        let sem_placa = "E ggml_cuda_init: failed to initialize CUDA: no CUDA-capable device is detected\n\
                         Available devices:\n  (none)\n";
        match ler_dispositivos_cuda(sem_placa) {
            DispositivosCuda::Ausente(causa) => assert!(causa.contains("no CUDA-capable device")),
            outro => panic!("esperava Ausente, veio {outro:?}"),
        }

        // O pacote como vem na release, sem o cudart: o carregador recusa
        // (código 127) antes de o binário dizer qualquer coisa.
        let sem_cudart = "./llama-server: error while loading shared libraries: libcudart.so.12: \
                          cannot open shared object file: No such file or directory\n";
        match ler_dispositivos_cuda(sem_cudart) {
            DispositivosCuda::Ausente(causa) => assert!(causa.contains("libcudart.so.12")),
            outro => panic!("esperava Ausente, veio {outro:?}"),
        }

        // Placa ocupada: transitório, nunca "esta máquina não serve".
        let cheia = "ggml_cuda_init: found 1 CUDA devices:\n  Device 0: NVIDIA GeForce RTX 3090, compute capability 8.6, VMM: yes\n\
                     CUDA error: out of memory\n  current device: 0, in function ggml_backend_cuda_device_get_memory\n";
        assert!(matches!(
            ler_dispositivos_cuda(cheia),
            DispositivosCuda::Transitoria(_)
        ));

        // Depois de uma suspensão (cudaErrorUnknown), em modo exclusivo com
        // outro processo na placa, ou com o fabric manager subindo: o
        // momento, não a máquina.
        for erro in [
            "unknown error",
            "CUDA-capable device(s) is/are busy or unavailable",
            "system not yet initialized",
        ] {
            let saida = format!(
                "E ggml_cuda_init: failed to initialize CUDA: {erro}\nAvailable devices:\n  (none)\n"
            );
            match ler_dispositivos_cuda(&saida) {
                DispositivosCuda::Transitoria(causa) => assert!(causa.contains(erro)),
                outro => panic!("{erro}: esperava Transitoria, veio {outro:?}"),
            }
        }

        // O driver velho demais para o runtime 12.8 é a máquina.
        let driver = "E ggml_cuda_init: failed to initialize CUDA: CUDA driver version is \
                      insufficient for CUDA runtime version\nAvailable devices:\n  (none)\n";
        assert!(matches!(
            ler_dispositivos_cuda(driver),
            DispositivosCuda::Ausente(_)
        ));

        // Um dispositivo Vulkan (o pacote errado) não conta como CUDA, nem
        // o cabeçalho "found 1 CUDA devices:".
        let vulkan =
            "Available devices:\n  Vulkan0: NVIDIA GeForce RTX 3090 (24822 MiB, 24000 MiB free)\n";
        assert!(matches!(
            ler_dispositivos_cuda(vulkan),
            DispositivosCuda::Ausente(_)
        ));
        assert!(matches!(
            ler_dispositivos_cuda("ggml_cuda_init: found 1 CUDA devices:\n"),
            DispositivosCuda::Ausente(_)
        ));
        assert!(matches!(
            ler_dispositivos_cuda(""),
            DispositivosCuda::Ausente(_)
        ));
    }

    #[test]
    fn the_cuda128_folder_name_maps_back_to_its_variant() {
        assert_eq!(variante_do_nome("cuda-12.8"), Some(BackendVariant::Cuda128));
        assert_eq!(
            runtime_dir(Path::new("/d"), crate::prism::TAG, BackendVariant::Cuda128)
                .file_name()
                .and_then(|n| n.to_str()),
            Some("cuda-12.8")
        );
    }
}
