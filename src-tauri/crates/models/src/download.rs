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
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

/// Máximo de artefatos baixando ao mesmo tempo (semáforo global).
const MAX_CONCURRENT_ARTIFACTS: usize = 2;
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
}

/// Decisão pura de retomada, a partir do `.part` local e do etag atual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeDecision {
    /// Recomeçar do zero (`.part` inválido, etag mudou ou não validável).
    Restart,
    /// Continuar com `Range: bytes=<offset>-`.
    Resume { offset: u64 },
    /// O `.part` já tem todos os bytes; só falta validar e renomear.
    AlreadyComplete,
}

/// Regras: sem `.part` ou sem sidecar → zero; total ou etag divergente (ou
/// impossível de validar) → zero; `.part` maior que o esperado → zero;
/// exatamente o esperado → renomear; senão → `Range` a partir do tamanho.
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
    if part_len > expected_size {
        return ResumeDecision::Restart;
    }
    if part_len == expected_size {
        return ResumeDecision::AlreadyComplete;
    }
    ResumeDecision::Resume { offset: part_len }
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
        let ua = concat!("Rift/", env!("CARGO_PKG_VERSION"));
        let http = reqwest::Client::builder()
            .user_agent(ua)
            .build()
            .expect("reqwest client");
        let http_noredir = reqwest::Client::builder()
            .user_agent(ua)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        Self {
            inner: Arc::new(Inner {
                models_dir,
                http,
                http_noredir,
                jobs: Mutex::new(BTreeMap::new()),
                events,
                sem: Semaphore::new(MAX_CONCURRENT_ARTIFACTS),
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
    pub async fn resume(&self, id: &str) -> Result<(), ModelsError> {
        {
            let mut jobs = self.inner.jobs.lock().await;
            let Some(job) = jobs.get_mut(id) else {
                return Err(ModelsError::Api(format!("download desconhecido: {id}")));
            };
            match job.status.state {
                DownloadState::Paused | DownloadState::Error => {}
                _ => return Ok(()),
            }
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

/// Baixa os arquivos do artefato em sequência. `.part` fica no disco em
/// caso de erro (retomada futura via `resume`).
async fn run_artifact(
    inner: &Inner,
    id: &str,
    req: &DownloadRequest,
    received: &AtomicU64,
    speed: &AtomicU64,
    token: &CancellationToken,
) -> Result<Outcome, ModelsError> {
    let mut base: u64 = 0;
    received.store(0, Ordering::Relaxed);

    for file in &req.files {
        let dest = dest_path(&inner.models_dir, &req.repo_id, &file.path)?;

        // Destino final já existe com o tamanho certo → pular.
        if let Ok(meta) = tokio::fs::metadata(&dest).await
            && meta.is_file()
            && meta.len() == file.size_bytes
        {
            base += file.size_bytes;
            received.store(base, Ordering::Relaxed);
            continue;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        match download_file(inner, id, req, file, &dest, base, received, speed, token).await? {
            Outcome::Interrupted => return Ok(Outcome::Interrupted),
            Outcome::Completed => {
                base += file.size_bytes;
                received.store(base, Ordering::Relaxed);
            }
        }
    }
    Ok(Outcome::Completed)
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
    base: u64,
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
            base,
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
    base: u64,
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
    let offset = match decision {
        ResumeDecision::AlreadyComplete => {
            finalize(dest, &part, &meta_path, file.size_bytes).await?;
            received.store(base + file.size_bytes, Ordering::Relaxed);
            return Ok(Outcome::Completed);
        }
        ResumeDecision::Restart => 0,
        ResumeDecision::Resume { offset } => offset,
    };

    // 3) GET no /resolve/ com Range; o reqwest segue o 302 até a CDN.
    let mut get = inner.http.get(url);
    if let Some(t) = &req.token {
        get = get.bearer_auth(t);
    }
    if offset > 0 {
        get = get.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let resp = tokio::select! {
        r = get.send() => r?,
        _ = token.cancelled() => return Ok(Outcome::Interrupted),
    };
    match resp.status().as_u16() {
        200 | 206 => {}
        401 | 403 => return Err(ModelsError::Gated),
        416 => {
            // Intervalo inválido: joga o .part fora e recomeça do zero.
            let _ = tokio::fs::remove_file(&part).await;
            let _ = tokio::fs::remove_file(&meta_path).await;
            *force_restart = true;
            return Err(ModelsError::Api(
                "HTTP 416: intervalo rejeitado, recomeçando do zero".to_string(),
            ));
        }
        s => return Err(ModelsError::Api(format!("download retornou HTTP {s}"))),
    }
    // Pedimos Range mas veio 200 → o servidor ignorou; recomeça do zero.
    let offset = if resp.status().as_u16() == 206 {
        offset
    } else {
        0
    };

    // 4) Sidecar antes dos bytes: se o app fechar no meio, a retomada
    //    sabe validar o etag.
    let meta = PartMeta {
        etag: current_etag.clone(),
        total_bytes: file.size_bytes,
    };
    let meta_json =
        serde_json::to_vec(&meta).map_err(|e| ModelsError::Api(format!("sidecar: {e}")))?;
    tokio::fs::write(&meta_path, meta_json).await?;

    let out_file = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .await?
    } else {
        tokio::fs::File::create(&part).await?
    };
    let mut writer = tokio::io::BufWriter::with_capacity(1 << 20, out_file);

    // 5) Streaming com contadores atômicos, eventos com throttle e
    //    velocidade por janela deslizante.
    let mut written = offset;
    received.store(base + written, Ordering::Relaxed);
    let mut stream = resp.bytes_stream();
    let mut last_emit = Instant::now();
    let mut window: VecDeque<(Instant, u64)> = VecDeque::new();
    window.push_back((Instant::now(), written));

    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = token.cancelled() => {
                // Pausa/cancelamento: preserva o que já está no .part.
                writer.flush().await?;
                return Ok(Outcome::Interrupted);
            }
        };
        let Some(chunk) = chunk else { break };
        let bytes = chunk?; // erro de rede → nova tentativa re-resolve
        writer.write_all(&bytes).await?;
        written += bytes.len() as u64;
        received.store(base + written, Ordering::Relaxed);

        let now = Instant::now();
        window.push_back((now, written));
        while window.len() > 2
            && now.duration_since(window.front().expect("janela não vazia").0) > SPEED_WINDOW
        {
            window.pop_front();
        }
        if let (Some((t0, b0)), Some((t1, b1))) = (window.front(), window.back()) {
            let dt = t1.duration_since(*t0).as_secs_f64();
            if dt > 0.05 {
                speed.store(((b1 - b0) as f64 / dt) as u64, Ordering::Relaxed);
            }
        }
        if now.duration_since(last_emit) >= EVENT_THROTTLE {
            last_emit = now;
            inner.emit_update(id).await;
        }
    }
    writer.flush().await?;
    finalize(dest, &part, &meta_path, file.size_bytes).await?;
    Ok(Outcome::Completed)
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

    #[test]
    fn resume_decisions() {
        let meta = |etag: Option<&str>, total: u64| PartMeta {
            etag: etag.map(String::from),
            total_bytes: total,
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
            ResumeDecision::Resume { offset: 50 }
        );
        // aspas e prefixo W/ são normalizados na comparação
        assert_eq!(
            decide_resume(50, Some(&meta(Some("abc"), 100)), Some("W/\"abc\""), 100),
            ResumeDecision::Resume { offset: 50 }
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
            if let DownloadEvent::Update { status } = ev {
                if matches!(status.state, DownloadState::Done | DownloadState::Error) {
                    return status;
                }
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
        let mgr = DownloadManager::new(std::env::temp_dir());
        assert!(mgr.pause("x").await.is_err());
        assert!(mgr.resume("x").await.is_err());
        assert!(mgr.cancel("x").await.is_err());
        assert!(mgr.list().await.is_empty());
    }

    #[tokio::test]
    async fn enqueue_rejects_path_traversal() {
        let mgr = DownloadManager::new(std::env::temp_dir());
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
