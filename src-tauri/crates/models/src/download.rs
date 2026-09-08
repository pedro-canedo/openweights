//! Gerenciador de downloads com retomada.
//!
//! Estratégia (verificada ago/2026): HEAD no `/resolve/` (sem seguir o
//! redirect) dá tamanho/etag (`x-linked-size`/`x-linked-etag`); o GET no
//! mesmo `/resolve/` é seguido pelo reqwest até a CDN assinada, que aceita
//! `Range` (HTTP 206). O `Authorization` é removido automaticamente no
//! redirect cross-host (comportamento correto aqui) e o `Range` é
//! preservado. A URL assinada expira — cada tentativa re-resolve.
//!
//! Progresso persistido em `<arquivo>.part` + sidecar `<arquivo>.part.json`
//! (`{etag, totalBytes}`) para sobreviver a fechamento do app no meio do
//! download; na retomada o etag salvo é validado contra o atual (mudou →
//! recomeça do zero).
//!
//! Layout em disco: `<models_dir>/<author>/<repo>/<arquivo>`.

use crate::{ModelsError, RepoFile, resolve_url};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

/// Máximo de artefatos baixando ao mesmo tempo (semáforo global).
const MAX_CONCURRENT_ARTIFACTS: usize = 2;
/// Arquivos do MESMO artefato baixando ao mesmo tempo.
///
/// Um GGUF grande vem partido em shards, e baixá-los em fila deixava a linha
/// ociosa: são arquivos distintos, não precisam de `Range` nem de remontagem.
const MAX_ARQUIVOS_PARALELOS: usize = 3;
/// Faixas (`Range`) por arquivo. Ver [`conexoes`] para o porquê do número.
const MAX_RANGES_POR_ARQUIVO: usize = 8;
/// Teto de conexões de dados abertas pelo app, somando todos os downloads.
///
/// Medido contra a CDN de LFS do Hugging Face numa linha de 900 Mbps: 1
/// conexão dá 9,5 MB/s, 4 dão 22,6, 8 dão 41,3 e 16 dão 52,9. Depois de 16 a
/// curva achata e a variância cresce, então é aí que o teto fica — o ganho de
/// abrir mais não paga a disputa nem o risco de o servidor limitar por IP.
const MAX_CONEXOES: usize = 16;
/// De quanto em quanto tempo o progresso das faixas vai para o sidecar.
/// É o que se re-baixa se a energia cair: alguns segundos por faixa.
const SIDECAR_INTERVAL: Duration = Duration::from_secs(2);
/// Intervalo mínimo entre eventos `Update` durante o streaming.
const EVENT_THROTTLE: Duration = Duration::from_millis(250);
/// Janela deslizante para o cálculo de bytes/s.
const SPEED_WINDOW: Duration = Duration::from_secs(3);
/// Tentativas por arquivo (cada uma re-resolve a URL da CDN).
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Queued,
    Running,
    Paused,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub id: String,
    pub repo_id: String,
    pub artifact_name: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_sec: u64,
    pub state: DownloadState,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DownloadEvent {
    Update { status: DownloadStatus },
    Removed { id: String },
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub repo_id: String,
    pub artifact_name: String,
    pub files: Vec<RepoFile>,
    pub token: Option<String>,
}

/// Identificador estável de um download: `repo_id::artifact_name`.
pub fn download_id(repo_id: &str, artifact_name: &str) -> String {
    format!("{repo_id}::{artifact_name}")
}

/// Sidecar `<arquivo>.part.json`: metadados para validar a retomada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartMeta {
    etag: Option<String>,
    total_bytes: u64,
    /// Bytes já gravados em cada faixa, na ordem das faixas.
    ///
    /// Com uma conexão só o `.part` cresce no fim e o tamanho do arquivo já
    /// dizia tudo. Com faixas paralelas ele nasce do tamanho final e os
    /// buracos ficam no meio, então quem sabe o progresso é isto — e um
    /// `.part` de download antigo, sem esta chave, retoma como faixa única.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chunks: Vec<u64>,
}

/// Decisão pura de retomada, a partir do `.part` local e do etag atual.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeDecision {
    /// Recomeçar do zero (`.part` inválido, etag mudou ou não validável).
    Restart,
    /// Continuar: quantos bytes cada faixa já tem. Um único elemento é o
    /// download sequencial de sempre.
    Resume { done: Vec<u64> },
    /// O `.part` já tem todos os bytes; só falta validar e renomear.
    AlreadyComplete,
}

/// Regras: sem `.part` ou sem sidecar → zero; total ou etag divergente (ou
/// impossível de validar) → zero; progresso maior que o esperado → zero;
/// exatamente o esperado → renomear; senão → retomar as faixas.
///
/// O progresso vem do sidecar quando ele tem faixas, e do tamanho do `.part`
/// quando não tem. Confiar no tamanho nos dois casos seria um erro caro: o
/// `.part` de um download paralelo nasce pré-alocado no tamanho final, e
/// medi-lo diria "completo" com o meio do arquivo ainda vazio.
fn decide_resume(
    part_len: u64,
    sidecar: Option<&PartMeta>,
    current_etag: Option<&str>,
    expected_size: u64,
) -> ResumeDecision {
    if part_len == 0 {
        return ResumeDecision::Restart;
    }
    let Some(meta) = sidecar else {
        return ResumeDecision::Restart;
    };
    if meta.total_bytes != expected_size {
        return ResumeDecision::Restart;
    }
    match (&meta.etag, current_etag) {
        (Some(saved), Some(cur)) if normalize_etag(saved) == normalize_etag(cur) => {}
        _ => return ResumeDecision::Restart,
    }
    // Sem faixas no sidecar: `.part` sequencial, do formato anterior.
    let done = if meta.chunks.is_empty() {
        vec![part_len]
    } else {
        // Uma faixa maior que a sua fatia é sidecar corrompido, não progresso.
        let limites = faixas(expected_size, meta.chunks.len());
        if limites.len() != meta.chunks.len()
            || limites
                .iter()
                .zip(&meta.chunks)
                .any(|((ini, fim), d)| *d > fim - ini)
        {
            return ResumeDecision::Restart;
        }
        meta.chunks.clone()
    };
    let total: u64 = done.iter().sum();
    if total > expected_size {
        return ResumeDecision::Restart;
    }
    if total == expected_size {
        return ResumeDecision::AlreadyComplete;
    }
    ResumeDecision::Resume { done }
}

/// Divide `total` em `n` faixas contíguas `[início, fim)`.
fn faixas(total: u64, n: usize) -> Vec<(u64, u64)> {
    let n = n.max(1) as u64;
    let passo = total / n;
    (0..n)
        .map(|i| {
            let ini = i * passo;
            let fim = if i + 1 == n { total } else { ini + passo };
            (ini, fim)
        })
        .collect()
}

/// Quantas conexões abrir para um arquivo.
///
/// Uma conexão TCP entrega no máximo `janela ÷ RTT`, e o RTT até a CDN de LFS
/// do Hugging Face é de ~133 ms daqui — daí os 9,5 MB/s de uma conexão só,
/// numa linha que dá 113. O paralelismo é o que fecha essa distância.
///
/// O piso por faixa existe porque uma conexão nova passa os primeiros ~10
/// RTT acelerando: abaixo de uns 16 MiB ela termina antes de chegar à
/// velocidade de regime, e o handshake custa mais do que a faixa rende.
fn conexoes(total: u64) -> usize {
    const MINIMO_POR_FAIXA: u64 = 16 * 1024 * 1024;
    ((total / MINIMO_POR_FAIXA) as usize).clamp(1, MAX_RANGES_POR_ARQUIVO)
}

/// Remove `W/` e aspas de um etag para comparação estável.
fn normalize_etag(etag: &str) -> &str {
    etag.trim_start_matches("W/").trim_matches('"')
}

/// Caminho de destino: `<models_dir>/<author>/<repo>/<arquivo>` — `repo_id`
/// tem formato `author/name`; sem `/` vira um nível único. Componentes
/// vazios, `.`/`..` ou com `\` são rejeitados (proteção contra travessia).
fn dest_path(models_dir: &Path, repo_id: &str, filename: &str) -> Result<PathBuf, ModelsError> {
    let mut out = models_dir.to_path_buf();
    for part in repo_id.split('/').chain(filename.split('/')) {
        // `:` cobre prefixo de drive do Windows ("C:evil.gguf") e ADS NTFS.
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('\\')
            || part.contains(':')
        {
            return Err(ModelsError::Api(format!(
                "caminho de destino inválido: {repo_id}/{filename}"
            )));
        }
        out.push(part);
    }
    Ok(out)
}

fn part_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".part")
}

fn part_meta_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".part.json")
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Índice do artefato incompleto: sobrevive a fechar o app / reiniciar o PC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteRecord {
    repo_id: String,
    artifact_name: String,
    files: Vec<RepoFile>,
}

fn owdl_path(
    models_dir: &Path,
    repo_id: &str,
    artifact_name: &str,
) -> Result<PathBuf, ModelsError> {
    Ok(append_suffix(
        &dest_path(models_dir, repo_id, artifact_name)?,
        ".owdl.json",
    ))
}

fn persist_incomplete(models_dir: &Path, req: &DownloadRequest) {
    let Ok(path) = owdl_path(models_dir, &req.repo_id, &req.artifact_name) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let rec = IncompleteRecord {
        repo_id: req.repo_id.clone(),
        artifact_name: req.artifact_name.clone(),
        files: req.files.clone(),
    };
    match serde_json::to_vec_pretty(&rec) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                log::warn!("não gravou índice de download incompleto: {e}");
            }
        }
        Err(e) => log::warn!("não serializou índice de download incompleto: {e}"),
    }
}

fn clear_incomplete(models_dir: &Path, repo_id: &str, artifact_name: &str) {
    if let Ok(path) = owdl_path(models_dir, repo_id, artifact_name) {
        let _ = std::fs::remove_file(path);
    }
}

fn bytes_on_disk(models_dir: &Path, repo_id: &str, file: &RepoFile) -> u64 {
    let Ok(dest) = dest_path(models_dir, repo_id, &file.path) else {
        return 0;
    };
    if let Ok(meta) = std::fs::metadata(&dest) {
        return meta.len();
    }
    std::fs::metadata(part_path(&dest))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn job_from_record(models_dir: &Path, rec: IncompleteRecord) -> Option<Job> {
    if rec.files.is_empty() {
        return None;
    }
    let all_done = rec.files.iter().all(|f| {
        dest_path(models_dir, &rec.repo_id, &f.path)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .is_some_and(|m| m.is_file() && m.len() == f.size_bytes)
    });
    if all_done {
        clear_incomplete(models_dir, &rec.repo_id, &rec.artifact_name);
        return None;
    }
    let received: u64 = rec
        .files
        .iter()
        .map(|f| bytes_on_disk(models_dir, &rec.repo_id, f))
        .sum();
    let total: u64 = rec.files.iter().map(|f| f.size_bytes).sum();
    let id = download_id(&rec.repo_id, &rec.artifact_name);
    Some(Job {
        req: DownloadRequest {
            repo_id: rec.repo_id.clone(),
            artifact_name: rec.artifact_name.clone(),
            files: rec.files,
            token: None,
        },
        status: DownloadStatus {
            id,
            repo_id: rec.repo_id,
            artifact_name: rec.artifact_name,
            received_bytes: received,
            total_bytes: total,
            bytes_per_sec: 0,
            state: DownloadState::Paused,
            error: None,
        },
        received: Arc::new(AtomicU64::new(received)),
        speed: Arc::new(AtomicU64::new(0)),
        cancel: CancellationToken::new(),
        handle: None,
    })
}

/// Reconstrói jobs pausados a partir dos índices `.owdl.json` e de `.part`
/// órfãos (downloads de versões anteriores, sem índice).
fn recover_jobs(models_dir: &Path) -> BTreeMap<String, Job> {
    let mut jobs = BTreeMap::new();
    recover_dir(models_dir, models_dir, "", &mut jobs);
    let Ok(authors) = std::fs::read_dir(models_dir) else {
        return jobs;
    };
    for author in authors.flatten() {
        if !author.path().is_dir() {
            continue;
        }
        let author_name = author.file_name().to_string_lossy().into_owned();
        let Ok(repos) = std::fs::read_dir(author.path()) else {
            continue;
        };
        for repo in repos.flatten() {
            if !repo.path().is_dir() {
                continue;
            }
            let repo_id = format!("{author_name}/{}", repo.file_name().to_string_lossy());
            recover_dir(models_dir, &repo.path(), &repo_id, &mut jobs);
        }
    }
    jobs
}

fn recover_dir(models_dir: &Path, dir: &Path, repo_id: &str, jobs: &mut BTreeMap<String, Job>) {
    if repo_id.is_empty() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut covered: HashSet<String> = HashSet::new();
    let mut orphans: Vec<String> = Vec::new();

    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".owdl.json") {
            let Ok(bytes) = std::fs::read(e.path()) else {
                continue;
            };
            let Ok(mut rec) = serde_json::from_slice::<IncompleteRecord>(&bytes) else {
                continue;
            };
            if rec.repo_id.is_empty() {
                rec.repo_id = repo_id.to_string();
            }
            let files = rec.files.clone();
            if let Some(job) = job_from_record(models_dir, rec) {
                for f in &files {
                    covered.insert(f.path.clone());
                }
                jobs.insert(job.status.id.clone(), job);
            }
        } else if let Some(dest) = name.strip_suffix(".part")
            && dest.ends_with(".gguf")
        {
            orphans.push(dest.to_string());
        }
    }

    for dest_name in orphans {
        if covered.contains(&dest_name) {
            continue;
        }
        let dest = dir.join(&dest_name);
        let part = part_path(&dest);
        let part_len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if part_len == 0 {
            continue;
        }
        let sidecar: Option<PartMeta> = std::fs::read(part_meta_path(&dest))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let total = sidecar
            .as_ref()
            .map(|m| m.total_bytes)
            .filter(|n| *n > 0)
            .unwrap_or(part_len);
        let rec = IncompleteRecord {
            repo_id: repo_id.to_string(),
            artifact_name: dest_name.clone(),
            files: vec![RepoFile {
                path: dest_name,
                size_bytes: total,
            }],
        };
        if let Some(job) = job_from_record(models_dir, rec) {
            jobs.entry(job.status.id.clone()).or_insert(job);
        }
    }
}

fn header_str(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Um job na fila (o `DownloadStatus` é a visão pública dele).
struct Job {
    req: DownloadRequest,
    status: DownloadStatus,
    /// Bytes recebidos do artefato inteiro (compartilhado com a task).
    received: Arc<AtomicU64>,
    /// Velocidade instantânea (janela deslizante), em bytes/s.
    speed: Arc<AtomicU64>,
    cancel: CancellationToken,
    handle: Option<tokio::task::JoinHandle<()>>,
}

fn snapshot_of(job: &Job) -> DownloadStatus {
    let mut s = job.status.clone();
    s.received_bytes = job.received.load(Ordering::Relaxed);
    s.bytes_per_sec = job.speed.load(Ordering::Relaxed);
    s
}

/// Re-enfileira um job existente (retomada ou nova tentativa).
fn prepare_restart(job: &mut Job) {
    job.status.state = DownloadState::Queued;
    job.status.error = None;
    job.received.store(0, Ordering::Relaxed);
    job.speed.store(0, Ordering::Relaxed);
    job.cancel = CancellationToken::new();
}

struct Inner {
    models_dir: PathBuf,
    /// Segue redirects: usado no GET de download (o reqwest remove o
    /// `Authorization` no redirect cross-host e preserva o `Range`).
    http: reqwest::Client,
    /// Não segue redirects: usado no HEAD do `/resolve/` para ler
    /// `x-linked-etag`/`x-linked-size` do 302.
    http_noredir: reqwest::Client,
    jobs: Mutex<BTreeMap<String, Job>>,
    events: broadcast::Sender<DownloadEvent>,
    sem: Semaphore,
    /// Teto global de conexões de dados: ver [`MAX_CONEXOES`].
    conexoes: Arc<Semaphore>,
}

impl Inner {
    /// Emite um `Update` com o snapshot atual do job (se ainda existir).
    async fn emit_update(&self, id: &str) {
        let snap = {
            let jobs = self.jobs.lock().await;
            jobs.get(id).map(snapshot_of)
        };
        if let Some(status) = snap {
            let _ = self.events.send(DownloadEvent::Update { status });
        }
    }

    /// Transição de estado + evento (sempre emite, sem throttle).
    async fn set_state(&self, id: &str, state: DownloadState, error: Option<String>) {
        {
            let mut jobs = self.jobs.lock().await;
            let Some(job) = jobs.get_mut(id) else {
                return;
            };
            job.status.state = state;
            job.status.error = error;
        }
        self.emit_update(id).await;
    }
}

/// Fila de downloads com eventos por broadcast (a UI assina via Tauri).
pub struct DownloadManager {
    inner: Arc<Inner>,
}

impl DownloadManager {
    pub fn new(models_dir: PathBuf) -> Self {
        let (events, _) = broadcast::channel(256);
        let ua = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));
        // HTTP/1.1 obrigatório para os bytes: sobre HTTP/2 o `reqwest`
        // multiplexa as requisições concorrentes ao mesmo host num ÚNICO cano
        // TCP, e oito faixas voltam a dividir a janela de uma conexão só —
        // todo o paralelismo vira enfeite. Com HTTP/1.1 o pool abre uma
        // conexão por faixa, que é o que a medição pede.
        let http = reqwest::Client::builder()
            .user_agent(ua)
            .http1_only()
            .pool_max_idle_per_host(MAX_CONEXOES)
            .build()
            .expect("reqwest client");
        let http_noredir = reqwest::Client::builder()
            .user_agent(ua)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        let jobs = recover_jobs(&models_dir);
        if !jobs.is_empty() {
            log::info!(
                "{} download(s) incompleto(s) recuperado(s) do disco",
                jobs.len()
            );
        }
        Self {
            inner: Arc::new(Inner {
                models_dir,
                http,
                http_noredir,
                jobs: Mutex::new(jobs),
                events,
                sem: Semaphore::new(MAX_CONCURRENT_ARTIFACTS),
                conexoes: Arc::new(Semaphore::new(MAX_CONEXOES)),
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.inner.events.subscribe()
    }

    /// Enfileira (ou retoma) o download de um artefato. Retorna o id.
    pub async fn enqueue(&self, req: DownloadRequest) -> Result<String, ModelsError> {
        let id = download_id(&req.repo_id, &req.artifact_name);
        // Valida os destinos antes de aceitar o job (nomes vêm da API).
        for f in &req.files {
            dest_path(&self.inner.models_dir, &req.repo_id, &f.path)?;
        }
        persist_incomplete(&self.inner.models_dir, &req);
        {
            let mut jobs = self.inner.jobs.lock().await;
            if let Some(job) = jobs.get_mut(&id) {
                match job.status.state {
                    // Já na fila ou baixando: idempotente.
                    DownloadState::Queued | DownloadState::Running => return Ok(id),
                    // Pausado/erro/feito: atualiza a requisição e re-enfileira.
                    _ => {
                        job.req = req;
                        job.status.total_bytes = job.req.files.iter().map(|f| f.size_bytes).sum();
                        prepare_restart(job);
                        job.handle = Some(spawn_job(self.inner.clone(), id.clone()));
                    }
                }
            } else {
                let total: u64 = req.files.iter().map(|f| f.size_bytes).sum();
                let status = DownloadStatus {
                    id: id.clone(),
                    repo_id: req.repo_id.clone(),
                    artifact_name: req.artifact_name.clone(),
                    received_bytes: 0,
                    total_bytes: total,
                    bytes_per_sec: 0,
                    state: DownloadState::Queued,
                    error: None,
                };
                let mut job = Job {
                    req,
                    status,
                    received: Arc::new(AtomicU64::new(0)),
                    speed: Arc::new(AtomicU64::new(0)),
                    cancel: CancellationToken::new(),
                    handle: None,
                };
                job.handle = Some(spawn_job(self.inner.clone(), id.clone()));
                jobs.insert(id.clone(), job);
            }
        }
        self.inner.emit_update(&id).await;
        Ok(id)
    }

    /// Para a task preservando o `.part` (estado `Paused`).
    pub async fn pause(&self, id: &str) -> Result<(), ModelsError> {
        let (token, handle) = {
            let mut jobs = self.inner.jobs.lock().await;
            let Some(job) = jobs.get_mut(id) else {
                return Err(ModelsError::Api(format!("download desconhecido: {id}")));
            };
            match job.status.state {
                DownloadState::Queued | DownloadState::Running => {}
                _ => return Ok(()),
            }
            let handle = job.handle.take();
            if handle.is_none() {
                // Outra pause()/cancel() concorrente já tomou o JoinHandle e
                // fará a transição de estado quando a task realmente morrer —
                // marcar Paused aqui permitiria um resume() duplicar a task.
                return Ok(());
            }
            (job.cancel.clone(), handle)
        };
        token.cancel();
        if let Some(h) = handle {
            let _ = h.await;
        }
        {
            let mut jobs = self.inner.jobs.lock().await;
            if let Some(job) = jobs.get_mut(id) {
                // A task pode ter terminado (Done/Error) na corrida; só
                // marca Paused se ainda estava em andamento.
                if matches!(
                    job.status.state,
                    DownloadState::Queued | DownloadState::Running
                ) {
                    job.status.state = DownloadState::Paused;
                    job.speed.store(0, Ordering::Relaxed);
                }
            }
        }
        self.inner.emit_update(id).await;
        Ok(())
    }

    /// Revalida (etag) e continua com `Range` de onde o `.part` parou.
    pub async fn resume(&self, id: &str, token: Option<String>) -> Result<(), ModelsError> {
        {
            let mut jobs = self.inner.jobs.lock().await;
            let Some(job) = jobs.get_mut(id) else {
                return Err(ModelsError::Api(format!("download desconhecido: {id}")));
            };
            match job.status.state {
                DownloadState::Paused | DownloadState::Error => {}
                _ => return Ok(()),
            }
            if token.is_some() {
                job.req.token = token;
            }
            persist_incomplete(&self.inner.models_dir, &job.req);
            prepare_restart(job);
            job.handle = Some(spawn_job(self.inner.clone(), id.to_string()));
        }
        self.inner.emit_update(id).await;
        Ok(())
    }

    /// Aborta e apaga `.part`, sidecar e o que já foi baixado do artefato.
    pub async fn cancel(&self, id: &str) -> Result<(), ModelsError> {
        let job = self.inner.jobs.lock().await.remove(id);
        let Some(mut job) = job else {
            return Err(ModelsError::Api(format!("download desconhecido: {id}")));
        };
        job.cancel.cancel();
        if let Some(h) = job.handle.take() {
            let _ = h.await;
        }
        for f in &job.req.files {
            if let Ok(dest) = dest_path(&self.inner.models_dir, &job.req.repo_id, &f.path) {
                let _ = tokio::fs::remove_file(part_path(&dest)).await;
                let _ = tokio::fs::remove_file(part_meta_path(&dest)).await;
                let _ = tokio::fs::remove_file(&dest).await;
            }
        }
        clear_incomplete(
            &self.inner.models_dir,
            &job.req.repo_id,
            &job.req.artifact_name,
        );
        let _ = self
            .inner
            .events
            .send(DownloadEvent::Removed { id: id.to_string() });
        Ok(())
    }

    /// Estado atual de todos os jobs (inclusive Done/Error da sessão).
    pub async fn list(&self) -> Vec<DownloadStatus> {
        self.inner
            .jobs
            .lock()
            .await
            .values()
            .map(snapshot_of)
            .collect()
    }
}

/// Resultado interno de uma task: `Interrupted` = pause/cancel cuidam do
/// estado; a task não o sobrescreve.
enum Outcome {
    Completed,
    Interrupted,
}

fn spawn_job(inner: Arc<Inner>, id: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (req, received, speed, token) = {
            let jobs = inner.jobs.lock().await;
            let Some(job) = jobs.get(&id) else { return };
            (
                job.req.clone(),
                job.received.clone(),
                job.speed.clone(),
                job.cancel.clone(),
            )
        };

        // Semáforo global: no máximo N artefatos simultâneos; os demais
        // ficam em Queued até liberar vaga.
        let permit = tokio::select! {
            p = inner.sem.acquire() => match p {
                Ok(p) => p,
                Err(_) => return,
            },
            _ = token.cancelled() => return,
        };
        if token.is_cancelled() {
            return;
        }

        inner.set_state(&id, DownloadState::Running, None).await;

        let result = run_artifact(&inner, &id, &req, &received, &speed, &token).await;
        drop(permit);
        speed.store(0, Ordering::Relaxed);

        match result {
            Ok(Outcome::Completed) => {
                clear_incomplete(&inner.models_dir, &req.repo_id, &req.artifact_name);
                inner.set_state(&id, DownloadState::Done, None).await;
            }
            Ok(Outcome::Interrupted) => {
                // pause()/cancel() definem o estado final e emitem eventos.
            }
            Err(e) => {
                log::warn!("download {id} falhou: {e}");
                inner
                    .set_state(&id, DownloadState::Error, Some(e.to_string()))
                    .await;
            }
        }
    })
}

/// Baixa os arquivos do artefato, alguns ao mesmo tempo. `.part` fica no
/// disco em caso de erro (retomada futura via `resume`).
///
/// O contador de progresso deixa de ser "soma dos anteriores + o atual": com
/// arquivos concorrentes não existe "o atual". Cada tarefa soma o que gravou
/// ao total, e desconta o que já tinha somado quando uma tentativa recomeça
/// do zero — assim a barra nunca anda para trás sozinha nem conta duas vezes.
async fn run_artifact(
    inner: &Inner,
    id: &str,
    req: &DownloadRequest,
    received: &AtomicU64,
    speed: &AtomicU64,
    token: &CancellationToken,
) -> Result<Outcome, ModelsError> {
    received.store(0, Ordering::Relaxed);

    // Já no disco: entra no total sem ocupar uma vaga de download.
    let mut pendentes = Vec::new();
    for file in &req.files {
        let dest = dest_path(&inner.models_dir, &req.repo_id, &file.path)?;
        if let Ok(meta) = tokio::fs::metadata(&dest).await
            && meta.is_file()
            && meta.len() == file.size_bytes
        {
            received.fetch_add(file.size_bytes, Ordering::Relaxed);
            continue;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        pendentes.push((file, dest));
    }

    // Os futures são montados num laço, e não por uma closure dada ao `map`:
    // uma closure assíncrona precisaria valer para qualquer lifetime, e a que
    // empresta `inner` e `req` não vale.
    let mut futuros = Vec::with_capacity(pendentes.len());
    for (file, dest) in pendentes {
        futuros.push(async move {
            download_file(inner, id, req, file, &dest, received, speed, token).await
        });
    }
    let resultados: Vec<Result<Outcome, ModelsError>> = stream::iter(futuros)
        .buffer_unordered(MAX_ARQUIVOS_PARALELOS)
        .collect()
        .await;

    // Um erro vale mais que uma interrupção: quem cancelou já sabe que
    // cancelou, e quem falhou precisa ler o motivo.
    let mut interrompido = false;
    for r in resultados {
        match r? {
            Outcome::Interrupted => interrompido = true,
            Outcome::Completed => {}
        }
    }
    Ok(if interrompido {
        Outcome::Interrupted
    } else {
        Outcome::Completed
    })
}

/// Loop de tentativas de um arquivo: cada tentativa re-resolve (a URL da
/// CDN expira) e retoma do tamanho atual do `.part`.
#[allow(clippy::too_many_arguments)]
async fn download_file(
    inner: &Inner,
    id: &str,
    req: &DownloadRequest,
    file: &RepoFile,
    dest: &Path,
    received: &AtomicU64,
    speed: &AtomicU64,
    token: &CancellationToken,
) -> Result<Outcome, ModelsError> {
    let url = resolve_url(&req.repo_id, &file.path);
    let mut force_restart = false;
    let mut last_err: Option<ModelsError> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        if token.is_cancelled() {
            return Ok(Outcome::Interrupted);
        }
        if attempt > 1 {
            // Backoff simples antes de re-resolver.
            let wait = Duration::from_millis(500 * u64::from(attempt));
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = token.cancelled() => return Ok(Outcome::Interrupted),
            }
        }
        match try_download_once(
            inner,
            id,
            req,
            file,
            dest,
            &url,
            received,
            speed,
            token,
            &mut force_restart,
        )
        .await
        {
            Ok(out) => return Ok(out),
            // Gated não é transitório: sem licença/token não adianta tentar.
            Err(e @ ModelsError::Gated) => return Err(e),
            Err(e) => {
                log::warn!("tentativa {attempt}/{MAX_ATTEMPTS} de {url} falhou: {e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| ModelsError::Api("download esgotou as tentativas".to_string())))
}

/// Uma tentativa completa: HEAD no resolve → decisão de retomada → GET com
/// `Range` → streaming para o `.part` → validação de tamanho → rename.
#[allow(clippy::too_many_arguments)]
async fn try_download_once(
    inner: &Inner,
    id: &str,
    req: &DownloadRequest,
    file: &RepoFile,
    dest: &Path,
    url: &str,
    received: &AtomicU64,
    speed: &AtomicU64,
    token: &CancellationToken,
    force_restart: &mut bool,
) -> Result<Outcome, ModelsError> {
    let part = part_path(dest);
    let meta_path = part_meta_path(dest);

    // 1) HEAD no /resolve/ sem seguir o redirect: o 302 traz o etag e o
    //    tamanho reais (x-linked-*); 200 direto acontece em arquivos
    //    pequenos fora do LFS.
    let mut head = inner.http_noredir.head(url);
    if let Some(t) = &req.token {
        head = head.bearer_auth(t);
    }
    let resp = tokio::select! {
        r = head.send() => r?,
        _ = token.cancelled() => return Ok(Outcome::Interrupted),
    };
    let status = resp.status();
    let current_etag = if status.is_redirection() {
        if let Some(size) = header_str(&resp, "x-linked-size").and_then(|s| s.parse::<u64>().ok())
            && size != file.size_bytes
        {
            return Err(ModelsError::Api(format!(
                "tamanho no servidor ({size} B) difere do esperado ({} B); recarregue a lista de arquivos do repositório",
                file.size_bytes
            )));
        }
        header_str(&resp, "x-linked-etag")
    } else if status.is_success() {
        header_str(&resp, "etag")
    } else if matches!(status.as_u16(), 401 | 403) {
        return Err(ModelsError::Gated);
    } else {
        return Err(ModelsError::Api(format!("resolve retornou HTTP {status}")));
    };

    // 2) Decide retomar ou recomeçar com base no .part + sidecar.
    let part_len = tokio::fs::metadata(&part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let sidecar: Option<PartMeta> = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(_) => None,
    };
    let decision = if std::mem::take(force_restart) {
        ResumeDecision::Restart
    } else {
        decide_resume(
            part_len,
            sidecar.as_ref(),
            current_etag.as_deref(),
            file.size_bytes,
        )
    };
    let done = match decision {
        ResumeDecision::AlreadyComplete => {
            finalize(dest, &part, &meta_path, file.size_bytes).await?;
            received.fetch_add(file.size_bytes, Ordering::Relaxed);
            return Ok(Outcome::Completed);
        }
        ResumeDecision::Restart => {
            let _ = tokio::fs::remove_file(&part).await;
            vec![0; conexoes(file.size_bytes)]
        }
        ResumeDecision::Resume { done } => done,
    };
    let limites = faixas(file.size_bytes, done.len());

    // 3) O `.part` nasce do tamanho final: as faixas escrevem cada uma no seu
    //    trecho, e um arquivo que cresce por cima de si mesmo não permitiria
    //    isso.
    let arquivo = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&part)
        .await?;
    marca_esparso(&arquivo).await;
    arquivo.set_len(file.size_bytes).await?;
    drop(arquivo);

    // O progresso já no disco entra no total antes do primeiro byte novo, e
    // sai de novo se esta tentativa fracassar — senão duas tentativas do mesmo
    // arquivo contariam os mesmos bytes duas vezes.
    let creditado = Arc::new(AtomicU64::new(0));
    let progresso: Arc<Vec<AtomicU64>> =
        Arc::new(done.iter().map(|d| AtomicU64::new(*d)).collect());
    let inicial: u64 = done.iter().sum();
    received.fetch_add(inicial, Ordering::Relaxed);
    creditado.fetch_add(inicial, Ordering::Relaxed);
    let estorno = Estorno {
        received,
        creditado: creditado.clone(),
    };

    // 4) Sidecar antes dos bytes: se o app fechar no meio, a retomada sabe
    //    validar o etag e onde cada faixa parou.
    escreve_sidecar(&meta_path, &current_etag, file.size_bytes, &progresso).await?;

    // 5) Uma tarefa por faixa. A velocidade agregada e os eventos saem de um
    //    observador só, senão quatro conexões brigariam pela mesma janela.
    let inicio_tarefas = Instant::now();
    let mut tarefas = Vec::new();
    for (i, (ini, fim)) in limites.iter().copied().enumerate() {
        let ja = progresso[i].load(Ordering::Relaxed);
        if ini + ja >= fim {
            continue;
        }
        let conexoes = Arc::clone(&inner.conexoes);
        let (http, token_hf) = (inner.http.clone(), req.token.clone());
        let (url, part, progresso) = (url.to_string(), part.clone(), progresso.clone());
        let creditado = creditado.clone();
        tarefas.push(async move {
            // A vaga é tomada AQUI, dentro da tarefa, e não no laço que as
            // monta. Pedir todas as vagas antes de rodar qualquer faixa trava
            // o download quando o artefato quer mais faixas do que o teto:
            // três shards ficariam segurando vagas parciais, cada um à espera
            // da que falta, e nenhum transferindo para liberar as suas.
            let _permit = conexoes
                .acquire_owned()
                .await
                .map_err(|e| ModelsError::Api(format!("semáforo de conexões: {e}")))?;
            baixa_faixa(
                &http, &token_hf, &url, &part, ini, fim, i, &progresso, received, &creditado, token,
            )
            .await
        });
    }
    let observador = observa_progresso(
        inner,
        id,
        &meta_path,
        &current_etag,
        file.size_bytes,
        &progresso,
        speed,
        inicio_tarefas,
        inicial,
        token,
    );
    let (resultados, _) = tokio::join!(futures_util::future::join_all(tarefas), observador);

    escreve_sidecar(&meta_path, &current_etag, file.size_bytes, &progresso).await?;
    for r in resultados {
        match r? {
            Outcome::Interrupted => return Ok(Outcome::Interrupted),
            Outcome::Completed => {}
        }
    }
    if token.is_cancelled() {
        return Ok(Outcome::Interrupted);
    }
    finalize(dest, &part, &meta_path, file.size_bytes).await?;
    // Chegou ao fim: o crédito vira definitivo.
    std::mem::forget(estorno);
    Ok(Outcome::Completed)
}

/// Devolve ao total o que esta tentativa creditou, se ela não terminar.
///
/// Sem isto, quatro tentativas de um arquivo de 10 GB deixariam a barra
/// marcando 40 GB de 10 GB.
struct Estorno<'a> {
    received: &'a AtomicU64,
    creditado: Arc<AtomicU64>,
}
impl Drop for Estorno<'_> {
    fn drop(&mut self) {
        let n = self.creditado.swap(0, Ordering::Relaxed);
        self.received.fetch_sub(n, Ordering::Relaxed);
    }
}

/// Marca o `.part` como esparso, onde isso existe.
///
/// No NTFS, `set_len` move o fim do arquivo mas não o *valid data length*.
/// Escrever depois no meio faz o sistema ZERAR fisicamente tudo entre um e
/// outro antes de aceitar os bytes — para não expor no arquivo novo o que
/// havia naqueles setores. Medido nesta máquina, num arquivo de 16 GiB:
/// escrever 1 MiB a 14 GB do início custa 9,54 s sem a marca e 0,51 ms com
/// ela.
///
/// Com faixas paralelas isso deixou de ser detalhe e virou o defeito: oito
/// conexões começam quase juntas, cada uma escreve longe do início, e o disco
/// — que é do sistema inteiro, não do app — passa minutos zerando. O app
/// parava de responder, o botão de pausa não chegava a ser processado, e a
/// máquina toda ia junto.
///
/// Falhar aqui não impede o download: o volume pode não suportar arquivos
/// esparsos (FAT32, alguns de rede), e nesse caso o que resta é o
/// comportamento lento de antes, não um erro na cara de quem só quer baixar.
/// Em Linux e macOS o `ftruncate` já não aloca nem zera nada.
async fn marca_esparso(arquivo: &tokio::fs::File) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::IO::DeviceIoControl;
        use windows::Win32::System::Ioctl::FSCTL_SET_SPARSE;

        let handle = HANDLE(arquivo.as_raw_handle() as _);
        let mut devolvidos = 0u32;
        // SAFETY: `handle` vem de um `File` vivo por toda a chamada, e o
        // controle não lê nem escreve buffer nenhum (entrada e saída vazias).
        let r = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_SPARSE,
                None,
                0,
                None,
                0,
                Some(&mut devolvidos),
                None,
            )
        };
        if let Err(e) = r {
            log::warn!(
                "volume sem suporte a arquivo esparso ({e}); o download vai escrever mais devagar"
            );
        }
    }
    #[cfg(not(windows))]
    let _ = arquivo;
}

async fn escreve_sidecar(
    meta_path: &Path,
    etag: &Option<String>,
    total: u64,
    progresso: &[AtomicU64],
) -> Result<(), ModelsError> {
    let meta = PartMeta {
        etag: etag.clone(),
        total_bytes: total,
        chunks: progresso
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect(),
    };
    let bytes = serde_json::to_vec(&meta).map_err(|e| ModelsError::Api(format!("sidecar: {e}")))?;
    tokio::fs::write(meta_path, bytes).await?;
    Ok(())
}

/// Baixa `[ini, fim)` de `url` para a posição certa do `.part`.
#[allow(clippy::too_many_arguments)]
async fn baixa_faixa(
    http: &reqwest::Client,
    token_hf: &Option<String>,
    url: &str,
    part: &Path,
    ini: u64,
    fim: u64,
    indice: usize,
    progresso: &[AtomicU64],
    received: &AtomicU64,
    creditado: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<Outcome, ModelsError> {
    use tokio::io::AsyncSeekExt;

    let mut ja = progresso[indice].load(Ordering::Relaxed);
    let de = ini + ja;
    let mut get = http
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={de}-{}", fim - 1));
    if let Some(t) = token_hf {
        get = get.bearer_auth(t);
    }
    let resp = tokio::select! {
        r = get.send() => r?,
        _ = cancel.cancelled() => return Ok(Outcome::Interrupted),
    };
    match resp.status().as_u16() {
        206 => {}
        // Sem `Range` o servidor mandaria o arquivo inteiro para cada faixa.
        // Melhor falhar e deixar a próxima tentativa usar uma conexão só.
        200 => {
            return Err(ModelsError::Api(
                "servidor ignorou o Range; refazendo com uma conexão".to_string(),
            ));
        }
        401 | 403 => return Err(ModelsError::Gated),
        s => return Err(ModelsError::Api(format!("download retornou HTTP {s}"))),
    }

    let mut arquivo = tokio::fs::OpenOptions::new().write(true).open(part).await?;
    arquivo.seek(std::io::SeekFrom::Start(de)).await?;
    let mut writer = tokio::io::BufWriter::with_capacity(1 << 20, arquivo);
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => {
                writer.flush().await?;
                return Ok(Outcome::Interrupted);
            }
        };
        let Some(chunk) = chunk else { break };
        let bytes = chunk?;
        // Um servidor generoso demais não pode passar por cima da faixa
        // seguinte, que outra conexão está escrevendo neste instante.
        let cabe = (fim - ini - ja).min(bytes.len() as u64) as usize;
        if cabe == 0 {
            break;
        }
        writer.write_all(&bytes[..cabe]).await?;
        ja += cabe as u64;
        progresso[indice].store(ja, Ordering::Relaxed);
        received.fetch_add(cabe as u64, Ordering::Relaxed);
        creditado.fetch_add(cabe as u64, Ordering::Relaxed);
    }
    writer.flush().await?;
    if ini + ja < fim {
        return Err(ModelsError::Api(format!(
            "faixa {indice} veio incompleta: {ja} de {} B",
            fim - ini
        )));
    }
    Ok(Outcome::Completed)
}

/// Velocidade agregada, eventos para a interface e sidecar periódico.
#[allow(clippy::too_many_arguments)]
async fn observa_progresso(
    inner: &Inner,
    id: &str,
    meta_path: &Path,
    etag: &Option<String>,
    total: u64,
    progresso: &[AtomicU64],
    speed: &AtomicU64,
    inicio: Instant,
    inicial: u64,
    cancel: &CancellationToken,
) {
    let soma = || -> u64 { progresso.iter().map(|a| a.load(Ordering::Relaxed)).sum() };
    let mut window: VecDeque<(Instant, u64)> = VecDeque::from([(inicio, inicial)]);
    let mut ultimo_sidecar = Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep(EVENT_THROTTLE) => {}
            _ = cancel.cancelled() => return,
        }
        let agora = Instant::now();
        let atual = soma();
        window.push_back((agora, atual));
        while window.len() > 2
            && agora.duration_since(window.front().expect("janela não vazia").0) > SPEED_WINDOW
        {
            window.pop_front();
        }
        if let (Some((t0, b0)), Some((t1, b1))) = (window.front(), window.back()) {
            let dt = t1.duration_since(*t0).as_secs_f64();
            if dt > 0.05 {
                speed.store(((b1 - b0) as f64 / dt) as u64, Ordering::Relaxed);
            }
        }
        inner.emit_update(id).await;
        if agora.duration_since(ultimo_sidecar) >= SIDECAR_INTERVAL {
            ultimo_sidecar = agora;
            let _ = escreve_sidecar(meta_path, etag, total, progresso).await;
        }
        if atual >= total {
            return;
        }
    }
}

/// Valida o tamanho final e promove `.part` → destino. Tamanho menor que o
/// esperado (stream cortado) vira erro — a próxima tentativa retoma do
/// ponto em que parou.
async fn finalize(
    dest: &Path,
    part: &Path,
    meta_path: &Path,
    expected: u64,
) -> Result<(), ModelsError> {
    let len = tokio::fs::metadata(part).await?.len();
    if len != expected {
        return Err(ModelsError::Api(format!(
            "tamanho final inesperado: {len} B (esperado {expected} B)"
        )));
    }
    tokio::fs::rename(part, dest).await?;
    let _ = tokio::fs::remove_file(meta_path).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_id_is_stable() {
        assert_eq!(
            download_id("unsloth/Qwen3-8B-GGUF", "Qwen3-8B-Q4_K_M.gguf"),
            "unsloth/Qwen3-8B-GGUF::Qwen3-8B-Q4_K_M.gguf"
        );
    }

    #[test]
    fn dest_path_from_repo_id() {
        let base = Path::new("/models");
        // author/name vira dois níveis
        assert_eq!(
            dest_path(base, "unsloth/Qwen3-8B-GGUF", "m.gguf").unwrap(),
            Path::new("/models/unsloth/Qwen3-8B-GGUF/m.gguf")
        );
        // repo_id sem barra vira um nível só
        assert_eq!(
            dest_path(base, "solo", "m.gguf").unwrap(),
            Path::new("/models/solo/m.gguf")
        );
        // subpasta no caminho do arquivo é preservada
        assert_eq!(
            dest_path(base, "a/b", "sub/m.gguf").unwrap(),
            Path::new("/models/a/b/sub/m.gguf")
        );
        // travessia e componentes suspeitos são rejeitados
        assert!(dest_path(base, "a/b", "../evil.gguf").is_err());
        assert!(dest_path(base, "../a", "m.gguf").is_err());
        assert!(dest_path(base, "a//b", "m.gguf").is_err());
        assert!(dest_path(base, "a/b", "c\\d.gguf").is_err());
    }

    #[test]
    fn part_paths_append_suffixes() {
        let dest = Path::new("/m/a/b/x.gguf");
        assert_eq!(part_path(dest), Path::new("/m/a/b/x.gguf.part"));
        assert_eq!(part_meta_path(dest), Path::new("/m/a/b/x.gguf.part.json"));
    }

    /// Servidor mínimo que entende `Range: bytes=a-b` e responde 206.
    /// Devolve a porta e o corpo servido.
    async fn servidor_com_range(corpo: Vec<u8>) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let porta = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let corpo = corpo.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let faixa = req.lines().find_map(|l| {
                        let v = l
                            .strip_prefix("range: ")
                            .or_else(|| l.strip_prefix("Range: "))?;
                        let v = v.trim().strip_prefix("bytes=")?;
                        let (a, b) = v.split_once('-')?;
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    });
                    let (ini, fim) = faixa.unwrap_or((0, corpo.len().saturating_sub(1)));
                    let fim = fim.min(corpo.len().saturating_sub(1));
                    let fatia = &corpo[ini..=fim];
                    let cab = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                         Content-Range: bytes {ini}-{fim}/{}\r\nConnection: close\r\n\r\n",
                        fatia.len(),
                        corpo.len()
                    );
                    let _ = sock.write_all(cab.as_bytes()).await;
                    let _ = sock.write_all(fatia).await;
                    let _ = sock.flush().await;
                });
            }
        });
        porta
    }

    /// O teste que importa: quatro conexões escrevendo no MESMO arquivo, cada
    /// uma no seu trecho. Um erro de `seek` de um byte aqui não quebra teste
    /// nenhum de aritmética — só produz um GGUF corrompido depois de horas de
    /// download, que é exatamente o defeito caro.
    #[tokio::test]
    async fn parallel_ranges_reassemble_the_original_file() {
        let total = 300_000usize;
        let original: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let porta = servidor_com_range(original.clone()).await;
        let url = format!("http://127.0.0.1:{porta}/m.gguf");

        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("m.gguf.part");
        tokio::fs::File::create(&part)
            .await
            .unwrap()
            .set_len(total as u64)
            .await
            .unwrap();

        let limites = faixas(total as u64, 4);
        let progresso: Arc<Vec<AtomicU64>> =
            Arc::new((0..limites.len()).map(|_| AtomicU64::new(0)).collect());
        let recebido = AtomicU64::new(0);
        let creditado = AtomicU64::new(0);
        let http = reqwest::Client::new();
        let cancel = CancellationToken::new();

        let mut fs = Vec::new();
        for (i, (ini, fim)) in limites.iter().copied().enumerate() {
            let (http, url, part, progresso) =
                (http.clone(), url.clone(), part.clone(), progresso.clone());
            let (recebido, creditado, cancel) = (&recebido, &creditado, cancel.clone());
            fs.push(async move {
                baixa_faixa(
                    &http, &None, &url, &part, ini, fim, i, &progresso, recebido, creditado,
                    &cancel,
                )
                .await
                .unwrap()
            });
        }
        futures_util::future::join_all(fs).await;

        assert_eq!(tokio::fs::read(&part).await.unwrap(), original);
        assert_eq!(recebido.load(Ordering::Relaxed), total as u64);
        assert_eq!(creditado.load(Ordering::Relaxed), total as u64);
    }

    /// Retomada por faixa: metade de cada trecho já no disco, e o que falta
    /// entra exatamente no buraco que sobrou.
    #[tokio::test]
    async fn a_half_written_range_resumes_where_it_stopped() {
        let total = 120_000usize;
        let original: Vec<u8> = (0..total).map(|i| (i % 97) as u8).collect();
        let porta = servidor_com_range(original.clone()).await;
        let url = format!("http://127.0.0.1:{porta}/m.gguf");

        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("m.gguf.part");
        let limites = faixas(total as u64, 2);
        // Metade de cada faixa já escrita; o resto é lixo que precisa sumir.
        let mut disco = vec![0xEEu8; total];
        let mut feito = Vec::new();
        for (ini, fim) in limites.iter().copied() {
            let metade = (fim - ini) / 2;
            let a = ini as usize;
            let b = a + metade as usize;
            disco[a..b].copy_from_slice(&original[a..b]);
            feito.push(metade);
        }
        tokio::fs::write(&part, &disco).await.unwrap();

        let progresso: Arc<Vec<AtomicU64>> =
            Arc::new(feito.iter().map(|d| AtomicU64::new(*d)).collect());
        let recebido = AtomicU64::new(0);
        let creditado = AtomicU64::new(0);
        let http = reqwest::Client::new();
        let cancel = CancellationToken::new();

        let mut fs = Vec::new();
        for (i, (ini, fim)) in limites.iter().copied().enumerate() {
            let (http, url, part, progresso) =
                (http.clone(), url.clone(), part.clone(), progresso.clone());
            let (recebido, creditado, cancel) = (&recebido, &creditado, cancel.clone());
            fs.push(async move {
                baixa_faixa(
                    &http, &None, &url, &part, ini, fim, i, &progresso, recebido, creditado,
                    &cancel,
                )
                .await
                .unwrap()
            });
        }
        futures_util::future::join_all(fs).await;

        assert_eq!(tokio::fs::read(&part).await.unwrap(), original);
        // Só o que faltava foi transferido de novo.
        let restante: u64 = limites
            .iter()
            .zip(&feito)
            .map(|((a, b), d)| (b - a) - d)
            .sum();
        assert_eq!(recebido.load(Ordering::Relaxed), restante);
    }

    /// Mais faixas do que vagas no semáforo: as tarefas têm de disputar a
    /// vaga POR DENTRO. Pedir todas as vagas antes de rodar qualquer faixa
    /// trava — três shards segurariam vagas parciais, cada um esperando a que
    /// falta, e nenhum transferindo para liberar a do vizinho. Este teste
    /// termina em segundos quando está certo, e nunca quando está errado.
    #[tokio::test]
    async fn more_ranges_than_permits_still_finishes() {
        let total = 200_000usize;
        let original: Vec<u8> = (0..total).map(|i| (i % 173) as u8).collect();
        let porta = servidor_com_range(original.clone()).await;
        let url = format!("http://127.0.0.1:{porta}/m.gguf");

        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("m.gguf.part");
        tokio::fs::File::create(&part)
            .await
            .unwrap()
            .set_len(total as u64)
            .await
            .unwrap();

        // Oito faixas para duas vagas: o aperto que o teto global impõe.
        let limites = faixas(total as u64, 8);
        let vagas = Arc::new(Semaphore::new(2));
        let progresso: Arc<Vec<AtomicU64>> =
            Arc::new((0..limites.len()).map(|_| AtomicU64::new(0)).collect());
        let recebido = AtomicU64::new(0);
        let creditado = AtomicU64::new(0);
        let http = reqwest::Client::new();
        let cancel = CancellationToken::new();

        let mut fs = Vec::new();
        for (i, (ini, fim)) in limites.iter().copied().enumerate() {
            let (http, url, part, progresso) =
                (http.clone(), url.clone(), part.clone(), progresso.clone());
            let (recebido, creditado, cancel) = (&recebido, &creditado, cancel.clone());
            let vagas = Arc::clone(&vagas);
            fs.push(async move {
                let _permit = vagas.acquire_owned().await.unwrap();
                baixa_faixa(
                    &http, &None, &url, &part, ini, fim, i, &progresso, recebido, creditado,
                    &cancel,
                )
                .await
                .unwrap()
            });
        }
        let feito =
            tokio::time::timeout(Duration::from_secs(30), futures_util::future::join_all(fs)).await;
        assert!(feito.is_ok(), "as faixas travaram disputando as vagas");
        assert_eq!(tokio::fs::read(&part).await.unwrap(), original);
    }

    #[test]
    fn ranges_cover_the_file_without_gaps_or_overlap() {
        for (total, n) in [(100u64, 3usize), (1, 4), (0, 1), (1 << 30, 4)] {
            let fs = faixas(total, n);
            assert_eq!(fs.first().expect("ao menos uma faixa").0, 0);
            assert_eq!(
                fs.last().expect("ao menos uma faixa").1,
                total,
                "{total}/{n}"
            );
            for par in fs.windows(2) {
                assert_eq!(par[0].1, par[1].0, "buraco ou sobreposição em {total}/{n}");
            }
            assert_eq!(fs.iter().map(|(a, b)| b - a).sum::<u64>(), total);
        }
    }

    /// Arquivo pequeno não ganha nada com quatro conexões: cada uma pagaria
    /// handshake e partida lenta para transferir alguns megabytes.
    #[test]
    fn connection_count_follows_the_file_size() {
        let mib = 1024 * 1024;
        assert_eq!(conexoes(10 * mib), 1, "abaixo do piso, uma conexão só");
        assert_eq!(conexoes(100 * mib), 6);
        assert_eq!(conexoes(200 * mib), MAX_RANGES_POR_ARQUIVO);
        assert_eq!(conexoes(50 * 1024 * mib), MAX_RANGES_POR_ARQUIVO);
    }

    /// O `.part` de um download paralelo nasce do tamanho final. Se a decisão
    /// olhasse o tamanho do arquivo, como fazia, chamaria de "completo" um
    /// download com o meio vazio — e o `finalize` promoveria lixo a modelo.
    #[test]
    fn a_preallocated_part_is_judged_by_its_ranges_not_by_its_size() {
        let com_faixas = |chunks: Vec<u64>| PartMeta {
            etag: Some("abc".into()),
            total_bytes: 100,
            chunks,
        };
        // Pré-alocado (part_len == total) mas só metade escrita.
        assert_eq!(
            decide_resume(100, Some(&com_faixas(vec![25, 25])), Some("abc"), 100),
            ResumeDecision::Resume { done: vec![25, 25] }
        );
        // Todas as faixas cheias → só falta renomear.
        assert_eq!(
            decide_resume(100, Some(&com_faixas(vec![50, 50])), Some("abc"), 100),
            ResumeDecision::AlreadyComplete
        );
        // Faixa maior que a própria fatia é sidecar corrompido, não progresso.
        assert_eq!(
            decide_resume(100, Some(&com_faixas(vec![80, 10])), Some("abc"), 100),
            ResumeDecision::Restart
        );
    }

    #[test]
    fn resume_decisions() {
        let meta = |etag: Option<&str>, total: u64| PartMeta {
            etag: etag.map(String::from),
            total_bytes: total,
            chunks: Vec::new(),
        };
        // sem .part → zero
        assert_eq!(
            decide_resume(0, None, Some("\"abc\""), 100),
            ResumeDecision::Restart
        );
        // .part sem sidecar → não dá para validar → zero
        assert_eq!(
            decide_resume(50, None, Some("\"abc\""), 100),
            ResumeDecision::Restart
        );
        // etag igual → retoma com Range a partir do tamanho do .part
        assert_eq!(
            decide_resume(50, Some(&meta(Some("\"abc\""), 100)), Some("\"abc\""), 100),
            ResumeDecision::Resume { done: vec![50] }
        );
        // aspas e prefixo W/ são normalizados na comparação
        assert_eq!(
            decide_resume(50, Some(&meta(Some("abc"), 100)), Some("W/\"abc\""), 100),
            ResumeDecision::Resume { done: vec![50] }
        );
        // etag mudou (arquivo republicado) → zero
        assert_eq!(
            decide_resume(50, Some(&meta(Some("old"), 100)), Some("new"), 100),
            ResumeDecision::Restart
        );
        // servidor sem etag → não validável → zero
        assert_eq!(
            decide_resume(50, Some(&meta(Some("abc"), 100)), None, 100),
            ResumeDecision::Restart
        );
        // tamanho total registrado difere do esperado → zero
        assert_eq!(
            decide_resume(50, Some(&meta(Some("abc"), 90)), Some("abc"), 100),
            ResumeDecision::Restart
        );
        // .part maior que o esperado → zero
        assert_eq!(
            decide_resume(150, Some(&meta(Some("abc"), 100)), Some("abc"), 100),
            ResumeDecision::Restart
        );
        // .part completo → só validar e renomear
        assert_eq!(
            decide_resume(100, Some(&meta(Some("abc"), 100)), Some("abc"), 100),
            ResumeDecision::AlreadyComplete
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr-dl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Espera o job atingir um estado terminal, consumindo eventos.
    async fn wait_terminal(rx: &mut broadcast::Receiver<DownloadEvent>) -> DownloadStatus {
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(30), rx.recv())
                .await
                .expect("timeout esperando eventos")
                .expect("canal fechado");
            if let DownloadEvent::Update { status } = ev
                && matches!(status.state, DownloadState::Done | DownloadState::Error)
            {
                return status;
            }
        }
    }

    #[tokio::test]
    async fn enqueue_skips_files_already_downloaded() {
        let dir = temp_dir("skip");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/m.gguf"), vec![0u8; 10]).unwrap();

        let mgr = DownloadManager::new(dir.clone());
        let mut rx = mgr.subscribe();
        let id = mgr
            .enqueue(DownloadRequest {
                repo_id: "a/b".to_string(),
                artifact_name: "m.gguf".to_string(),
                files: vec![RepoFile {
                    path: "m.gguf".to_string(),
                    size_bytes: 10,
                }],
                token: None,
            })
            .await
            .unwrap();
        assert_eq!(id, "a/b::m.gguf");

        let status = wait_terminal(&mut rx).await;
        assert_eq!(status.state, DownloadState::Done);
        assert_eq!(status.received_bytes, 10);
        assert_eq!(status.total_bytes, 10);
        assert_eq!(mgr.list().await.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unknown_ids_are_rejected() {
        let dir = temp_dir("ids");
        let mgr = DownloadManager::new(dir.clone());
        assert!(mgr.pause("x").await.is_err());
        assert!(mgr.resume("x", None).await.is_err());
        assert!(mgr.cancel("x").await.is_err());
        assert!(mgr.list().await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn recovers_incomplete_from_owdl_and_orphan_part() {
        let dir = temp_dir("recover");
        let repo = dir.join("acme/mod");
        std::fs::create_dir_all(&repo).unwrap();

        let dest = repo.join("m.gguf");
        std::fs::write(part_path(&dest), vec![0u8; 40]).unwrap();
        std::fs::write(
            part_meta_path(&dest),
            serde_json::to_vec(&PartMeta {
                etag: Some("abc".into()),
                total_bytes: 100,
                chunks: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            owdl_path(&dir, "acme/mod", "m.gguf").unwrap(),
            serde_json::to_vec(&IncompleteRecord {
                repo_id: "acme/mod".into(),
                artifact_name: "m.gguf".into(),
                files: vec![RepoFile {
                    path: "m.gguf".into(),
                    size_bytes: 100,
                }],
            })
            .unwrap(),
        )
        .unwrap();

        let orphan_dest = repo.join("n.gguf");
        std::fs::write(part_path(&orphan_dest), vec![0u8; 12]).unwrap();
        std::fs::write(
            part_meta_path(&orphan_dest),
            serde_json::to_vec(&PartMeta {
                etag: Some("xyz".into()),
                total_bytes: 50,
                chunks: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();

        let mgr = DownloadManager::new(dir.clone());
        let mut list = mgr.list().await;
        list.sort_by(|a, b| a.artifact_name.cmp(&b.artifact_name));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].artifact_name, "m.gguf");
        assert_eq!(list[0].state, DownloadState::Paused);
        assert_eq!(list[0].received_bytes, 40);
        assert_eq!(list[0].total_bytes, 100);
        assert_eq!(list[1].artifact_name, "n.gguf");
        assert_eq!(list[1].received_bytes, 12);
        assert_eq!(list[1].total_bytes, 50);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn recover_skips_already_complete_owdl() {
        let dir = temp_dir("recover-done");
        let repo = dir.join("acme/mod");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("m.gguf"), vec![0u8; 10]).unwrap();
        let owdl = owdl_path(&dir, "acme/mod", "m.gguf").unwrap();
        std::fs::write(
            &owdl,
            serde_json::to_vec(&IncompleteRecord {
                repo_id: "acme/mod".into(),
                artifact_name: "m.gguf".into(),
                files: vec![RepoFile {
                    path: "m.gguf".into(),
                    size_bytes: 10,
                }],
            })
            .unwrap(),
        )
        .unwrap();

        let mgr = DownloadManager::new(dir.clone());
        assert!(mgr.list().await.is_empty());
        assert!(!owdl.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn enqueue_rejects_path_traversal() {
        let dir = temp_dir("trav");
        let mgr = DownloadManager::new(dir.clone());
        let err = mgr
            .enqueue(DownloadRequest {
                repo_id: "a/b".to_string(),
                artifact_name: "evil".to_string(),
                files: vec![RepoFile {
                    path: "../../evil".to_string(),
                    size_bytes: 1,
                }],
                token: None,
            })
            .await;
        assert!(err.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Teste live (rede): baixa o menor arquivo de um repo público real,
    /// verificando resolve → 302 → CDN, tamanho final e rename.
    #[tokio::test]
    #[ignore = "acessa a rede (Hugging Face)"]
    async fn live_downloads_small_file() {
        let repo = "Qwen/Qwen3-0.6B-GGUF";
        let client = crate::HfClient::new(None);
        let files = client.repo_files(repo).await.unwrap();
        let small = files
            .iter()
            .filter(|f| f.size_bytes > 0 && f.size_bytes < 1_000_000)
            .min_by_key(|f| f.size_bytes)
            .expect("repo deveria ter um arquivo pequeno")
            .clone();

        let dir = temp_dir("live");
        let mgr = DownloadManager::new(dir.clone());
        let mut rx = mgr.subscribe();
        mgr.enqueue(DownloadRequest {
            repo_id: repo.to_string(),
            artifact_name: small.path.clone(),
            files: vec![small.clone()],
            token: None,
        })
        .await
        .unwrap();

        let status = wait_terminal(&mut rx).await;
        assert_eq!(
            status.state,
            DownloadState::Done,
            "erro: {:?}",
            status.error
        );
        assert_eq!(status.received_bytes, small.size_bytes);

        let dest = dest_path(&dir, repo, &small.path).unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), small.size_bytes);
        assert!(!part_path(&dest).exists());
        assert!(!part_meta_path(&dest).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
