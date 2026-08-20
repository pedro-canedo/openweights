//! Juiz do `done_when`: a terceira camada da prova de conclusão.
//!
//! As duas primeiras são mecânicas (arquivos em disco, comando de aceitação
//! com código 0) e mandam sozinhas. O juiz só entra quando NÃO há evidência
//! mecânica que decida — etapa sem arquivos e sem comando, ou um comando que
//! já passava antes do trabalho — e ele NUNCA reverte uma reprovação
//! mecânica: é desempate para cima do nada, não veto sobre o disco.
//!
//! É uma chamada barata com schema forçado, no molde da decomposição:
//! resposta ilegível ou servidor fora do ar viram abstenção (`None`), porque
//! travar a etapa por causa do próprio juiz seria pior que não julgar.

use crate::run::head_chars;
use crate::scout::json_from_text;
use crate::verify::CommandRecord;
use lr_engine::{ChatMessage, ChatRequest, LlamaClient};
use lr_types::scout::Task;
use serde_json::json;

/// O que o juiz decidiu — `porque` vai para a pessoa quando reprova.
#[derive(Debug, Clone)]
pub(crate) struct Veredito {
    pub passou: bool,
    pub porque: String,
}

/// Julga se a etapa cumpriu o próprio `done_when`, com base no que ela
/// respondeu e no que ficou registrado. Abstém-se (`None`) quando o servidor
/// não respondeu ou a resposta não deu para ler.
pub(crate) async fn julgar(
    client: &LlamaClient,
    model: &str,
    task: &Task,
    resposta: &str,
    files: &[String],
    commands: &[CommandRecord],
    aviso: Option<&str>,
) -> Option<Veredito> {
    let mut user = format!(
        "Critério de pronto da etapa (done_when):\n{}\n\n\
         O que o agente respondeu ao fim da etapa:\n{}\n",
        task.done_when.trim(),
        head_chars(resposta.trim(), 1_200),
    );
    if files.is_empty() {
        user.push_str("\nArquivos escritos nesta etapa: nenhum.\n");
    } else {
        user.push_str(&format!(
            "\nArquivos escritos nesta etapa:\n{}\n",
            files.join("\n")
        ));
    }
    if !commands.is_empty() {
        let linhas: Vec<String> = commands
            .iter()
            .map(|c| {
                format!(
                    "- `{}` → {}",
                    head_chars(&c.display, 120),
                    match c.exit_code {
                        Some(code) => format!("código {code}"),
                        None if c.ok => "terminou sem erro".into(),
                        None => "falhou".into(),
                    }
                )
            })
            .collect();
        user.push_str(&format!("\nComandos executados:\n{}\n", linhas.join("\n")));
    }
    if let Some(a) = aviso {
        user.push_str(&format!("\nAtenção: {a}.\n"));
    }
    user.push_str(
        "\nA etapa cumpriu o critério? `passou` só pode ser true com evidência CONCRETA \
         acima (arquivo, comando, fato verificável na resposta). Anúncio de intenção \
         (\"vou fazer\", \"o próximo passo é\") NÃO é evidência. Responda somente o JSON.",
    );

    let mut req = ChatRequest::new(
        model,
        vec![
            ChatMessage::system(
                "Você confere entregas de um agente. Seja cético: na dúvida, reprove e \
                 diga em uma frase o que falta. Responda em JSON, no idioma do pedido.",
            ),
            ChatMessage::user(user),
        ],
    );
    req.temperature = Some(0.0);
    req.max_tokens = Some(200);
    let req = req.with_extra(
        "response_format",
        json!({
            "type": "json_schema",
            "json_schema": {
                "name": "veredito",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "passou": { "type": "boolean" },
                        "porque": { "type": "string" }
                    },
                    "required": ["passou", "porque"]
                }
            }
        }),
    );

    let out = match client.complete_once(&req).await {
        Ok(out) => out,
        Err(e) => {
            log::warn!("o juiz do done_when não respondeu ({e}); sigo sem ele");
            return None;
        }
    };
    let v = json_from_text(&out.content)?;
    Some(Veredito {
        passou: v.get("passou").and_then(|b| b.as_bool())?,
        porque: v
            .get("porque")
            .and_then(|s| s.as_str())
            .unwrap_or("sem justificativa")
            .trim()
            .to_string(),
    })
}
