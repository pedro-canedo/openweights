//! Catálogo de flags do llama.cpp, validação, preview e carga de modelo.
//!
//! O contrato com a interface: TODA flag da build pinada aparece (curadas +
//! extraídas do `--help` do binário instalado), a validação usa o mesmo
//! catálogo, e o preview do INI/argumentos é renderizado pelas MESMAS funções
//! Rust que o boot usa — a tela nunca reimplementa a montagem, então nunca
//! mente.

use crate::commands::{
    ServerStatusView, VISION_SUFFIX, engine_busy_with, profile_for, router_preset_entries,
    router_star_section, start_engine,
};
use crate::state::AppState;
use lr_types::flags::{
    FlagIssue, FlagScope, FlagSpec, curated_flags, merge_catalog, validate_extras,
};
use serde::Serialize;
use tauri::{AppHandle, State};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// O catálogo completo para a tela: curadas + dinâmicas do `--help`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagCatalog {
    pub tag: String,
    pub variant: String,
    /// `true` quando o `--help` do binário não pôde ser lido (runtime ainda
    /// não instalado, parse aquém do mínimo). A tela opera só com as curadas
    /// e diz isso — nunca esconde a seção inteira.
    pub degraded: bool,
    pub flags: Vec<FlagSpec>,
}

/// Curadas + dinâmicas, com degradação explícita quando o binário não ajuda.
async fn full_catalog(state: &AppState) -> FlagCatalog {
    let variant = lr_runtime::select_variant(&state.profile);
    let rt = state.runtime_mgr.state(variant);
    let vname = format!("{variant:?}").to_lowercase();

    let help = match &rt.server_exe {
        Some(exe) => {
            lr_advisor::help::help_flags_cached(exe, &state.data_dir, &rt.tag, &vname).await
        }
        None => Err(lr_advisor::help::HelpError::Missing("llama-server".into())),
    };
    match help {
        Ok(flags) => FlagCatalog {
            tag: rt.tag,
            variant: vname,
            degraded: false,
            flags: merge_catalog(curated_flags(), &flags),
        },
        Err(e) => {
            log::warn!("catálogo de flags sem o --help ({e}); seguindo só com as curadas");
            FlagCatalog {
                tag: rt.tag,
                variant: vname,
                degraded: true,
                flags: merge_catalog(curated_flags(), &[]),
            }
        }
    }
}

#[tauri::command]
pub async fn flags_catalog(state: State<'_, AppState>) -> CmdResult<FlagCatalog> {
    Ok(full_catalog(&state).await)
}

/// Valida extras antes de gravar. `scope` é `"perModel"` ou `"global"`;
/// com `model`, as chaves que o perfil tipado já emite entram na conta
/// (duplicata e dependência satisfeita).
#[tauri::command]
pub async fn flags_validate(
    state: State<'_, AppState>,
    scope: String,
    extras: Vec<(String, String)>,
    model: Option<String>,
) -> CmdResult<Vec<FlagIssue>> {
    let escopo = match scope.as_str() {
        "perModel" => FlagScope::PerModel,
        "global" => FlagScope::Global,
        outro => return Err(format!("escopo desconhecido: {outro}")),
    };
    let typed_keys = model
        .as_deref()
        .and_then(|m| profile_for(&state, m))
        .map(|p| p.typed_ini_keys())
        .unwrap_or_default();
    let catalogo = full_catalog(&state).await;
    Ok(validate_extras(
        &catalogo.flags,
        escopo,
        &extras,
        &typed_keys,
    ))
}

/// O que o motor vai receber no próximo boot — literalmente.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePreview {
    /// Argumentos de linha de comando do processo (sem o executável).
    pub args: Vec<String>,
    /// O INI do `--models-preset`; com `model`, só a seção `[*]` e as seções
    /// desse modelo (inclusive a companheira de visão).
    pub ini: String,
    pub ini_path: String,
}

#[tauri::command]
pub async fn engine_preview(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<EnginePreview> {
    let cfg = crate::commands::preview_server_config(&state).await;
    let star = router_star_section(&state);
    let mut entries = router_preset_entries(&state);
    if let Some(m) = model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        let stem = m.trim_end_matches(".gguf");
        entries.retain(|e| {
            let id = e.id.trim_end_matches(VISION_SUFFIX);
            id == m || id.trim_end_matches(".gguf") == stem
        });
    }
    Ok(EnginePreview {
        args: cfg.to_args(),
        ini: lr_engine::render_models_preset(&star, &entries),
        ini_path: state
            .data_dir
            .join("router-models.ini")
            .to_string_lossy()
            .into_owned(),
    })
}

/// Um modelo do Router com o estado de carga, para a tela do servidor.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterModelView {
    pub id: String,
    /// `unloaded` | `loading` | `loaded` | `unknown`.
    pub state: String,
}

async fn router_call<F, Fut, T>(state: &AppState, f: F) -> CmdResult<T>
where
    F: FnOnce(lr_engine::ServerConfig) -> Fut,
    Fut: std::future::Future<Output = Result<T, lr_engine::EngineError>>,
{
    // O lock solta antes do HTTP: carregar um modelo grande leva minutos e o
    // mutex do servidor é disputado pelo status/stop/health.
    let cfg = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(srv) if srv.is_spawned() => srv.config().clone(),
            _ => return Err("servidor não está rodando".to_string()),
        }
    };
    f(cfg).await.map_err(err_str)
}

/// Estado de todos os modelos registrados no Router.
#[tauri::command]
pub async fn router_models(state: State<'_, AppState>) -> CmdResult<Vec<RouterModelView>> {
    let modelos = router_call(&state, |cfg| async move {
        lr_engine::LlamaServer::new(cfg).models_status().await
    })
    .await?;
    Ok(modelos
        .into_iter()
        .map(|m| RouterModelView {
            id: m.id,
            state: m.state,
        })
        .collect())
}

/// Sobe o servidor se preciso e pede ao Router para carregar o modelo.
///
/// A API do Router não aceita configuração por requisição — o INI foi lido no
/// boot. Por isso a tela marca "requer reinício" quando há mudança pendente e
/// reinicia ANTES de chamar isto.
#[tauri::command]
pub async fn router_load_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
) -> CmdResult<ServerStatusView> {
    let rodando = {
        let guard = state.server.lock().await;
        guard.as_ref().map(|s| s.is_spawned()).unwrap_or(false)
    };
    let view = if rodando {
        crate::commands::current_status_view(&state).await
    } else {
        start_engine(&app, &state).await?
    };
    router_call(&state, |cfg| {
        let model = model.clone();
        async move { lr_engine::LlamaServer::new(cfg).load_model(&model).await }
    })
    .await?;
    Ok(view)
}

/// Descarrega um modelo do Router — a memória de vídeo volta na hora.
#[tauri::command]
pub async fn router_unload_model(state: State<'_, AppState>, model: String) -> CmdResult<()> {
    router_call(&state, |cfg| {
        let model = model.clone();
        async move { lr_engine::LlamaServer::new(cfg).unload_model(&model).await }
    })
    .await
}

/// Fatos do arquivo que ligam badges na tela de configuração.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCaps {
    /// `Some(true)` = MoE declarado no cabeçalho; `None` = o arquivo não diz.
    pub moe: Option<bool>,
    /// `Some(true)` = cabeça MTP declarada (`nextn_predict_layers` > 0).
    /// `None` = não sei — a interface mostra a opção mesmo assim.
    pub mtp_head: Option<bool>,
    pub has_mmproj: bool,
    pub n_layers: Option<u32>,
    pub train_ctx: Option<u32>,
    pub busy_with: Vec<&'static str>,
}

// ------------------------------------------------ presets de configuração ---

/// Um preset como a tela o vê: embutido (id estável, nome via i18n) ou salvo
/// pela pessoa (nome literal).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePresetView {
    /// `builtin.<slug>` para os embutidos; o id numérico para os salvos.
    pub id: String,
    /// Vazio nos embutidos — o rótulo vem de `server.enginePresets.builtin.*`.
    pub name: String,
    pub builtin: bool,
    pub profile: lr_types::tuning::ModelProfile,
}

/// Os presets que o app traz de fábrica. Ids estáveis: são chave de i18n.
fn builtin_presets() -> Vec<(&'static str, lr_types::tuning::ModelProfile)> {
    use lr_types::tuning::{KvType, ModelProfile, SpecType};
    vec![
        // Nada escolhido: o llama.cpp decide tudo (`--fit` ligado).
        ("builtin.default", ModelProfile::default()),
        // MTP: rascunho de 4 tokens é o equilíbrio medido pela comunidade;
        // p-min 0.75 evita ciclos desperdiçados em contexto longo.
        (
            "builtin.mtpTurbo",
            ModelProfile {
                spec: Some(SpecType::Mtp),
                spec_draft_n_max: Some(4),
                spec_draft_p_min: Some(0.75),
                flash_attn: Some(true),
                ..Default::default()
            },
        ),
        // KV comprimido a q8_0 (precisa de flash attention) corta o cache
        // quase pela metade sem perda perceptível.
        (
            "builtin.vramSaver",
            ModelProfile {
                kv_k: Some(KvType::Q8_0),
                kv_v: Some(KvType::Q8_0),
                flash_attn: Some(true),
                ..Default::default()
            },
        ),
        // Janela grande + reaproveitamento de cache entre requisições.
        (
            "builtin.longContext",
            ModelProfile {
                ctx: Some(65_536),
                kv_k: Some(KvType::Q8_0),
                kv_v: Some(KvType::Q8_0),
                flash_attn: Some(true),
                extras: vec![("cache-reuse".into(), "256".into())],
                ..Default::default()
            },
        ),
    ]
}

#[tauri::command]
pub fn engine_presets_list(state: State<'_, AppState>) -> CmdResult<Vec<EnginePresetView>> {
    let mut out: Vec<EnginePresetView> = builtin_presets()
        .into_iter()
        .map(|(id, profile)| EnginePresetView {
            id: id.to_string(),
            name: String::new(),
            builtin: true,
            profile,
        })
        .collect();
    for row in state.store.list_engine_presets("model").map_err(err_str)? {
        let Ok(profile) = serde_json::from_str(&row.json) else {
            continue;
        };
        out.push(EnginePresetView {
            id: row.id.to_string(),
            name: row.name,
            builtin: false,
            profile,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn engine_preset_save(
    state: State<'_, AppState>,
    name: String,
    profile: lr_types::tuning::ModelProfile,
) -> CmdResult<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err("nome vazio".into());
    }
    let json = serde_json::to_string(&profile).map_err(err_str)?;
    state
        .store
        .save_engine_preset(name, "model", &json)
        .map_err(err_str)
}

#[tauri::command]
pub fn engine_preset_delete(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.store.delete_engine_preset(id).map_err(err_str)
}

/// Aplica um preset sobre o perfil atual do modelo (merge, não substituição):
/// campo presente no preset vence, campo ausente preserva o que a pessoa já
/// tinha; extras se unem com o preset ganhando por chave. Grava como
/// `Manual` — o auto-tune nunca sobrescreve escolha de gente.
#[tauri::command]
pub fn engine_preset_apply(
    state: State<'_, AppState>,
    model: String,
    preset_id: String,
) -> CmdResult<lr_types::tuning::ModelProfile> {
    let model = model.trim();
    if model.is_empty() {
        return Err("modelo vazio".into());
    }
    let preset = if let Some(p) = builtin_presets()
        .into_iter()
        .find(|(id, _)| *id == preset_id)
        .map(|(_, p)| p)
    {
        p
    } else {
        let id: i64 = preset_id.parse().map_err(|_| "preset desconhecido")?;
        let row = state
            .store
            .list_engine_presets("model")
            .map_err(err_str)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("preset desconhecido")?;
        serde_json::from_str(&row.json).map_err(err_str)?
    };

    // O preset "Padrão" (vazio) é a exceção do merge: aplicá-lo é pedir
    // "volte tudo ao automático", não "não mude nada".
    let base = if preset.is_empty() {
        lr_types::tuning::ModelProfile::default()
    } else {
        merge_profile(profile_for(&state, model).unwrap_or_default(), &preset)
    };
    crate::commands::save_profile(&state, model, base)
}

/// Merge de perfil: campo do preset vence, ausente preserva.
fn merge_profile(
    mut atual: lr_types::tuning::ModelProfile,
    preset: &lr_types::tuning::ModelProfile,
) -> lr_types::tuning::ModelProfile {
    macro_rules! leva {
        ($campo:ident) => {
            if preset.$campo.is_some() {
                atual.$campo = preset.$campo.clone();
            }
        };
    }
    leva!(ctx);
    leva!(ngl);
    leva!(ncmoe);
    leva!(kv_k);
    leva!(kv_v);
    leva!(flash_attn);
    leva!(batch);
    leva!(ubatch);
    leva!(threads);
    leva!(spec);
    leva!(spec_draft_n_max);
    leva!(spec_draft_n_min);
    leva!(spec_draft_p_min);
    leva!(spec_draft_model);
    leva!(mmproj);
    leva!(vision);
    leva!(kv_offload);
    leva!(mmap);
    leva!(mlock);
    leva!(parallel);
    for (k, v) in &preset.extras {
        let chave = lr_types::flags::normalize_key(k);
        if let Some(existente) = atual
            .extras
            .iter_mut()
            .find(|(ek, _)| lr_types::flags::normalize_key(ek) == chave)
        {
            existente.1 = v.clone();
        } else {
            atual.extras.push((k.clone(), v.clone()));
        }
    }
    atual
}

#[tauri::command]
pub fn model_capabilities(state: State<'_, AppState>, model: String) -> CmdResult<ModelCaps> {
    let alvo = model.trim().to_lowercase();
    let sem_ext = alvo.trim_end_matches(".gguf").to_string();
    let artefato = lr_models::scan_local(&state.models_dir)
        .into_iter()
        .find(|a| {
            let nome = a.name.to_lowercase();
            nome == alvo || nome.trim_end_matches(".gguf") == sem_ext
        })
        .ok_or("modelo não encontrado na biblioteca")?;
    let meta = lr_models::read_local_meta(&artefato.primary_path);
    Ok(ModelCaps {
        moe: meta.n_experts.map(|n| n > 0),
        mtp_head: meta.nextn_layers.map(|n| n > 0),
        has_mmproj: artefato.vision_projector.is_some(),
        n_layers: meta.n_layers,
        train_ctx: meta.context_length,
        busy_with: engine_busy_with(&state),
    })
}
