//! Download, verificação e extração dos runtimes llama.cpp.
//!
//! Fluxo do `ensure`:
//! 1. Já instalado? Retorna na hora.
//! 2. Baixa o asset da release pinada em streaming para um `.part` dentro do
//!    diretório de sessão do job, emitindo `Progress` com throttle de 200 ms.
//! 3. Verifica o SHA256 contra o campo `digest` da API do GitHub — se a API
//!    falhar (rate limit etc.), loga e prossegue sem verificação.
//! 4. Extrai num diretório temporário, localiza o `llama-server(.exe)`
//!    recursivamente (os zips têm estrutura variável) e "achata" o conteúdo
//!    do diretório que o contém.
//! 5. Variantes CUDA: baixa também o pacote cudart e extrai as DLLs ao lado
//!    do executável.
//! 6. Instalação atômica: tudo montado no tmp e movido com um único
//!    `fs::rename` para o destino final; tmp limpo em qualquer desfecho.

use crate::BackendVariant;
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// User-Agent exigido pela API do GitHub (e boa educação nos downloads).
const USER_AGENT: &str = concat!("LlamaRunner/", env!("CARGO_PKG_VERSION"));

/// Intervalo mínimo entre eventos de progresso.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

/// Buffer de escrita do download (os assets têm 40–380 MB; nunca carregar
/// inteiro em memória).
const DOWNLOAD_BUFFER: usize = 1024 * 1024;

/// Sanidade final: um llama-server real sempre passa de 1 MiB.
const MIN_SERVER_SIZE: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("verificação falhou: {0}")]
    Verification(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum RuntimeEvent {
    Progress {
        asset: String,
        received_bytes: u64,
        total_bytes: u64,
    },
    Extracting {
        asset: String,
    },
    Ready,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub tag: String,
    pub variant: BackendVariant,
    pub installed: bool,
    pub server_exe: Option<PathBuf>,
}

/// Gerencia a instalação dos runtimes em `<data_dir>/runtimes/`.
pub struct RuntimeManager {
    data_dir: PathBuf,
    /// Serializa instalações: uma segunda chamada concorrente de `ensure`
    /// espera a primeira (e então vê o runtime já instalado).
    install_lock: tokio::sync::Mutex<()>,
}

impl RuntimeManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            install_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Estado atual do runtime pinado para a variante dada.
    pub fn state(&self, variant: BackendVariant) -> RuntimeState {
        let dir = crate::runtime_dir(&self.data_dir, crate::PINNED_TAG, variant);
        let exe = dir.join(crate::server_exe_name());
        let installed = exe.exists();
        RuntimeState {
            tag: crate::PINNED_TAG.to_string(),
            variant,
            installed,
            server_exe: installed.then_some(exe),
        }
    }

    /// Garante que o runtime está instalado, baixando/extraindo se preciso.
    pub async fn ensure(
        &self,
        variant: BackendVariant,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        // Serializa instalações concorrentes (reentrada do comando, remount
        // do webview etc.); quem esperar re-checa e sai cedo se já instalou.
        let _install_guard = self.install_lock.lock().await;

        let state = self.state(variant);
        if state.installed {
            return Ok(state);
        }

        let tmp_root = self.data_dir.join("runtimes").join(".tmp");
        // Diretório de sessão único: TODO o trabalho temporário (inclusive os
        // .part) vive aqui, então jobs não colidem e a limpeza é um rmdir só.
        let session = tmp_root.join(format!("job-{}", unique_suffix()));

        let result = self.install(variant, &session, &on_event).await;

        // Limpeza best-effort do tmp em qualquer desfecho (sucesso ou falha).
        let _ = std::fs::remove_dir_all(&session);

        match result {
            Ok(()) => {
                on_event(RuntimeEvent::Ready);
                Ok(self.state(variant))
            }
            Err(e) => {
                on_event(RuntimeEvent::Failed {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Baixa, verifica, extrai e instala atomicamente o runtime no destino
    /// final. Trabalha inteiramente dentro de `session`; o último passo é um
    /// único `fs::rename`.
    async fn install<F>(
        &self,
        variant: BackendVariant,
        session: &Path,
        on_event: &F,
    ) -> Result<(), RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        std::fs::create_dir_all(session)?;
        let tag = crate::PINNED_TAG;
        let client = build_client()?;

        // Digests de todos os assets da release, de uma vez só. `None` se a
        // API falhar — nesse caso prosseguimos sem verificação (com warning).
        let digests = fetch_release_digests(&client, tag).await;

        // ---- Asset principal -------------------------------------------
        let asset = crate::asset_name(tag, variant);
        let part = session.join(format!("{asset}.part"));
        download_to(
            &client,
            &crate::asset_url(tag, &asset),
            &part,
            &asset,
            on_event,
        )
        .await?;
        verify_sha256(&part, &asset, digests.as_ref()).await?;

        on_event(RuntimeEvent::Extracting {
            asset: asset.clone(),
        });
        let extract_dir = session.join("extract-main");
        extract_archive_async(part.clone(), asset.clone(), extract_dir.clone()).await?;
        let _ = std::fs::remove_file(&part);

        // Estrutura variável nos pacotes: achamos o diretório que contém o
        // llama-server e usamos ele como raiz real do runtime.
        let root = find_server_root(&extract_dir).ok_or_else(|| {
            RuntimeError::Verification(format!(
                "{} não encontrado dentro de {asset}",
                crate::server_exe_name()
            ))
        })?;
        let staging = session.join("install");
        std::fs::create_dir_all(&staging)?;
        move_dir_contents(&root, &staging)?;

        // ---- cudart (obrigatório para CUDA) -----------------------------
        if let Some(cudart) = crate::cudart_asset_name(tag, variant) {
            let cpart = session.join(format!("{cudart}.part"));
            download_to(
                &client,
                &crate::asset_url(tag, &cudart),
                &cpart,
                &cudart,
                on_event,
            )
            .await?;
            verify_sha256(&cpart, &cudart, digests.as_ref()).await?;

            on_event(RuntimeEvent::Extracting {
                asset: cudart.clone(),
            });
            let cudart_dir = session.join("extract-cudart");
            extract_archive_async(cpart.clone(), cudart.clone(), cudart_dir.clone()).await?;
            let _ = std::fs::remove_file(&cpart);
            // As DLLs precisam ficar AO LADO do llama-server.exe, então
            // achatamos qualquer subestrutura do pacote cudart.
            move_files_flat(&cudart_dir, &staging)?;
        }

        // ---- Sanidade final ---------------------------------------------
        let exe = staging.join(crate::server_exe_name());
        let exe_ok = std::fs::metadata(&exe)
            .map(|m| m.is_file() && m.len() > MIN_SERVER_SIZE)
            .unwrap_or(false);
        if !exe_ok {
            return Err(RuntimeError::Verification(format!(
                "{} ausente ou menor que 1 MiB após a extração",
                crate::server_exe_name()
            )));
        }

        // ---- Instalação atômica -----------------------------------------
        let final_dir = crate::runtime_dir(&self.data_dir, tag, variant);
        if let Some(parent) = final_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if final_dir.exists() {
            // Sobra de uma instalação quebrada (sem o executável): remove
            // para o rename atômico não falhar.
            std::fs::remove_dir_all(&final_dir)?;
        }
        std::fs::rename(&staging, &final_dir)?;
        log::info!(
            "runtime {tag}/{variant:?} instalado em {}",
            final_dir.display()
        );
        Ok(())
    }

    /// Seleciona a melhor variante para o perfil e tenta instalá-la,
    /// descendo a cadeia de fallback (CUDA → Vulkan → CPU) em caso de falha.
    pub async fn ensure_best(
        &self,
        profile: &lr_types::HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let mut variant = crate::select_variant(profile);
        loop {
            match self.ensure(variant, &on_event).await {
                Ok(state) => return Ok(state),
                Err(e) => match variant.fallback() {
                    Some(next) => {
                        log::warn!("runtime {variant:?} falhou ({e}); tentando {next:?}");
                        variant = next;
                    }
                    None => return Err(e),
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .build()
}

/// Baixa `url` em streaming para `dest`, emitindo `Progress` com throttle.
async fn download_to<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    asset: &str,
    on_event: &F,
) -> Result<(), RuntimeError>
where
    F: Fn(RuntimeEvent) + Send + Sync,
{
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let response = client.get(url).send().await?.error_for_status()?;
    let total_bytes = response.content_length().unwrap_or(0);

    let file = std::fs::File::create(dest)?;
    let mut writer = std::io::BufWriter::with_capacity(DOWNLOAD_BUFFER, file);
    let mut stream = response.bytes_stream();
    let mut received_bytes: u64 = 0;
    let mut last_emit: Option<Instant> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        writer.write_all(&chunk)?;
        received_bytes += chunk.len() as u64;
        if last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_THROTTLE) {
            on_event(RuntimeEvent::Progress {
                asset: asset.to_string(),
                received_bytes,
                total_bytes,
            });
            last_emit = Some(Instant::now());
        }
    }
    writer.flush()?;

    // Evento final sem throttle, para a UI fechar em 100%.
    on_event(RuntimeEvent::Progress {
        asset: asset.to_string(),
        received_bytes,
        total_bytes,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Verificação SHA256
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct ReleaseAsset {
    name: String,
    /// Formato `"sha256:<hex>"`, presente nos assets de release do GitHub.
    digest: Option<String>,
}

#[derive(serde::Deserialize)]
struct Release {
    assets: Vec<ReleaseAsset>,
}

/// Busca os digests SHA256 de todos os assets da release `tag`.
///
/// Retorna `None` se a API falhar (403 por rate limit, rede etc.) — a
/// instalação prossegue sem verificação nesse caso, apenas com warning.
async fn fetch_release_digests(
    client: &reqwest::Client,
    tag: &str,
) -> Option<HashMap<String, String>> {
    let url = format!("https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/{tag}");
    let result: Result<Release, reqwest::Error> = async {
        client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .json::<Release>()
            .await
    }
    .await;

    match result {
        Ok(release) => {
            let mut map = HashMap::new();
            for asset in release.assets {
                if let Some(hex) = asset.digest.as_deref().and_then(parse_sha256_digest) {
                    map.insert(asset.name, hex);
                }
            }
            Some(map)
        }
        Err(e) => {
            log::warn!(
                "não foi possível obter digests da release {tag} ({e}); \
                 prosseguindo sem verificação SHA256"
            );
            None
        }
    }
}

/// Extrai o hex de um digest `"sha256:<hex>"` (64 dígitos), normalizado em
/// minúsculas. Retorna `None` para qualquer outro formato.
fn parse_sha256_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

/// SHA256 de um arquivo em streaming (nunca carrega tudo em memória).
fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Verifica o SHA256 de `path` contra o digest esperado para `asset`.
/// Digest indisponível não é erro (já logado); divergência é.
async fn verify_sha256(
    path: &Path,
    asset: &str,
    digests: Option<&HashMap<String, String>>,
) -> Result<(), RuntimeError> {
    let Some(digests) = digests else {
        return Ok(());
    };
    let Some(expected) = digests.get(asset) else {
        log::warn!("digest ausente para {asset}; prosseguindo sem verificação");
        return Ok(());
    };
    let file = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&file))
        .await
        .map_err(std::io::Error::other)??;
    if actual != *expected {
        return Err(RuntimeError::Verification(format!(
            "SHA256 divergente para {asset}: esperado {expected}, obtido {actual}"
        )));
    }
    log::info!("SHA256 verificado para {asset}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Extração
// ---------------------------------------------------------------------------

/// Extrai `archive` (formato deduzido pelo nome do `asset`) para `dest`,
/// em thread de blocking para não travar o runtime async.
async fn extract_archive_async(
    archive: PathBuf,
    asset: String,
    dest: PathBuf,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || extract_archive(&archive, &asset, &dest))
        .await
        .map_err(std::io::Error::other)?
}

fn extract_archive(archive: &Path, asset: &str, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    if asset.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if asset.ends_with(".tar.gz") {
        extract_tar_gz(archive, dest)
    } else {
        Err(std::io::Error::other(format!(
            "formato de pacote desconhecido: {asset}"
        )))
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> std::io::Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(std::io::Error::other)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(std::io::Error::other)?;
        // `enclosed_name` rejeita path traversal (`../`, caminho absoluto).
        let Some(relative) = entry.enclosed_name() else {
            log::warn!("entrada suspeita ignorada no zip: {:?}", entry.name());
            continue;
        };
        let out_path = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
        // Preserva bits de permissão (ex.: +x do llama-server) em Unix.
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> std::io::Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    // `unpack` já protege contra path traversal e preserva permissões.
    tar.unpack(dest)
}

// ---------------------------------------------------------------------------
// Layout: achatamento da raiz que contém o llama-server
// ---------------------------------------------------------------------------

/// Localiza recursivamente (BFS, o mais raso primeiro) o diretório que
/// contém o `llama-server(.exe)` dentro da árvore extraída.
fn find_server_root(dir: &Path) -> Option<PathBuf> {
    let exe_name = crate::server_exe_name();
    let mut queue = VecDeque::from([dir.to_path_buf()]);
    while let Some(current) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push_back(path);
            } else if entry.file_name().to_string_lossy() == exe_name {
                return Some(current);
            }
        }
    }
    None
}

/// Move o conteúdo (arquivos e subdiretórios) de `src` para dentro de `dst`.
fn move_dir_contents(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        std::fs::rename(entry.path(), dst.join(entry.file_name()))?;
    }
    Ok(())
}

/// Move todos os ARQUIVOS da árvore `src` (recursivamente) direto para a
/// raiz de `dst` — usado para garantir as DLLs do cudart ao lado do exe.
fn move_files_flat(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            move_files_flat(&path, dst)?;
        } else {
            std::fs::rename(&path, dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Sufixo único para o diretório de sessão no tmp.
fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Escreve um zip com estrutura aninhada (como os pacotes reais do
    /// llama.cpp) contendo um llama-server fake.
    fn write_nested_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        let exe = crate::server_exe_name();

        zip.add_directory("llama-b1-bin/build/bin/", opts).unwrap();
        zip.start_file(format!("llama-b1-bin/build/bin/{exe}"), opts)
            .unwrap();
        zip.write_all(b"fake-server-binary").unwrap();
        zip.start_file("llama-b1-bin/build/bin/ggml-base.dll", opts)
            .unwrap();
        zip.write_all(b"fake-dll").unwrap();
        // Arquivo fora da raiz do servidor: não deve ir para o destino.
        zip.start_file("llama-b1-bin/LICENSE", opts).unwrap();
        zip.write_all(b"mit").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn zip_extraction_finds_and_flattens_server_root() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("pkg.zip");
        write_nested_zip(&zip_path);

        let extract = tmp.path().join("extract");
        extract_archive(&zip_path, "pkg.zip", &extract).unwrap();

        let root = find_server_root(&extract).expect("raiz com llama-server");
        assert!(root.ends_with("llama-b1-bin/build/bin"));

        let staging = tmp.path().join("install");
        std::fs::create_dir_all(&staging).unwrap();
        move_dir_contents(&root, &staging).unwrap();

        let exe = staging.join(crate::server_exe_name());
        assert!(exe.is_file(), "llama-server deve estar na raiz do destino");
        assert_eq!(std::fs::read(&exe).unwrap(), b"fake-server-binary");
        assert!(staging.join("ggml-base.dll").is_file());
        // O LICENSE ficou acima da raiz do servidor e não é movido.
        assert!(!staging.join("LICENSE").exists());
    }

    #[test]
    fn zip_path_traversal_entries_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("../evil.txt", opts).unwrap();
        zip.write_all(b"pwned").unwrap();
        zip.start_file("ok.txt", opts).unwrap();
        zip.write_all(b"ok").unwrap();
        zip.finish().unwrap();

        let extract = tmp.path().join("extract");
        extract_archive(&zip_path, "evil.zip", &extract).unwrap();

        assert!(extract.join("ok.txt").is_file());
        assert!(
            !tmp.path().join("evil.txt").exists(),
            "entrada com ../ não pode escapar do destino"
        );
    }

    #[test]
    fn tar_gz_extraction_finds_server_root() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("pkg.tar.gz");
        let exe = crate::server_exe_name();

        let file = std::fs::File::create(&tar_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let data = b"fake-macos-server";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("llama-b1/bin/{exe}"), data.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let extract = tmp.path().join("extract");
        extract_archive(&tar_path, "pkg.tar.gz", &extract).unwrap();

        let root = find_server_root(&extract).expect("raiz com llama-server");
        assert!(root.ends_with("llama-b1/bin"));
        assert_eq!(std::fs::read(root.join(exe)).unwrap(), data);
    }

    #[test]
    fn cudart_dlls_are_flattened_next_to_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("cudart");
        std::fs::create_dir_all(src.join("nested/deep")).unwrap();
        std::fs::write(src.join("cudart64_12.dll"), b"a").unwrap();
        std::fs::write(src.join("nested/deep/cublas64_12.dll"), b"b").unwrap();

        let dst = tmp.path().join("install");
        std::fs::create_dir_all(&dst).unwrap();
        move_files_flat(&src, &dst).unwrap();

        assert!(dst.join("cudart64_12.dll").is_file());
        assert!(dst.join("cublas64_12.dll").is_file());
        assert!(!dst.join("nested").exists());
    }

    #[test]
    fn digest_parsing_accepts_only_sha256_hex() {
        let hex = "a".repeat(64);
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{hex}")),
            Some(hex.clone())
        );
        // Normaliza para minúsculas.
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", hex.to_uppercase())),
            Some(hex)
        );
        assert_eq!(parse_sha256_digest("md5:abc"), None);
        assert_eq!(parse_sha256_digest("sha256:tooshort"), None);
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", "g".repeat(64))),
            None
        );
        assert_eq!(parse_sha256_digest(&"a".repeat(71)), None);
    }

    #[test]
    fn sha256_file_matches_known_vector() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("abc.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Teste live contra a API do GitHub — roda só com `--ignored`.
    #[tokio::test]
    #[ignore = "rede: consulta a API de releases do GitHub"]
    async fn live_release_digests_cover_pinned_assets() {
        let client = build_client().unwrap();
        let digests = fetch_release_digests(&client, crate::PINNED_TAG)
            .await
            .expect("API do GitHub indisponível ou rate-limited");
        let asset = crate::asset_name(crate::PINNED_TAG, BackendVariant::Cpu);
        assert!(
            digests.contains_key(&asset),
            "release pinada deve ter digest para {asset}"
        );
    }
}
