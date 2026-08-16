//! `clipboard_read` e `clipboard_write`: a ponte mais curta entre o que a
//! pessoa está fazendo e o que o agente está fazendo.
//!
//! "Copiei o erro do console" e "me deixa isso pronto para colar" são pedidos
//! comuns, e sem estas duas ferramentas viram um pedido de digitação manual.
//!
//! As duas metades não são simétricas, e por isso as categorias diferem:
//!
//! - **Ler** o que a própria pessoa copiou é [`ToolCategory::Read`]: nada sai
//!   da máquina e nada muda. O cuidado aqui é de **tamanho** — a área de
//!   transferência pode ter um arquivo inteiro dentro, e despejar isso no
//!   contexto empurraria para fora da janela justamente o trabalho em
//!   andamento. Daí o corte em [`MAX_CLIPBOARD_CHARS`], sempre anunciado:
//!   corte silencioso faria o modelo concluir que o texto acabou ali.
//! - **Escrever** é [`ToolCategory::Edit`] porque *sobrescreve*: o que a
//!   pessoa tinha copiado se perde, e não há como desfazer. Por isso a prévia
//!   mostra o que vai ser copiado antes de a confirmação acontecer.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};

use crate::{DesktopHost, cut_chars};

/// Teto de caracteres que uma leitura da área de transferência devolve.
pub const MAX_CLIPBOARD_CHARS: usize = 8_000;

/// Quanto do texto aparece na prévia do `clipboard_write`.
///
/// A pessoa precisa reconhecer o que vai ser copiado, não reler tudo: uma
/// prévia de dez mil caracteres não é lida, e prévia não lida é confirmação
/// automática.
const PREVIEW_CHARS: usize = 400;

/// Traduz a falha do sistema em algo que o agente consiga usar.
fn host_failure(action: &str, detail: String) -> ToolError {
    ToolError::Other(format!(
        "Não consegui {action}: {detail}. A área de transferência pode estar ocupada por outro \
         programa — tente de novo em seguida ou trate o conteúdo direto na conversa."
    ))
}

// ------------------------------------------------------------------ read ---

/// Ferramenta `clipboard_read`.
pub struct ClipboardRead {
    host: Arc<dyn DesktopHost>,
}

impl ClipboardRead {
    pub fn new(host: Arc<dyn DesktopHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for ClipboardRead {
    fn name(&self) -> &str {
        "clipboard_read"
    }

    fn description(&self) -> &str {
        "Lê o texto que está na área de transferência do computador — o que a pessoa copiou com \
         Ctrl+C. Use quando ela disser \"copiei o erro\" ou \"está na área de transferência\". Só \
         traz texto: imagem ou arquivo copiado não vêm por aqui."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Read
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let texto = self
            .host
            .clipboard_read()
            .map_err(|e| host_failure("ler a área de transferência", e))?;

        // Vazio não é falha: é uma informação sobre o mundo, e o agente
        // precisa dela para pedir a coisa certa em vez de tentar de novo.
        if texto.trim().is_empty() {
            return Ok(ToolOutput::text(
                "A área de transferência está vazia (ou o que está lá não é texto — imagem e \
                 arquivo não são lidos por esta ferramenta). Peça ao usuário para copiar o \
                 conteúdo com Ctrl+C, ou para colar direto na conversa.",
            ));
        }

        let total = texto.chars().count();
        let (conteudo, cortou) = cut_chars(&texto, MAX_CLIPBOARD_CHARS);
        let conteudo = if cortou {
            format!(
                "{conteudo}\n[...cortado: a área de transferência tem {total} caracteres e só os \
                 primeiros {MAX_CLIPBOARD_CHARS} vieram. Se precisar do resto, peça ao usuário \
                 para salvar em um arquivo do projeto e leia com `fs_read`.]"
            )
        } else {
            conteudo
        };

        Ok(ToolOutput::text(conteudo).truncated_to(ctx.max_output_bytes))
    }
}

// ----------------------------------------------------------------- write ---

/// Ferramenta `clipboard_write`.
pub struct ClipboardWrite {
    host: Arc<dyn DesktopHost>,
}

impl ClipboardWrite {
    pub fn new(host: Arc<dyn DesktopHost>) -> Self {
        Self { host }
    }
}

/// Lê e valida o texto a copiar.
fn read_text(args: &Value) -> ToolResult<String> {
    let texto = arg_str(args, "text")?;
    if texto.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "`text` está vazio, e copiar vazio apagaria o que a pessoa tinha copiado sem colocar \
             nada no lugar. Mande o texto que ela deve poder colar."
                .into(),
        ));
    }
    Ok(texto)
}

#[async_trait]
impl Tool for ClipboardWrite {
    fn name(&self) -> &str {
        "clipboard_write"
    }

    fn description(&self) -> &str {
        "Coloca um texto na área de transferência do computador, pronto para a pessoa colar com \
         Ctrl+V. Use para entregar um comando, uma senha gerada ou um trecho que ela vai levar \
         para outro programa. SUBSTITUI o que estava copiado antes, e isso não tem como desfazer."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "O texto exato que a pessoa vai colar. Vai como está — sem aspas ao redor e sem formatação extra."
                }
            },
            "required": ["text"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let texto = match read_text(args) {
            Ok(t) => t,
            Err(e) => {
                return Some(ToolPreview::Text {
                    body: e.to_model_message(),
                });
            }
        };
        let total = texto.chars().count();
        let (mostra, cortou) = cut_chars(&texto, PREVIEW_CHARS);
        let cauda = if cortou {
            format!("\n[...e mais {} caractere(s)]", total - PREVIEW_CHARS)
        } else {
            String::new()
        };
        Some(ToolPreview::Text {
            body: format!(
                "Copiar para a área de transferência ({total} caractere(s)); o que estiver \
                 copiado agora se perde:\n---\n{mostra}{cauda}\n---"
            ),
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let texto = read_text(&args)?;
        self.host
            .clipboard_write(&texto)
            .map_err(|e| host_failure("escrever na área de transferência", e))?;
        Ok(ToolOutput::text(format!(
            "Copiado para a área de transferência ({} caractere(s)). O usuário já pode colar com \
             Ctrl+V.",
            texto.chars().count()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Bench, FakeHost};

    fn read_tool(bench: &Bench) -> ClipboardRead {
        ClipboardRead::new(bench.shared())
    }

    fn write_tool(bench: &Bench) -> ClipboardWrite {
        ClipboardWrite::new(bench.shared())
    }

    #[tokio::test]
    async fn reading_brings_back_what_was_copied() {
        let bench = Bench::with_host(FakeHost::with_clipboard("erro: faltou ponto e vírgula"));
        let out = read_tool(&bench)
            .execute(json!({}), &bench.ctx)
            .await
            .expect("leitura");
        assert_eq!(out.content, "erro: faltou ponto e vírgula");
    }

    #[tokio::test]
    async fn reading_an_empty_clipboard_explains_instead_of_failing() {
        for vazio in ["", "   \n\t "] {
            let bench = Bench::with_host(FakeHost::with_clipboard(vazio));
            let out = read_tool(&bench)
                .execute(json!({}), &bench.ctx)
                .await
                .expect("vazio não é erro");
            assert!(out.content.contains("vazia"), "{}", out.content);
            // A saída tem de dizer o que fazer em seguida.
            assert!(out.content.contains("Ctrl+C"), "{}", out.content);
        }
    }

    #[tokio::test]
    async fn a_huge_clipboard_comes_back_cut_and_says_so() {
        let gigante = "linha de log\n".repeat(5_000);
        let total = gigante.chars().count();
        let bench = Bench::with_host(FakeHost::with_clipboard(&gigante));

        let out = read_tool(&bench)
            .execute(json!({}), &bench.ctx)
            .await
            .expect("leitura");
        assert!(out.content.chars().count() < total, "deveria cortar");
        assert!(out.content.contains("cortado"), "{}", out.content);
        assert!(
            out.content.contains(&total.to_string()),
            "precisa dizer o tamanho real: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_clipboard_failure_says_what_to_do_next() {
        let bench = Bench::with_host(FakeHost::failing("sem acesso à área de transferência"));
        let err = read_tool(&bench)
            .execute(json!({}), &bench.ctx)
            .await
            .expect_err("host recusou");
        let msg = err.to_model_message();
        assert!(msg.contains("sem acesso"), "{msg}");
        assert!(msg.contains("conversa"), "deve dar a saída: {msg}");
    }

    #[tokio::test]
    async fn writing_puts_the_text_in_the_host() {
        let bench = Bench::new();
        let out = write_tool(&bench)
            .execute(json!({"text": "cargo test -p lr_desktop"}), &bench.ctx)
            .await
            .expect("escrita");
        assert_eq!(bench.host.clipboard(), "cargo test -p lr_desktop");
        assert!(out.content.contains("Ctrl+V"), "{}", out.content);
        // Copiar não mexe em arquivo nenhum.
        assert!(out.changed_files.is_empty());
    }

    #[tokio::test]
    async fn the_write_preview_shows_what_will_be_copied() {
        let bench = Bench::new();
        let preview = write_tool(&bench)
            .preview(&json!({"text": "ssh-keygen -t ed25519"}), &bench.ctx)
            .await
            .expect("prévia");
        match preview {
            ToolPreview::Text { body } => {
                assert!(body.contains("ssh-keygen -t ed25519"), "{body}");
                assert!(body.contains("se perde"), "avisa da sobrescrita: {body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }
        // Prévia não pode ter efeito: nada foi copiado ainda.
        assert_eq!(bench.host.clipboard(), "");
    }

    #[tokio::test]
    async fn a_long_preview_is_cut_so_it_still_gets_read() {
        let bench = Bench::new();
        let texto = "á".repeat(PREVIEW_CHARS * 3);
        let preview = write_tool(&bench)
            .preview(&json!({ "text": &texto }), &bench.ctx)
            .await
            .expect("prévia");
        match preview {
            ToolPreview::Text { body } => {
                assert!(body.chars().count() < texto.chars().count(), "{body}");
                assert!(body.contains("e mais"), "{body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn writing_an_empty_text_is_refused_with_a_way_out() {
        let bench = Bench::with_host(FakeHost::with_clipboard("o que estava lá"));
        for args in [json!({"text": ""}), json!({"text": "   \n"}), json!({})] {
            let err = write_tool(&bench)
                .execute(args, &bench.ctx)
                .await
                .expect_err("vazio é recusado");
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
            assert!(err.to_model_message().contains("text"), "{err:?}");
        }
        // E o que a pessoa tinha copiado continua lá.
        assert_eq!(bench.host.clipboard(), "o que estava lá");
    }
}
