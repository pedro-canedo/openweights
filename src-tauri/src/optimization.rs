//! One action: provision, measure isolated candidates, retain a reviewable result.
use crate::{
    commands, commands_tuning,
    comparison::{self, Arm, Comparison},
    state::AppState,
};
use lr_types::tuning::{EngineSource, LoadMode, ModelProfile, ProfileSource, SpecType};
use tauri::{AppHandle, Emitter, State};

const MIB: u64 = 1024 * 1024;

fn progress(app: &AppHandle, model: &str, stage: &str) {
    let _ = app.emit(
        "comparison-progress",
        serde_json::json!({"model":model,"arm":0,"sample":0,"stage":stage}),
    );
}

/// O eixo medido aqui é o número de threads, e só ele.
///
/// A especulação do usuário é preservada: desligá-la "para medir limpo"
/// significaria gravar um perfil sem MTP no momento em que ele aplicasse o
/// vencedor — uma escolha dele, apagada em silêncio por uma medição que nem
/// pediu isso. Quem desliga a especulação é a comparação do cache de
/// especialistas, no braço do fork, onde ela atrapalha a leitura.
fn candidates(base: &ModelProfile, physical: usize, logical: usize) -> Vec<ModelProfile> {
    let mut result = Vec::new();
    for threads in [physical.max(1), logical.max(1)] {
        let mut p = base.clone();
        p.engine = Some(EngineSource::Official);
        p.moe_cache_slots = None;
        p.threads = Some(threads as u32);
        p.extras
            .retain(|(k, _)| k != "threads" && !k.starts_with("moe-expert-cache"));
        p.source = ProfileSource::Tested;
        if !result.contains(&p) {
            result.push(p);
        }
    }
    result
}

/// A comparação do cache roda sem especulação: com MTP ligado, o ganho do
/// cache fica escondido atrás da taxa de aceitação do rascunho.
fn without_speculation(base: &ModelProfile) -> ModelProfile {
    let mut p = base.clone();
    p.spec = Some(SpecType::None.into());
    p.spec_draft_model = None;
    p.extras.retain(|(k, _)| !k.starts_with("spec-"));
    p.extras.push(("spec-type".into(), "none".into()));
    p
}

fn winner(arms: &[Arm]) -> usize {
    let mut best = 0;
    for i in 1..arms.len() {
        let a = &arms[i];
        // Require a clear total-latency win, beyond observed variability.
        if a.total_ms.max < arms[best].total_ms.min * 0.98
            && (a.total_ms.max - a.total_ms.min) / a.total_ms.median <= 0.10
            && a.peak_ram_bytes.is_some()
        {
            best = i;
        }
    }
    best
}

#[tauri::command]
pub async fn optimize_run(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
) -> Result<Comparison, String> {
    let _measurement = commands_tuning::begin_measurement()?;
    let _active = comparison::activate();
    comparison::assert_idle(&state).await?;
    if commands_tuning::cluster_args(&state).is_some() {
        return Err("optimization-cluster-active".into());
    }
    let artifact = lr_models::scan_local(&state.models_dir)
        .into_iter()
        .find(|a| a.name.trim_end_matches(".gguf") == model.trim_end_matches(".gguf"))
        .ok_or("comparison-model-missing")?;
    let model = artifact.name.clone();
    let original = commands::profile_for(&state, &model).unwrap_or_default();
    if original.ctx.is_none() {
        return Err("comparison-context-required".into());
    }
    let original_engine = commands::selected_engine(&state);
    let mut config = commands::preview_server_config(&state).await;
    // Match the live configuration, including environment, when present.
    let was_running = {
        let guard = state.server.lock().await;
        if let Some(srv) = guard.as_ref().filter(|s| s.is_spawned()) {
            config = srv.config().clone();
            true
        } else {
            false
        }
    };
    let entry = commands::router_preset_entries(&state)
        .into_iter()
        .find(|e| e.id == model)
        .ok_or("comparison-model-missing")?;
    let meta = lr_models::read_local_meta(&artifact.primary_path);
    let mut warnings = Vec::new();
    progress(&app, &model, "installing");
    let fork = if meta.n_experts.is_some_and(|n| n > 0)
        && lr_runtime::experimental::supported(&state.profile)
    {
        match state
            .runtime_mgr
            .ensure_experimental(&state.profile, &commands_tuning::CANCELAR, |event| {
                let _ = app.emit("runtime", &event);
            })
            .await
        {
            Ok(rt) => Some(rt),
            Err(e) if e == "comparison-cancelled" => return Err(e),
            Err(e) => {
                warnings.push(e);
                None
            }
        }
    } else {
        None
    };
    comparison::assert_idle(&state).await?;
    commands::stop_engine(&app, &state).await?;
    let measured = async {
        let official_identity = lr_runtime::experimental::official_identity(&state.profile);
        let fork_identity = lr_runtime::experimental::identity();
        let original_identity = if original_engine == EngineSource::MoeCache {
            fork_identity.clone()
        } else {
            official_identity.clone()
        };
        let mut arms = vec![
            comparison::arm(
                Some(&app),
                &config,
                &model,
                &original,
                &entry,
                0,
                original_identity,
            )
            .await?,
        ];
        let official = state
            .runtime_mgr
            .state(lr_runtime::select_variant(&state.profile));
        let mut official_config = config.clone();
        official_config.exe_path = official.server_exe.ok_or("optimization-official-missing")?;
        let options = candidates(
            &original,
            lr_hw::physical_cores().unwrap_or(state.profile.cpu_cores),
            state.profile.cpu_cores,
        );
        progress(&app, &model, "measuring");
        for p in &options {
            match comparison::arm(
                Some(&app),
                &official_config,
                &model,
                p,
                &entry,
                arms.len(),
                official_identity.clone(),
            )
            .await
            {
                Ok(a) => arms.push(a),
                Err(e) if e == "comparison-cancelled" => return Err(e),
                Err(e) => warnings.push(e),
            }
        }
        let fork_result = async {
            if let Some(rt) = fork {
                let dir = rt.dir.ok_or("optimization-package-incomplete")?;
                let mut fork_config = config.clone();
                fork_config.exe_path = rt.server_exe.ok_or("optimization-package-incomplete")?;
                let mut p = without_speculation(&options[0]);
                p.engine = Some(EngineSource::MoeCache);
                // Keep an explicit manual GPU-layer choice. With the default
                // profile, all layers are the measured alternative so the
                // cache can own only the routed expert tensors.
                p.ngl = p.ngl.or(meta.n_layers);
                // The fork owns expert placement when its cache is enabled.  `ncmoe`
                // means "keep the first N MoE layers on CPU"; setting it to the
                // total layer count would silently disable the CUDA expert cache.
                p.ncmoe = None;
                p.load_mode = Some(LoadMode::Mmap);
                p.extras.retain(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "fit" | "cpu-moe" | "n-cpu-moe" | "n-gpu-layers" | "load-mode"
                    )
                });
                // A single local CUDA device in v1: row/tensor split and RPC cache
                // ownership need a separate validation matrix.
                let devices = comparison::cancellable(async {
                    lr_advisor::devices::list_devices(&dir, None)
                        .await
                        .map_err(|e| e.to_string())
                })
                .await?;
                let cuda: Vec<_> = devices
                    .iter()
                    .filter(|d| d.name.starts_with("CUDA"))
                    .collect();
                let slot_bytes = meta.n_experts.and_then(|n| {
                    artifact.files.iter().try_fold(0u64, |sum, path| {
                        sum.checked_add(lr_models::expert_slot_bytes(path, n)?)
                    })
                });
                if cuda.len() == 1 && p.ngl.is_some() && slot_bytes.is_some_and(|n| n > 0) {
                    let slot_bytes = slot_bytes.unwrap();
                    // Baseline probe has CPU expert placement and no cache; explicitly
                    // add the per-tensor pool and overflow reserve to its GPU cost.
                    let report = comparison::cancellable(async {
                        lr_advisor::probe::probe(&dir, &artifact.primary_path, &p, None)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await?;
                    let free_ram = lr_hw::available_memory();
                    // Full file is only a conservative bound for resident loading,
                    // never an eligibility gate for mmap/lookup-table architectures.
                    if artifact.total_bytes.saturating_add(4 * 1024 * MIB) < free_ram {
                        p.load_mode = Some(LoadMode::None);
                    }
                    match comparison::arm(
                        Some(&app),
                        &fork_config,
                        &model,
                        &p,
                        &entry,
                        arms.len(),
                        fork_identity.clone(),
                    )
                    .await
                    {
                        Ok(a) => arms.push(a),
                        Err(e) if e == "comparison-cancelled" => return Err(e),
                        Err(e) => warnings.push(e),
                    }
                    let reserve =
                        slot_bytes.saturating_mul(2 * meta.n_experts_used.unwrap_or(16) as u64);
                    let available = cuda[0]
                        .free_bytes
                        .saturating_sub(report.gpu_bytes())
                        .saturating_sub(1024 * MIB)
                        .saturating_sub(reserve);
                    for slots in [16u32, 32, 64] {
                        if slots > meta.n_experts.unwrap_or(0)
                            || slot_bytes.saturating_mul(slots as u64) > available
                        {
                            continue;
                        }
                        p.moe_cache_slots = Some(slots);
                        match comparison::arm(
                            Some(&app),
                            &fork_config,
                            &model,
                            &p,
                            &entry,
                            arms.len(),
                            fork_identity.clone(),
                        )
                        .await
                        {
                            Ok(a) if a.peak_vram_bytes.is_some() => arms.push(a),
                            Ok(_) => warnings.push("optimization-memory-unverified".into()),
                            Err(e) if e == "comparison-cancelled" => return Err(e),
                            Err(e) => {
                                warnings.push(e);
                                break;
                            }
                        }
                    }
                } else if cuda.len() != 1 {
                    // Uma placa só, na v1. Dizer "memória não verificada" aqui
                    // seria mentira: o braço do fork nem chegou a rodar.
                    warnings.push("optimization-single-gpu-only".into());
                } else {
                    warnings.push("optimization-memory-unverified".into());
                }
            }
            Ok::<_, String>(())
        }
        .await;
        if let Err(e) = fork_result {
            if e == "comparison-cancelled" {
                return Err(e);
            }
            warnings.push(e);
        }
        let best = winner(&arms);
        // O braço do fork foi medido sem especulação. Aplicá-lo desliga o MTP
        // que o usuário configurou, e isso precisa estar escrito antes do
        // clique, não descoberto depois.
        if arms[best].profile.engine == Some(EngineSource::MoeCache)
            && original.spec.as_ref().is_some_and(|s| !s.is_off())
        {
            warnings.push("optimization-speculation-disabled".into());
        }
        Ok::<_, String>(Comparison {
            model: model.clone(),
            workload: "moe-interactive-v1".into(),
            machine_key: state.profile.machine_key(),
            runtime: comparison::runtime_key(&config),
            config_key: comparison::config_key(&config),
            model_key: comparison::file_key(&entry.path),
            applied: false,
            current_arm: Some(0),
            original,
            arms,
            inconclusive: best == 0,
            winner: best,
            original_engine,
            warnings,
        })
    }
    .await;
    progress(&app, &model, "restoring");
    let restored = if was_running {
        commands::start_engine(&app, &state)
            .await
            .map(|_| ())
            .map_err(|e| format!("comparison-restore-failed: {e}"))
    } else {
        Ok(())
    };
    // Uma bateria de minutos não se perde porque o servidor não voltou a subir.
    // O resultado é gravado, a falha de restauração vai junto, e o usuário
    // decide se reabre o motor ou aplica o que foi medido.
    let mut result = match measured {
        Ok(value) => value,
        Err(e) => {
            return Err(match restored {
                Err(r) => format!("{e}; {r}"),
                Ok(()) => e,
            });
        }
    };
    if let Err(e) = &restored {
        result.warnings.push(e.clone());
    }
    state
        .store
        .add_comparison(
            &model,
            &result.workload,
            &serde_json::to_string(&result).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    restored?;
    Ok(result)
}

/// App-originated model switches also switch the executable automatically.
#[tauri::command]
pub async fn optimize_prepare_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
) -> Result<(), String> {
    if comparison::active() {
        return Err("engine-busy:benchmark".into());
    }
    let mut target = commands::profile_for(&state, model.trim_end_matches(commands::VISION_SUFFIX))
        .and_then(|p| p.engine)
        .unwrap_or_default();
    // Um perfil que pede o motor opcional não pode impedir a conversa quando
    // o pacote não está mais instalado: o oficial carrega o mesmo modelo, só
    // sem o cache de especialistas.
    if target == EngineSource::MoeCache && !state.runtime_mgr.experimental_state().installed {
        target = EngineSource::Official;
    }
    let previous = commands::selected_engine(&state);
    if target == previous {
        return Ok(());
    }
    let _measurement = commands_tuning::begin_measurement()?;
    let _active = comparison::activate();
    comparison::assert_idle(&state).await?;
    commands::select_engine(&state, target)?;
    if let Err(e) = commands::restart_engine(&app, &state, false).await {
        commands::select_engine(&state, previous)?;
        commands::restart_engine(&app, &state, true)
            .await
            .map_err(|r| format!("comparison-restore-failed: {r}; {e}"))?;
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_preserve_context_precision_and_manual_profile() {
        let base = ModelProfile {
            ctx: Some(32768),
            threads: Some(3),
            spec: Some(SpecType::DraftMtp.into()),
            ..Default::default()
        };
        let list = candidates(&base, 8, 16);
        assert_eq!(list.len(), 2);
        assert_eq!(base.threads, Some(3));
        assert!(list.iter().all(|p| p.ctx == base.ctx
            && p.kv_k == base.kv_k
            && p.spec == base.spec
            && p.spec_draft_model == base.spec_draft_model));
        assert_eq!(candidates(&base, 8, 8).len(), 1);
    }

    /// Aplicar o vencedor grava exatamente o perfil medido. Se a medição
    /// desligasse a especulação por conta própria, quem tinha MTP perderia o
    /// MTP ao clicar em "usar configuração" — sem pedir e sem aviso.
    #[test]
    fn only_the_cache_comparison_turns_speculation_off() {
        let base = ModelProfile {
            ctx: Some(32768),
            spec: Some(SpecType::DraftMtp.into()),
            spec_draft_model: Some("draft.gguf".into()),
            ..Default::default()
        };
        let official = candidates(&base, 8, 16);
        assert!(official.iter().all(|p| !p.spec.as_ref().unwrap().is_off()));
        let fork = without_speculation(&official[0]);
        assert!(fork.spec.as_ref().unwrap().is_off());
        assert_eq!(fork.spec_draft_model, None);
        assert_eq!(fork.ctx, base.ctx);
    }

    #[test]
    fn fastest_decode_does_not_win_when_total_latency_is_worse() {
        let dist = |n| comparison::Distribution {
            median: n,
            min: n,
            max: n,
        };
        let baseline = Arm {
            profile: ModelProfile::default(),
            gen_tps: dist(10.0),
            prompt_tps: dist(100.0),
            total_ms: dist(1000.0),
            gpu_free_bytes: None,
            runtime: String::new(),
            runtime_identity: lr_runtime::experimental::RuntimeIdentity {
                source: EngineSource::Official,
                revision: "test".into(),
                backend: "test".into(),
                platform: "test".into(),
            },
            peak_ram_bytes: Some(1024),
            peak_vram_bytes: None,
        };
        let mut slower = baseline.clone();
        slower.gen_tps = dist(30.0);
        slower.total_ms = dist(1100.0);
        assert_eq!(winner(&[baseline.clone(), slower]), 0);
        let mut faster = baseline.clone();
        faster.total_ms = dist(900.0);
        assert_eq!(winner(&[baseline.clone(), faster.clone()]), 1);
        faster.total_ms.max = 1100.0;
        assert_eq!(winner(&[baseline, faster]), 0);
    }
}
