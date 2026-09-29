//! Exportar e importar a configuração: o que a pessoa ajustou (servidor,
//! perfis por modelo, presets, provedores), num JSON que se leva para outra
//! máquina ou guarda de reserva.
//!
//! Só o que é escolha entra: nada de estado da máquina (motor instalado,
//! versão do 9router, pastas). Segredos (chave da API local, chave da
//! OpenRouter, token do Hugging Face) ficam de fora, e só vão se a pessoa
//! marcar — o arquivo então é tão sensível quanto as próprias chaves.
//!
//! Importar mescla: o que o arquivo traz substitui o que existe, o resto fica.
//! Segredos que o arquivo não traz nunca são apagados.

use lr_store::Store;
use serde_json::{Map, Value, json};
use tauri::State;

use crate::state::AppState;

pub const FORMATO: u64 = 1;

/// Settings simples que viajam como estão (texto). Todos são escolhas.
const SETTINGS_SERVIDOR: &[&str] = &[
    "active_engine",
    "server_port",
    "server_lan",
    "server_models_max",
    "server_parallel",
    // JSON: flags globais e variáveis de ambiente do motor.
    "server_extra_flags",
    "server_env_vars",
];
const SEGREDOS_SIMPLES: &[&str] = &["server_api_key", "hf_token"];
const PROVEDORES: &str = "providers.config";

/// Um nome que parece guardar um segredo (`HF_TOKEN`, `--api-key`…).
fn parece_segredo(nome: &str) -> bool {
    let n = nome.to_ascii_uppercase();
    [
        "TOKEN",
        "KEY",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
    ]
    .iter()
    .any(|p| n.contains(p))
}

/// Variáveis que mudam o que o processo do motor CARREGA: um arquivo alheio
/// não pode pôr uma delas (`LD_PRELOAD` roda código no próximo start).
fn env_perigosa(nome: &str) -> bool {
    let n = nome.trim().to_ascii_uppercase();
    n == "PATH" || n.starts_with("LD_") || n.starts_with("DYLD_") || n == "PYTHONPATH"
}

/// `server_env_vars` sem os segredos: o export "sem chaves" não pode levar um
/// `HF_TOKEN` posto nas variáveis do motor.
fn env_sem_segredos(json: &str) -> String {
    match serde_json::from_str::<Vec<lr_types::flags::EnvVar>>(json) {
        Ok(v) => serde_json::to_string(
            &v.into_iter()
                .filter(|e| !parece_segredo(&e.key))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".into()),
        Err(_) => "[]".into(),
    }
}

/// `server_extra_flags` sem as gerenciadas pelo app e sem as que carregam segredo.
fn flags_sem_segredos(json: &str) -> String {
    match serde_json::from_str::<Vec<lr_types::flags::GlobalFlag>>(json) {
        Ok(v) => serde_json::to_string(
            &v.into_iter()
                .filter(|f| {
                    let k = lr_types::flags::normalize_key(&f.key);
                    !lr_types::flags::managed_keys().contains(&k.as_str()) && !parece_segredo(&k)
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".into()),
        Err(_) => "[]".into(),
    }
}

/// O que um import fez, para a tela contar.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resumo {
    /// Itens do arquivo que não foram aplicados por não serem seguros ou
    /// válidos (uma variável `LD_PRELOAD`, uma porta que não é número…).
    pub ignored: usize,
    pub settings: usize,
    pub profiles: usize,
    pub presets: usize,
    pub engine_presets: usize,
    pub providers: bool,
    pub secrets: usize,
}

fn provedores_de(store: &Store) -> Value {
    store
        .get_setting(PROVEDORES)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn exportar(
    store: &Store,
    incluir_segredos: bool,
    versao_do_app: &str,
) -> Result<Value, String> {
    let mut server = Map::new();
    for k in SETTINGS_SERVIDOR {
        if let Some(v) = store.get_setting(k).map_err(|e| e.to_string())? {
            let v = match (*k, incluir_segredos) {
                ("server_env_vars", false) => env_sem_segredos(&v),
                ("server_extra_flags", false) => flags_sem_segredos(&v),
                _ => v,
            };
            server.insert((*k).into(), Value::String(v));
        }
    }

    let profiles: Vec<Value> = store
        .model_profiles()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(model, profile)| json!({ "model": model, "profile": profile }))
        .collect();
    let mut profiles = profiles;
    profiles.sort_by_key(|p| p["model"].as_str().unwrap_or("").to_string());

    let presets: Vec<Value> = store
        .list_presets()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| json!({ "name": p.name, "json": p.json }))
        .collect();
    let engine_presets: Vec<Value> = store
        .list_engine_presets("model")
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| json!({ "name": p.name, "json": p.json }))
        .collect();

    // Provedores: só as escolhas (ligar, favoritos, porta), sem estado de
    // instalação. As chaves entram separadas, sob `secrets`.
    let prov = provedores_de(store);
    let mut providers = Map::new();
    if let Some(o) = prov.get("openRouter") {
        providers.insert(
            "openRouter".into(),
            json!({
                "enabled": o.get("enabled").cloned().unwrap_or(json!(false)),
                "favorites": o.get("favorites").cloned().unwrap_or(json!([])),
            }),
        );
    }
    if let Some(n) = prov.get("nineRouter")
        && let Some(port) = n.get("port")
    {
        providers.insert("nineRouter".into(), json!({ "port": port }));
    }

    let mut saida = json!({
        "format": FORMATO,
        "app": "OpenWeights",
        "appVersion": versao_do_app,
        "server": server,
        "profiles": profiles,
        "presets": presets,
        "enginePresets": engine_presets,
        "providers": providers,
        "includesSecrets": incluir_segredos,
    });

    if incluir_segredos {
        let mut secrets = Map::new();
        for k in SEGREDOS_SIMPLES {
            if let Some(v) = store.get_setting(k).map_err(|e| e.to_string())? {
                secrets.insert((*k).into(), Value::String(v));
            }
        }
        if let Some(k) = prov.pointer("/openRouter/apiKey").and_then(Value::as_str)
            && !k.is_empty()
        {
            secrets.insert("openRouterApiKey".into(), Value::String(k.into()));
        }
        saida["secrets"] = Value::Object(secrets);
    }
    Ok(saida)
}

/// Confere o arquivo sem aplicar nada: o que ele diz ter, ou por que não serve.
pub fn conferir(doc: &Value) -> Result<(), String> {
    if doc.get("app").and_then(Value::as_str) != Some("OpenWeights") {
        return Err("este arquivo não é uma exportação do OpenWeights".into());
    }
    match doc.get("format").and_then(Value::as_u64) {
        Some(FORMATO) => Ok(()),
        Some(n) => Err(format!(
            "o arquivo é do formato {n}; esta versão lê o formato {FORMATO}"
        )),
        None => Err("o arquivo não diz o formato".into()),
    }
}

/// Um setting do servidor do arquivo, conferido: `Some(texto)` é o que se grava;
/// `None`, o que se ignora. O arquivo pode vir de qualquer lugar.
fn setting_valido(chave: &str, valor: &str) -> Option<String> {
    match chave {
        "server_port" => valor
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1024)
            .map(|p| p.to_string()),
        // Um arquivo nunca abre o servidor para a rede: ligar o acesso é escolha
        // da pessoa, na tela, com a chave de API à vista.
        "server_lan" => (valor == "false").then(|| valor.to_string()),
        "server_models_max" | "server_parallel" => valor
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=64).contains(n))
            .map(|n| n.to_string()),
        "active_engine" => matches!(valor, "official" | "prism").then(|| valor.to_string()),
        "server_env_vars" => {
            let v: Vec<lr_types::flags::EnvVar> = serde_json::from_str(valor).ok()?;
            let bom: Vec<_> = v
                .into_iter()
                .filter(|e| {
                    !e.key.trim().is_empty()
                        && !env_perigosa(&e.key)
                        && !lr_types::flags::env_is_managed(&e.key)
                })
                .collect();
            serde_json::to_string(&bom).ok()
        }
        "server_extra_flags" => {
            let v: Vec<lr_types::flags::GlobalFlag> = serde_json::from_str(valor).ok()?;
            let bom: Vec<_> = v
                .into_iter()
                .filter(|f| {
                    !lr_types::flags::managed_keys()
                        .contains(&lr_types::flags::normalize_key(&f.key).as_str())
                })
                .collect();
            serde_json::to_string(&bom).ok()
        }
        _ => None,
    }
}

pub fn importar(store: &Store, doc: &Value, com_segredos: bool) -> Result<Resumo, String> {
    conferir(doc)?;
    let mut r = Resumo::default();

    // Primeiro se lê e se confere tudo; só depois se grava. Um perfil que o app
    // não entende derruba o import inteiro ANTES de qualquer escrita.
    let mut settings: Vec<(&str, String)> = Vec::new();
    if let Some(server) = doc.get("server").and_then(Value::as_object) {
        for k in SETTINGS_SERVIDOR {
            if let Some(v) = server.get(*k).and_then(Value::as_str) {
                match setting_valido(k, v) {
                    Some(ok) => settings.push((k, ok)),
                    None => r.ignored += 1,
                }
            }
        }
    }

    let mut perfis: Vec<(String, lr_types::tuning::ModelProfile)> = Vec::new();
    for p in doc
        .get("profiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(model), Some(profile)) =
            (p.get("model").and_then(Value::as_str), p.get("profile"))
        else {
            continue;
        };
        // Passa pelo tipo: o que o app não entende não entra no banco.
        let profile = serde_json::from_value(profile.clone())
            .map_err(|e| format!("perfil de {model}: {e}"))?;
        perfis.push((model.to_string(), profile));
    }

    // Provedores: só as escolhas conhecidas, com o tipo certo. O resto do
    // arquivo (chaves, senhas, estado de instalação) não passa por aqui.
    let mut escolhas_de_provedores: Vec<(&str, &str, Value)> = Vec::new();
    if let Some(imp) = doc.get("providers").and_then(Value::as_object) {
        if let Some(o) = imp.get("openRouter").and_then(Value::as_object) {
            match o.get("enabled") {
                Some(Value::Bool(b)) => {
                    escolhas_de_provedores.push(("openRouter", "enabled", json!(b)))
                }
                Some(_) => r.ignored += 1,
                None => {}
            }
            match o.get("favorites") {
                Some(Value::Array(a)) if a.iter().all(Value::is_string) => escolhas_de_provedores
                    .push(("openRouter", "favorites", Value::Array(a.clone()))),
                Some(_) => r.ignored += 1,
                None => {}
            }
        }
        if let Some(n) = imp.get("nineRouter").and_then(Value::as_object)
            && let Some(port) = n.get("port")
        {
            match port.as_u64().filter(|p| (1..=65535).contains(p)) {
                Some(p) => escolhas_de_provedores.push(("nineRouter", "port", json!(p))),
                None => r.ignored += 1,
            }
        }
    }

    let mut presets: Vec<(&str, &str)> = Vec::new();
    for p in doc
        .get("presets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(name), Some(j)) = (
            p.get("name").and_then(Value::as_str),
            p.get("json").and_then(Value::as_str),
        ) {
            presets.push((name, j));
        }
    }
    let mut presets_do_motor: Vec<(&str, &str)> = Vec::new();
    for p in doc
        .get("enginePresets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(name), Some(j)) = (
            p.get("name").and_then(Value::as_str),
            p.get("json").and_then(Value::as_str),
        ) {
            presets_do_motor.push((name, j));
        }
    }

    // ---- daqui em diante só se grava.
    for (k, v) in &settings {
        store.set_setting(k, v).map_err(|e| e.to_string())?;
        r.settings += 1;
    }
    for (model, profile) in &perfis {
        store
            .set_model_profile(model, profile)
            .map_err(|e| e.to_string())?;
        r.profiles += 1;
    }
    for (name, j) in &presets {
        store.save_preset(name, j).map_err(|e| e.to_string())?;
        r.presets += 1;
    }
    for (name, j) in &presets_do_motor {
        store
            .save_engine_preset(name, "model", j)
            .map_err(|e| e.to_string())?;
        r.engine_presets += 1;
    }

    if !escolhas_de_provedores.is_empty() {
        let mut atual = provedores_de(store);
        if !atual.is_object() {
            atual = json!({});
        }
        for (grupo, campo, valor) in escolhas_de_provedores {
            let alvo = atual
                .as_object_mut()
                .expect("objeto")
                .entry(grupo)
                .or_insert_with(|| json!({}));
            if let Some(alvo) = alvo.as_object_mut() {
                alvo.insert(campo.into(), valor);
            }
        }
        store
            .set_setting(PROVEDORES, &atual.to_string())
            .map_err(|e| e.to_string())?;
        r.providers = true;
    }

    if com_segredos && let Some(secrets) = doc.get("secrets").and_then(Value::as_object) {
        for k in SEGREDOS_SIMPLES {
            if let Some(v) = secrets.get(*k).and_then(Value::as_str) {
                store.set_setting(k, v).map_err(|e| e.to_string())?;
                r.secrets += 1;
            }
        }
        if let Some(k) = secrets.get("openRouterApiKey").and_then(Value::as_str) {
            let mut atual = provedores_de(store);
            if !atual.is_object() {
                atual = json!({});
            }
            let o = atual
                .as_object_mut()
                .expect("objeto")
                .entry("openRouter")
                .or_insert_with(|| json!({}));
            if let Some(o) = o.as_object_mut() {
                o.insert("apiKey".into(), Value::String(k.into()));
            }
            store
                .set_setting(PROVEDORES, &atual.to_string())
                .map_err(|e| e.to_string())?;
            r.secrets += 1;
        }
    }
    Ok(r)
}

/// O JSON pronto para copiar ou salvar.
#[tauri::command]
pub fn config_export(state: State<'_, AppState>, include_secrets: bool) -> Result<String, String> {
    let doc = exportar(&state.store, include_secrets, env!("CARGO_PKG_VERSION"))?;
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

/// Aplica o JSON colado. Tudo é lido e conferido antes de qualquer escrita: se o
/// arquivo não serve, nada é gravado.
#[tauri::command]
pub fn config_import(
    state: State<'_, AppState>,
    text: String,
    include_secrets: bool,
) -> Result<Resumo, String> {
    let doc: Value =
        serde_json::from_str(&text).map_err(|e| format!("o texto não é um JSON válido: {e}"))?;
    importar(&state.store, &doc, include_secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armado() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.set_setting("server_port", "11712").unwrap();
        s.set_setting("server_parallel", "3").unwrap();
        s.set_setting(
            "server_extra_flags",
            r#"[{"key":"threads","value":"8","place":"args","switch":false}]"#,
        )
        .unwrap();
        s.set_setting("server_api_key", "segredo-local").unwrap();
        s.set_setting("hf_token", "hf_abc").unwrap();
        s.set_setting(
            PROVEDORES,
            r#"{"openRouter":{"enabled":true,"apiKey":"sk-or-1","favorites":["a/b"]},"nineRouter":{"installed":true,"version":"0.5","port":20129,"password":"pw","jwtSecret":"j","apiKey":"nk"}}"#,
        )
        .unwrap();
        let p = lr_types::tuning::ModelProfile {
            parallel: Some(4),
            ctx: Some(16384),
            ..Default::default()
        };
        s.set_model_profile("modelo.gguf", &p).unwrap();
        s.save_preset("Criativo", r#"{"temperature":1.1}"#).unwrap();
        s.save_engine_preset("MTP turbo", "model", r#"{"parallel":2}"#)
            .unwrap();
        s
    }

    #[test]
    fn sem_segredos_o_arquivo_nao_tem_nenhum() {
        let doc = exportar(&armado(), false, "0.27.0").unwrap();
        let texto = doc.to_string();
        for s in [
            "segredo-local",
            "hf_abc",
            "sk-or-1",
            "\"pw\"",
            "\"nk\"",
            "jwtSecret",
        ] {
            assert!(!texto.contains(s), "vazou {s}: {texto}");
        }
        assert_eq!(doc["includesSecrets"], false);
        assert!(doc.get("secrets").is_none());
        // Estado da máquina também fica de fora.
        assert!(!texto.contains("installed") && !texto.contains("\"version\":\"0.5\""));
        assert_eq!(doc["providers"]["nineRouter"]["port"], 20129);
        assert_eq!(doc["providers"]["openRouter"]["favorites"][0], "a/b");
    }

    #[test]
    fn ida_e_volta_sem_perda() {
        let a = armado();
        let doc = exportar(&a, true, "0.27.0").unwrap();
        let b = Store::open_in_memory().unwrap();
        let r = importar(&b, &doc, true).unwrap();
        assert_eq!(r.profiles, 1);
        assert_eq!(r.presets, 1);
        assert_eq!(r.engine_presets, 1);
        assert_eq!(r.secrets, 3);
        assert!(r.providers);
        // O que b exporta é o que a exportou.
        assert_eq!(exportar(&b, true, "0.27.0").unwrap(), doc);
        assert_eq!(
            b.get_setting("server_parallel").unwrap().as_deref(),
            Some("3")
        );
        assert_eq!(
            b.model_profile("modelo.gguf").unwrap().unwrap().parallel,
            Some(4)
        );
    }

    #[test]
    fn importar_sem_marcar_segredos_ignora_os_do_arquivo_e_nao_apaga_os_de_agora() {
        let doc = exportar(&armado(), true, "0.27.0").unwrap();
        let b = Store::open_in_memory().unwrap();
        b.set_setting("server_api_key", "minha-chave").unwrap();
        b.set_setting(PROVEDORES, r#"{"openRouter":{"apiKey":"minha-or"},"nineRouter":{"installed":true,"password":"meu-pw"}}"#).unwrap();
        let r = importar(&b, &doc, false).unwrap();
        assert_eq!(r.secrets, 0);
        assert_eq!(
            b.get_setting("server_api_key").unwrap().as_deref(),
            Some("minha-chave")
        );
        let prov: Value =
            serde_json::from_str(&b.get_setting(PROVEDORES).unwrap().unwrap()).unwrap();
        // As escolhas chegam; as chaves e o estado da máquina de b ficam.
        assert_eq!(prov["openRouter"]["apiKey"], "minha-or");
        assert_eq!(prov["openRouter"]["enabled"], true);
        assert_eq!(prov["nineRouter"]["password"], "meu-pw");
        assert_eq!(prov["nineRouter"]["installed"], true);
        assert_eq!(prov["nineRouter"]["port"], 20129);
    }

    #[test]
    fn sem_segredos_as_variaveis_e_flags_com_chave_tambem_ficam_de_fora() {
        let s = armado();
        s.set_setting(
            "server_env_vars",
            r#"[{"key":"HF_TOKEN","value":"hf_segredo"},{"key":"OPENAI_API_KEY","value":"sk-x"},{"key":"GGML_OP_OFFLOAD_MIN_BATCH","value":"32"}]"#,
        )
        .unwrap();
        s.set_setting(
            "server_extra_flags",
            r#"[{"key":"--api-key","value":"segredo-flag","place":"args","switch":false},{"key":"--threads","value":"8","place":"args","switch":false}]"#,
        )
        .unwrap();
        let sem = exportar(&s, false, "0.27.0").unwrap().to_string();
        for x in ["hf_segredo", "OPENAI_API_KEY", "segredo-flag"] {
            assert!(!sem.contains(x), "vazou {x}");
        }
        assert!(sem.contains("GGML_OP_OFFLOAD_MIN_BATCH") && sem.contains("--threads"));
        // Com a caixa marcada, tudo vai.
        let com = exportar(&s, true, "0.27.0").unwrap().to_string();
        assert!(com.contains("hf_segredo") && com.contains("segredo-flag"));
    }

    #[test]
    fn importar_so_aceita_o_que_e_seguro() {
        let b = Store::open_in_memory().unwrap();
        b.set_setting(PROVEDORES, r#"{"openRouter":{"apiKey":"minha-or"},"nineRouter":{"password":"meu-pw","port":20128}}"#).unwrap();
        let doc = json!({
            "app": "OpenWeights", "format": 1,
            "server": {
                "server_port": "abc",
                "server_lan": "true",
                "server_models_max": "0",
                "server_parallel": "3",
                "server_env_vars": r#"[{"key":"LD_PRELOAD","value":"/tmp/x.so"},{"key":"PATH","value":"/tmp"},{"key":"GGML_X","value":"1"}]"#,
            },
            "providers": {
                "openRouter": {"enabled": true, "apiKey": "sk-do-arquivo", "favorites": ["a/b"]},
                "nineRouter": {"port": "não é número", "password": "pw-do-arquivo", "jwtSecret": "j"},
                "outro": {"x": 1}
            }
        });
        let r = importar(&b, &doc, false).unwrap();
        // Ignorados: porta inválida, lan ligada, models_max 0, a porta do 9router.
        assert_eq!(r.ignored, 4, "{r:?}");
        assert!(b.get_setting("server_port").unwrap().is_none());
        assert!(b.get_setting("server_lan").unwrap().is_none());
        assert_eq!(
            b.get_setting("server_parallel").unwrap().as_deref(),
            Some("3")
        );
        let env = b.get_setting("server_env_vars").unwrap().unwrap();
        assert!(env.contains("GGML_X") && !env.contains("LD_PRELOAD") && !env.contains("PATH"));
        let prov: Value =
            serde_json::from_str(&b.get_setting(PROVEDORES).unwrap().unwrap()).unwrap();
        // Chaves e senhas do arquivo não entram, mesmo sem `secrets`.
        assert_eq!(prov["openRouter"]["apiKey"], "minha-or");
        assert_eq!(prov["openRouter"]["enabled"], true);
        assert_eq!(prov["nineRouter"]["password"], "meu-pw");
        assert_eq!(prov["nineRouter"]["port"], 20128);
        assert!(prov.get("outro").is_none());
    }

    #[test]
    fn perfil_invalido_no_meio_nao_deixa_gravacao_pela_metade() {
        let b = Store::open_in_memory().unwrap();
        let doc = json!({
            "app": "OpenWeights", "format": 1,
            "server": {"server_parallel": "3"},
            "profiles": [{"model": "bom", "profile": {"ctx": 4096}}, {"model": "ruim", "profile": {"ctx": "x"}}],
        });
        assert!(importar(&b, &doc, false).is_err());
        assert!(b.get_setting("server_parallel").unwrap().is_none());
        assert!(b.model_profile("bom").unwrap().is_none());
    }

    #[test]
    fn arquivo_que_nao_serve_e_recusado_antes_de_tocar_em_qualquer_coisa() {
        let b = Store::open_in_memory().unwrap();
        for ruim in [
            json!({}),
            json!({"app":"Outro","format":1}),
            json!({"app":"OpenWeights","format":2}),
            json!({"app":"OpenWeights"}),
        ] {
            assert!(importar(&b, &ruim, false).is_err(), "{ruim}");
        }
        assert!(b.get_setting("server_port").unwrap().is_none());
        // Um perfil que o app não entende derruba o import inteiro no meio?
        // Não: chega a esse ponto só depois de conferido, e falha com o nome.
        let doc = json!({"app":"OpenWeights","format":1,"profiles":[{"model":"m","profile":{"ctx":"não é número"}}]});
        assert!(
            importar(&b, &doc, false)
                .unwrap_err()
                .contains("perfil de m")
        );
    }
}
