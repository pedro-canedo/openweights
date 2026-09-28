//! O histórico do OwCLI, pelo `app-server` dele.
//!
//! O OwCLI grava cada conversa na casa dele (`OWCLI_HOME/sessions`), venha
//! ela da tela do app ou do `owcli` do terminal do sistema. Quem lê é o
//! próprio OwCLI: `owcli app-server` fala JSON-RPC, uma mensagem por linha,
//! pelo stdio. Cada consulta sobe o processo, faz `initialize`, o pedido e
//! fecha o stdin, e o processo sai sozinho. São 0,1 s, então não compensa
//! manter um servidor vivo nem cuidar do ciclo de vida dele.
//!
//! O `app-server` não conversa com modelo: funciona com o gateway desligado.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Teto de uma consulta inteira (subir, conversar, sair).
const PRAZO: Duration = Duration::from_secs(10);
/// Quantas conversas a lista mostra.
const LIMITE: u32 = 50;

/// Uma conversa gravada do OwCLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessaoPassada {
    pub id: String,
    /// O nome dado pela pessoa, ou o começo da primeira mensagem.
    pub titulo: String,
    pub renomeada: bool,
    pub pasta: String,
    pub modelo: Option<String>,
    /// Segundos desde a época.
    pub atualizada_em: i64,
}

/// Uma conversa JSON-RPC curta com `programa app-server`: devolve o
/// `result` de cada pedido, na ordem.
pub async fn conversar(
    programa: &Path,
    args: &[&str],
    casa: &Path,
    pedidos: &[(&str, Value)],
) -> Result<Vec<Value>, String> {
    tokio::time::timeout(PRAZO, conversar_ja(programa, args, casa, pedidos))
        .await
        .map_err(|_| "o OwCLI não respondeu a tempo".to_string())?
}

async fn conversar_ja(
    programa: &Path,
    args: &[&str],
    casa: &Path,
    pedidos: &[(&str, Value)],
) -> Result<Vec<Value>, String> {
    let mut cmd = tokio::process::Command::new(programa);
    cmd.args(args)
        .env("OWCLI_HOME", casa)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    lr_proc::prepare(&mut cmd);
    let mut filho = lr_proc::spawn_supervised(&mut cmd).map_err(|e| format!("OwCLI: {e}"))?;
    let mut entrada = filho.stdin.take().ok_or("OwCLI sem stdin")?;
    let mut saida = BufReader::new(filho.stdout.take().ok_or("OwCLI sem stdout")?).lines();

    let mut linhas = vec![
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": { "clientInfo": {
                "name": "openweights",
                "title": "OpenWeights",
                "version": env!("CARGO_PKG_VERSION"),
            } },
        }),
        json!({ "jsonrpc": "2.0", "method": "initialized" }),
    ];
    for (i, (metodo, params)) in pedidos.iter().enumerate() {
        linhas.push(json!({ "jsonrpc": "2.0", "id": i + 1, "method": metodo, "params": params }));
    }
    let mut texto = String::new();
    for l in &linhas {
        texto.push_str(&l.to_string());
        texto.push('\n');
    }
    entrada
        .write_all(texto.as_bytes())
        .await
        .map_err(|e| format!("OwCLI: {e}"))?;
    entrada.flush().await.map_err(|e| format!("OwCLI: {e}"))?;

    // As respostas chegam na ordem dos pedidos, entremeadas de notificações.
    let mut resultados: Vec<Option<Value>> = vec![None; pedidos.len()];
    while resultados.iter().any(Option::is_none) {
        let Some(linha) = saida.next_line().await.map_err(|e| format!("OwCLI: {e}"))? else {
            return Err("o OwCLI fechou sem responder".to_string());
        };
        let Ok(msg) = serde_json::from_str::<Value>(&linha) else {
            continue;
        };
        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
            continue; // notificação
        };
        if let Some(erro) = msg.get("error") {
            let texto = erro["message"].as_str().unwrap_or("erro desconhecido");
            return Err(format!("OwCLI: {texto}"));
        }
        if id >= 1
            && let Some(vaga) = resultados.get_mut(id as usize - 1)
        {
            *vaga = Some(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }
    // Sem stdin, o app-server termina sozinho.
    drop(entrada);
    let _ = tokio::time::timeout(Duration::from_secs(2), filho.wait()).await;
    Ok(resultados.into_iter().flatten().collect())
}

/// O primeiro trecho da primeira mensagem, numa linha só.
fn resumo(preview: &str) -> String {
    const MAX: usize = 80;
    let linha = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    if linha.chars().count() <= MAX {
        return linha;
    }
    let mut curto: String = linha.chars().take(MAX - 1).collect();
    curto.push('…');
    curto
}

/// As conversas de uma resposta do `thread/list`.
pub fn sessoes_de(resposta: &Value) -> Vec<SessaoPassada> {
    let Some(lista) = resposta["data"].as_array() else {
        return Vec::new();
    };
    lista
        .iter()
        .filter_map(|t| {
            let id = t["id"].as_str()?.to_string();
            let nome = t["name"].as_str().map(str::trim).filter(|n| !n.is_empty());
            let titulo = match nome {
                Some(n) => n.to_string(),
                None => resumo(t["preview"].as_str().unwrap_or_default()),
            };
            Some(SessaoPassada {
                id,
                titulo,
                renomeada: nome.is_some(),
                pasta: t["cwd"].as_str().unwrap_or_default().to_string(),
                modelo: t["model"].as_str().map(str::to_string),
                atualizada_em: t["updatedAt"].as_i64().unwrap_or_default(),
            })
        })
        // Uma conversa sem mensagem nenhuma não tem o que retomar.
        .filter(|s| !s.titulo.is_empty())
        .collect()
}

/// As conversas gravadas, a mais recente primeiro.
pub async fn listar(programa: &Path, casa: &Path) -> Result<Vec<SessaoPassada>, String> {
    let r = conversar(
        programa,
        &["app-server"],
        casa,
        &[(
            "thread/list",
            json!({ "limit": LIMITE, "sortKey": "updated_at" }),
        )],
    )
    .await?;
    Ok(r.first().map(sessoes_de).unwrap_or_default())
}

pub async fn renomear(programa: &Path, casa: &Path, id: &str, nome: &str) -> Result<(), String> {
    conversar(
        programa,
        &["app-server"],
        casa,
        &[(
            "thread/name/set",
            json!({ "threadId": id, "name": nome.trim() }),
        )],
    )
    .await
    .map(|_| ())
}

/// Um id de conversa que pode ir para a linha de comando sem virar opção.
pub fn id_valido(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lista_usa_o_nome_ou_o_comeco_da_primeira_mensagem() {
        let r = json!({ "data": [
            { "id": "a1", "name": "Refatorar o parser", "preview": "oi", "cwd": "/p/api",
              "model": "local:qwen", "updatedAt": 20 },
            { "id": "b2", "name": null, "preview": "  Conserte\no teste   que falha ",
              "cwd": "/p/web", "updatedAt": 10 },
            { "id": "c3", "name": "  ", "preview": "", "cwd": "/p", "updatedAt": 5 },
        ] });
        let s = sessoes_de(&r);
        assert_eq!(s.len(), 2, "sem mensagem não entra");
        assert_eq!(s[0].titulo, "Refatorar o parser");
        assert!(s[0].renomeada);
        assert_eq!(s[0].modelo.as_deref(), Some("local:qwen"));
        assert_eq!(s[1].titulo, "Conserte o teste que falha");
        assert!(!s[1].renomeada);
        assert_eq!(s[1].pasta, "/p/web");
        assert_eq!(s[1].atualizada_em, 10);
    }

    #[test]
    fn a_primeira_mensagem_longa_e_cortada() {
        let longa = "palavra ".repeat(30);
        let t = resumo(&longa);
        assert_eq!(t.chars().count(), 80);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn so_id_de_conversa_vai_para_a_linha_de_comando() {
        assert!(id_valido("01a0e747-cde3-79e1-bd44-168d74d9976c"));
        assert!(!id_valido("--sandbox"));
        assert!(!id_valido("a b"));
        assert!(!id_valido(""));
        assert!(!id_valido("../x"));
    }

    /// Um app-server de mentira: responde ao `initialize`, solta uma
    /// notificação e responde ao pedido; sai quando o stdin fecha.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_conversa_ignora_notificacoes_e_devolve_o_resultado() {
        let dir = std::env::temp_dir().join(format!("owcli-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"read a; echo '{"id":0,"result":{}}'; read b; read c
echo '{"method":"remoteControl/status/changed","params":{}}'
echo '{"id":1,"result":{"data":[{"id":"x1","preview":"oi","cwd":"/p","updatedAt":3}]}}'
read d"#;
        let r = conversar(
            Path::new("sh"),
            &["-c", script],
            &dir,
            &[("thread/list", json!({}))],
        )
        .await
        .unwrap();
        let s = sessoes_de(&r[0]);
        assert_eq!(s[0].id, "x1");
        assert_eq!(s[0].titulo, "oi");

        let erro = conversar(
            Path::new("sh"),
            &["-c", r#"read a; echo '{"id":0,"result":{}}'; read b; read c; echo '{"id":1,"error":{"message":"thread not found"}}'"#],
            &dir,
            &[("thread/name/set", json!({}))],
        )
        .await
        .unwrap_err();
        assert!(erro.contains("thread not found"), "{erro}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
