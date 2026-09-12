//! Optional private training service. No system Python, no arbitrary URLs.
use crate::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

pub static GPU_RESERVED: AtomicBool = AtomicBool::new(false);
pub static GPU_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct Service {
    child: tokio::process::Child,
    _job: Option<lr_proc::JobGuard>,
    url: String,
    token: String,
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            lr_proc::kill_process_tree(pid);
        }
        lr_proc::reap_child(&mut self.child);
        GPU_RESERVED.store(false, Ordering::SeqCst);
    }
}

fn runtime_path(state: &AppState) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("OW_STUDIO_DEV_RUNTIME") {
        return Ok(PathBuf::from(path));
    }
    let root = state.data_dir.join("runtimes/studio");
    let version = std::fs::read_to_string(root.join("active.txt"))
        .map_err(|_| "Instale o módulo de treinamento para começar.".to_string())?;
    let version = version.trim();
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("O runtime instalado está inválido. Repare o módulo.".into());
    }
    Ok(root.join(version))
}

#[tauri::command]
pub async fn studio_status(state: State<'_, AppState>) -> Result<Value, String> {
    let installed = runtime_path(&state)
        .is_ok_and(|p| p.join("python/python.exe").is_file() && p.join("app/main.py").is_file());
    Ok(
        json!({"installed": installed, "running": state.studio.lock().await.is_some(), "gpu_reserved": GPU_RESERVED.load(Ordering::SeqCst)}),
    )
}

async fn start(state: &AppState) -> Result<(), String> {
    let mut guard = state.studio.lock().await;
    if let Some(service) = guard.as_mut()
        && service
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none()
    {
        return Ok(());
    }
    *guard = None;
    let root = runtime_path(state)?;
    let python = root.join("python/python.exe");
    if !python.is_file() {
        return Err("Runtime incompleto. Repare o módulo de treinamento.".into());
    }
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let port = lr_proc::free_port(7860);
    let url = format!("http://127.0.0.1:{port}");
    let data = state.data_dir.join("studio");
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join("service.log"))
        .map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(python);
    lr_proc::prepare(&mut cmd);
    cmd.args([
        "-m",
        "uvicorn",
        "app.main:app",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ])
    .current_dir(&root)
    .env("LAB_DATA", &data)
    .env("HF_HOME", data.join("cache/huggingface"))
    .env("OW_MODELS_DIR", &state.models_dir)
    .env("OW_LLAMA_DIR", root.join("llama.cpp"))
    .env("OW_GPU_LOCK", state.data_dir.join("gpu.lock"))
    .env(
        "OW_OCR_COMPONENTS",
        state.data_dir.join("runtimes/studio-ocr"),
    )
    .env("OW_STUDIO_TOKEN", &token)
    .env("OW_STUDIO_RUNTIME", root.file_name().unwrap_or_default())
    .env("PYTHONNOUSERSITE", "1")
    .env("PYTHONIOENCODING", "utf-8")
    .env("PYTHONPATH", &root)
    .stdout(log.try_clone().map_err(|e| e.to_string())?)
    .stderr(log);
    let child = lr_proc::spawn_supervised(&mut cmd).map_err(|e| e.to_string())?;
    let job = lr_proc::attach_job(&child);
    let service = Service {
        child,
        _job: job,
        url,
        token,
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    for _ in 0..30 {
        if client
            .get(format!("{}/api/health", service.url))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            *guard = Some(service);
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    Err("O Studio não iniciou. Repare o runtime; os detalhes estão no registro do serviço.".into())
}

async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    start(state).await?;
    let (url, token) = {
        let guard = state.studio.lock().await;
        let service = guard.as_ref().ok_or("Studio não está disponível")?;
        (service.url.clone(), service.token.clone())
    };
    let verb = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut request = state
        .http
        .request(verb, format!("{url}{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "A conexão com o Studio foi interrompida. Reabra o módulo.".to_string())?;
    let success = response.status().is_success();
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if !success {
        return Err(value["message"]
            .as_str()
            .or(value["detail"].as_str())
            .unwrap_or("O Studio não pôde concluir a operação. Confira os dados e tente novamente.")
            .into());
    }
    Ok(value)
}

#[tauri::command]
pub async fn studio_request(
    state: State<'_, AppState>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    // Allow only the guided contract. GPU mutations use the guarded command below.
    let allowed = (method == "GET"
        && (path == "/api/v1/capabilities"
            || path == "/api/v1/datasets"
            || path == "/api/v1/runs"
            || valid_run_path(&path, "")))
        || (method == "POST"
            && (path == "/api/v1/prepare"
                || path == "/api/v1/preflight"
                || valid_run_path(&path, "/cancel")
                || (path.starts_with("/api/v1/preparations/")
                    && valid_run_path(&path.replacen("/preparations/", "/runs/", 1), "/resume"))));
    if !allowed {
        return Err("Operação do Studio não permitida.".into());
    }
    request(&state, &method, &path, body).await
}

fn valid_run_path(path: &str, suffix: &str) -> bool {
    path.strip_prefix("/api/v1/runs/")
        .and_then(|s| s.strip_suffix(suffix))
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[tauri::command]
pub async fn studio_train(
    app: AppHandle,
    state: State<'_, AppState>,
    body: Value,
    stop_engine: bool,
    resume_id: Option<String>,
) -> Result<Value, String> {
    let _gate = GPU_GATE.lock().await;
    if GPU_RESERVED.load(Ordering::SeqCst) {
        return Err("Já existe um treinamento em andamento. Aguarde ou cancele-o.".into());
    }
    if crate::comparison::active() {
        return Err("Aguarde a medição de desempenho terminar.".into());
    }
    if state.cluster.snapshot().await.enabled {
        return Err("Desative a GPU extra na rede antes de treinar.".into());
    }
    if state
        .server
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.is_spawned())
    {
        if !stop_engine {
            return Err("motor_active".into());
        }
        crate::commands::stop_engine(&app, &state).await?;
    }
    let path = match resume_id {
        Some(id) if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) => {
            format!("/api/v1/runs/{id}/resume")
        }
        Some(_) => return Err("Treinamento inválido.".into()),
        None => "/api/v1/runs".into(),
    };
    start(&state).await?;
    GPU_RESERVED.store(true, Ordering::SeqCst);
    if crate::commands_tuning::gpu_measurement_active() {
        GPU_RESERVED.store(false, Ordering::SeqCst);
        return Err("Aguarde a medição da GPU terminar.".into());
    }
    let result = request(&state, "POST", &path, Some(body)).await;
    if result.is_err() {
        GPU_RESERVED.store(false, Ordering::SeqCst);
    }
    if let Ok(value) = &result {
        let id = value["id"].as_str().unwrap_or_default().to_string();
        tauri::async_runtime::spawn(async move {
            use tauri::Manager;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let state = app.state::<AppState>();
                match request(&state, "GET", &format!("/api/v1/runs/{id}"), None).await {
                    Ok(v)
                        if matches!(
                            v["status"].as_str(),
                            Some("completed" | "failed" | "cancelled" | "interrupted")
                        ) =>
                    {
                        break;
                    }
                    // A transient HTTP error must never release a live worker's GPU.
                    Err(_) => {
                        let mut service = state.studio.lock().await;
                        if service
                            .as_mut()
                            .is_none_or(|s| s.child.try_wait().ok().flatten().is_some())
                        {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            GPU_RESERVED.store(false, Ordering::SeqCst);
        });
    }
    result
}

static INSTALL_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(serde::Deserialize)]
struct Catalog {
    version: String,
    url: String,
    sha256: String,
    size: u64,
    expanded_size: u64,
    #[serde(default)]
    parts: Vec<DownloadPart>,
}

#[derive(serde::Deserialize)]
struct DownloadPart {
    url: String,
    size: u64,
}

#[tauri::command]
pub async fn studio_install(
    app: AppHandle,
    state: State<'_, AppState>,
    component: Option<String>,
) -> Result<(), String> {
    use tauri::Emitter;
    let _gate = INSTALL_GATE.lock().await;
    if !cfg!(target_os = "windows") || !cfg!(target_arch = "x86_64") {
        return Err(
            "O treino local está disponível inicialmente no Windows x64 com NVIDIA.".into(),
        );
    }
    let ocr = match component.as_deref() {
        None | Some("training") => false,
        Some("ocr") => true,
        _ => return Err("Componente inválido.".into()),
    };
    if GPU_RESERVED.load(Ordering::SeqCst) {
        return Err("Aguarde o treinamento terminar antes de instalar componentes.".into());
    }
    if !ocr && state.studio.lock().await.is_some() {
        return Err("Feche o Studio antes de atualizar o runtime.".into());
    }
    let url = if ocr {
        "https://github.com/pedro-canedo/openweights/releases/download/studio-runtime-v1/ocr-windows-x64.json"
    } else {
        "https://github.com/pedro-canedo/openweights/releases/download/studio-runtime-v1/windows-x64.json"
    };
    let client = &state.http;
    let manifest = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|_| {
            "O pacote do Studio ainda não está disponível neste canal de atualização.".to_string()
        })?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let sig = client
        .get(format!("{url}.minisig"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let key = minisign_verify::PublicKey::from_base64(
        "RWTyWbbHuzzs+27D891id7WuKkUnC3ddOsoiovbo7B5nn6ysiFS9na01",
    )
    .map_err(|e| e.to_string())?;
    key.verify(
        &manifest,
        &minisign_verify::Signature::decode(&sig).map_err(|e| e.to_string())?,
        false,
    )
    .map_err(|_| "Assinatura do catálogo inválida. Nenhum pacote foi instalado.".to_string())?;
    let catalog: Catalog = serde_json::from_slice(&manifest).map_err(|e| e.to_string())?;
    if catalog.version.is_empty()
        || !catalog
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || !catalog
            .url
            .starts_with("https://github.com/pedro-canedo/openweights/releases/download/")
        || catalog.sha256.len() != 64
        || !catalog.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || catalog.size == 0
        || catalog.expanded_size == 0
        || catalog.size > 20 * 1024 * 1024 * 1024
        || catalog.expanded_size > 40 * 1024 * 1024 * 1024
        || (!catalog.parts.is_empty()
            && (catalog.parts.iter().any(|part| {
                part.size == 0
                    || part.size > 2 * 1024 * 1024 * 1024
                    || !part.url.starts_with(
                        "https://github.com/pedro-canedo/openweights/releases/download/",
                    )
            }) || catalog.parts.iter().map(|part| part.size).sum::<u64>() != catalog.size))
    {
        return Err("Catálogo de runtime inválido.".into());
    }
    let root = state.data_dir.join(if ocr {
        "runtimes/studio-ocr"
    } else {
        "runtimes/studio"
    });
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(format!("{}.zip.partial", catalog.version));
    let mut offset = tokio::fs::metadata(&archive)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if offset > catalog.size {
        offset = 0;
    }
    let mut received = offset;
    let free = lr_hw::Monitor::new(&state.profile, Some(root.clone()))
        .sample()
        .disk_free_bytes;
    let required = catalog.size.saturating_sub(offset) + catalog.expanded_size + 512 * 1024 * 1024;
    if free.is_none_or(|bytes| bytes < required) {
        return Err(format!(
            "Libere pelo menos {:.1} GB no disco do OpenWeights e tente instalar novamente.",
            required as f64 / 1_073_741_824.0
        ));
    }
    let fallback = [DownloadPart {
        url: catalog.url.clone(),
        size: catalog.size,
    }];
    let parts = if catalog.parts.is_empty() {
        &fallback[..]
    } else {
        &catalog.parts[..]
    };
    let mut base = 0u64;
    for part in parts {
        if received >= base + part.size {
            base += part.size;
            continue;
        }
        let part_offset = received.saturating_sub(base);
        let mut response = client
            .get(&part.url)
            .header("Range", format!("bytes={part_offset}-"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            received = base;
        }
        response = response.error_for_status().map_err(|e| e.to_string())?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive)
            .await
            .map_err(|e| e.to_string())?;
        file.set_len(received).await.map_err(|e| e.to_string())?;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            received += chunk.len() as u64;
            if received > base + part.size {
                return Err("O download excedeu o tamanho assinado.".into());
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            let _ = app.emit(
                "studio-install",
                json!({"received": received, "total": catalog.size}),
            );
        }
        file.sync_all().await.map_err(|e| e.to_string())?;
        drop(file);
        if received != base + part.size {
            return Err("Download incompleto. Tente novamente para continuar.".into());
        }
        base += part.size;
    }
    if received != catalog.size {
        return Err("Download incompleto. Tente instalar novamente para continuar.".into());
    }
    tokio::task::spawn_blocking(move || install_archive(root, archive, catalog, ocr))
        .await
        .map_err(|e| e.to_string())?
}

fn install_archive(
    root: PathBuf,
    archive: PathBuf,
    catalog: Catalog,
    ocr: bool,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut source = std::fs::File::open(&archive).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != catalog.sha256.to_lowercase() {
        // Keep evidence but do not keep appending to a corrupt download.
        std::fs::rename(
            &archive,
            archive.with_extension(format!(
                "corrupt-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
            )),
        )
        .map_err(|e| e.to_string())?;
        return Err("O pacote falhou na verificação. Tente instalar novamente.".into());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let stage = root.join(format!("{}.staging-{nonce}", catalog.version));
    std::fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let relative = entry.enclosed_name().ok_or("Caminho inseguro no pacote")?;
        if relative.to_string_lossy().contains(':') || entry.is_symlink() {
            return Err("Entrada insegura no pacote.".into());
        }
        total = total.checked_add(entry.size()).ok_or("Pacote excessivo")?;
        if total > catalog.expanded_size {
            return Err("Pacote expandido maior que o catálogo.".into());
        }
        let target = stage.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            std::fs::create_dir_all(target.parent().ok_or("Caminho inválido")?)
                .map_err(|e| e.to_string())?;
            std::io::copy(
                &mut entry,
                &mut std::fs::File::create(target).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let required_files: &[&str] = if ocr {
        &[
            "bin/pdftoppm.exe",
            "bin/tesseract.exe",
            "share/tessdata/por.traineddata",
            "share/tessdata/eng.traineddata",
        ]
    } else {
        &[
            "python/python.exe",
            "app/main.py",
            "llama.cpp/llama-quantize.exe",
            "llama.cpp/convert_hf_to_gguf.py",
            "app/static/studio.html",
        ]
    };
    for required in required_files {
        if !stage.join(required).is_file() {
            return Err(format!("Pacote incompleto: {required}"));
        }
    }
    let destination = root.join(&catalog.version);
    if destination.exists() {
        if std::fs::read_to_string(destination.join(".package-sha256"))
            .ok()
            .as_deref()
            != Some(catalog.sha256.as_str())
        {
            return Err(
                "Existe outra instalação com esta versão. Preserve os dados e repare o módulo."
                    .into(),
            );
        }
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    } else {
        std::fs::write(stage.join(".package-sha256"), &catalog.sha256)
            .map_err(|e| e.to_string())?;
        std::fs::rename(stage, destination).map_err(|e| e.to_string())?;
    }
    std::fs::write(root.join("active.tmp"), catalog.version).map_err(|e| e.to_string())?;
    std::fs::rename(root.join("active.tmp"), root.join("active.txt")).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn studio_upload(
    state: State<'_, AppState>,
    name: String,
    bytes: Vec<u8>,
) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
        return Err("Escolha um arquivo de até 32 MB.".into());
    }
    // JSON literal upload avoids exposing arbitrary filesystem reads over IPC.
    start(&state).await?;
    let (url, token) = {
        let guard = state.studio.lock().await;
        let s = guard.as_ref().unwrap();
        (s.url.clone(), s.token.clone())
    };
    let part = reqwest::multipart::Part::bytes(bytes).file_name(name);
    let response = state
        .http
        .post(format!("{url}/api/studio/sources"))
        .bearer_auth(token)
        .multipart(reqwest::multipart::Form::new().part("files", part))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let success = response.status().is_success();
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if !success {
        return Err(value["detail"]
            .as_str()
            .unwrap_or("Não foi possível importar o arquivo.")
            .into());
    }
    Ok(value)
}

#[tauri::command]
pub async fn studio_import_legacy(state: State<'_, AppState>) -> Result<Value, String> {
    let folder = rfd::AsyncFileDialog::new()
        .set_title("Escolha a pasta do MVP Open Weights Studio")
        .pick_folder()
        .await;
    match folder {
        Some(folder) => {
            request(
                &state,
                "POST",
                "/api/v1/import-legacy",
                Some(json!({"path": folder.path()})),
            )
            .await
        }
        None => Ok(Value::Null),
    }
}

#[tauri::command]
pub async fn studio_uninstall(state: State<'_, AppState>) -> Result<(), String> {
    let _gate = INSTALL_GATE.lock().await;
    if GPU_RESERVED.load(Ordering::SeqCst) {
        return Err("Cancele ou aguarde o treino antes de remover o módulo.".into());
    }
    let running = state.studio.lock().await.is_some();
    if running {
        let jobs = request(&state, "GET", "/api/v1/runs", None).await?;
        if jobs.as_array().is_some_and(|jobs| {
            jobs.iter().any(|job| {
                matches!(
                    job["status"].as_str(),
                    Some("queued" | "running" | "preparing" | "cancelling")
                )
            })
        }) {
            return Err(
                "Cancele ou aguarde a preparação terminar antes de remover o módulo.".into(),
            );
        }
    }
    *state.studio.lock().await = None;
    let root = state
        .data_dir
        .join("runtimes")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    for name in ["studio", "studio-ocr"] {
        let target = root.join(name);
        if target.exists() {
            let checked = target.canonicalize().map_err(|e| e.to_string())?;
            if !checked.starts_with(&root) || checked == root {
                return Err("Diretório de runtime inválido.".into());
            }
            std::fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture(name: &str, unsafe_path: bool) -> (PathBuf, PathBuf, Catalog) {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("ow-studio-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("runtime.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let names = if unsafe_path {
            vec!["../escaped.exe"]
        } else {
            vec![
                "bin/pdftoppm.exe",
                "bin/tesseract.exe",
                "share/tessdata/por.traineddata",
                "share/tessdata/eng.traineddata",
            ]
        };
        let size = names.len() as u64;
        for name in names {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        let catalog = Catalog {
            version: "1.0.0".into(),
            url: String::new(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            expanded_size: size,
            parts: vec![],
        };
        (root, archive, catalog)
    }

    #[test]
    fn package_corruption_cannot_activate_runtime() {
        let (root, archive, catalog) = fixture("corrupt", false);
        std::fs::write(&archive, b"corrupted download").unwrap();
        assert!(install_archive(root.clone(), archive, catalog, true).is_err());
        assert!(!root.join("active.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_zip_cannot_escape_staging() {
        let (root, archive, catalog) = fixture("unsafe", true);
        assert!(install_archive(root.clone(), archive, catalog, true).is_err());
        assert!(!root.join("escaped.exe").exists());
        assert!(!root.join("active.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn complete_package_activates_only_after_extraction() {
        let (root, archive, catalog) = fixture("valid", false);
        install_archive(root.clone(), archive, catalog, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("active.txt")).unwrap(),
            "1.0.0"
        );
        assert!(root.join("1.0.0/bin/tesseract.exe").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires signed release assets supplied through OW_STUDIO_TEST_ASSETS"]
    fn signed_release_packages_install_with_production_extractor() {
        let source = PathBuf::from(std::env::var("OW_STUDIO_TEST_ASSETS").unwrap());
        let key = minisign_verify::PublicKey::from_base64(
            "RWTyWbbHuzzs+27D891id7WuKkUnC3ddOsoiovbo7B5nn6ysiFS9na01",
        )
        .unwrap();
        for (name, ocr) in [("windows-x64", false), ("ocr-windows-x64", true)] {
            let manifest = std::fs::read(source.join(format!("{name}.json"))).unwrap();
            let signature =
                std::fs::read_to_string(source.join(format!("{name}.json.minisig"))).unwrap();
            key.verify(
                &manifest,
                &minisign_verify::Signature::decode(&signature).unwrap(),
                false,
            )
            .unwrap();
            let catalog: Catalog = serde_json::from_slice(&manifest).unwrap();
            let archive = source.join(catalog.url.rsplit('/').next().unwrap());
            let destination = source.join(format!("installed-{name}"));
            std::fs::create_dir_all(&destination).unwrap();
            install_archive(destination, archive, catalog, ocr).unwrap();
        }
    }

    #[test]
    fn guided_paths_never_accept_traversal_or_arbitrary_methods() {
        assert!(valid_run_path(
            "/api/v1/runs/0123456789abcdef0123456789abcdef",
            ""
        ));
        assert!(valid_run_path(
            "/api/v1/runs/0123456789abcdef0123456789abcdef/cancel",
            "/cancel"
        ));
        assert!(!valid_run_path(
            "/api/v1/runs/../../reset/cancel",
            "/cancel"
        ));
        assert!(!valid_run_path(
            "/api/v1/runs/0123456789abcdef0123456789abcdef?x=1",
            ""
        ));
        assert!(!valid_run_path("/api/reset", ""));
    }

    #[test]
    fn release_key_is_valid_and_rejects_invalid_signatures() {
        let key = minisign_verify::PublicKey::from_base64(
            "RWTyWbbHuzzs+27D891id7WuKkUnC3ddOsoiovbo7B5nn6ysiFS9na01",
        );
        assert!(key.is_ok());
        assert!(minisign_verify::Signature::decode("not a signature").is_err());
    }
}
