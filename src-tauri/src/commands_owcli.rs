//! O OwCLI do lado do app: o gateway e o `openweights.json`.
//!
//! O OwCLI (fork do Codex CLI) pensa só com os modelos do app. Ele lê, na casa
//! dele (`OWCLI_HOME`, padrão `~/.owcli`), um `openweights.json` com o
//! endereço do gateway, o token e o catálogo; o gateway (`lr_owgw`) roteia
//! pelo prefixo do modelo e injeta a chave de cada fonte. As chaves nunca
//! saem do app. Contrato: `FORK.md` do fork, seção "O contrato com o app".
//!
//! Nada disso acontece para quem nunca usou o OwCLI: a casa e o gateway só
//! nascem depois da primeira sessão (setting `owcli.ativo`).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::catalogo::{self, Fonte};
use crate::owcli_historico::SessaoPassada;
use crate::state::AppState;

/// Setting: o OwCLI já foi usado (liga o gateway e o arquivo no boot).
pub const SETTING_ATIVO: &str = "owcli.ativo";
/// Setting: o token do gateway (persistido: quem abriu o `owcli` no terminal
/// do sistema continua valendo depois de o app reiniciar).
const SETTING_TOKEN: &str = "owcli.token";

/// O prefixo de cada fonte no id do modelo (`local:…`).
fn prefixo(fonte: &Fonte) -> &'static str {
    match fonte.id {
        catalogo::LOCAL => "local",
        catalogo::OPENROUTER => "openrouter",
        _ => "ninerouter",
    }
}

/// A casa do OwCLI, a mesma regra do lançador do fork.
pub fn casa(app: &AppHandle) -> Option<PathBuf> {
    if let Some(v) = lr_proc::host_var("OWCLI_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    app.path().home_dir().ok().map(|h| h.join(".owcli"))
}

pub fn ativo(state: &AppState) -> bool {
    state
        .store
        .get_setting(SETTING_ATIVO)
        .ok()
        .flatten()
        .is_some_and(|v| v == "1")
}

fn token(state: &AppState) -> Result<String, String> {
    if let Some(t) = state
        .store
        .get_setting(SETTING_TOKEN)
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())
    {
        return Ok(t);
    }
    let novo = lr_owgw::novo_token()?;
    state
        .store
        .set_setting(SETTING_TOKEN, &novo)
        .map_err(|e| e.to_string())?;
    Ok(novo)
}

/// As rotas do gateway a partir das fontes do catálogo.
fn rotas(fontes: &[Fonte]) -> Vec<lr_owgw::Rota> {
    fontes
        .iter()
        .map(|f| lr_owgw::Rota {
            prefixo: prefixo(f).to_string(),
            base_url: f.base_url.clone(),
            chave: f.chave.clone(),
            modelos: f.modelos.iter().map(|m| m.id.clone()).collect(),
        })
        .collect()
}

/// O `openweights.json` (contrato v1 do fork).
pub fn conexao(base_url: &str, token: &str, fontes: &[Fonte]) -> Value {
    let modelos: Vec<Value> = fontes
        .iter()
        .flat_map(|f| {
            let p = prefixo(f);
            let local = f.id == catalogo::LOCAL;
            f.modelos.iter().map(move |m| {
                let nome = if local {
                    format!("{} (local)", m.id)
                } else {
                    m.id.clone()
                };
                json!({
                    "id": format!("{p}:{}", m.id),
                    "nome": nome,
                    "janela": m.janela,
                    "esforcos": m.esforcos,
                })
            })
        })
        .collect();
    json!({
        "versao": 1,
        "baseUrl": base_url,
        "token": token,
        "modelos": modelos,
        // O primeiro modelo local, se houver: o OwCLI pensa com a máquina.
        "modeloPadrao": modelos.first().and_then(|m| m["id"].as_str()),
    })
}

/// Um modelo que a tela oferece ao abrir o OwCLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeloDoOwcli {
    /// O id com o prefixo da fonte (`local:…`), o que vai no `-m`.
    pub id: String,
    /// O id que a fonte conhece.
    pub nome: String,
    /// O nome da fonte, para agrupar.
    pub fonte: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelosDoOwcli {
    pub modelos: Vec<ModeloDoOwcli>,
    /// Sem modelo nenhum, a tela mostra o que falta em cada fonte.
    pub fontes: catalogo::EstadoDasFontes,
}

fn modelos_do_owcli(fontes: &[Fonte]) -> Vec<ModeloDoOwcli> {
    fontes
        .iter()
        .flat_map(|f| {
            f.modelos.iter().map(|m| ModeloDoOwcli {
                id: format!("{}:{}", prefixo(f), m.id),
                nome: m.id.clone(),
                fonte: f.nome,
            })
        })
        .collect()
}

/// Um id de modelo que pode ir para o `-m` sem virar opção da linha de comando.
pub fn modelo_valido(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.starts_with('-') && !id.chars().any(char::is_control)
}

/// Escreve de forma atômica e, no Unix, só para o dono (o token vale a chave
/// do OpenRouter de quem o tiver).
fn escrever_privado(caminho: &Path, conteudo: &[u8]) -> std::io::Result<()> {
    let dir = caminho.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(".openweights.{}.tmp", std::process::id()));
    std::fs::write(&temp, conteudo)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, caminho)
}

/// Garante o gateway no ar com as fontes de agora e reescreve o arquivo.
/// Não faz nada enquanto o OwCLI não foi usado.
pub async fn sincronizar(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !ativo(&state) {
        return;
    }
    if let Err(e) = sincronizar_ja(app, &state).await {
        log::warn!("OwCLI: não foi possível atualizar o gateway ou o openweights.json: {e}");
    }
}

async fn sincronizar_ja(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let token = token(state)?;
    let fontes = catalogo::fontes(state).await;
    let config = lr_owgw::Configuracao {
        token: token.clone(),
        rotas: rotas(&fontes),
    };
    let base_url = {
        let mut guard = state.owcli_gateway.lock().await;
        match guard.as_ref() {
            Some(gw) => {
                gw.atualizar(config);
                gw.base_url()
            }
            None => {
                let gw = lr_owgw::Gateway::iniciar(lr_owgw::PORTA_PREFERIDA, config)
                    .await
                    .map_err(|e| format!("gateway: {e}"))?;
                let base = gw.base_url();
                *guard = Some(gw);
                base
            }
        }
    };
    let casa = casa(app).ok_or("sem pasta pessoal para a casa do OwCLI")?;
    let arquivo = casa.join("openweights.json");
    let json = serde_json::to_vec_pretty(&conexao(&base_url, &token, &fontes))
        .map_err(|e| e.to_string())?;
    escrever_privado(&arquivo, &json).map_err(|e| format!("{}: {e}", arquivo.display()))
}

/// O executável do OwCLI.
///
/// Em desenvolvimento, `OW_OWCLI_BIN` aponta para um build do fork (o cargo o
/// gera como `codex`; o lançador vira OwCLI porque a sessão define
/// `OWCLI_HOME`). No app instalado, o runtime pinado (`lr_owcli`).
pub fn binario() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("OW_OWCLI_BIN").map(PathBuf::from)
        && p.is_file()
    {
        return Ok(p);
    }
    Err("O OwCLI ainda não está instalado neste app.".to_string())
}

/// Liga o OwCLI (primeira sessão): daqui em diante o gateway sobe com o app.
pub async fn ativar(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !ativo(&state) {
        let _ = state.store.set_setting(SETTING_ATIVO, "1");
    }
    sincronizar(app).await;
}

/// O agente pode abrir aqui? Sem o runtime do OwCLI (nem um build de
/// desenvolvimento em `OW_OWCLI_BIN`), a tela mostra só os terminais.
#[tauri::command]
pub fn owcli_disponivel() -> bool {
    binario().is_ok()
}

/// A tela liga o OwCLI ao abrir a primeira sessão dele.
#[tauri::command]
pub async fn owcli_ligar(app: AppHandle) -> Result<(), String> {
    ativar(&app).await;
    Ok(())
}

/// Os modelos com que o OwCLI pode abrir agora, e o estado de cada fonte.
#[tauri::command]
pub async fn owcli_modelos(app: AppHandle) -> Result<ModelosDoOwcli, String> {
    let state = app.state::<AppState>();
    Ok(ModelosDoOwcli {
        modelos: modelos_do_owcli(&catalogo::fontes(&state).await),
        fontes: catalogo::estado_das_fontes(&state),
    })
}

/// As conversas gravadas do OwCLI. Vazio quando ele não está instalado ou
/// nunca foi usado — não é erro para a tela.
#[tauri::command]
pub async fn owcli_historico(app: AppHandle) -> Result<Vec<SessaoPassada>, String> {
    let Ok(exe) = binario() else {
        return Ok(Vec::new());
    };
    let Some(casa) = casa(&app).filter(|c| c.join("sessions").is_dir()) else {
        return Ok(Vec::new());
    };
    crate::owcli_historico::listar(&exe, &casa).await
}

#[tauri::command]
pub async fn owcli_renomear(app: AppHandle, id: String, nome: String) -> Result<(), String> {
    if !crate::owcli_historico::id_valido(&id) || nome.trim().is_empty() {
        return Err("conversa ou nome inválido".to_string());
    }
    let exe = binario()?;
    let casa = casa(&app).ok_or("sem pasta pessoal para a casa do OwCLI")?;
    crate::owcli_historico::renomear(&exe, &casa, &id, &nome).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogo::ModeloDaFonte;

    fn fontes() -> Vec<Fonte> {
        vec![
            Fonte {
                id: catalogo::LOCAL,
                nome: "OpenWeights (local)",
                base_url: "http://127.0.0.1:11711/v1".into(),
                chave: None,
                modelos: vec![ModeloDaFonte {
                    id: "Qwen3-Coder-30B".into(),
                    janela: Some(65_536),
                    esforcos: vec![],
                    raciocinio: false,
                }],
            },
            Fonte {
                id: catalogo::OPENROUTER,
                nome: "OpenRouter",
                base_url: "https://openrouter.ai/api/v1".into(),
                chave: Some("sk-or-segredo".into()),
                modelos: vec![ModeloDaFonte {
                    id: "openai/gpt-oss-20b:free".into(),
                    janela: None,
                    esforcos: vec![],
                    raciocinio: false,
                }],
            },
        ]
    }

    #[test]
    fn o_arquivo_leva_o_token_do_gateway_e_nunca_a_chave_das_fontes() {
        let v = conexao("http://127.0.0.1:11740/owcli/v1", "tok", &fontes());
        let texto = v.to_string();
        assert!(!texto.contains("sk-or-segredo"));
        assert_eq!(v["token"], "tok");
        assert_eq!(v["versao"], 1);
        let ids: Vec<&str> = v["modelos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "local:Qwen3-Coder-30B",
                "openrouter:openai/gpt-oss-20b:free"
            ]
        );
        assert_eq!(v["modeloPadrao"], "local:Qwen3-Coder-30B");
        assert_eq!(v["modelos"][0]["janela"], 65_536);
    }

    #[test]
    fn a_tela_recebe_o_id_com_prefixo_e_o_nome_da_fonte() {
        let m = modelos_do_owcli(&fontes());
        assert_eq!(m[0].id, "local:Qwen3-Coder-30B");
        assert_eq!(m[0].nome, "Qwen3-Coder-30B");
        assert_eq!(m[0].fonte, "OpenWeights (local)");
        assert_eq!(m[1].id, "openrouter:openai/gpt-oss-20b:free");
        assert_eq!(m[1].fonte, "OpenRouter");
    }

    #[test]
    fn so_modelo_de_verdade_vai_para_o_dash_m() {
        assert!(modelo_valido("openrouter:openai/gpt-oss-20b:free"));
        assert!(!modelo_valido("--sandbox"));
        assert!(!modelo_valido(""));
        assert!(!modelo_valido("a\nb"));
    }

    #[test]
    fn as_rotas_do_gateway_levam_a_chave_de_cada_fonte() {
        let r = rotas(&fontes());
        assert_eq!(r[0].prefixo, "local");
        assert_eq!(r[0].chave, None);
        assert_eq!(r[1].prefixo, "openrouter");
        assert_eq!(r[1].chave.as_deref(), Some("sk-or-segredo"));
        assert_eq!(r[1].modelos, ["openai/gpt-oss-20b:free"]);
    }

    #[cfg(unix)]
    #[test]
    fn o_arquivo_nasce_so_para_o_dono() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("owcli-teste-{}", std::process::id()));
        let arquivo = dir.join("openweights.json");
        escrever_privado(&arquivo, b"{}").unwrap();
        let modo = std::fs::metadata(&arquivo).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }
}
