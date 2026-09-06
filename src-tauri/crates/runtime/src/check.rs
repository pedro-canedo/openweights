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
    /// Os outros pacotes no disco — builds antigas, variantes que sobraram.
    pub others: Vec<InstalledRuntime>,
    /// Soma de `others`: o que uma limpeza devolveria ao disco.
    pub reclaimable_bytes: u64,
    /// A última release do llama.cpp, quando a consulta funcionou.
    pub upstream_tag: Option<String>,
    /// Ela é mais nova que a nossa homologada.
    pub upstream_newer: bool,
}

/// `b10441` → `10441`. O que não tem essa forma não é tag de release.
pub fn tag_number(tag: &str) -> Option<u64> {
    tag.strip_prefix('b')?.parse().ok()
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
        // Só pastas de release: a sessão de download também mora aqui.
        if tag_number(&tag).is_none() {
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
        tag_number(&b.tag)
            .cmp(&tag_number(&a.tag))
            .then_with(|| a.variant_dir.cmp(&b.variant_dir))
    });
    encontrados
}

/// O caminho inverso de [`crate::runtime_dir`]: nome da pasta → variante.
fn variante_do_nome(nome: &str) -> Option<BackendVariant> {
    Some(match nome {
        "cuda-13.3" => BackendVariant::Cuda13,
        "cuda-12.4" => BackendVariant::Cuda12,
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
/// O llama-server imprime `version: 10441 (0a1b2c3)` — em `stderr` em umas
/// builds, em `stdout` em outras, daí a busca nas duas.
pub fn parse_reported_build(saida: &str) -> Option<u64> {
    for linha in saida.lines() {
        let baixo = linha.to_ascii_lowercase();
        for marca in ["version:", "build:", "build ="] {
            if let Some(p) = baixo.find(marca) {
                let resto = linha[p + marca.len()..].trim_start();
                let numero: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = numero.parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Executa o motor instalado e lê a build que ele reporta.
///
/// O `current_dir` é a pasta do pacote de propósito: as DLLs do CUDA moram ao
/// lado do executável, e é essa vizinhança que o teste precisa exercitar.
async fn probe_build(dir: &Path) -> Result<(u64, u64), String> {
    let exe = dir.join(server_exe_name());
    if !exe.is_file() {
        return Err(format!(
            "{} não está em {}",
            server_exe_name(),
            dir.display()
        ));
    }
    let inicio = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(dir)
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let saida = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("{} não respondeu em {PROBE_TIMEOUT:?}", server_exe_name()))?
        .map_err(|e| e.to_string())?;
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
        None => Err(primeira_linha_util(&texto)
            .unwrap_or_else(|| format!("{} não reportou versão nenhuma", server_exe_name()))),
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
            let outra_build = outros.iter().any(|r| r.has_server);
            match (mesma_build, outra_build) {
                // A build certa está lá, só que compilada para outra placa.
                (true, _) => (Verdict::VariantChanged, None, None, None),
                (false, true) => (Verdict::UpdateAvailable, None, None, None),
                (false, false) => (Verdict::NotInstalled, None, None, None),
            }
        }
    }
}

/// A verificação completa: disco, execução e — se der — a release lá fora.
pub async fn check(data_dir: &Path, expected_variant: BackendVariant) -> EngineCheck {
    let esperado = runtime_dir(data_dir, PINNED_TAG, expected_variant);
    let instalados = scan_installed(data_dir);

    let (ativo, outros): (Vec<_>, Vec<_>) = instalados.into_iter().partition(|r| r.dir == esperado);
    let active = ativo.into_iter().next();
    let reclaimable_bytes = outros.iter().map(|r| r.size_bytes).sum();

    // A execução só faz sentido no pacote que o app usaria.
    let probe = match &active {
        Some(r) if r.has_server => Some(probe_build(&r.dir).await),
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
pub fn prune(data_dir: &Path, expected_variant: BackendVariant) -> PruneResult {
    let manter = runtime_dir(data_dir, PINNED_TAG, expected_variant);
    let mut r = PruneResult::default();
    for pacote in scan_installed(data_dir) {
        if pacote.dir == manter {
            continue;
        }
        match std::fs::remove_dir_all(&pacote.dir) {
            Ok(()) => {
                r.freed_bytes += pacote.size_bytes;
                // A pasta da tag fica vazia quando era a última variante dela.
                if let Some(pai) = pacote.dir.parent() {
                    let _ = std::fs::remove_dir(pai);
                }
            }
            Err(e) => r.failed.push(format!("{}: {e}", pacote.dir.display())),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toca(caminho: &Path, bytes: usize) {
        std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
        std::fs::write(caminho, vec![b'x'; bytes]).unwrap();
    }

    /// A saída REAL do `llama-server --version` — se o formato mudar, é aqui
    /// que se descobre, e não numa tela dizendo "motor quebrado" para um
    /// motor perfeito.
    #[test]
    fn the_build_number_comes_from_the_binarys_own_output() {
        let real = "version: 10441 (3f7c9d2a)\nbuilt with MSVC 19.44 for x64\n";
        assert_eq!(parse_reported_build(real), Some(10441));

        // Variante que imprime `build:` em vez de `version:`.
        assert_eq!(parse_reported_build("build: 9911 (abc)"), Some(9911));

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

        let limpeza = prune(d, BackendVariant::Cuda13);
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

        // Rodou e reportou a build: pronto.
        let (v, d, build, ms) = classifica(&ativo, &[], Some(Ok((10_441, 820))));
        assert_eq!(v, Verdict::Ready);
        assert_eq!((d, build, ms), (None, Some(10_441), Some(820)));

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
        assert_eq!(prune(tmp.path(), BackendVariant::Cpu).freed_bytes, 0);
    }
}
