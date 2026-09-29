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

/// O que um import fez, para a tela contar.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resumo {
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

pub fn importar(store: &Store, doc: &Value, com_segredos: bool) -> Result<Resumo, String> {
    conferir(doc)?;
    let mut r = Resumo::default();

    if let Some(server) = doc.get("server").and_then(Value::as_object) {
        for k in SETTINGS_SERVIDOR {
            if let Some(v) = server.get(*k).and_then(Value::as_str) {
                store.set_setting(k, v).map_err(|e| e.to_string())?;
                r.settings += 1;
            }
        }
    }

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
        let profile: lr_types::tuning::ModelProfile = serde_json::from_value(profile.clone())
            .map_err(|e| format!("perfil de {model}: {e}"))?;
        store
            .set_model_profile(model, &profile)
            .map_err(|e| e.to_string())?;
        r.profiles += 1;
    }

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
            store.save_preset(name, j).map_err(|e| e.to_string())?;
            r.presets += 1;
        }
    }
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
            store
                .save_engine_preset(name, "model", j)
                .map_err(|e| e.to_string())?;
            r.engine_presets += 1;
        }
    }

    // Provedores: as escolhas do arquivo sobre a configuração de agora, que
    // guarda o que é da máquina (instalação, chaves).
    if let Some(imp) = doc.get("providers").and_then(Value::as_object)
        && !imp.is_empty()
    {
        let mut atual = provedores_de(store);
        if !atual.is_object() {
            atual = json!({});
        }
        for (grupo, campos) in imp {
            if let Some(campos) = campos.as_object() {
                let alvo = atual
                    .as_object_mut()
                    .expect("objeto")
                    .entry(grupo.clone())
                    .or_insert_with(|| json!({}));
                if let Some(alvo) = alvo.as_object_mut() {
                    for (k, v) in campos {
                        alvo.insert(k.clone(), v.clone());
                    }
                }
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

/// Aplica o JSON colado. Nada é gravado se o arquivo não passar na conferência.
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
        s.set_setting("server_extra_flags", r#"[{"key":"x"}]"#)
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
