//! Comparação em servidor isolado: medir nunca grava um perfil experimental.
use crate::{commands, commands_tuning, state::AppState};
use lr_engine::{LlamaServer, PresetEntry, ServerConfig};
use lr_types::tuning::ModelProfile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};

static ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}
pub(crate) struct ActiveGuard;
pub(crate) fn activate() -> ActiveGuard {
    ACTIVE.store(true, Ordering::SeqCst);
    ActiveGuard
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}
const WORKLOAD: &str = "interactive-code-v2-short-long-temp0-seed42-cacheoff-128";
const PROMPT: &str = "Refactor the following Rust function using iterators. Explain edge cases and provide tests.\nfn sum_positive(values: &[i32]) -> i64 { let mut sum = 0i64; for value in values { if *value > 0 { sum += *value as i64; } } sum }";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub median: f64,
    pub min: f64,
    pub max: f64,
}
fn distribution(mut xs: Vec<f64>) -> Result<Distribution, String> {
    if xs.len() != 3 || xs.iter().any(|x| !x.is_finite() || *x <= 0.0) {
        return Err("comparison-invalid-samples".into());
    }
    xs.sort_by(f64::total_cmp);
    Ok(Distribution {
        median: xs[1],
        min: xs[0],
        max: xs[2],
    })
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arm {
    pub profile: ModelProfile,
    pub gen_tps: Distribution,
    pub prompt_tps: Distribution,
    pub total_ms: Distribution,
    pub gpu_free_bytes: Option<u64>,
    #[serde(default)]
    pub runtime: String,
    #[serde(default = "legacy_runtime_identity")]
    pub runtime_identity: lr_runtime::experimental::RuntimeIdentity,
    #[serde(default)]
    pub peak_ram_bytes: Option<u64>,
    #[serde(default)]
    pub peak_vram_bytes: Option<u64>,
}
fn legacy_runtime_identity() -> lr_runtime::experimental::RuntimeIdentity {
    lr_runtime::experimental::RuntimeIdentity {
        source: lr_types::tuning::EngineSource::Official,
        revision: "legacy".into(),
        backend: "unknown".into(),
        platform: "unknown".into(),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub model: String,
    pub workload: String,
    pub machine_key: String,
    pub runtime: String,
    pub config_key: String,
    pub model_key: String,
    #[serde(default)]
    pub applied: bool,
    #[serde(default)]
    pub current_arm: Option<usize>,
    pub original: ModelProfile,
    pub arms: Vec<Arm>,
    pub inconclusive: bool,
    #[serde(default = "legacy_winner")]
    pub winner: usize,
    #[serde(default)]
    pub original_engine: lr_types::tuning::EngineSource,
    #[serde(default)]
    pub warnings: Vec<String>,
}
fn legacy_winner() -> usize {
    1
}

// Só alteramos eixos de execução. Contexto, KV, especulação, extras e fonte
// personalizada permanecem idênticos, inclusive quando o advisor propõe menos contexto.
fn candidate(base: &ModelProfile, advice: &ModelProfile) -> ModelProfile {
    ModelProfile {
        ngl: advice.ngl.or(base.ngl),
        ncmoe: advice.ncmoe.or(base.ncmoe),
        batch: advice.batch.or(base.batch),
        ubatch: advice.ubatch.or(base.ubatch),
        threads: advice.threads.or(base.threads),
        ..base.clone()
    }
}

pub(crate) async fn cancellable<T>(
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        if commands_tuning::CANCELAR.load(Ordering::SeqCst) {
            return Err("comparison-cancelled".into());
        }
        tokio::select! { value = &mut future => return value, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
    }
}

/// Falha fechada: um endpoint sem estado verificável não é licença para parar.
pub(crate) async fn assert_idle(state: &AppState) -> Result<(), String> {
    let cfg = state
        .server
        .lock()
        .await
        .as_ref()
        .filter(|s| s.is_spawned())
        .map(|s| s.config().clone());
    let Some(cfg) = cfg else {
        return Ok(());
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let models = LlamaServer::new(cfg.clone())
        .models_status()
        .await
        .map_err(|e| e.to_string())?;
    for model in models {
        if model.state == "unloaded" {
            continue;
        }
        if model.state != "loaded" {
            return Err("engine-busy:loading".into());
        }
        let slots: Value = client
            .get(format!("{}/slots", cfg.connect_url()))
            .query(&[("model", &model.id)])
            .bearer_auth(cfg.api_key.as_deref().unwrap_or_default())
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|_| "comparison-unverifiable-activity")?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        assert_slots_idle(&slots)?;
    }
    Ok(())
}

fn assert_slots_idle(slots: &Value) -> Result<(), String> {
    let rows = slots
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("comparison-unverifiable-activity")?;
    if rows
        .iter()
        .any(|s| s["is_processing"].as_bool() != Some(false))
    {
        return Err("engine-busy:external".into());
    }
    Ok(())
}
fn noisy(d: &Distribution) -> bool {
    (d.max - d.min) / d.median > 0.10
}
fn inconclusive(reference: &Arm, proposed: &Arm) -> bool {
    [
        &reference.gen_tps,
        &proposed.gen_tps,
        &reference.prompt_tps,
        &proposed.prompt_tps,
        &reference.total_ms,
        &proposed.total_ms,
    ]
    .into_iter()
    .any(noisy)
        || proposed.gen_tps.min <= reference.gen_tps.max
        || proposed.prompt_tps.median < reference.prompt_tps.median * 0.95
        || proposed.total_ms.median > reference.total_ms.median * 1.05
}

pub(crate) fn runtime_key(cfg: &ServerConfig) -> String {
    // llama-server.exe is a small launcher; its sibling implementation and
    // backend libraries must invalidate measurements too.
    let mut files: Vec<_> = cfg
        .exe_path
        .parent()
        .and_then(|p| std::fs::read_dir(p).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| file_key(&e.path()))
        .collect();
    files.sort();
    format!("{}:{}", cfg.exe_path.display(), files.join("|"))
}

pub(crate) fn file_key(path: &std::path::Path) -> String {
    let meta = std::fs::metadata(path).ok();
    format!(
        "{}:{:?}:{:?}",
        path.display(),
        meta.as_ref().map(|m| m.len()),
        meta.and_then(|m| m.modified().ok())
    )
}
pub(crate) fn config_key(cfg: &ServerConfig) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    // O hash não expõe conteúdo de variáveis de ambiente.
    format!(
        "{:?}{:?}{:?}:{}",
        cfg.global_ini_extras, cfg.extra_args, cfg.env_extra, cfg.parallel
    )
    .hash(&mut hash);
    format!("{:x}", hash.finish())
}

async fn sample(
    client: &reqwest::Client,
    cfg: &ServerConfig,
    model: &str,
    ctx: u32,
) -> Result<(f64, f64, f64), String> {
    let start = std::time::Instant::now();
    let mut generated = 0.0;
    let mut prompt_tokens = 0.0;
    let mut generation_ms = 0.0;
    let mut prompt_ms = 0.0;
    for repetitions in [1, (ctx as usize / 256).clamp(1, 32)] {
        let content = PROMPT.repeat(repetitions);
        let response: Value = client.post(format!("{}/v1/chat/completions", cfg.connect_url()))
            .bearer_auth(cfg.api_key.as_deref().unwrap_or_default())
            .json(&json!({"model": model, "messages": [{"role":"user","content":content}], "temperature":0, "seed":42, "max_tokens":128, "stream":false, "cache_prompt":false}))
            .send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?
            .json().await.map_err(|e| e.to_string())?;
        let t = &response["timings"];
        generated += t["predicted_n"]
            .as_f64()
            .ok_or("comparison-missing-timings")?;
        prompt_tokens += t["prompt_n"].as_f64().ok_or("comparison-missing-timings")?;
        generation_ms += t["predicted_ms"]
            .as_f64()
            .ok_or("comparison-missing-timings")?;
        prompt_ms += t["prompt_ms"]
            .as_f64()
            .ok_or("comparison-missing-timings")?;
    }
    // Um motor que devolve zero em qualquer denominador não mediu nada; dizer
    // isso é mais útil que deixar um infinito virar "amostra inválida".
    if generation_ms <= 0.0 || prompt_ms <= 0.0 || generated <= 0.0 || prompt_tokens <= 0.0 {
        return Err("comparison-missing-timings".into());
    }
    Ok((
        1000.0 * generated / generation_ms,
        1000.0 * prompt_tokens / prompt_ms,
        start.elapsed().as_secs_f64() * 1000.0,
    ))
}

pub(crate) async fn arm(
    app: Option<&AppHandle>,
    cfg: &ServerConfig,
    model: &str,
    profile: &ModelProfile,
    entry: &PresetEntry,
    index: usize,
    runtime_identity: lr_runtime::experimental::RuntimeIdentity,
) -> Result<Arm, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut config = cfg.clone();
    config.port = port;
    config.host = "127.0.0.1".into();
    let mut key = [0u8; 24];
    getrandom::fill(&mut key).map_err(|e| e.to_string())?;
    config.api_key = Some(key.iter().map(|b| format!("{b:02x}")).collect());
    let preset = std::env::temp_dir().join(format!(
        "openweights-compare-{}-{port}.ini",
        std::process::id()
    ));
    config.models_preset = Some(preset.clone());
    let mut model_entry = entry.clone();
    model_entry.extras = profile.to_ini_extras();
    lr_engine::write_models_preset(&preset, &cfg.global_ini_extras, &[model_entry])
        .map_err(|e| e.to_string())?;
    drop(listener);
    let mut server = LlamaServer::new(config.clone());
    let mut monitor = lr_hw::Monitor::new(&lr_hw::detect(), None);
    let mut peak_ram = 0u64;
    let mut peak_vram = None::<u64>;
    // A guarda de memória só vale depois que o modelo terminou de carregar: é
    // durante a carga que a memória some de propósito.
    let loaded = std::sync::Arc::new(AtomicBool::new(false));
    let mut starving = 0u32;
    let result = {
        let loaded = loaded.clone();
        let work = async {
            server.spawn().map_err(|e| e.to_string())?;
            // Consumir pipes evita bloquear o processo quando o buffer do SO enche.
            let (stdout, stderr) = server.take_output();
            if let Some(out) = stdout {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(out).lines();
                    while let Ok(Some(_)) = lines.next_line().await {}
                });
            }
            if let Some(out) = stderr {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(out).lines();
                    while let Ok(Some(_)) = lines.next_line().await {}
                });
            }
            cancellable(async {
                server
                    .wait_ready(Duration::from_secs(60))
                    .await
                    .map_err(|e| e.to_string())
            })
            .await?;
            loaded.store(true, Ordering::SeqCst);
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(|e| e.to_string())?;
            if let Some(app) = app {
                let _ = app.emit(
                    "comparison-progress",
                    json!({"model":model,"arm":index,"sample":0}),
                );
            }
            cancellable(sample(&client, &config, model, profile.ctx.unwrap_or(8192))).await?; // aquecimento descartado
            let gpu_free = if let Some(dir) = config.exe_path.parent() {
                cancellable(async { Ok(lr_advisor::devices::list_devices(dir, None).await.ok()) })
                    .await?
                    .filter(|ds| !ds.is_empty() && ds.iter().all(|d| d.free_bytes <= d.total_bytes))
                    .map(|ds| ds.iter().map(|d| d.free_bytes).sum())
            } else {
                None
            };
            let mut samples = Vec::new();
            for n in 1..=3 {
                if let Some(app) = app {
                    let _ = app.emit(
                        "comparison-progress",
                        json!({"model":model,"arm":index,"sample":n}),
                    );
                }
                samples.push(
                    cancellable(sample(&client, &config, model, profile.ctx.unwrap_or(8192)))
                        .await?,
                );
            }
            Ok(Arm {
                profile: profile.clone(),
                gen_tps: distribution(samples.iter().map(|s| s.0).collect())?,
                prompt_tps: distribution(samples.iter().map(|s| s.1).collect())?,
                total_ms: distribution(samples.iter().map(|s| s.2).collect())?,
                gpu_free_bytes: gpu_free,
                runtime: runtime_key(cfg),
                runtime_identity,
                peak_ram_bytes: None,
                peak_vram_bytes: None,
            })
        };
        tokio::pin!(work);
        loop {
            tokio::select! {
                value = &mut work => break value,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let sample = monitor.sample();
                    peak_ram = peak_ram.max(sample.ram_used_bytes);
                    let used = sample.gpus.iter().filter_map(|g| g.vram_used_bytes).max();
                    if let Some(used) = used { peak_vram = Some(peak_vram.unwrap_or(0).max(used)); }
                    // VRAM cheia NÃO é defeito: encher a placa é o objetivo do
                    // dimensionamento, e o que não couber falha ao carregar —
                    // erro do braço, não interrupção da bateria. Só a RAM do
                    // sistema justifica parar: sem ela a máquina inteira trava.
                    // Exige leituras consecutivas para não confundir o pico de
                    // um `mmap` recém-tocado com exaustão real.
                    let floor = (sample.ram_total_bytes / 32).min(1024 * 1024 * 1024);
                    let free_ram = sample.ram_total_bytes.saturating_sub(sample.ram_used_bytes);
                    starving = if loaded.load(Ordering::SeqCst) && free_ram < floor { starving + 1 } else { 0 };
                    if starving >= 4 { break Err("optimization-memory-pressure".into()); }
                }
            }
        }
    };
    server.stop().await;
    let _ = std::fs::remove_file(preset);
    result.map(|mut arm| {
        arm.peak_ram_bytes = Some(peak_ram);
        arm.peak_vram_bytes = peak_vram;
        arm
    })
}

#[tauri::command]
pub async fn compare_run(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
) -> Result<Comparison, String> {
    let _measurement = commands_tuning::begin_measurement()?;
    ACTIVE.store(true, Ordering::SeqCst);
    let _active = ActiveGuard;
    assert_idle(&state).await?;
    let advice = cancellable(commands_tuning::tune_advise(state.clone(), model.clone())).await?;
    let original = commands::profile_for(&state, &model).unwrap_or_default();
    if original.ctx.is_none() {
        return Err("comparison-context-required".into());
    }
    let suggested = advice
        .options
        .get(advice.recommended)
        .ok_or("comparison-no-candidate")?;
    let next = candidate(&original, &suggested.profile);
    // Extras explícitos vencem campos do perfil: não anunciar uma mudança que
    // o INI anularia. Comparação só existe quando os argumentos efetivos diferem.
    if original.to_ini_extras() == next.to_ini_extras() {
        return Err("comparison-no-candidate".into());
    }
    if original == next {
        return Err("comparison-no-candidate".into());
    }
    let was_running = state
        .server
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.is_spawned());
    let preview = commands::preview_server_config(&state).await;
    let config = state
        .server
        .lock()
        .await
        .as_ref()
        .filter(|s| s.is_spawned())
        .map(|s| s.config().clone())
        .unwrap_or(preview);
    let entry = commands::router_preset_entries(&state)
        .into_iter()
        .find(|e| e.id == model)
        .ok_or("comparison-model-missing")?;
    assert_idle(&state).await?;
    commands::stop_engine(&app, &state).await?;
    let measured = async {
        let runtime_identity =
            if commands::selected_engine(&state) == lr_types::tuning::EngineSource::MoeCache {
                lr_runtime::experimental::identity()
            } else {
                lr_runtime::experimental::official_identity(&state.profile)
            };
        let reference = arm(
            Some(&app),
            &config,
            &model,
            &original,
            &entry,
            0,
            runtime_identity.clone(),
        )
        .await?;
        let proposed = arm(
            Some(&app),
            &config,
            &model,
            &next,
            &entry,
            1,
            runtime_identity,
        )
        .await?;
        // Só há vencedor se geração melhora além da variação e o prompt não piora.
        let inconclusive = inconclusive(&reference, &proposed);
        Ok::<_, String>(Comparison {
            model: model.clone(),
            workload: WORKLOAD.into(),
            machine_key: state.profile.machine_key(),
            runtime: runtime_key(&config),
            config_key: config_key(&config),
            model_key: file_key(&entry.path),
            applied: false,
            current_arm: Some(0),
            original,
            arms: vec![reference, proposed],
            inconclusive,
            winner: 1,
            original_engine: commands::selected_engine(&state),
            warnings: Vec::new(),
        })
    }
    .await;
    // Nenhum perfil foi alterado. Repor disponibilidade também é obrigatório em erro.
    if was_running {
        commands::start_engine(&app, &state)
            .await
            .map_err(|e| format!("comparison-restore-failed: {e}"))?;
    }
    let result = measured?;
    state
        .store
        .add_comparison(
            &model,
            WORKLOAD,
            &serde_json::to_string(&result).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
pub fn compare_latest(
    state: State<'_, AppState>,
    model: String,
) -> Result<Option<Comparison>, String> {
    let mut saved: Option<Comparison> = state
        .store
        .latest_comparison(&model)
        .map_err(|e| e.to_string())?
        .map(|(_, payload)| serde_json::from_str(&payload).map_err(|e| e.to_string()))
        .transpose()?;
    if let Some(ref mut item) = saved {
        // A escolha manual pode ser diferente da recomendação. O braço 0
        // é apenas a referência; estar nele não é uma mudança a restaurar.
        let current = commands::profile_for(&state, &model).unwrap_or_default();
        let engine = commands::selected_engine(&state);
        item.current_arm = item.arms.iter().enumerate().position(|(i, arm)| {
            current == arm.profile
                && engine
                    == if i == 0 {
                        item.original_engine
                    } else {
                        arm.profile.engine.unwrap_or_default()
                    }
        });
        item.applied = item.current_arm.is_some_and(|i| i > 0);
    }
    Ok(saved)
}

#[tauri::command]
pub async fn compare_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    restore: bool,
    arm_index: Option<usize>,
) -> Result<(), String> {
    let _measurement = commands_tuning::begin_measurement()?;
    ACTIVE.store(true, Ordering::SeqCst);
    let _active = ActiveGuard;
    assert_idle(&state).await?;
    let saved = compare_latest(state.clone(), model.clone())?.ok_or("comparison-missing")?;
    let current = commands::profile_for(&state, &model).unwrap_or_default();
    let selected = selected_arm(&saved, restore, arm_index)?;
    let proposed = &saved.arms[selected].profile;
    let target_engine = if selected == 0 {
        saved.original_engine
    } else {
        proposed.engine.unwrap_or_default()
    };
    if restore && !saved.applied {
        return Err("comparison-profile-changed".into());
    }
    if !restore && saved.machine_key != state.profile.machine_key() {
        return Err("comparison-stale".into());
    }
    if !restore {
        let cfg = commands::preview_server_config(&state).await;
        let entry = commands::router_preset_entries(&state)
            .into_iter()
            .find(|e| e.id == model)
            .ok_or("comparison-model-missing")?;
        if saved.config_key != config_key(&cfg) || saved.model_key != file_key(&entry.path) {
            return Err("comparison-stale".into());
        }
        let mut target_cfg = cfg.clone();
        if target_engine == lr_types::tuning::EngineSource::MoeCache {
            target_cfg.exe_path = state
                .runtime_mgr
                .experimental_state()
                .server_exe
                .ok_or("comparison-stale")?;
        } else {
            target_cfg.exe_path = state
                .runtime_mgr
                .state(lr_runtime::select_variant(&state.profile))
                .server_exe
                .ok_or("comparison-stale")?;
        }
        let arm = &saved.arms[selected];
        if !arm.runtime.is_empty() && arm.runtime != runtime_key(&target_cfg) {
            return Err("comparison-stale".into());
        }
    }
    let target = if restore {
        saved.original
    } else {
        proposed.clone()
    };
    let current_engine = commands::selected_engine(&state);
    state
        .store
        .set_model_profile(&model, &target)
        .map_err(|e| e.to_string())?;
    let attempt = async {
        commands::select_engine(
            &state,
            if restore {
                saved.original_engine
            } else {
                target_engine
            },
        )?;
        commands::restart_engine(&app, &state, false).await?;
        let endpoint = state.llama_endpoint().await?;
        let client =
            lr_engine::LlamaClient::new(&endpoint.base_url).with_optional_api_key(endpoint.api_key);
        let mut request =
            lr_engine::ChatRequest::new(&model, vec![lr_engine::ChatMessage::user("hi")]);
        request.max_tokens = Some(1);
        client
            .complete_once(&request)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    }
    .await;
    if let Err(error) = attempt {
        // A volta atrás não pode ter condição de parada: se o motor anterior
        // deixou de estar disponível no meio da aplicação, o oficial sempre
        // está — e o perfil manual do usuário precisa voltar de qualquer jeito.
        if commands::select_engine(&state, current_engine).is_err() {
            let _ = commands::select_engine(&state, lr_types::tuning::EngineSource::Official);
        }
        state
            .store
            .set_model_profile(&model, &current)
            .map_err(|e| e.to_string())?;
        commands::restart_engine(&app, &state, true)
            .await
            .map_err(|e| format!("comparison-restore-failed: {e}; {error}"))?;
        return Err(error);
    }
    Ok(())
}

/// A recomendação é conservadora; uma escolha explícita pode usar qualquer
/// braço completo. Nunca aceitar um índice ausente ou inferir um vencedor.
fn selected_arm(
    saved: &Comparison,
    restore: bool,
    requested: Option<usize>,
) -> Result<usize, String> {
    let index = if restore {
        0
    } else {
        requested.unwrap_or(saved.winner)
    };
    if saved.arms.get(index).is_none() {
        return Err("comparison-missing".into());
    }
    if !restore && requested.is_none() && saved.inconclusive {
        return Err("comparison-inconclusive".into());
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_does_not_trade_context_or_precision_for_speed() {
        let base = ModelProfile {
            ctx: Some(32768),
            kv_k: Some(lr_types::tuning::KvType::F16),
            ..Default::default()
        };
        let advice = ModelProfile {
            ctx: Some(4096),
            batch: Some(512),
            kv_k: Some(lr_types::tuning::KvType::Q4_0),
            ..Default::default()
        };
        let result = candidate(&base, &advice);
        assert_eq!(result.ctx, base.ctx);
        assert_eq!(result.kv_k, base.kv_k);
        assert_eq!(result.batch, Some(512));
    }
    #[test]
    fn distribution_uses_median_and_rejects_partial_or_invalid_runs() {
        assert_eq!(distribution(vec![100.0, 1.0, 10.0]).unwrap().median, 10.0);
        assert!(distribution(vec![1.0, 2.0]).is_err());
        assert!(distribution(vec![1.0, f64::NAN, 2.0]).is_err());
    }
    #[test]
    fn activity_checks_fail_closed_for_missing_unknown_and_busy_slots() {
        assert!(assert_slots_idle(&json!([])).is_err());
        assert!(assert_slots_idle(&json!([{}])).is_err());
        assert!(assert_slots_idle(&json!([{"is_processing":true}])).is_err());
        assert!(assert_slots_idle(&json!([{"is_processing":false}])).is_ok());
    }
    #[test]
    fn unstable_samples_cannot_be_recommended() {
        assert!(noisy(&Distribution {
            median: 10.0,
            min: 4.0,
            max: 12.0
        }));
        assert!(!noisy(&Distribution {
            median: 10.0,
            min: 9.9,
            max: 10.1
        }));
    }
    #[test]
    fn faster_generation_does_not_hide_latency_regression() {
        fn steady(value: f64) -> Distribution {
            Distribution {
                median: value,
                min: value,
                max: value,
            }
        }
        let reference = Arm {
            profile: ModelProfile::default(),
            gen_tps: steady(10.0),
            prompt_tps: steady(100.0),
            total_ms: steady(1000.0),
            gpu_free_bytes: None,
            runtime: String::new(),
            runtime_identity: legacy_runtime_identity(),
            peak_ram_bytes: None,
            peak_vram_bytes: None,
        };
        let mut proposed = reference.clone();
        proposed.gen_tps = steady(12.0);
        assert!(!inconclusive(&reference, &proposed));
        proposed.total_ms = steady(1060.0);
        assert!(inconclusive(&reference, &proposed));
        let saved = Comparison {
            model: "test".into(),
            workload: "test".into(),
            machine_key: "test".into(),
            runtime: String::new(),
            config_key: String::new(),
            model_key: String::new(),
            applied: false,
            current_arm: Some(0),
            original: ModelProfile::default(),
            arms: vec![reference, proposed],
            inconclusive: true,
            winner: 0,
            original_engine: Default::default(),
            warnings: Vec::new(),
        };
        // Não recomendar automaticamente não deve impedir a escolha manual.
        assert!(selected_arm(&saved, false, None).is_err());
        assert_eq!(selected_arm(&saved, false, Some(1)).unwrap(), 1);
        assert_eq!(selected_arm(&saved, true, None).unwrap(), 0);
        assert!(selected_arm(&saved, false, Some(2)).is_err());
        assert!(selected_arm(&saved, false, Some(usize::MAX)).is_err());
    }
    #[tokio::test]
    #[ignore = "requires OW_TEST_RUNTIME and OW_TEST_MODEL; loads a real GPU model"]
    async fn real_isolated_arm_warms_up_measures_and_stops() {
        let exe = std::path::PathBuf::from(std::env::var("OW_TEST_RUNTIME").unwrap());
        let model = std::path::PathBuf::from(std::env::var("OW_TEST_MODEL").unwrap());
        let cfg = ServerConfig::new(exe, model.parent().unwrap().into(), 14821);
        let profile = ModelProfile {
            ctx: Some(8192),
            ngl: Some(99),
            flash_attn: Some(true),
            ..Default::default()
        };
        let entry = PresetEntry::new("test", model);
        let measured = arm(
            None,
            &cfg,
            "test",
            &profile,
            &entry,
            0,
            legacy_runtime_identity(),
        )
        .await
        .unwrap();
        assert!(measured.gen_tps.median > 0.0);
        println!("{}", serde_json::to_string(&measured).unwrap());
    }
}
