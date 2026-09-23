//! Protocolo de controle com o Host do AgenticOw (versão 1).
//!
//! Uma mensagem JSON por linha, sempre com `"ow": 1`. O stdout do Host é
//! EXCLUSIVO do protocolo; linha sem a marca é descartada (vai para o log como
//! ruído). O stdin recebe os comandos do app, e o EOF dele desliga o Host.
//! Espelho do `FORK.md` do fork (seção "Protocolo de controle").

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

/// Versão que este app fala. Outra → o Host é recusado na saudação.
pub const PROTOCOLO: u64 = 1;

/// O que o Host manda.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Mensagem {
    Hello {
        protocol: u64,
        host: String,
        revision: String,
        #[serde(rename = "upstreamTag")]
        upstream_tag: String,
        dsh: String,
        node: String,
        #[serde(rename = "nodeAbi")]
        node_abi: String,
        pid: u32,
    },
    /// A URL carrega o token de lançamento: nunca vai para log nem para evento.
    Ready {
        url: String,
        port: u16,
    },
    Fatal {
        message: String,
    },
    CatalogApplied {
        revision: Option<u64>,
    },
    CatalogError {
        revision: Option<u64>,
        message: String,
    },
}

/// Lê uma linha do stdout do Host. `None` para linha fora do protocolo ou de
/// tipo que esta versão não conhece (o Host pode ser mais novo).
pub fn ler(linha: &str) -> Option<Mensagem> {
    let valor: Value = serde_json::from_str(linha.trim()).ok()?;
    if valor.get("ow").and_then(Value::as_u64) != Some(PROTOCOLO) {
        return None;
    }
    serde_json::from_value(valor).ok()
}

/// O que o app manda.
#[derive(Clone, PartialEq)]
pub enum Comando {
    Shutdown,
    Locale(String),
    Catalog {
        revision: u64,
        /// A seção `llm-pi-ai` inteira, como o app a monta.
        pi_ai: Value,
        /// Chaves `*_API_KEY` → valor. Vão só para a memória do Host.
        env: BTreeMap<String, String>,
    },
}

impl Comando {
    /// A linha pronta para o stdin do Host (com o `\n`).
    pub fn linha(&self) -> String {
        let valor = match self {
            Comando::Shutdown => json!({ "ow": PROTOCOLO, "type": "shutdown" }),
            Comando::Locale(locale) => {
                json!({ "ow": PROTOCOLO, "type": "locale", "locale": locale })
            }
            Comando::Catalog {
                revision,
                pi_ai,
                env,
            } => json!({
                "ow": PROTOCOLO, "type": "catalog", "revision": revision, "piAi": pi_ai, "env": env,
            }),
        };
        format!("{valor}\n")
    }
}

/// O catálogo leva chaves de API: o `Debug` nunca as mostra.
impl std::fmt::Debug for Comando {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Comando::Shutdown => f.write_str("Shutdown"),
            Comando::Locale(l) => f.debug_tuple("Locale").field(l).finish(),
            Comando::Catalog { revision, env, .. } => f
                .debug_struct("Catalog")
                .field("revision", revision)
                .field("env", &env.keys().collect::<Vec<_>>())
                .finish_non_exhaustive(),
        }
    }
}

/// Tira o token de lançamento (`token=...`) de qualquer texto que vá para log
/// ou para a tela. O Host já não imprime a URL, mas uma dependência pode.
pub fn redigir(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len());
    let mut resto = texto;
    while let Some(i) = resto.find("token=") {
        saida.push_str(&resto[..i + "token=".len()]);
        let depois = &resto[i + "token=".len()..];
        let fim = depois
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(depois.len());
        if fim > 0 {
            saida.push_str("***");
        }
        resto = &depois[fim..];
    }
    saida.push_str(resto);
    saida
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_e_ready_do_host_real() {
        let hello = r#"{"ow":1,"type":"hello","protocol":1,"host":"0.1.0","revision":"9f54237c","upstreamTag":"dsh-v0.1.5-rc.3","dsh":"0.1.5-rc.3","node":"v22.20.0","nodeAbi":"127","pid":42}"#;
        assert_eq!(
            ler(hello),
            Some(Mensagem::Hello {
                protocol: 1,
                host: "0.1.0".into(),
                revision: "9f54237c".into(),
                upstream_tag: "dsh-v0.1.5-rc.3".into(),
                dsh: "0.1.5-rc.3".into(),
                node: "v22.20.0".into(),
                node_abi: "127".into(),
                pid: 42,
            })
        );
        assert_eq!(
            ler(
                r#"{"ow":1,"type":"ready","url":"http://127.0.0.1:11730/?token=abc","port":11730}"#
            ),
            Some(Mensagem::Ready {
                url: "http://127.0.0.1:11730/?token=abc".into(),
                port: 11730
            })
        );
    }

    #[test]
    fn fora_do_protocolo_nao_vira_mensagem() {
        assert_eq!(ler("dsh web: http://127.0.0.1:1/?token=x"), None);
        assert_eq!(
            ler(r#"{"type":"ready","url":"u","port":1}"#),
            None,
            "sem a marca ow"
        );
        assert_eq!(
            ler(r#"{"ow":2,"type":"ready","url":"u","port":1}"#),
            None,
            "outra versão"
        );
        assert_eq!(
            ler(r#"{"ow":1,"type":"coisa-nova"}"#),
            None,
            "tipo desconhecido"
        );
        assert_eq!(ler(""), None);
    }

    #[test]
    fn catalogo_e_erros_do_catalogo() {
        assert_eq!(
            ler(r#"{"ow":1,"type":"catalog-applied","revision":7}"#),
            Some(Mensagem::CatalogApplied { revision: Some(7) })
        );
        assert_eq!(
            ler(r#"{"ow":1,"type":"catalog-error","revision":null,"message":"schema"}"#),
            Some(Mensagem::CatalogError {
                revision: None,
                message: "schema".into()
            })
        );
    }

    #[test]
    fn comandos_viram_uma_linha_com_a_marca() {
        assert_eq!(
            Comando::Shutdown.linha(),
            "{\"ow\":1,\"type\":\"shutdown\"}\n"
        );
        let l: Value = serde_json::from_str(&Comando::Locale("pt-BR".into()).linha()).unwrap();
        assert_eq!(l, json!({ "ow": 1, "type": "locale", "locale": "pt-BR" }));
        let cat = Comando::Catalog {
            revision: 3,
            pi_ai: json!({ "providers": {} }),
            env: BTreeMap::from([("OPENWEIGHTS_API_KEY".to_string(), "sk-segredo".to_string())]),
        };
        let linha = cat.linha();
        assert!(linha.ends_with('\n') && !linha[..linha.len() - 1].contains('\n'));
        let v: Value = serde_json::from_str(&linha).unwrap();
        assert_eq!(v["env"]["OPENWEIGHTS_API_KEY"], "sk-segredo");
        assert_eq!(v["piAi"], json!({ "providers": {} }));
    }

    #[test]
    fn o_debug_do_catalogo_nao_mostra_a_chave() {
        let cat = Comando::Catalog {
            revision: 1,
            pi_ai: json!({}),
            env: BTreeMap::from([(
                "OPENROUTER_API_KEY".to_string(),
                "sk-or-v1-segredo".to_string(),
            )]),
        };
        let texto = format!("{cat:?}");
        assert!(!texto.contains("segredo"), "{texto}");
        assert!(texto.contains("OPENROUTER_API_KEY"));
    }

    #[test]
    fn redacao_do_token() {
        assert_eq!(
            redigir("abrindo http://127.0.0.1:5/?token=Ab-c_9 agora"),
            "abrindo http://127.0.0.1:5/?token=*** agora"
        );
        assert_eq!(
            redigir("a?token=x&b=1 e ?token=y"),
            "a?token=***&b=1 e ?token=***"
        );
        assert_eq!(redigir("sem nada"), "sem nada");
        assert_eq!(redigir("token="), "token=");
    }
}
