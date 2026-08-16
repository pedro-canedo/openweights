//! `web_download`: trazer um arquivo da internet para dentro do projeto.
//!
//! É a única ferramenta deste crate que **escreve no disco**, e por isso a que
//! mais depende do destino ser conferido: o caminho passa por
//! `ToolContext::resolve`, que recusa absoluto, `..` e symlink apontando para
//! fora. Um download com destino livre seria o caminho mais curto para
//! sobrescrever `~/.ssh/authorized_keys`.
//!
//! O arquivo é gravado **em fluxo, com teto** ([`WebConfig::max_download_bytes`]):
//! nada de carregar 2 GB na memória para só então descobrir que passou do
//! limite. A gravação vai para um `.part` ao lado e só vira o arquivo final no
//! último passo — assim uma queda no meio não deixa meio arquivo (que o
//! próximo passo do agente leria como se estivesse completo) nem destrói o
//! arquivo que já estava ali.
//!
//! O tempo aqui é de *inatividade*, não do total: 80 MB numa conexão lenta
//! passam de 30s sem nada de errado; o que não pode é a transferência parar e
//! o run ficar preso.

use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::WebConfig;
use crate::net;

/// De quanto em quanto avisamos o progresso no log.
const PROGRESS_STEP_BYTES: u64 = 4 * 1024 * 1024;

/// Fecha e apaga o arquivo temporário de um download que deu errado.
async fn discard(file: tokio::fs::File, partial: &std::path::Path) {
    let mut file = file;
    let _ = file.shutdown().await;
    drop(file);
    let _ = tokio::fs::remove_file(partial).await;
}

/// Baixa um arquivo para dentro da pasta do projeto.
pub struct WebDownload {
    config: Arc<WebConfig>,
}

impl WebDownload {
    pub fn new(config: Arc<WebConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WebDownload {
    fn name(&self) -> &str {
        "web_download"
    }

    fn description(&self) -> &str {
        "Baixa um arquivo da internet e salva DENTRO da pasta do projeto. Use para trazer um \
         .zip, .csv, .pdf ou imagem que você precisa usar no trabalho. O destino é sempre um \
         caminho relativo à pasta do projeto."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Endereço completo do arquivo, começando com https:// ou http://."
                },
                "dest_path": {
                    "type": "string",
                    "description": "Onde salvar, relativo à raiz do projeto, ex.: dados/tabela.csv. Pastas que faltarem são criadas."
                }
            },
            "required": ["url", "dest_path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Network
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        match args.get("dest_path").and_then(Value::as_str) {
            Some(rel) => ctx.resolve(rel).is_ok(),
            None => false,
        }
    }

    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        // O checkpoint precisa do arquivo antes: baixar por cima de algo que
        // já existe tem de ser reversível. Mesmo formato de caminho que as
        // ferramentas de arquivo usam (relativo, com `/`).
        match args.get("dest_path").and_then(Value::as_str) {
            Some(rel) => match ctx.resolve(rel) {
                Ok(path) => vec![ctx.relativize(&path)],
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let raw_url = arg_str(args, "url").ok()?;
        let rel = arg_str(args, "dest_path").ok()?;

        let url = match net::parse_http_url(&raw_url) {
            Ok(u) => u,
            Err(e) => {
                return Some(ToolPreview::Text {
                    body: e.to_model_message(),
                });
            }
        };
        let destino = match ctx.resolve(&rel) {
            Ok(path) => {
                let exists = path.is_file();
                format!(
                    "{rel} ({}){}",
                    path.to_string_lossy(),
                    if exists {
                        " — ATENÇÃO: o arquivo já existe e será sobrescrito"
                    } else {
                        ""
                    }
                )
            }
            Err(e) => e.to_model_message(),
        };

        Some(ToolPreview::Text {
            body: format!(
                "Baixar arquivo da internet\nDe: {}\nHost: {}\nPara: {destino}\nLimite de \
                 tamanho: {}",
                net::display_url(&url),
                net::host_of(&url),
                net::human_bytes(self.config.max_download_bytes)
            ),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let url = net::parse_http_url(&arg_str(&args, "url")?)?;
        let rel = arg_str(&args, "dest_path")?;
        let rel = rel.trim().to_string();
        if rel.is_empty() || rel.ends_with('/') || rel.ends_with('\\') {
            return Err(ToolError::InvalidArgs(
                "`dest_path` precisa ser um caminho de arquivo dentro do projeto, ex.: \
                 dados/tabela.csv"
                    .into(),
            ));
        }
        let dest = ctx.resolve(&rel)?;
        if dest.is_dir() {
            return Err(ToolError::InvalidArgs(format!(
                "`{rel}` é uma pasta — inclua o nome do arquivo no destino"
            )));
        }
        // Mesmo caminho que `files_at_risk` anunciou ao checkpoint.
        let rel = ctx.relativize(&dest);

        let limit = self.config.max_download_bytes;
        let timeout = self.config.timeout(None);
        let client = net::streaming_client(timeout, self.config.max_redirects)?;
        let mut resp = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| net::send_error(e, &url, timeout))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(ToolError::Other(format!(
                "`{}` respondeu HTTP {} — {}",
                net::host_of(&url),
                status.as_u16(),
                net::status_hint(status.as_u16())
            )));
        }

        // Quando o servidor declara o tamanho, nem começamos o que não cabe.
        if let Some(len) = resp.content_length()
            && len > limit
        {
            return Err(ToolError::InvalidArgs(format!(
                "o arquivo tem {} e o limite é {}. Baixe uma versão menor ou peça ao usuário \
                 para salvar manualmente.",
                net::human_bytes(len),
                net::human_bytes(limit)
            )));
        }

        let content_type = net::content_type(resp.headers());
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Grava num `.part` ao lado e só renomeia no fim. Se a conexão cair no
        // meio de um download que ia sobrescrever algo, o arquivo bom continua
        // lá — apagar o original para deixar meio arquivo no lugar seria a
        // pior das duas falhas.
        let partial = dest.with_file_name(format!(
            "{}.part",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut file = tokio::fs::File::create(&partial).await?;
        let mut written: u64 = 0;
        let mut next_log = PROGRESS_STEP_BYTES;

        loop {
            let chunk = match resp.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    discard(file, &partial).await;
                    if e.is_timeout() {
                        return Err(ToolError::Timeout(timeout));
                    }
                    return Err(ToolError::Other(format!(
                        "o download parou no meio depois de {} ({e}). Tente de novo.",
                        net::human_bytes(written)
                    )));
                }
            };

            written += chunk.len() as u64;
            if written > limit {
                discard(file, &partial).await;
                return Err(ToolError::InvalidArgs(format!(
                    "o download passou do limite de {} e foi cancelado (nada foi salvo). \
                     Escolha um arquivo menor.",
                    net::human_bytes(limit)
                )));
            }

            if let Err(e) = file.write_all(&chunk).await {
                discard(file, &partial).await;
                return Err(ToolError::Io(e));
            }

            if written >= next_log {
                log::debug!("web_download {rel}: {} baixados", net::human_bytes(written));
                next_log += PROGRESS_STEP_BYTES;
            }
        }

        file.flush().await?;
        file.shutdown().await?;
        drop(file);
        if let Err(e) = tokio::fs::rename(&partial, &dest).await {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(ToolError::Other(format!(
                "baixei o arquivo mas não consegui colocá-lo em `{rel}` ({e}). Confira se o \
                 caminho está livre e tente outro nome."
            )));
        }

        let tipo = if content_type.is_empty() {
            String::new()
        } else {
            format!(", tipo `{content_type}`")
        };
        Ok(ToolOutput::text(format!(
            "Baixado de {} para `{rel}` ({}{tipo}).\nO arquivo veio da internet: confira o \
             conteúdo antes de executar ou confiar nele.",
            net::display_url(&url),
            net::human_bytes(written),
        ))
        .with_changed(vec![rel]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{FakeResponse, FakeServer};
    use tempfile::TempDir;

    fn project() -> (TempDir, ToolContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("dados")).unwrap();
        let ctx = ToolContext::new(Some(dir.path().to_path_buf()), "call-dl");
        (dir, ctx)
    }

    fn tool(max_download_bytes: u64) -> WebDownload {
        WebDownload::new(Arc::new(WebConfig {
            timeout_secs: 5,
            max_download_bytes,
            ..WebConfig::default()
        }))
    }

    #[tokio::test]
    async fn saves_the_file_inside_the_project() {
        let (dir, ctx) = project();
        let server = FakeServer::spawn(|_| {
            FakeResponse::bytes("text/csv", b"nome,valor\ncaneca,10\n".to_vec())
        });

        let out = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES)
            .execute(
                json!({"url": server.url_for("/tabela.csv"), "dest_path": "dados/tabela.csv"}),
                &ctx,
            )
            .await
            .unwrap();

        let saved = std::fs::read_to_string(dir.path().join("dados/tabela.csv")).unwrap();
        assert_eq!(saved, "nome,valor\ncaneca,10\n");
        assert_eq!(out.changed_files, vec!["dados/tabela.csv".to_string()]);
        assert!(out.content.contains("21 B"), "{}", out.content);
        assert!(out.content.contains("text/csv"), "{}", out.content);
        assert!(out.content.contains("antes de executar"), "{}", out.content);
    }

    #[tokio::test]
    async fn creates_missing_folders_on_the_way() {
        let (dir, ctx) = project();
        let server = FakeServer::spawn(|_| FakeResponse::text("conteúdo"));
        tool(net::DEFAULT_MAX_DOWNLOAD_BYTES)
            .execute(
                json!({"url": server.url(), "dest_path": "novo/sub/arquivo.txt"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(dir.path().join("novo/sub/arquivo.txt").is_file());
    }

    #[tokio::test]
    async fn a_destination_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let server = FakeServer::spawn(|_| FakeResponse::text("x"));
        let tool = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES);

        for bad in ["../fora.txt", "/etc/cron.d/tarefa", "dados/../../fora.txt"] {
            let args = json!({"url": server.url(), "dest_path": bad});
            assert!(
                !tool.within_workspace(&args, &ctx),
                "{bad} devia ser sinalizado fora do projeto"
            );
            let err = tool.execute(args, &ctx).await.unwrap_err();
            assert!(
                matches!(err, ToolError::OutsideWorkspace(_)),
                "{bad}: {err:?}"
            );
            assert!(err.to_model_message().contains("fora da pasta"));
        }
        // Nada foi baixado: o destino é conferido antes da rede.
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_folder_or_empty_destination_is_refused() {
        let (_dir, ctx) = project();
        let server = FakeServer::spawn(|_| FakeResponse::text("x"));
        let tool = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES);

        let err = tool
            .execute(json!({"url": server.url(), "dest_path": "dados"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("pasta"), "{err:?}");

        let err = tool
            .execute(json!({"url": server.url(), "dest_path": "  "}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_cancelled_and_leaves_nothing_behind() {
        let (dir, ctx) = project();
        // Sem `content-length` conhecido de antemão o teto vale durante a
        // gravação; com ele, antes de começar. Os dois caminhos importam.
        let server = FakeServer::spawn(|_| FakeResponse::bytes("application/zip", vec![7u8; 4096]));

        let err = tool(1024)
            .execute(
                json!({"url": server.url_for("/grande.zip"), "dest_path": "dados/grande.zip"}),
                &ctx,
            )
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("limite"), "{msg}");
        assert!(msg.contains("1.0 KB"), "{msg}");
        assert!(
            !dir.path().join("dados/grande.zip").exists(),
            "o arquivo parcial tem de sumir"
        );
        assert!(
            !dir.path().join("dados/grande.zip.part").exists(),
            "o temporário também"
        );
    }

    #[tokio::test]
    async fn a_failed_download_keeps_the_file_that_was_already_there() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("dados/tabela.csv"), "dado bom antigo").unwrap();
        let server = FakeServer::spawn(|_| FakeResponse::bytes("text/csv", vec![7u8; 4096]));

        let err = tool(1024)
            .execute(
                json!({"url": server.url(), "dest_path": "dados/tabela.csv"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("limite"));
        // O download que falha não pode levar junto o arquivo que existia.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dados/tabela.csv")).unwrap(),
            "dado bom antigo"
        );
    }

    #[tokio::test]
    async fn a_successful_download_replaces_the_previous_file() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("dados/tabela.csv"), "versão antiga").unwrap();
        let server =
            FakeServer::spawn(|_| FakeResponse::bytes("text/csv", b"versao nova".to_vec()));

        tool(net::DEFAULT_MAX_DOWNLOAD_BYTES)
            .execute(
                json!({"url": server.url(), "dest_path": "dados/tabela.csv"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dados/tabela.csv")).unwrap(),
            "versao nova"
        );
        assert!(
            !dir.path().join("dados/tabela.csv.part").exists(),
            "o temporário não pode ficar para trás"
        );
    }

    #[tokio::test]
    async fn http_errors_do_not_create_a_file() {
        let (dir, ctx) = project();
        let server = FakeServer::spawn(|_| FakeResponse::status(404, "sumiu"));
        let err = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES)
            .execute(
                json!({"url": server.url_for("/x.zip"), "dest_path": "dados/x.zip"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.to_model_message().contains("404"));
        assert!(!dir.path().join("dados/x.zip").exists());
    }

    #[tokio::test]
    async fn preview_shows_source_destination_and_overwrite() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("dados/tabela.csv"), "antigo").unwrap();
        let tool = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES);

        match tool
            .preview(
                &json!({"url": "https://exemplo.com/t.csv", "dest_path": "dados/tabela.csv"}),
                &ctx,
            )
            .await
            .unwrap()
        {
            ToolPreview::Text { body } => {
                assert!(body.contains("De: https://exemplo.com/t.csv"), "{body}");
                assert!(body.contains("Host: exemplo.com"), "{body}");
                assert!(body.contains("dados/tabela.csv"), "{body}");
                assert!(body.contains("será sobrescrito"), "{body}");
                assert!(body.contains("100.0 MB"), "{body}");
            }
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }

        // Arquivo novo não fala em sobrescrita.
        match tool
            .preview(
                &json!({"url": "https://exemplo.com/t.csv", "dest_path": "dados/novo.csv"}),
                &ctx,
            )
            .await
            .unwrap()
        {
            ToolPreview::Text { body } => assert!(!body.contains("sobrescrito"), "{body}"),
            other => panic!("esperava prévia de texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_file_at_risk_is_declared_for_the_checkpoint() {
        let (_dir, ctx) = project();
        let tool = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES);
        let args = json!({"url": "https://exemplo.com/a.bin", "dest_path": "dados/a.bin"});
        assert_eq!(
            tool.files_at_risk(&args, &ctx),
            vec!["dados/a.bin".to_string()]
        );
        assert!(tool.within_workspace(&args, &ctx));
        // Sem destino válido não há o que proteger.
        assert!(
            tool.files_at_risk(&json!({"url": "https://x.com"}), &ctx)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn without_a_project_folder_there_is_nowhere_to_save() {
        let ctx = ToolContext::new(None, "call-dl");
        let args = json!({"url": "https://exemplo.com/a.bin", "dest_path": "a.bin"});
        let tool = tool(net::DEFAULT_MAX_DOWNLOAD_BYTES);
        assert!(!tool.within_workspace(&args, &ctx));
        let err = tool.execute(args, &ctx).await.unwrap_err();
        assert!(err.to_model_message().contains("pasta"), "{err:?}");
    }
}
