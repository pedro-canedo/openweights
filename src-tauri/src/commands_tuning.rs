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

use crate::commands::{profile_for, restart_engine, stop_engine};
use crate::state::AppState;
use lr_advisor::bench::{self, BenchResult};
use lr_advisor::probe::{ProbeReport, probe};
use lr_advisor::tune::{self, Intent, Measured};
use lr_store::perf::PerfRun;
use lr_types::tuning::{ModelProfile, ProfileSource};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Garante que todo modelo local sem perfil suba já na configuração
/// agêntica: janela de [`tune::AGENT_MIN_CTX`] tokens, KV comprimido só o
/// necessário, `fit` do llama.cpp ligado para as camadas.
///
/// Roda no início do motor, ANTES de escrever o preset (que só é lido no
/// boot). É instantâneo — a conta é pura, sem sonda — e acontece uma vez por
/// modelo: o perfil gravado (`source = Recommended`) responde pelas próximas
/// partidas. Perfil escolhido pela pessoa (`Manual`) ou já recomendado nunca
/// é tocado: este caminho só preenche o vazio, onde antes o `fit` decidia
/// sozinho e numa placa apertada entregava 8k — que mata o modo agente em
/// silêncio.
/// O par ligado, traduzido para os flags que a sonda e o bench entendem.
///
/// Sem isto os dois medem a máquina sozinha enquanto o servidor roda
/// repartido: dois números para duas configurações diferentes, e a tela
/// mostrando o que não vai acontecer.
pub(crate) fn cluster_args(state: &AppState) -> Option<lr_advisor::devices::ClusterArgs> {
    let (rpc_addr, devices, tensor_split) = state.cluster.measure_args_now()?;
    Some(lr_advisor::devices::ClusterArgs {
        rpc_addr,
        devices,
        tensor_split,
    })
}

pub(crate) fn ensure_agent_profiles(state: &AppState) {
    let budget = lr_advisor::MemoryBudget::from_profile(&state.profile)
        .with_extra_vram(state.cluster.remote_vram_now());
    for a in lr_models::scan_local(&state.models_dir) {
        if profile_for(state, &a.name).is_some() {
            continue;
        }
        // O cabeçalho local dá a janela de TREINO (clamp da agêntica) —
        // ler custa menos de um milissegundo por modelo.
        let cabecalho = lr_models::read_local_meta(&a.primary_path);
        let perfil = tune::agent_profile(&budget, a.total_bytes, cabecalho.context_length);
        match state.store.set_model_profile(a.name.trim(), &perfil) {
            Ok(()) => log::info!(
                "perfil agêntico para {}: janela {}, kv {:?}",
                a.name,
                tune::AGENT_MIN_CTX,
                perfil.kv_k
            ),
            Err(e) => log::warn!("não gravei o perfil agêntico de {}: {e}", a.name),
        }
    }
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

    let cluster = cluster_args(&state);
    let budget = lr_advisor::MemoryBudget::from_profile(&state.profile)
        .with_extra_vram(state.cluster.remote_vram_now());
    // Sem cabeçalho GGUF lido, a geometria vem do tamanho do arquivo: é
    // grosseira, mas só decide QUAIS candidatos sondar — quem dá o número
    // final é a sonda.
    let params_estimados = artefato.total_bytes.saturating_mul(2);
    let meta = lr_advisor::ModelMeta::estimate_from_params(params_estimados, 8192);

    // O `ngl` dos candidatos só pode vir do NÚMERO REAL de camadas, lido do
    // cabeçalho do arquivo. A tabela de chute escreveu "48" num modelo de 65
    // camadas e, com `fit = off`, as 17 restantes moraram na CPU: 23 → 4
    // tok/s, sem erro nenhum. Sem leitura, os candidatos vão sem `ngl` e o
    // `fit` do llama.cpp continua ligado.
    let cabecalho = lr_models::read_local_meta(&artefato.primary_path);
    let candidatos = tune::candidates(&budget, &meta, artefato.total_bytes, cabecalho.n_layers);
    let teto_gpu = budget
        .vram_bytes
        .saturating_sub(lr_advisor::tune::MARGEM_VRAM_BYTES);

    let mut medidos: Vec<Measured> = Vec::new();
    for c in candidatos {
        match probe(&dir, &artefato.primary_path, &c.profile, cluster.as_ref()).await {
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

    // Fecha o laço com a tela Descobrir: aqui temos o que a nossa aritmética
    // teria dito e o que a sonda de fato disse, para o mesmo arquivo. Guardar
    // o par é o que permite corrigir as estimativas de antes do download.
    if let Some(primeiro) = medidos.first() {
        let estimado = lr_advisor::evaluate(&budget, &meta, artefato.total_bytes).est_total_bytes;
        let medido = primeiro.report.gpu_bytes() + primeiro.report.host_bytes();
        crate::commands::record_calibration(&state, estimado, medido);
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

// ------------------------------------------------------------- medição ---

/// Uma medição em curso. Só uma por vez: o `llama-bench` quer a placa
/// inteira, e duas ao mesmo tempo mediriam uma à outra.
static MEDINDO: AtomicBool = AtomicBool::new(false);
static CANCELAR: AtomicBool = AtomicBool::new(false);

/// Progresso de uma medição, para a tela não ficar olhando um spinner mudo.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BenchProgress {
    model: String,
    /// Configuração atual (1-based) e quantas ao todo.
    step: usize,
    total: usize,
    /// Resultado da configuração que acabou de ser medida.
    #[serde(skip_serializing_if = "Option::is_none")]
    last: Option<BenchResult>,
}

/// O que a medição concluiu.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchOutcome {
    pub model: String,
    /// Uma entrada por configuração medida, na ordem em que foram medidas.
    pub results: Vec<(ModelProfile, BenchResult)>,
    /// A que rendeu mais.
    pub best: Option<usize>,
    /// A placa esquentou durante a corrida: os números servem, com ressalva.
    pub suspect: bool,
}

/// Mede as configurações propostas nesta máquina.
///
/// Derruba o motor antes de começar (o bench quer a placa inteira) e emite
/// `tune-bench` a cada configuração concluída. Ao fim, grava tudo com a
/// impressão digital da máquina e do build do llama.cpp — é isso que faz o
/// número caducar sozinho quando a placa, o driver ou o runtime mudam.
#[tauri::command]
pub async fn tune_bench(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    profiles: Vec<ModelProfile>,
    force: Option<bool>,
) -> CmdResult<BenchOutcome> {
    if profiles.is_empty() {
        return Err("nada para medir".into());
    }
    let ocupado = crate::commands::engine_busy_with(&state);
    if !ocupado.is_empty() && !force.unwrap_or(false) {
        return Err(format!("engine-busy:{}", ocupado.join(",")));
    }
    let cluster = cluster_args(&state);
    if MEDINDO.swap(true, Ordering::SeqCst) {
        return Err("já existe uma medição em andamento".into());
    }
    CANCELAR.store(false, Ordering::SeqCst);
    let _fim = MedindoGuard;

    let artefato = local_by_name(&state, &model).ok_or("modelo não encontrado na biblioteca")?;
    let runtime = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant)
    };
    let dir = runtime
        .dir
        .ok_or("o runtime do llama.cpp ainda não está instalado")?;

    // O bench e o servidor não dividem a placa.
    let _ = stop_engine(&app, &state).await;

    let total = profiles.len();
    let mut results: Vec<(ModelProfile, BenchResult)> = Vec::new();
    let mut primeiro_tps = 0.0_f64;

    for (i, perfil) in profiles.iter().enumerate() {
        let r = bench::bench(
            &dir,
            &artefato.primary_path,
            perfil,
            cluster.as_ref(),
            &CANCELAR,
        )
        .await;
        match r {
            Ok(res) => {
                if i == 0 {
                    primeiro_tps = res.gen_tps;
                }
                let _ = app.emit(
                    "tune-bench",
                    BenchProgress {
                        model: artefato.name.clone(),
                        step: i + 1,
                        total,
                        last: Some(res.clone()),
                    },
                );
                results.push((perfil.clone(), res));
            }
            Err(lr_advisor::bench::BenchError::Cancelled) => break,
            Err(e) => {
                log::warn!("medição falhou para {}: {e}", artefato.name);
            }
        }
    }

    // Repete o primeiro braço no fim: se a placa esquentou, a comparação
    // inteira está enviesada em favor de quem foi medido primeiro. Com um
    // braço só a re-medição também vale — é ela que permite ao "Medir agora"
    // do histórico marcar um número de placa quente como suspeito (custo:
    // dobra o tempo do bench de 1 perfil).
    let mut suspect = false;
    if !results.is_empty()
        && let Ok(repetido) = bench::bench(
            &dir,
            &artefato.primary_path,
            &profiles[0],
            cluster.as_ref(),
            &CANCELAR,
        )
        .await
    {
        suspect = bench::drifted(primeiro_tps, repetido.gen_tps);
    }

    let machine = state.profile.machine_key();
    let agora = crate::scheduler::now_ms();
    // A série é uma por machine_key, nomeada pela placa principal — None
    // significa "rodou na CPU" e a tela mostra exatamente isso.
    let gpu = state.profile.best_gpu().map(|g| g.name.clone());
    for (perfil, res) in &results {
        let _ = state.store.add_perf_run(&PerfRun {
            machine_key: machine.clone(),
            model_id: artefato.name.clone(),
            profile_key: perfil.key().unwrap_or_default(),
            build_number: res.build_number,
            gen_tps: res.gen_tps,
            prompt_tps: res.prompt_tps,
            gen_stddev: res.gen_stddev,
            gpu_bytes: None,
            source: "bench".into(),
            suspect,
            measured_at: agora,
            gpu_name: gpu.clone(),
            // Os pares INI legíveis do perfil: é o que deixa o histórico
            // mostrar "ngl=99 · ctx=16k" em vez de um hash.
            profile_json: serde_json::to_string(&perfil.to_ini_extras()).ok(),
        });
    }

    let best = results
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.gen_tps.total_cmp(&b.1.1.gen_tps))
        .map(|(i, _)| i);

    // O motor volta como estava; medir não pode deixar o app sem servidor.
    let _ = restart_engine(&app, &state, true).await;

    Ok(BenchOutcome {
        model: artefato.name.clone(),
        results,
        best,
        suspect,
    })
}

/// Para a medição em curso (a configuração atual termina; as próximas não
/// começam — resultado parcial de uma configuração não serve para nada).
#[tauri::command]
pub fn tune_bench_cancel() {
    CANCELAR.store(true, Ordering::SeqCst);
}

/// Libera a trava mesmo quando a medição sai por erro ou `?`.
struct MedindoGuard;

impl Drop for MedindoGuard {
    fn drop(&mut self) {
        MEDINDO.store(false, Ordering::SeqCst);
    }
}

/// Mede especulação (MTP e n-grama) nesta máquina.
///
/// É o eixo do pedido original, e o único que não dá para responder sem gerar
/// de verdade: especulação é configuração do servidor, então cada braço exige
/// o motor de pé — e um reinício entre eles. Custa mais que o `llama-bench`,
/// e por isso é um botão à parte.
///
/// Não aplica nada: devolve os números e deixa a decisão para a tela.
#[tauri::command]
pub async fn tune_spec_bench(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    profile: ModelProfile,
    force: Option<bool>,
) -> CmdResult<crate::spec_bench::SpecOutcome> {
    let ocupado = crate::commands::engine_busy_with(&state);
    if !ocupado.is_empty() && !force.unwrap_or(false) {
        return Err(format!("engine-busy:{}", ocupado.join(",")));
    }
    if MEDINDO.swap(true, Ordering::SeqCst) {
        return Err("já existe uma medição em andamento".into());
    }
    let _fim = MedindoGuard;

    let artefato = local_by_name(&state, &model).ok_or("modelo não encontrado na biblioteca")?;
    // MTP só faz sentido quando o arquivo traz as camadas; oferecê-lo a um
    // GGUF comum seria medir um erro de carga.
    let mut candidatos = vec![
        lr_types::tuning::SpecType::None,
        lr_types::tuning::SpecType::Ngram,
    ];
    if artefato.name.to_lowercase().contains("mtp") {
        candidatos.push(lr_types::tuning::SpecType::Mtp);
    }

    let anterior = profile_for(&state, &model);
    let resultado =
        crate::spec_bench::measure(&app, &state, &artefato.name, &profile, &candidatos).await;

    // A medição mexe no perfil a cada braço: o que estava valendo tem de
    // voltar, dê certo ou dê errado.
    let volta = anterior.unwrap_or_default();
    let _ = state.store.set_model_profile(model.trim(), &volta);
    let _ = restart_engine(&app, &state, true).await;

    resultado
}

// ----------------------------------------------------------- histórico ---

/// Quantas medições o histórico devolve, da mais recente para a mais antiga.
const HISTORICO_LIMITE: usize = 50;

/// Uma linha do histórico de bench, pronta para a tabela da tela.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfRowDto {
    /// Como gravado no banco.
    pub measured_at: i64,
    pub gen_tps: f64,
    pub prompt_tps: Option<f64>,
    pub gen_stddev: Option<f64>,
    pub suspect: bool,
    pub source: String,
    pub build_number: u64,
    pub gpu_name: Option<String>,
    pub profile_key: String,
    /// Pares INI legíveis da configuração medida; `None` em linhas gravadas
    /// antes de `profile_json` existir (a tela cai no profileKey encurtado).
    pub profile_summary: Option<std::collections::BTreeMap<String, String>>,
    pub delta_pct: Option<f64>,
    /// `"ok"` | `"first"` | `"buildChange"` | `"suspect"`.
    pub delta_reason: &'static str,
}

/// O que o card de histórico desenha de uma vez.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfHistoryDto {
    /// Placa principal do hardware ATUAL (`None` = CPU).
    pub gpu_name: Option<String>,
    /// `""` quando o perfil vigente é vazio — a MESMA normalização que o
    /// bench usa ao gravar (`key().unwrap_or_default()`).
    pub current_profile_key: String,
    pub best_profile_key: Option<String>,
    pub rows: Vec<PerfRowDto>,
    pub usage: Vec<lr_store::perf::UsageRow>,
}

/// Os pares INI gravados em `profile_json`, como mapa para a tela.
fn resumo_do_perfil(json: Option<&str>) -> Option<std::collections::BTreeMap<String, String>> {
    let pares: Vec<(String, String)> = serde_json::from_str(json?).ok()?;
    Some(pares.into_iter().collect())
}

/// Histórico de medições deste modelo NESTA máquina.
///
/// A série é recortada pela `machine_key` atual: medições feitas com outra
/// placa ou outro driver ficam de fora por design — a série antiga
/// "aposenta-se" sem ser apagada, e volta se a pessoa voltar atrás.
#[tauri::command]
pub fn perf_history(state: State<'_, AppState>, model_id: String) -> CmdResult<PerfHistoryDto> {
    let machine = state.profile.machine_key();
    // LIMIT+1: a linha extra existe só para ser a antecessora do delta da
    // última linha visível; ela não aparece na resposta.
    let mut runs = state
        .store
        .perf_history_rows(&machine, &model_id, HISTORICO_LIMITE + 1)
        .map_err(err_str)?;
    let anotacoes = lr_store::perf::annotate_deltas(&runs);
    runs.truncate(HISTORICO_LIMITE);

    // Build atual = o da linha mais recente (data-driven, sem perguntar ao
    // runtime): é dentro dele que a melhor configuração é eleita.
    let build_atual = runs.first().map(|r| r.build_number);
    let best_profile_key = runs
        .iter()
        .filter(|r| !r.suspect && Some(r.build_number) == build_atual)
        .max_by(|a, b| a.gen_tps.total_cmp(&b.gen_tps))
        .map(|r| r.profile_key.clone());

    let rows = runs
        .into_iter()
        .zip(anotacoes)
        .map(|(r, (delta_pct, delta_reason))| PerfRowDto {
            measured_at: r.measured_at,
            gen_tps: r.gen_tps,
            prompt_tps: Some(r.prompt_tps),
            gen_stddev: Some(r.gen_stddev),
            suspect: r.suspect,
            source: r.source,
            build_number: r.build_number,
            gpu_name: r.gpu_name,
            profile_key: r.profile_key,
            profile_summary: resumo_do_perfil(r.profile_json.as_deref()),
            delta_pct,
            delta_reason,
        })
        .collect();

    // Nota: o uso real NÃO é recortado por GPU/driver — `messages` não tem
    // machine_key. A tela avisa disso ao lado do bloco.
    let usage = state.store.perf_usage_rows(&model_id).map_err(err_str)?;

    Ok(PerfHistoryDto {
        gpu_name: state.profile.best_gpu().map(|g| g.name.clone()),
        current_profile_key: profile_for(&state, &model_id)
            .and_then(|p| p.key())
            .unwrap_or_default(),
        best_profile_key,
        rows,
        usage,
    })
}

// ---------------------------------------------------------------------------
// Ajuste automático
// ---------------------------------------------------------------------------

/// Uma varredura por vez. O `llama-fit-params` é barato, mas dez modelos em
/// paralelo disputariam a mesma placa e devolveriam memória livre errada.
static AUTO_RODANDO: AtomicBool = AtomicBool::new(false);

/// Chave de "já ajustei este modelo NESTA situação".
///
/// Entra tudo que muda a resposta: a máquina (placa, driver, RAM), a versão do
/// motor e o conjunto de dispositivos — parear com outro PC muda o que cabe
/// tanto quanto trocar de placa. Se a chave bate, não há o que remedir.
fn auto_key(state: &AppState) -> String {
    let par = state
        .cluster
        .measure_args_now()
        .map(|(addr, dev, ts)| format!("{addr}|{dev}|{ts}"))
        .unwrap_or_else(|| "solo".into());
    format!(
        "{}|{}|{}",
        state.profile.machine_key(),
        lr_runtime::PINNED_TAG,
        par
    )
}

fn auto_setting(model: &str) -> String {
    format!("tune.auto.{}", model.trim())
}

/// A busca dirigida para UM modelo: candidatos pela aritmética, memória pela
/// sonda, escolha pelo mesmo `pick` que a tela usa.
///
/// Devolve `None` quando nenhuma sonda respondeu — melhor manter o que estava
/// do que gravar um palpite.
async fn auto_profile_for(
    state: &AppState,
    dir: &std::path::Path,
    artefato: &lr_models::LocalArtifact,
    cluster: Option<&lr_advisor::devices::ClusterArgs>,
) -> Option<ModelProfile> {
    let budget = lr_advisor::MemoryBudget::from_profile(&state.profile)
        .with_extra_vram(state.cluster.remote_vram_now());
    let meta = lr_advisor::ModelMeta::estimate_from_params(
        artefato.total_bytes.saturating_mul(2),
        tune::AGENT_MIN_CTX,
    );
    let cabecalho = lr_models::read_local_meta(&artefato.primary_path);
    let candidatos = tune::candidates(&budget, &meta, artefato.total_bytes, cabecalho.n_layers);
    let teto = budget
        .vram_bytes
        .saturating_sub(lr_advisor::tune::MARGEM_VRAM_BYTES);

    let mut medidos: Vec<Measured> = Vec::new();
    for c in candidatos {
        match probe(dir, &artefato.primary_path, &c.profile, cluster).await {
            Ok(report) => {
                let fits_gpu = budget.vram_bytes > 0 && report.gpu_bytes() <= teto;
                medidos.push(Measured {
                    profile: c.profile,
                    intent: c.intent,
                    report,
                    fits_gpu,
                });
            }
            Err(e) => log::warn!("sonda automática falhou para {}: {e}", artefato.name),
        }
    }
    tune::pick(&medidos).map(|m| m.profile.clone())
}

/// Ajusta sozinho o que ainda não foi ajustado nesta máquina.
///
/// Roda em segundo plano: a garantia barata do boot (janela agêntica, sem
/// `ngl`) continua valendo desde o primeiro segundo, e esta passada a
/// substitui por uma configuração medida assim que os números chegam. O INI do
/// Router é lido no boot, então o que muda aqui vale no próximo start — a
/// interface é avisada pelo evento `tune-auto`.
pub(crate) async fn auto_tune_pending(app: AppHandle, state: &AppState) {
    if AUTO_RODANDO.swap(true, Ordering::SeqCst) {
        return;
    }
    let chave = auto_key(state);
    let cluster = cluster_args(state);
    let dir = {
        let variant = lr_runtime::select_variant(&state.profile);
        state.runtime_mgr.state(variant).dir
    };
    let Some(dir) = dir else {
        AUTO_RODANDO.store(false, Ordering::SeqCst);
        return;
    };

    let mut mudou = 0usize;
    for a in lr_models::scan_local(&state.models_dir) {
        // Escolha da pessoa é lei: o automático só preenche o que ele mesmo
        // deixou, ou o que ainda não tem nada.
        if let Some(p) = crate::commands::profile_for(state, &a.name)
            && p.source == ProfileSource::Manual
        {
            continue;
        }
        let setting = auto_setting(&a.name);
        if state
            .store
            .get_setting(&setting)
            .ok()
            .flatten()
            .is_some_and(|v| v == chave)
        {
            continue;
        }
        if let Some(perfil) = auto_profile_for(state, &dir, &a, cluster.as_ref()).await {
            let anterior = crate::commands::profile_for(state, &a.name);
            if anterior.as_ref() != Some(&perfil) {
                mudou += 1;
            }
            if let Err(e) = state.store.set_model_profile(a.name.trim(), &perfil) {
                log::warn!("não gravei o perfil automático de {}: {e}", a.name);
                continue;
            }
            log::info!(
                "perfil automático de {}: janela {:?}, kv {:?}, ngl {:?}",
                a.name,
                perfil.ctx,
                perfil.kv_k,
                perfil.ngl
            );
        }
        let _ = state.store.set_setting(&setting, &chave);
    }

    AUTO_RODANDO.store(false, Ordering::SeqCst);
    if mudou > 0 {
        let _ = app.emit("tune-auto", mudou);
    }
}

/// Dispara a varredura sem segurar quem chamou.
pub(crate) fn spawn_auto_tune(app: &AppHandle, _state: &AppState) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<AppState>();
        auto_tune_pending(app2.clone(), &state).await;
    });
}
