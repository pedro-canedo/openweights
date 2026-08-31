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
use std::time::Duration;
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
/// Traduz o cabeçalho do GGUF para a geometria que o advisor entende.
///
/// `head_dim` sai de `attention.key_length` quando declarado — é o número
/// exato — e o resto o advisor completa sozinho com o que faltar.
pub(crate) fn geometria_do_cabecalho(m: &lr_models::LocalGgufMeta) -> lr_advisor::Geometry {
    lr_advisor::Geometry {
        n_layers: m.n_layers,
        n_kv_heads: m.n_kv_heads,
        n_heads: m.n_heads,
        head_dim: m.key_length,
        d_model: m.embedding_length,
        n_experts: m.n_experts,
        n_experts_used: m.n_experts_used,
        expert_ffn: m.expert_ffn_length,
        shared_ffn: m.expert_shared_ffn_length,
    }
}

/// O menor `n-cpu-moe` que faz esta configuração caber na placa.
///
/// É a receita do manual — "comece com todos os especialistas na CPU e vá
/// descendo o número até faltar VRAM, então volte uma" — só que sem tentativa
/// e erro: a sonda responde quanto cada tentativa custa de memória **sem
/// carregar o modelo**, e a relação é monótona (mais camadas de especialista
/// na CPU, menos VRAM), então uma busca binária acha o ponto em ~7 perguntas
/// de menos de dois segundos em vez de dezenas de carregamentos.
///
/// Devolve `None` quando nem com TODOS os especialistas fora a placa dá conta
/// — aí a resposta não é este caminho, e quem decide é o veredito denso.
async fn menor_ncmoe(
    dir: &std::path::Path,
    model_path: &std::path::Path,
    base: &ModelProfile,
    cluster: Option<&lr_advisor::devices::ClusterArgs>,
    teto: u64,
    camadas: u32,
) -> Option<u32> {
    let cabe = async |n: u32| -> bool {
        let tentativa = ModelProfile {
            ncmoe: Some(n),
            ..base.clone()
        };
        matches!(
            probe(dir, model_path, &tentativa, cluster).await,
            Ok(r) if r.gpu_bytes() <= teto
        )
    };

    // O extremo seguro primeiro: se nem ele couber, não há ponto nenhum.
    if !cabe(camadas).await {
        return None;
    }
    let (mut lo, mut hi) = (0u32, camadas);
    while lo < hi {
        let meio = lo + (hi - lo) / 2;
        // Sonda que falha conta como "não coube": empurrar o número para
        // cima erra para o lado de carregar, que é o lado seguro.
        if cabe(meio).await {
            hi = meio;
        } else {
            lo = meio + 1;
        }
    }
    Some(lo)
}

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
    // final é a sonda. Com o cabeçalho, ela deixa de ser chute: camadas,
    // cabeças e especialistas vêm do arquivo.
    let params_estimados = artefato.total_bytes.saturating_mul(2);
    let cabecalho = lr_models::read_local_meta(&artefato.primary_path);
    let meta = lr_advisor::ModelMeta::from_geometry(
        params_estimados,
        8192,
        &geometria_do_cabecalho(&cabecalho),
    );

    // O `ngl` dos candidatos só pode vir do NÚMERO REAL de camadas, lido do
    // cabeçalho do arquivo. A tabela de chute escreveu "48" num modelo de 65
    // camadas e, com `fit = off`, as 17 restantes moraram na CPU: 23 → 4
    // tok/s, sem erro nenhum. Sem leitura, os candidatos vão sem `ngl` e o
    // `fit` do llama.cpp continua ligado.
    let candidatos = tune::candidates(&budget, &meta, artefato.total_bytes, cabecalho.n_layers);
    let teto_gpu = budget
        .vram_bytes
        .saturating_sub(lr_advisor::tune::MARGEM_VRAM_BYTES);

    // Onde parar de empurrar especialista para a CPU. A busca roda UMA vez,
    // no primeiro candidato que pede — e o número serve aos outros, que são
    // medidos com ele como qualquer outra configuração. Repetir a busca por
    // candidato multiplicaria por quatro uma espera que a pessoa vê.
    let mut ncmoe_base: Option<u32> = None;
    if let Some(c) = candidatos.iter().find(|c| c.busca_ncmoe)
        && let Some(camadas) = cabecalho.n_layers
    {
        ncmoe_base = menor_ncmoe(
            &dir,
            &artefato.primary_path,
            &c.profile,
            cluster.as_ref(),
            teto_gpu,
            camadas,
        )
        .await;
        log::info!(
            "{}: especialistas na CPU a partir da camada {:?}",
            artefato.name,
            ncmoe_base
        );
    }

    let mut medidos: Vec<Measured> = Vec::new();
    for c in candidatos {
        let c = tune::Candidate {
            profile: ModelProfile {
                ncmoe: c.busca_ncmoe.then_some(ncmoe_base).flatten(),
                ..c.profile
            },
            ..c
        };
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
    if tem_cabeca_mtp(&cabecalho, &artefato.name) {
        facts.push("mtp");
    }
    if cabecalho.n_experts.is_some_and(|n| n > 0) {
        facts.push("moe");
        // Especular num MoE com os especialistas fora da placa costuma custar
        // mais do que rende: adivinhar quatro tokens deixa de acordar oito
        // especialistas e passa a acordar quase todos, e cada um a mais é
        // outra viagem pelo barramento de memória. Não é proibição — é o
        // aviso de que aqui a resposta tem que ser medida.
        if escolhido.profile.ncmoe.is_some_and(|n| n > 0) {
            facts.push("specOnMoe");
        }
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
    let endpoint = state.llama_endpoint().await?;
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

/// Há uma medição em curso? O coletor de estatísticas pergunta antes de
/// carimbar "a máquina está em uso": a própria bateria gera tokens, e sem
/// esta pergunta ela se marcaria como tráfego e nunca sairia do portão de
/// ocioso que ela mesma espera.
pub(crate) fn medindo() -> bool {
    MEDINDO.load(Ordering::SeqCst)
}
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
/// A curva de um botão, medida de verdade.
///
/// Duas perguntas não têm resposta calculável, só medível: **onde parar** de
/// empurrar especialista para a CPU, e **de que tamanho** compensa a passada
/// de prompt. As duas têm o mesmo formato — um número, uma curva com um
/// joelho — e o `llama-bench` aceita lista em qualquer parâmetro, então a
/// curva inteira sai de UMA invocação, com um carregamento de modelo só.
///
/// Para `ncmoe` a varredura começa no MENOR valor que a sonda diz caber e
/// sobe: valores abaixo dele não carregam, e um ponto que não carrega
/// derruba a invocação inteira, levando junto os pontos que carregariam.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepOutcome {
    pub dim: lr_advisor::bench::SweepDim,
    pub points: Vec<lr_advisor::bench::SweepPoint>,
    /// Com que tamanho de prompt a curva foi medida.
    pub n_prompt: u32,
}

#[tauri::command]
pub async fn tune_sweep(
    app: AppHandle,
    state: State<'_, AppState>,
    model: String,
    dim: String,
    force: Option<bool>,
) -> CmdResult<SweepOutcome> {
    use lr_advisor::bench::SweepDim;
    let dim = match dim.as_str() {
        "ncmoe" => SweepDim::Ncmoe,
        "ubatch" => SweepDim::Ubatch,
        outro => return Err(format!("dimensão desconhecida: {outro}")),
    };
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
    let cabecalho = lr_models::read_local_meta(&artefato.primary_path);

    // A varredura mede o motor por baixo: o servidor não pode estar de pé.
    let _ = stop_engine(&app, &state).await;

    let base = profile_for(&state, &model).unwrap_or_default();
    let valores = match dim {
        SweepDim::Ubatch => vec![128, 256, 512, 1024, 2048],
        SweepDim::Ncmoe => {
            let camadas = cabecalho
                .n_layers
                .ok_or("o cabeçalho do arquivo não diz quantas camadas o modelo tem")?;
            let budget = lr_advisor::MemoryBudget::from_profile(&state.profile)
                .with_extra_vram(state.cluster.remote_vram_now());
            let teto = budget
                .vram_bytes
                .saturating_sub(lr_advisor::tune::MARGEM_VRAM_BYTES);
            let piso = menor_ncmoe(
                &dir,
                &artefato.primary_path,
                &ModelProfile {
                    ngl: Some(camadas),
                    ncmoe: None,
                    ..base.clone()
                },
                cluster.as_ref(),
                teto,
                camadas,
            )
            .await
            .ok_or("nem com todos os especialistas na CPU este modelo cabe nesta placa")?;
            // Do piso para cima: é o lado onde toda configuração carrega, e a
            // curva mostra o que se paga por folgar demais.
            let mut v: Vec<u32> = [0u32, 2, 4, 8, 16]
                .iter()
                .map(|d| (piso + d).min(camadas))
                .collect();
            v.dedup();
            v
        }
    };

    let perfil = ModelProfile {
        // Com o `ngl` fixo, a curva mede uma coisa só: onde os especialistas
        // moram. Sem ele, o `fit` do llama.cpp mexeria nas camadas a cada
        // ponto e a curva descreveria duas decisões ao mesmo tempo.
        ngl: cabecalho.n_layers.or(base.ngl),
        ..base
    };
    let points = lr_advisor::bench::sweep(
        &dir,
        &artefato.primary_path,
        &perfil,
        cluster.as_ref(),
        dim,
        &valores,
        &CANCELAR,
    )
    .await
    .map_err(err_str)?;

    let _ = restart_engine(&app, &state, true).await;
    Ok(SweepOutcome {
        dim,
        points,
        n_prompt: lr_advisor::bench::N_PROMPT_LONGO,
    })
}

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
    let agora = crate::commands::now_ms();
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
            n_prompt: Some(res.n_prompt),
            n_depth: Some(res.n_depth),
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
        lr_types::tuning::SpecSet::new([lr_types::tuning::SpecType::None]),
        lr_types::tuning::SpecSet::new([lr_types::tuning::SpecType::NgramMod]),
    ];
    if tem_cabeca_mtp(
        &lr_models::read_local_meta(&artefato.primary_path),
        &artefato.name,
    ) {
        candidatos.push(lr_types::tuning::SpecSet::new([
            lr_types::tuning::SpecType::DraftMtp,
        ]));
        // O braço que o motor sempre aceitou e o app nunca ofereceu: os dois
        // juntos. Um rascunho adivinha texto novo, o n-grama adivinha o que
        // já está no prompt — e um agente de código passa o dia reescrevendo
        // arquivos que ele mesmo acabou de ler.
        candidatos.push(lr_types::tuning::SpecSet::new([
            lr_types::tuning::SpecType::DraftMtp,
            lr_types::tuning::SpecType::NgramMod,
        ]));
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
    /// Com que tamanho de prompt o `prompt_tps` desta linha foi medido —
    /// `None` em linhas antigas. A tela mostra ao lado do número, porque
    /// 800 tok/s num prompt de 512 e 300 num de 4096 não são o mesmo eixo.
    pub n_prompt: Option<u32>,
    pub delta_pct: Option<f64>,
    /// `"ok"` | `"first"` | `"buildChange"` | `"suspect"` | `"promptChanged"`.
    pub delta_reason: &'static str,
    /// Variação do processamento do prompt, quando as duas medições usaram o
    /// mesmo tamanho de prompt.
    pub prompt_delta_pct: Option<f64>,
    pub prompt_delta_reason: &'static str,
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
        .map(|(r, d)| PerfRowDto {
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
            n_prompt: r.n_prompt,
            delta_pct: d.gen_pct,
            delta_reason: d.gen_reason,
            prompt_delta_pct: d.prompt_pct,
            prompt_delta_reason: d.prompt_reason,
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
/// O arquivo traz as camadas de previsão de múltiplos tokens?
///
/// `nextn_predict_layers` é o que o llama.cpp de fato lê para aceitar
/// `--spec-type draft-mtp`. O NOME do arquivo só opina quando o cabeçalho não
/// diz nada, porque ausência ali é "não sei", não "não tem" — arquitetura
/// nova pode usar outra chave. Confiar no nome era o bug que deixava o
/// Qwen3.8-27B (sem "mtp" no nome, com a cabeça no arquivo) fora da medição.
pub(crate) fn tem_cabeca_mtp(meta: &lr_models::LocalGgufMeta, nome: &str) -> bool {
    match meta.nextn_layers {
        Some(n) => n > 0,
        None => nome.to_lowercase().contains("mtp"),
    }
}

/// Leva a especulação de um perfil para outro.
///
/// A fase de memória (`tune::candidates` → `pick`) só decide janela, camadas e
/// cache: ela devolve um perfil com `spec: None`, e gravá-lo inteiro por cima
/// apagaria uma especulação que custou minutos de medição para ser escolhida.
/// Como a chave do ajuste automático é invalidada por troca de driver, build
/// ou cluster, sem isto o recurso funcionaria uma vez e sumiria.
pub(crate) fn carry_spec(de: &ModelProfile, para: &mut ModelProfile) {
    para.spec = de.spec.clone();
    para.spec_draft_n_max = de.spec_draft_n_max;
    para.spec_draft_n_min = de.spec_draft_n_min;
    para.spec_draft_p_min = de.spec_draft_p_min;
    para.spec_draft_model = de.spec_draft_model.clone();
}

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
    // Uma medição em curso reinicia o motor a cada braço, e cada reinício
    // passa por aqui (`start_engine` → `spawn_auto_tune`). Sem esta guarda, a
    // varredura reescreveria o perfil DO MODELO SENDO MEDIDO no meio da
    // bateria, e o resultado viraria ruído sem ninguém perceber.
    if MEDINDO.load(Ordering::SeqCst) {
        return;
    }
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
        if let Some(mut perfil) = auto_profile_for(state, &dir, &a, cluster.as_ref()).await {
            let anterior = crate::commands::profile_for(state, &a.name);
            // A memória manda na memória; a especulação que já foi medida
            // continua valendo.
            if let Some(ant) = anterior.as_ref() {
                carry_spec(ant, &mut perfil);
            }
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
    // Memória primeiro, especulação depois: o quanto a especulação rende
    // depende da janela e das camadas que acabaram de ser decididas.
    spawn_auto_spec(&app);
}

/// Dispara a varredura sem segurar quem chamou.
pub(crate) fn spawn_auto_tune(app: &AppHandle, _state: &AppState) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<AppState>();
        auto_tune_pending(app2.clone(), &state).await;
    });
}

// ------------------------------------------- especulação automática ---

/// Uma bateria de especulação por vez, e cancelável.
static SPEC_CANCELAR: AtomicBool = AtomicBool::new(false);

/// Carência depois de o motor subir. Ninguém liga o servidor para vê-lo
/// reiniciar seis vezes em seguida.
const SPEC_ESPERA_INICIAL: Duration = Duration::from_secs(180);
/// Quanto tempo sem tráfego conta como "a máquina está livre".
const SPEC_OCIOSO_MIN: Duration = Duration::from_secs(300);
/// Teto da espera pelo ocioso. Passado isto, fica para o próximo boot.
const SPEC_ESPERA_MAX: Duration = Duration::from_secs(1800);
/// Intervalo entre duas conferências do portão de ocioso.
const SPEC_TENTATIVA: Duration = Duration::from_secs(60);

/// Setting do interruptor: `"off"` desliga a medição automática.
const SPEC_AUTO_SETTING: &str = "tune.spec.auto";

fn spec_setting(model: &str) -> String {
    format!("tune.spec.{}", model.trim())
}

/// O que ficou decidido para um modelo, e em que situação.
///
/// A chave inclui o `key()` do perfil além da situação da máquina: tokens por
/// segundo dependem de janela, camadas e cache, então mexer na memória reabre
/// a pergunta da especulação — de graça, porque a chave deixa de bater
/// sozinha.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecDecision {
    pub key: String,
    pub spec: Vec<lr_types::tuning::SpecType>,
    pub gain_pct: Option<f64>,
    /// `applied` | `inconclusive` | `rejected` | `deferred` | `unverifiable`
    pub verdict: String,
    pub at: i64,
}

fn spec_key(state: &AppState, perfil: &ModelProfile) -> String {
    format!(
        "{}|{}",
        auto_key(state),
        perfil.key().unwrap_or_else(|| "vazio".into())
    )
}

/// Interrompe a bateria automática.
#[tauri::command]
pub async fn tune_spec_cancel() -> CmdResult<()> {
    SPEC_CANCELAR.store(true, Ordering::SeqCst);
    Ok(())
}

/// Mede a especulação sozinha, uma vez por modelo/máquina/motor.
///
/// Roda DEPOIS da fase de memória (é ela quem chama), porque o resultado
/// depende da janela e das camadas já decididas. Espera a máquina ficar livre:
/// são seis reinícios do motor, e fazer isso enquanto alguém conversa seria
/// trocar velocidade futura por indisponibilidade agora.
pub(crate) async fn auto_spec_pending(app: AppHandle, state: &AppState) {
    if state
        .store
        .get_setting(SPEC_AUTO_SETTING)
        .ok()
        .flatten()
        .is_some_and(|v| v == "off")
    {
        return;
    }
    SPEC_CANCELAR.store(false, Ordering::SeqCst);
    tokio::time::sleep(SPEC_ESPERA_INICIAL).await;

    // Um modelo por vez: varrer a biblioteca inteira levaria uma hora de
    // servidor indo e voltando. O carregado é o que a pessoa está usando.
    let Some(modelo) = modelo_em_foco(state).await else {
        return;
    };
    let Some(perfil) = crate::commands::profile_for(state, &modelo) else {
        return;
    };
    // Escolha da pessoa é lei, aqui como no ajuste de memória.
    if perfil.source == ProfileSource::Manual {
        return;
    }
    let chave = spec_key(state, &perfil);
    if state
        .store
        .get_setting(&spec_setting(&modelo))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<SpecDecision>(&v).ok())
        .is_some_and(|d| d.key == chave)
    {
        return;
    }

    if !esperar_ocioso(state).await {
        gravar_decisao(state, &modelo, &chave, &[], None, "deferred");
        return;
    }

    if MEDINDO.swap(true, Ordering::SeqCst) {
        return; // uma medição manual tem precedência
    }
    let _fim = MedindoGuard;
    let _ = app.emit(
        "tune-spec",
        serde_json::json!({ "phase": "start", "model": modelo }),
    );

    let candidatos = candidatos_de_spec(state, &modelo);
    let resultado = crate::spec_bench::measure(&app, state, &modelo, &perfil, &candidatos).await;
    let Ok(outcome) = resultado else {
        let _ = restart_engine(&app, state, true).await;
        return;
    };

    // Aplica só o que foi medido, no perfil RELIDO: a fase de memória pode
    // tê-lo mudado enquanto isto rodava.
    let mut verdict = "inconclusive";
    let mut escolhido: Vec<lr_types::tuning::SpecType> = Vec::new();
    let mut ganho = None;
    if outcome.quality_gate == crate::spec_bench::QualityGate::Unverifiable {
        verdict = "unverifiable";
    } else if let Some(i) = outcome.best
        && !outcome.inconclusive
    {
        let braco = &outcome.arms[i];
        let base = outcome.arms[outcome.reference].avg_tps;
        ganho = (base > 0.0).then(|| (braco.avg_tps - base) / base * 100.0);
        escolhido = braco.spec.iter().collect();
        let atual = crate::commands::profile_for(state, &modelo).unwrap_or_default();
        let novo = ModelProfile {
            spec: Some(braco.spec.clone()),
            source: ProfileSource::Tested,
            ..atual
        };
        if state.store.set_model_profile(modelo.trim(), &novo).is_ok() {
            verdict = "applied";
        }
    } else if !outcome.rejected.is_empty() {
        verdict = "rejected";
    }
    gravar_decisao(state, &modelo, &chave, &escolhido, ganho, verdict);

    let _ = restart_engine(&app, state, true).await;
    let _ = app.emit(
        "tune-spec",
        serde_json::json!({ "phase": "done", "model": modelo, "outcome": outcome, "verdict": verdict }),
    );
}

/// Os braços que vale medir para este modelo — incluindo o combinado, que é o
/// que o motor sempre aceitou e o app nunca ofereceu.
fn candidatos_de_spec(state: &AppState, modelo: &str) -> Vec<lr_types::tuning::SpecSet> {
    use lr_types::tuning::{SpecSet, SpecType};
    let mut v = vec![
        SpecSet::new([SpecType::None]),
        SpecSet::new([SpecType::NgramMod]),
    ];
    if let Some(a) = local_by_name(state, modelo)
        && tem_cabeca_mtp(&lr_models::read_local_meta(&a.primary_path), &a.name)
    {
        v.push(SpecSet::new([SpecType::DraftMtp]));
        v.push(SpecSet::new([SpecType::DraftMtp, SpecType::NgramMod]));
    }
    v
}

/// O modelo que o Router tem carregado agora.
async fn modelo_em_foco(state: &AppState) -> Option<String> {
    let cfg = {
        let guard = state.server.lock().await;
        match guard.as_ref() {
            Some(s) if s.is_spawned() => s.config().clone(),
            _ => return None,
        }
    };
    lr_engine::LlamaServer::new(cfg)
        .models_status()
        .await
        .ok()?
        .into_iter()
        .find(|m| m.state == "loaded")
        .map(|m| m.id)
        .filter(|id| !id.ends_with(crate::commands::VISION_SUFFIX))
}

/// Espera a máquina ficar livre. `false` = desistiu por ora.
async fn esperar_ocioso(state: &AppState) -> bool {
    let limite = std::time::Instant::now() + SPEC_ESPERA_MAX;
    loop {
        if SPEC_CANCELAR.load(Ordering::SeqCst) {
            return false;
        }
        let ultimo = state.last_engine_use.load(Ordering::SeqCst);
        let parado = crate::commands::now_ms().saturating_sub(ultimo);
        if ultimo == 0 || parado >= SPEC_OCIOSO_MIN.as_millis() as i64 {
            return true;
        }
        if std::time::Instant::now() >= limite {
            return false;
        }
        tokio::time::sleep(SPEC_TENTATIVA).await;
    }
}

fn gravar_decisao(
    state: &AppState,
    modelo: &str,
    chave: &str,
    spec: &[lr_types::tuning::SpecType],
    gain_pct: Option<f64>,
    verdict: &str,
) {
    let d = SpecDecision {
        key: chave.to_string(),
        spec: spec.to_vec(),
        gain_pct,
        verdict: verdict.to_string(),
        at: crate::commands::now_ms(),
    };
    if let Ok(json) = serde_json::to_string(&d) {
        let _ = state.store.set_setting(&spec_setting(modelo), &json);
    }
}

/// Dispara a medição automática sem segurar quem chamou.
pub(crate) fn spawn_auto_spec(app: &AppHandle) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<AppState>();
        auto_spec_pending(app2.clone(), &state).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(nextn: Option<u32>) -> lr_models::LocalGgufMeta {
        lr_models::LocalGgufMeta {
            nextn_layers: nextn,
            ..Default::default()
        }
    }

    /// O cabeçalho manda. O nome do arquivo só fala quando o cabeçalho cala —
    /// e era confiar nele que deixava o Qwen3.8-27B, que TEM a cabeça e não
    /// tem "mtp" no nome, fora da medição de especulação.
    #[test]
    fn the_mtp_head_is_read_from_the_file_not_from_its_name() {
        assert!(tem_cabeca_mtp(&meta(Some(2)), "Qwen3.8-27B-UD-IQ4_XS.gguf"));
        assert!(
            !tem_cabeca_mtp(&meta(Some(0)), "modelo-mtp.gguf"),
            "zero camadas é 'não tem', mesmo com o nome prometendo"
        );
        assert!(tem_cabeca_mtp(&meta(None), "Qwen3-MTP-Q4.gguf"));
        assert!(!tem_cabeca_mtp(&meta(None), "llama-3-8b.gguf"));
    }

    /// A fase de memória decide memória. A especulação medida atravessa a
    /// varredura inteira — senão ela funcionaria uma vez e sumiria na próxima
    /// troca de driver.
    #[test]
    fn a_memory_pass_keeps_the_speculation_that_was_measured() {
        let medido = ModelProfile {
            spec: Some(lr_types::tuning::SpecType::DraftMtp.into()),
            spec_draft_n_max: Some(4),
            spec_draft_p_min: Some(0.25),
            ctx: Some(8192),
            ..ModelProfile::default()
        };
        let mut novo = ModelProfile {
            ctx: Some(65_536),
            ngl: Some(65),
            ..ModelProfile::default()
        };
        carry_spec(&medido, &mut novo);

        assert_eq!(novo.spec, Some(lr_types::tuning::SpecType::DraftMtp.into()));
        assert_eq!(novo.spec_draft_n_max, Some(4));
        assert_eq!(novo.spec_draft_p_min, Some(0.25));
        // E não desfaz o trabalho da memória.
        assert_eq!(novo.ctx, Some(65_536));
        assert_eq!(novo.ngl, Some(65));
    }
}
