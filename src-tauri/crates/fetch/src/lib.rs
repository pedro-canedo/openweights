//! Baixar, verificar e extrair pacotes de release — a mecânica comum a todo
//! artefato que o app instala (runtime do llama.cpp, Node portátil, Traefik).
//!
//! Nasceu extraído de `lr_runtime`, que era o único a fazer isso. A fronteira
//! escolhida é "bytes e arquivos": este crate não conhece llama.cpp, Node nem
//! o formato de evento que a UI espera — o progresso sai por callback cru e
//! quem chama traduz para o seu próprio enum. Foi o que permitiu reusar o
//! mesmo código em três instaladores sem que um contamine o outro.
//!
//! O padrão de instalação que ele suporta é sempre o mesmo, e é ele que
//! garante que uma instalação interrompida não deixa meia-árvore no destino:
//! todo trabalho acontece num diretório de sessão temporário e o último passo
//! é um único `fs::rename`.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Intervalo mínimo entre callbacks de progresso.
///
/// Sem isto um download de 300 MB dispara dezenas de milhares de eventos de
/// IPC e a UI engasga — o gargalo vira o próprio relatório de progresso.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

/// Buffer de escrita do download. Os assets vão de 30 MB a 380 MB; nunca
/// carregar o arquivo inteiro em memória.
const DOWNLOAD_BUFFER: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("verificação falhou: {0}")]
    Verification(String),
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// Cliente com User-Agent próprio — a API do GitHub recusa requisições sem um.
pub fn client(user_agent: &str) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(30))
        .build()
}

/// Baixa `url` em streaming para `dest`, chamando `on_progress(recebidos, total)`
/// com throttle.
///
/// `total` é 0 quando o servidor não manda `content-length` — quem exibe
/// precisa tratar esse caso em vez de dividir por zero. Emite um último
/// progresso sem throttle para a barra fechar em 100%.
pub async fn download_to(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(), FetchError> {
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
            on_progress(received_bytes, total_bytes);
            last_emit = Some(Instant::now());
        }
    }
    writer.flush()?;

    on_progress(received_bytes, total_bytes);
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

/// Digests SHA256 de todos os assets de uma release do GitHub.
///
/// `repo` é `"dono/projeto"`. Retorna `None` se a API falhar (403 por rate
/// limit é o caso comum em CI e em rede compartilhada) — quem chama decide se
/// isso é fatal. Para o runtime do llama.cpp não é, porque bloquear a
/// instalação por rate limit do GitHub seria pior que o risco que se evita.
pub async fn github_release_digests(
    client: &reqwest::Client,
    repo: &str,
    tag: &str,
) -> Option<HashMap<String, String>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
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
                "não foi possível obter digests da release {repo}@{tag} ({e}); \
                 prosseguindo sem verificação SHA256"
            );
            None
        }
    }
}

/// Extrai o hex de um digest `"sha256:<hex>"` (64 dígitos), em minúsculas.
pub fn parse_sha256_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?.trim();
    if !is_sha256_hex(hex) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

/// Lê um `SHASUMS256.txt` no formato do nodejs.org: `<hex>  <arquivo>`,
/// separados por DOIS espaços.
///
/// Ignora entradas em subpasta (`win-x64/node.exe`): o mesmo arquivo aparece
/// solto e dentro de diretório, e só o nome solto casa com o asset que
/// baixamos. Linhas malformadas são puladas em silêncio — um checksum a menos
/// vira "sem verificação" lá na frente, não um erro de parse aqui.
pub fn parse_shasums256(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let Some((hex, name)) = line.split_once("  ") else {
            continue;
        };
        let (hex, name) = (hex.trim(), name.trim());
        if !is_sha256_hex(hex) || name.is_empty() || name.contains('/') || name.contains('\\') {
            continue;
        }
        map.insert(name.to_string(), hex.to_ascii_lowercase());
    }
    map
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// SHA256 de um arquivo em streaming (nunca carrega tudo em memória).
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Verifica `path` contra o digest esperado para `asset`.
///
/// Digest indisponível **não** é erro (já logado na origem); divergência é.
pub async fn verify_sha256(
    path: &Path,
    asset: &str,
    digests: Option<&HashMap<String, String>>,
) -> Result<(), FetchError> {
    let Some(digests) = digests else {
        return Ok(());
    };
    let Some(expected) = digests.get(asset) else {
        log::warn!("digest ausente para {asset}; prosseguindo sem verificação");
        return Ok(());
    };
    verify_sha256_strict(path, asset, expected).await
}

/// Igual à anterior, mas o digest é obrigatório: divergência **e** ausência
/// são erro. É o modo usado no Node, onde o `SHASUMS256.txt` sempre vem junto
/// e não há desculpa para instalar sem conferir.
pub async fn verify_sha256_strict(
    path: &Path,
    asset: &str,
    expected: &str,
) -> Result<(), FetchError> {
    let file = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&file))
        .await
        .map_err(std::io::Error::other)??;
    if actual != expected.to_ascii_lowercase() {
        return Err(FetchError::Verification(format!(
            "SHA256 divergente para {asset}: esperado {expected}, obtido {actual}"
        )));
    }
    log::info!("SHA256 verificado para {asset}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Extração
// ---------------------------------------------------------------------------

/// Extrai `archive` (formato deduzido pelo nome do `asset`) para `dest`, em
/// thread de blocking para não travar o runtime async.
pub async fn extract_archive_async(
    archive: PathBuf,
    asset: String,
    dest: PathBuf,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || extract_archive(&archive, &asset, &dest))
        .await
        .map_err(std::io::Error::other)?
}

pub fn extract_archive(archive: &Path, asset: &str, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    if asset.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if asset.ends_with(".tar.gz") || asset.ends_with(".tgz") {
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
        // Preserva bits de permissão (ex.: +x do binário) em Unix.
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
// Layout: achar a raiz real e achatar
// ---------------------------------------------------------------------------

/// Localiza (BFS, o mais raso primeiro) o diretório que contém `file_name`
/// dentro da árvore extraída.
///
/// Existe porque os pacotes não têm estrutura previsível: o llama.cpp aninha
/// em `build/bin/`, o Node em `node-vX-plataforma/bin/`, o npm em tarball tem
/// prefixo `package/`. Procurar pelo arquivo que importa é mais robusto que
/// codificar o prefixo de cada um.
pub fn find_dir_containing(root: &Path, file_name: &str) -> Option<PathBuf> {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(current) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push_back(path);
            } else if entry.file_name().to_string_lossy() == file_name {
                return Some(current);
            }
        }
    }
    None
}

/// Move o conteúdo (arquivos e subdiretórios) de `src` para dentro de `dst`.
pub fn move_dir_contents(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        std::fs::rename(entry.path(), dst.join(entry.file_name()))?;
    }
    Ok(())
}

/// Move todos os ARQUIVOS da árvore `src` (recursivamente) direto para a raiz
/// de `dst` — usado quando as bibliotecas precisam ficar ao lado do binário.
pub fn move_files_flat(src: &Path, dst: &Path) -> std::io::Result<()> {
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

/// Soma recursiva dos tamanhos de arquivo sob `dir` (sanidade pós-extração).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut queue = VecDeque::from([dir.to_path_buf()]);
    while let Some(d) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                queue.push_back(p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Publica `staging` como `final_dir` num único `fs::rename`.
///
/// É este passo que torna a instalação atômica: ou o destino não existe, ou
/// existe completo. Um destino preexistente é removido antes — é sobra de
/// instalação quebrada, e o `rename` falharia por cima dela.
pub fn install_atomically(staging: &Path, final_dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if final_dir.exists() {
        remove_dir_all_retrying(final_dir)?;
    }
    std::fs::rename(staging, final_dir)
}

/// `remove_dir_all` com repetição.
///
/// No Windows, apagar árvores grandes (um `node_modules` tem dezenas de
/// milhares de arquivos) falha de forma intermitente: o antivírus ou o
/// indexador ainda seguram um handle recém-fechado. Repetir com pausa resolve
/// a esmagadora maioria dos casos; o resto vira erro visível, com o caminho.
pub fn remove_dir_all_retrying(dir: &Path) -> std::io::Result<()> {
    const TENTATIVAS: u32 = 5;
    let mut ultimo = None;
    for tentativa in 0..TENTATIVAS {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                log::warn!(
                    "falha ao remover {} (tentativa {}/{TENTATIVAS}): {e}",
                    dir.display(),
                    tentativa + 1
                );
                ultimo = Some(e);
                std::thread::sleep(Duration::from_millis(150 * u64::from(tentativa + 1)));
            }
        }
    }
    Err(ultimo.unwrap_or_else(|| std::io::Error::other("remoção falhou sem erro registrado")))
}

// ---------------------------------------------------------------------------
// Sessão temporária
// ---------------------------------------------------------------------------

/// Diretório de trabalho de uma instalação, em `<root>/.tmp/job-<pid>-<nanos>`.
///
/// Tudo — o `.part` do download, a árvore extraída, o staging — vive aqui, de
/// modo que jobs concorrentes não colidem e a limpeza é um `rmdir` só. O
/// `Drop` limpa em qualquer desfecho, inclusive em erro e em `?` no meio.
pub struct Session {
    path: PathBuf,
}

impl Session {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        let path = root.join(".tmp").join(format!("job-{}", unique_suffix()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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

    #[test]
    fn parse_sha256_digest_accepts_the_github_format() {
        let hex = "a".repeat(64);
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{hex}")),
            Some(hex.clone())
        );
    }

    #[test]
    fn parse_sha256_digest_normalises_to_lowercase() {
        let hex = "A".repeat(64);
        assert_eq!(parse_sha256_digest(&format!("sha256:{hex}")), Some("a".repeat(64)));
    }

    #[test]
    fn parse_sha256_digest_rejects_a_short_hex() {
        assert_eq!(parse_sha256_digest("sha256:abc"), None);
    }

    #[test]
    fn parse_sha256_digest_rejects_another_algorithm() {
        assert_eq!(parse_sha256_digest(&format!("sha512:{}", "a".repeat(64))), None);
    }

    #[test]
    fn parse_shasums256_reads_the_nodejs_two_space_format() {
        let hex = "c".repeat(64);
        let txt = format!("{hex}  node-v22.20.0-linux-x64.tar.gz\n");
        let map = parse_shasums256(&txt);
        assert_eq!(map.get("node-v22.20.0-linux-x64.tar.gz"), Some(&hex));
    }

    /// O mesmo arquivo aparece solto e dentro de diretório; só o nome solto
    /// casa com o asset que baixamos.
    #[test]
    fn parse_shasums256_ignores_entries_in_subdirectories() {
        let hex = "d".repeat(64);
        let txt = format!("{hex}  win-x64/node.exe\n{hex}  node-v22.20.0-win-x64.zip\n");
        let map = parse_shasums256(&txt);
        assert!(!map.contains_key("win-x64/node.exe"));
        assert!(map.contains_key("node-v22.20.0-win-x64.zip"));
    }

    #[test]
    fn parse_shasums256_skips_malformed_lines() {
        let map = parse_shasums256("lixo\n\nabc  curto.tar.gz\n");
        assert!(map.is_empty());
    }

    /// Os pacotes reais aninham o binário; achar por nome evita codificar o
    /// prefixo de cada distribuição.
    #[test]
    fn extracting_a_nested_zip_finds_the_directory_with_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pacote.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.add_directory("raiz/build/bin/", opts).unwrap();
            zip.start_file("raiz/build/bin/alvo.bin", opts).unwrap();
            zip.write_all(b"conteudo").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.path().join("saida");
        extract_archive(&archive, "pacote.zip", &dest).unwrap();

        let root = find_dir_containing(&dest, "alvo.bin").unwrap();
        assert!(root.join("alvo.bin").is_file());
    }

    #[test]
    fn zip_path_traversal_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"pwned").unwrap();
            zip.start_file("ok.txt", opts).unwrap();
            zip.write_all(b"ok").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.path().join("saida");
        extract_archive(&archive, "evil.zip", &dest).unwrap();

        assert!(dest.join("ok.txt").is_file());
        assert!(
            !dir.path().join("evil.txt").exists(),
            "entrada com ../ não pode escapar do destino"
        );
    }

    #[test]
    fn an_unknown_archive_format_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pacote.rar");
        std::fs::write(&archive, b"x").unwrap();
        let err = extract_archive(&archive, "pacote.rar", &dir.path().join("saida")).unwrap_err();
        assert!(err.to_string().contains("desconhecido"));
    }

    #[test]
    fn find_dir_containing_returns_none_when_the_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        assert_eq!(find_dir_containing(dir.path(), "ausente.bin"), None);
    }

    #[test]
    fn install_atomically_replaces_a_broken_previous_install() {
        let dir = tempfile::tempdir().unwrap();
        let destino = dir.path().join("destino");
        std::fs::create_dir_all(&destino).unwrap();
        std::fs::write(destino.join("sobra.txt"), b"antigo").unwrap();

        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("novo.txt"), b"novo").unwrap();

        install_atomically(&staging, &destino).unwrap();
        assert!(destino.join("novo.txt").is_file());
        assert!(!destino.join("sobra.txt").exists());
    }

    #[test]
    fn move_files_flat_brings_nested_files_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let origem = dir.path().join("origem/a/b");
        std::fs::create_dir_all(&origem).unwrap();
        std::fs::write(origem.join("lib.dll"), b"x").unwrap();
        let destino = dir.path().join("destino");
        std::fs::create_dir_all(&destino).unwrap();

        move_files_flat(&dir.path().join("origem"), &destino).unwrap();
        assert!(destino.join("lib.dll").is_file());
    }

    #[test]
    fn dir_size_sums_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("sub/b.bin"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size(dir.path()), 150);
    }

    #[test]
    fn a_session_cleans_its_directory_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let caminho = {
            let session = Session::new(dir.path()).unwrap();
            let p = session.path().to_path_buf();
            std::fs::write(p.join("trabalho.part"), b"x").unwrap();
            assert!(p.is_dir());
            p
        };
        assert!(!caminho.exists());
    }

    #[test]
    fn removing_a_missing_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        remove_dir_all_retrying(&dir.path().join("nao-existe")).unwrap();
    }

    #[test]
    fn sha256_of_a_known_file_matches() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("v.txt");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn a_diverging_checksum_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("v.txt");
        std::fs::write(&f, b"abc").unwrap();
        let err = verify_sha256_strict(&f, "v.txt", &"0".repeat(64))
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Verification(_)));
    }

    /// Rate limit da API do GitHub não pode bloquear a instalação.
    #[tokio::test]
    async fn a_missing_digest_map_skips_verification() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("v.txt");
        std::fs::write(&f, b"abc").unwrap();
        verify_sha256(&f, "v.txt", None).await.unwrap();
    }
}
