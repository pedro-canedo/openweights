//! "Ajustar para esta máquina": recomendar, explicar e aplicar.
//!
//! O que sustenta esta tela é uma escolha de projeto: a memória não é
//! estimada por nós, é perguntada ao `llama-fit-params`, que vem no mesmo
//! pacote do motor e responde em menos de dois segundos sem carregar o
//! modelo. A nossa heurística serve só para decidir **o que perguntar** —
//! sondar tudo levaria meio minuto de ampulheta.
//!
//! Aplicar tem rede: o perfil anterior é guardado, o motor reinicia, e se o
//! modelo não carregar a configuração antiga volta sozinha. Sem isso, o
//! recurso seria "aquilo que quebrou o app".

use crate::commands::{profile_for, restart_engine};
use crate::state::AppState;
use lr_advisor::probe::{ProbeReport, probe};
use lr_advisor::tune::{self, Intent, Measured};
use lr_types::tuning::{ModelProfile, ProfileSource};
use serde::Serialize;
use tauri::{AppHandle, State};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Uma configuração proposta, já com a memória medida.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneOption {
    pub profile: ModelProfile,
    pub intent: Intent,
    pub report: ProbeReport,
    pub fits_gpu: bool,
    /// Memória somada nos dispositivos que não são a CPU.
    pub gpu_bytes: u64,
    pub host_bytes: u64,
}

/// O que a tela precisa para desenhar o painel inteiro.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneAdvice {
    pub model: String,
    /// A configuração recomendada (índice em `options`).
    pub recommended: usize,
    pub options: Vec<TuneOption>,
    pub reasons: Vec<lr_advisor::tune::Reason>,
    /// Perfil gravado hoje para este modelo, se houver.
    pub current: Option<ModelProfile>,
    /// VRAM da melhor placa, para a tela mostrar "x de y".
    pub vram_bytes: u64,
    /// Fatos detectados que ainda não viram decisão.
    pub facts: Vec<&'static str>,
}

/// Modelo local pelo nome, tolerando com e sem `.gguf`.
fn local_by_name(state: &AppState, model: &str) -> Option<lr_models::LocalArtifact> {
    let alvo = model.trim().to_lowercase();
    let sem_ext = alvo.trim_end_matches(".gguf").to_string();
    lr_models::scan_local(&state.models_dir)
        .into_iter()
        .find(|a| {
            let nome = a.name.to_lowercase();
            nome == alvo || nome.trim_end_matches(".gguf") == sem_ext
        })
}

/// Calcula a configuração recomendada para um modelo já baixado.
///
/// Custa uma sonda por candidato (menos de dois segundos cada), e por isso a
/// heurística entrega poucos candidatos.
#[tauri::command]
pub async fn tune_advise(state: State<'_, AppState>, model: String) -> CmdResult<TuneAdvice> {
    let artefato = local_by_name(&state, &model).ok_or("modelo não encontrado na biblioteca")?;

    let runtime = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant)
    };
    let dir = runtime
        .dir
        .ok_or("o runtime do llama.cpp ainda não está instalado")?;

    let budget = lr_advisor::MemoryBudget::from_profile(&state.profile);
    // Sem cabeçalho GGUF lido, a geometria vem do tamanho do arquivo: é
    // grosseira, mas só decide QUAIS candidatos sondar — quem dá o número
    // final é a sonda.
    let params_estimados = artefato.total_bytes.saturating_mul(2);
    let meta = lr_advisor::ModelMeta::estimate_from_params(params_estimados, 8192);

    let candidatos = tune::candidates(&budget, &meta, artefato.total_bytes);
    let teto_gpu = budget
        .vram_bytes
        .saturating_sub(lr_advisor::tune::MARGEM_VRAM_BYTES);

    let mut medidos: Vec<Measured> = Vec::new();
    for c in candidatos {
        match probe(&dir, &artefato.primary_path, &c.profile).await {
            Ok(report) => {
                let fits_gpu = budget.vram_bytes > 0 && report.gpu_bytes() <= teto_gpu;
                medidos.push(Measured {
                    profile: c.profile,
                    intent: c.intent,
                    report,
                    fits_gpu,
                });
            }
            // Uma sonda que falha não derruba o painel: as outras seguem, e
            // se nenhuma responder o erro sobe com o motivo.
            Err(e) => log::warn!("sonda falhou para {}: {e}", artefato.name),
        }
    }
    if medidos.is_empty() {
        return Err("não consegui estimar a memória deste modelo".into());
    }

    let escolhido = tune::pick(&medidos).ok_or("nenhum candidato")?.clone();
    let indice = medidos
        .iter()
        .position(|m| m.profile == escolhido.profile)
        .unwrap_or(0);
    let alternativa = medidos.iter().find(|m| m.profile != escolhido.profile);
    let reasons = tune::explain(&escolhido, alternativa, budget.vram_bytes);

    let mut facts: Vec<&'static str> = Vec::new();
    if artefato.name.to_lowercase().contains("mtp") {
        facts.push("mtp");
    }
    if artefato.vision_projector.is_some() {
        facts.push("vision");
    }

    Ok(TuneAdvice {
        model: artefato.name.clone(),
        recommended: indice,
        options: medidos
            .into_iter()
            .map(|m| TuneOption {
                gpu_bytes: m.report.gpu_bytes(),
                host_bytes: m.report.host_bytes(),
                fits_gpu: m.fits_gpu,
                profile: m.profile,
                intent: m.intent,
                report: m.report,
            })
            .collect(),
        reasons,
        current: profile_for(&state, &model),
        vram_bytes: budget.vram_bytes,
        facts,
    })
}

/// Resultado de aplicar uma configuração.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneApplied {
    /// `false` quando o modelo não carregou e a configuração anterior voltou.
    pub ok: bool,
    /// O que deu errado, quando deu.
    pub error: Option<String>,
    /// Perfil em vigor agora (o novo, ou o antigo restaurado).
    pub profile: Option<ModelProfile>,
}

/// Grava o perfil, reinicia o motor e confere que o modelo carrega.
///
/// Se não carregar, restaura o perfil anterior e reinicia de novo. É a
/// diferença entre "o app sugeriu e quebrou" e "o app tentou e voltou".
#[tauri::command]
pub async fn tune_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    profile: ModelProfile,
    force: Option<bool>,
) -> CmdResult<TuneApplied> {
    let anterior = profile_for(&state, &model);
    let mut novo = profile;
    if novo.source == ProfileSource::Manual {
        novo.source = ProfileSource::Recommended;
    }

    state
        .store
        .set_model_profile(model.trim(), &novo)
        .map_err(err_str)?;

    // O INI é lido no boot do motor: sem reiniciar, nada disto vale.
    if let Err(e) = restart_engine(&app, &state, force.unwrap_or(false)).await {
        // Guarda de ocupação não é falha da configuração: devolve o motivo
        // sem desfazer nada, porque a escolha continua válida.
        if e.starts_with("engine-busy:") {
            return Err(e);
        }
        return Ok(rollback(&app, &state, &model, anterior, e).await);
    }

    match carga_de_prova(&state, &model).await {
        Ok(()) => Ok(TuneApplied {
            ok: true,
            error: None,
            profile: Some(novo),
        }),
        Err(e) => Ok(rollback(&app, &state, &model, anterior, e).await),
    }
}

/// Devolve o perfil anterior e reinicia o motor com ele.
async fn rollback(
    app: &AppHandle,
    state: &AppState,
    model: &str,
    anterior: Option<ModelProfile>,
    motivo: String,
) -> TuneApplied {
    let volta = anterior.unwrap_or_default();
    let _ = state.store.set_model_profile(model.trim(), &volta);
    // `force`: a guarda já passou uma vez, e deixar o motor no estado quebrado
    // seria pior do que interromper o que quer que tenha começado agora.
    let _ = restart_engine(app, state, true).await;
    TuneApplied {
        ok: false,
        error: Some(motivo),
        profile: (!volta.is_empty()).then_some(volta),
    }
}

/// Prova que a configuração funciona: pede UM token ao modelo.
///
/// O processo subir não prova nada — o modelo só é carregado no primeiro
/// pedido, e é aí que a memória falta. Um token é o menor pedido que força a
/// carga, e é ele que faz o erro de "não coube" aparecer aqui, onde ainda dá
/// para voltar atrás, em vez de na cara da pessoa na primeira mensagem.
async fn carga_de_prova(state: &AppState, model: &str) -> Result<(), String> {
    let endpoint = state.agent_endpoint().await?;
    let client = lr_engine::LlamaClient::new(&endpoint.base_url)
        .with_optional_api_key(endpoint.api_key.clone());

    let mut req = lr_engine::ChatRequest::new(
        model.trim(),
        vec![lr_engine::ChatMessage::user("oi".to_string())],
    );
    req.max_tokens = Some(1);
    client
        .complete_once(&req)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
