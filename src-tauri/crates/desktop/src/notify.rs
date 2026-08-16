//! `notify_user`: o agente chamando a pessoa de volta.
//!
//! Um run longo acontece com a pessoa em outra janela — ou em outra sala. Sem
//! esta ferramenta, "terminei" e "preciso de uma decisão para continuar"
//! ficam esperando em silêncio dentro do app, e o tempo economizado pelo
//! agente vira tempo perdido de espera.
//!
//! É [`ToolCategory::Meta`] porque não tem efeito nenhum no mundo além do
//! aviso: não muda arquivo, não roda programa, não manda nada para fora da
//! máquina. Pedir confirmação para mostrar um aviso seria interromper a pessoa
//! para perguntar se ela pode ser interrompida.
//!
//! Os dois limites de tamanho não são capricho: quem desenha o aviso é o
//! sistema operacional, e ele corta o excedente **sem avisar**. Cortar aqui,
//! com reticências, é a diferença entre "Terminei de refatorar o módulo de…" e
//! uma frase que some no meio.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::ToolCategory;
use serde_json::{Value, json};

use crate::{DesktopHost, cut_chars};

/// Teto de caracteres do título do aviso.
pub const MAX_NOTIFY_TITLE_CHARS: usize = 80;

/// Teto de caracteres do corpo do aviso.
pub const MAX_NOTIFY_BODY_CHARS: usize = 300;

/// Ajusta o texto ao que cabe no aviso, marcando o corte com reticências.
///
/// O resultado nunca passa de `max` caracteres, reticências incluídas.
fn fit(text: &str, max: usize) -> String {
    let text = text.trim();
    let (completo, cortou) = cut_chars(text, max);
    if !cortou {
        return completo;
    }
    let (curto, _) = cut_chars(text, max.saturating_sub(1));
    format!("{}…", curto.trim_end())
}

/// Ferramenta `notify_user`.
pub struct NotifyUser {
    host: Arc<dyn DesktopHost>,
}

impl NotifyUser {
    pub fn new(host: Arc<dyn DesktopHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for NotifyUser {
    fn name(&self) -> &str {
        "notify_user"
    }

    fn description(&self) -> &str {
        "Mostra um aviso do sistema para chamar a pessoa de volta ao computador — ela pode estar \
         em outra janela. Use quando a tarefa longa terminar ou quando você precisar de uma \
         decisão para continuar. O aviso aparece fora do app e não altera nada no projeto; a \
         resposta continua sendo dada na conversa."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Uma linha curta dizendo o que aconteceu, ex.: \"Testes passaram\" ou \"Preciso de uma decisão\". Até 80 caracteres; o que passar disso é cortado."
                },
                "body": {
                    "type": "string",
                    "description": "Uma ou duas frases com o essencial, ex.: \"18 testes, todos verdes. Falta revisar o diff.\". Até 300 caracteres; o que passar disso é cortado."
                }
            },
            "required": ["title", "body"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let title = fit(&arg_str(&args, "title")?, MAX_NOTIFY_TITLE_CHARS);
        let body = fit(&arg_str(&args, "body")?, MAX_NOTIFY_BODY_CHARS);

        // Aviso sem texto nenhum aparece como um retângulo vazio na tela: a
        // pessoa é interrompida e não descobre por quê.
        if title.is_empty() && body.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`title` e `body` estão vazios, e um aviso em branco só interrompe a pessoa sem \
                 dizer nada. Escreva pelo menos o título, ex.: title \"Terminei\", body \"O \
                 relatório está em docs/vendas.md\"."
                    .into(),
            ));
        }

        self.host.notify(&title, &body).map_err(|e| {
            ToolError::Other(format!(
                "Não consegui mostrar o aviso do sistema: {e}. Pode ser que as notificações \
                 estejam desligadas para o app — escreva o recado na conversa em vez disso."
            ))
        })?;

        Ok(ToolOutput::text(format!(
            "Aviso mostrado na tela do usuário: \"{title}\". Ele pode não estar olhando agora, \
             então continue explicando o essencial também na conversa."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Bench, FakeHost};

    fn tool(bench: &Bench) -> NotifyUser {
        NotifyUser::new(bench.shared())
    }

    #[tokio::test]
    async fn the_notification_reaches_the_host() {
        let bench = Bench::new();
        let out = tool(&bench)
            .execute(
                json!({"title": "Testes passaram", "body": "18 testes, todos verdes."}),
                &bench.ctx,
            )
            .await
            .expect("aviso");

        assert_eq!(
            bench.host.notifications(),
            vec![(
                "Testes passaram".to_string(),
                "18 testes, todos verdes.".to_string()
            )]
        );
        assert!(out.content.contains("Testes passaram"), "{}", out.content);
    }

    #[tokio::test]
    async fn title_and_body_reach_the_host_cut_to_the_limits() {
        let bench = Bench::new();
        tool(&bench)
            .execute(
                json!({
                    "title": "ç".repeat(MAX_NOTIFY_TITLE_CHARS * 2),
                    "body": "é".repeat(MAX_NOTIFY_BODY_CHARS * 2),
                }),
                &bench.ctx,
            )
            .await
            .expect("aviso");

        let (title, body) = bench.host.notifications().remove(0);
        assert_eq!(title.chars().count(), MAX_NOTIFY_TITLE_CHARS);
        assert_eq!(body.chars().count(), MAX_NOTIFY_BODY_CHARS);
        // O corte fica visível — o sistema cortaria em silêncio.
        assert!(title.ends_with('…'), "{title}");
        assert!(body.ends_with('…'), "{body}");
    }

    #[test]
    fn short_texts_pass_through_untouched() {
        assert_eq!(fit("Terminei", MAX_NOTIFY_TITLE_CHARS), "Terminei");
        assert_eq!(fit("  Terminei  ", MAX_NOTIFY_TITLE_CHARS), "Terminei");
    }

    #[tokio::test]
    async fn an_empty_notification_is_refused_with_a_way_out() {
        let bench = Bench::new();
        for args in [
            json!({"title": "", "body": ""}),
            json!({"title": "  ", "body": "\n"}),
        ] {
            let err = tool(&bench)
                .execute(args, &bench.ctx)
                .await
                .expect_err("aviso em branco é recusado");
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
            assert!(err.to_model_message().contains("title"), "{err:?}");
        }
        assert!(bench.host.notifications().is_empty());
    }

    /// Só um dos dois é o suficiente: título sem corpo ainda comunica.
    #[tokio::test]
    async fn a_title_without_a_body_is_enough() {
        let bench = Bench::new();
        tool(&bench)
            .execute(json!({"title": "Terminei", "body": ""}), &bench.ctx)
            .await
            .expect("aviso");
        assert_eq!(bench.host.notifications().len(), 1);
    }

    #[tokio::test]
    async fn a_missing_argument_says_which_one() {
        let bench = Bench::new();
        let err = tool(&bench)
            .execute(json!({"title": "Terminei"}), &bench.ctx)
            .await
            .expect_err("faltou o corpo");
        assert!(err.to_model_message().contains("body"), "{err:?}");
    }

    #[tokio::test]
    async fn a_system_failure_suggests_the_conversation() {
        let bench = Bench::with_host(FakeHost::failing("notificações desligadas"));
        let err = tool(&bench)
            .execute(
                json!({"title": "Terminei", "body": "tudo certo"}),
                &bench.ctx,
            )
            .await
            .expect_err("host recusou");
        let msg = err.to_model_message();
        assert!(msg.contains("notificações desligadas"), "{msg}");
        assert!(msg.contains("conversa"), "deve dar a saída: {msg}");
    }
}
