//! Gera o que o modelo vê: o módulo JavaScript e as assinaturas do prompt.
//!
//! ## Duas saídas, e não uma
//!
//! O módulo (`ow.mjs`) é para a máquina: uma função por ferramenta, com o
//! nome real preservado na chamada. As assinaturas são para o modelo, e
//! custam uma linha por ferramenta — contra as dezenas de linhas do JSON
//! Schema que o modo nativo manda no campo `tools` a cada passo. É a metade
//! barata do Code Mode: mesmo antes de o script rodar, o prompt já encolheu.
//!
//! ## Nome de ferramenta não é identificador
//!
//! As nativas são `fs_read`, `terminal_run` — identificadores válidos. As de
//! conectores MCP chegam como `servidor__criar-issue`, e `-` não é nome de
//! função em JavaScript. Por isso o identificador exposto é uma versão
//! saneada, e a string enviada à ponte continua sendo o nome real: quem
//! precisa casar com o registro é a ponte, não o script.

use lr_types::agent::ToolSpec;
use serde_json::Value;
use std::collections::BTreeSet;

/// Quanto da descrição entra na assinatura.
///
/// Uma frase. O modelo já tem o nome e os campos; parágrafo aqui é o que
/// enche a janela de um modelo de 8k sem ensinar nada novo.
const DESC_CHARS: usize = 160;

/// Palavras que não podem virar nome de função.
const RESERVADAS: [&str; 22] = [
    "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do",
    "else", "export", "for", "function", "if", "import", "in", "new", "return", "switch", "try",
    "while",
];

/// Identificador JavaScript válido para um nome de ferramenta.
pub fn safe_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        let ok = c.is_ascii_alphanumeric() || c == '_' || c == '$';
        let comeca_com_digito = i == 0 && c.is_ascii_digit();
        if ok && !comeca_com_digito {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("ferramenta");
    }
    if RESERVADAS.contains(&out.as_str()) {
        out.insert(0, '_');
    }
    out
}

/// Campos do schema: `(nome, obrigatório)`, obrigatórios primeiro.
///
/// A ordem não é a do JSON Schema: o mapa do `serde_json` é ordenado por
/// nome, e a do arquivo se perde na desserialização. Em vez de fingir que
/// preservamos, escolhemos a ordem que serve ao modelo — o que ele **precisa**
/// preencher aparece antes do que é opcional.
fn campos(spec: &ToolSpec) -> Vec<(String, bool)> {
    let obrigatorios: BTreeSet<&str> = spec
        .parameters
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    spec.parameters
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            let mut campos: Vec<(String, bool)> = props
                .keys()
                .map(|k| (k.clone(), obrigatorios.contains(k.as_str())))
                .collect();
            campos.sort_by_key(|(nome, req)| (!*req, nome.clone()));
            campos
        })
        .unwrap_or_default()
}

/// Primeira frase da descrição, sem quebras de linha.
fn resumo(desc: &str) -> String {
    let limpa = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    let fim = limpa
        .find(". ")
        .map(|i| i + 1)
        .unwrap_or_else(|| limpa.len());
    let frase = &limpa[..fim.min(limpa.len())];
    if frase.chars().count() <= DESC_CHARS {
        return frase.to_string();
    }
    let cortada: String = frase.chars().take(DESC_CHARS).collect();
    format!("{}…", cortada.trim_end())
}

/// Uma linha por ferramenta, para o prompt de sistema.
pub fn render_signatures(specs: &[ToolSpec]) -> String {
    let mut out = String::with_capacity(specs.len() * 96);
    for spec in specs {
        let campos = campos(spec);
        let args = if campos.is_empty() {
            String::new()
        } else {
            let lista: Vec<String> = campos
                .iter()
                .map(|(nome, req)| {
                    if *req {
                        nome.clone()
                    } else {
                        format!("{nome}?")
                    }
                })
                .collect();
            format!("{{ {} }}", lista.join(", "))
        };
        out.push_str(&format!(
            "await {}({args}) — {}\n",
            safe_ident(&spec.name),
            resumo(&spec.description)
        ));
    }
    out
}

/// O módulo que o script importa.
pub fn render_module(specs: &[ToolSpec]) -> String {
    let mut out = String::with_capacity(1024 + specs.len() * 160);
    out.push_str(PRELUDIO);

    let mut vistos: BTreeSet<String> = BTreeSet::new();
    for spec in specs {
        let mut ident = safe_ident(&spec.name);
        // Dois nomes diferentes podem sanear para o mesmo identificador
        // (`a-b` e `a_b`). O segundo ganha sufixo em vez de sobrescrever o
        // primeiro — sobrescrever faria uma ferramenta chamar a outra.
        if !vistos.insert(ident.clone()) {
            for n in 2..u32::MAX {
                let tentativa = format!("{ident}_{n}");
                if vistos.insert(tentativa.clone()) {
                    ident = tentativa;
                    break;
                }
            }
        }
        let nome = spec.name.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "\n/** {} */\nexport async function {ident}(args) {{\n  return __chamar(\"{nome}\", args);\n}}\n",
            resumo(&spec.description).replace("*/", "*\\/")
        ));
    }
    out
}

/// A parte fixa do módulo.
///
/// `ToolError` existe para o script poder se recuperar sozinho: uma leitura
/// que falha vira `catch` e o programa segue com os outros arquivos, em vez
/// de morrer e devolver nada ao modelo — que é o pior desfecho possível,
/// porque gasta o passo inteiro sem produzir informação.
const PRELUDIO: &str = r#"// Gerado pelo OpenWeights a cada execução. Não edite: some no fim.
const __URL = process.env.OW_BRIDGE_URL;
const __TOKEN = process.env.OW_BRIDGE_TOKEN;

export class ToolError extends Error {
  constructor(message) {
    super(message);
    this.name = "ToolError";
  }
}

async function __chamar(tool, args) {
  const resposta = await fetch(`${__URL}/call`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${__TOKEN}`,
    },
    body: JSON.stringify({ tool, args: args ?? {} }),
  });
  const dado = await resposta.json();
  if (!dado.ok) throw new ToolError(dado.content);
  return dado.content;
}

/** Imprime o resultado. Só o que for impresso volta para o modelo. */
export function say(...partes) {
  console.log(partes.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join(" "));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::agent::{ToolCategory, ToolOrigin, ToolTier};
    use serde_json::json;

    fn spec(name: &str, desc: &str, params: Value) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: desc.into(),
            parameters: params,
            category: ToolCategory::Read,
            tier: ToolTier::Safe,
            origin: ToolOrigin::Builtin,
            read_only: true,
        }
    }

    fn fs_read() -> ToolSpec {
        spec(
            "fs_read",
            "Lê um arquivo do projeto. Use caminhos relativos à pasta de trabalho.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "offset": {"type": "integer"}
                },
                "required": ["path"]
            }),
        )
    }

    #[test]
    fn assinatura_poe_o_obrigatorio_na_frente_e_corta_na_primeira_frase() {
        let texto = render_signatures(&[fs_read()]);
        assert_eq!(
            texto.trim(),
            "await fs_read({ path, offset? }) — Lê um arquivo do projeto."
        );
    }

    #[test]
    fn modulo_expoe_uma_funcao_por_ferramenta_com_o_nome_real_na_chamada() {
        let modulo = render_module(&[fs_read()]);
        assert!(modulo.contains("export async function fs_read(args)"));
        assert!(modulo.contains("__chamar(\"fs_read\", args)"));
        assert!(modulo.contains("export class ToolError"));
    }

    #[test]
    fn nome_de_conector_mcp_vira_identificador_valido_sem_perder_o_original() {
        let t = spec("srv__criar-issue", "Cria uma issue.", json!({}));
        let modulo = render_module(std::slice::from_ref(&t));
        assert!(modulo.contains("export async function srv__criar_issue(args)"));
        // O que vai para a ponte continua sendo o nome que o registro conhece.
        assert!(modulo.contains("__chamar(\"srv__criar-issue\", args)"));
        assert_eq!(
            render_signatures(&[t]).trim(),
            "await srv__criar_issue() — Cria uma issue."
        );
    }

    #[test]
    fn dois_nomes_que_saneiam_igual_nao_se_sobrescrevem() {
        let modulo = render_module(&[
            spec("a-b", "Primeira.", json!({})),
            spec("a_b", "Segunda.", json!({})),
        ]);
        assert!(modulo.contains("export async function a_b(args)"));
        assert!(modulo.contains("export async function a_b_2(args)"));
        assert!(modulo.contains("__chamar(\"a-b\", args)"));
        assert!(modulo.contains("__chamar(\"a_b\", args)"));
    }

    #[test]
    fn palavra_reservada_ganha_prefixo() {
        assert_eq!(safe_ident("delete"), "_delete");
        assert_eq!(safe_ident("2fast"), "_fast");
        assert_eq!(safe_ident(""), "ferramenta");
    }
}
