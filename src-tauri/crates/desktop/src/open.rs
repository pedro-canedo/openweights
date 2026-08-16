//! `open_path`: mostrar à pessoa o que o agente produziu.
//!
//! Um relatório em Markdown, um gráfico em PNG, a pasta com os arquivos
//! gerados, a página da documentação. Sem esta ferramenta o agente termina
//! dizendo "está em `docs/vendas.html`" e a pessoa vai procurar à mão.
//!
//! # Abrir é executar
//!
//! Esta é a razão de a ferramenta ser [`ToolCategory::Execute`] e de a maior
//! parte do arquivo ser recusa. "Abrir no aplicativo padrão" é o mesmo que a
//! pessoa dar dois cliques: abrir `instalador.exe` **roda** o instalador,
//! abrir `script.ps1` pode **rodar** o script, e `atalho.lnk` aponta para
//! qualquer coisa. O agente que pede isso raramente quer executar — ele quer
//! mostrar —, e o engano é justamente o tipo de erro do modelo que a
//! confirmação existe para pegar. Por isso extensão de programa e de script é
//! recusada com o caminho certo na mensagem: `terminal_run`, que passa pela
//! análise do comando.
//!
//! # Três portas, três validações
//!
//! 1. **Esquema**: só `http` e `https`. `file:` burlaria a pasta do projeto,
//!    `javascript:` executa, e esquema de aplicativo (`steam:`, `ms-settings:`)
//!    entrega a linha inteira a outro programa, com os argumentos dele.
//! 2. **Caminho**: sempre relativo à pasta do projeto, por `ToolContext::resolve`
//!    — que já recusa `..`, caminho absoluto e escape por link simbólico.
//! 3. **Extensão**: comparada em minúsculas e sempre a **última**, porque
//!    `nota.txt.exe` é um executável que se veste de texto.
//!
//! [`ToolCategory::Execute`]: lr_types::agent::ToolCategory::Execute

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_str};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};

use crate::DesktopHost;

/// Extensões que o sistema **roda** em vez de exibir.
///
/// A lista é de tipos cujo aplicativo padrão é um interpretador ou o próprio
/// carregador de programas. Um `.py` aberto com dois cliques executa no
/// Windows; um `.js` vai para o Windows Script Host; um `.app` é um programa
/// inteiro em pasta no macOS.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "ps1", "psm1", "vbs", "js", "jse", "wsf", "wsh",
    "sh", "py", "jar", "app", "command", "reg", "lnk", "msc", "hta",
];

/// O que a chamada pediu, depois de classificada.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// Endereço `http`/`https`.
    Url(String),
    /// Caminho relativo à pasta do projeto.
    Path(String),
}

/// O que vai acontecer, já validado — o que a prévia mostra e o que o host
/// recebe.
#[derive(Debug)]
enum Opening {
    /// Link entregue ao navegador padrão.
    Link(String),
    /// Caminho absoluto entregue ao aplicativo padrão.
    File { rel: String, abs: PathBuf },
}

/// Esquema de URL no início do texto, se houver.
///
/// Exige duas letras ou mais de propósito: `C:\Users\...` casaria com a forma
/// de um esquema de uma letra só, e recusá-lo como "esquema não aceito"
/// esconderia o problema real — caminho absoluto não entra na pasta do
/// projeto.
fn scheme_of(raw: &str) -> Option<&str> {
    let idx = raw.find(':')?;
    let scheme = &raw[..idx];
    if scheme.len() < 2 {
        return None;
    }
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some(scheme)
}

/// Decide se o alvo é link ou caminho, recusando o que não é nenhum dos dois.
fn classify(raw: &str) -> ToolResult<Target> {
    let alvo = raw.trim();
    if alvo.is_empty() {
        return Err(ToolError::InvalidArgs(
            "`target` está vazio — diga o que abrir: um caminho relativo à pasta do projeto \
             (ex.: docs/relatorio.pdf) ou um endereço completo (ex.: https://exemplo.com)."
                .into(),
        ));
    }

    let Some(scheme) = scheme_of(alvo) else {
        return Ok(Target::Path(alvo.to_string()));
    };

    let lower = scheme.to_ascii_lowercase();
    if lower != "http" && lower != "https" {
        return Err(ToolError::InvalidArgs(format!(
            "o esquema `{scheme}:` não é aceito por `open_path` — só `http://` e `https://`. \
             Esquema de aplicativo entrega a linha inteira a outro programa do sistema, e \
             `file:` sairia da pasta do projeto. Para abrir um arquivo do projeto, passe o \
             caminho relativo a ela (ex.: docs/relatorio.pdf)."
        )));
    }

    // Sem host não há o que abrir, e o navegador faria uma busca pelo texto.
    let resto = alvo.get(scheme.len() + 1..).unwrap_or_default();
    let host = resto
        .strip_prefix("//")
        .unwrap_or_default()
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        return Err(ToolError::InvalidArgs(format!(
            "`{alvo}` não tem endereço depois do esquema. Escreva a URL completa, ex.: \
             https://exemplo.com/pagina."
        )));
    }

    Ok(Target::Url(alvo.to_string()))
}

/// Última extensão do caminho, em minúsculas e sem o ponto.
fn last_extension(rel: &str) -> Option<String> {
    let nome = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let (_, ext) = nome.rsplit_once('.')?;
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

/// Recusa o que o sistema executaria em vez de exibir.
fn refuse_executable(rel: &str) -> ToolResult<()> {
    let Some(ext) = last_extension(rel) else {
        return Ok(());
    };
    if !EXECUTABLE_EXTENSIONS.contains(&ext.as_str()) {
        return Ok(());
    }
    Err(ToolError::Other(format!(
        "`{rel}` termina em `.{ext}`, e abrir isso no aplicativo padrão do sistema não mostra o \
         arquivo: RODA o arquivo. `open_path` não faz isso. Para executar um programa ou script \
         use `terminal_run`, que passa pela análise do comando antes de rodar; para ver o que o \
         arquivo contém use `fs_read`."
    )))
}

/// Ferramenta `open_path`.
pub struct OpenPath {
    host: Arc<dyn DesktopHost>,
}

impl OpenPath {
    pub fn new(host: Arc<dyn DesktopHost>) -> Self {
        Self { host }
    }
}

/// Valida o alvo por inteiro e diz o que vai acontecer.
fn plan(raw: &str, ctx: &ToolContext) -> ToolResult<Opening> {
    match classify(raw)? {
        Target::Url(url) => Ok(Opening::Link(url)),
        Target::Path(rel) => {
            let abs = ctx.resolve(&rel)?;
            refuse_executable(&rel)?;
            if !abs.exists() {
                return Err(ToolError::NotFound(rel));
            }
            Ok(Opening::File { rel, abs })
        }
    }
}

#[async_trait]
impl Tool for OpenPath {
    fn name(&self) -> &str {
        "open_path"
    }

    fn description(&self) -> &str {
        "Abre um arquivo, uma pasta ou um link no aplicativo padrão do sistema — o mesmo efeito de \
         a pessoa dar dois cliques. Use para MOSTRAR o resultado a ela: o relatório que você \
         escreveu, a imagem gerada, a pasta de saída, a página da documentação. Aceita caminho \
         relativo à pasta do projeto ou endereço http://https:// (link abre o navegador e é, na \
         prática, acesso à internet). Não abre programa nem script (.exe, .ps1, .sh, .py e \
         afins): para rodar alguma coisa use `terminal_run`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "O que abrir: caminho relativo à pasta do projeto (ex.: relatorios/vendas.pdf, ou docs para abrir a pasta) ou endereço completo começando com https:// ou http://."
                }
            },
            "required": ["target"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execute
    }

    /// Um link é rede, e a política trata rede como confirmação obrigatória
    /// em qualquer modo. Dizer `Execute` para uma URL deixaria o modo
    /// automático abrir endereço sem ninguém ver — e o que a pessoa veria
    /// depois é o navegador subindo sozinho.
    fn category_for(&self, args: &Value) -> ToolCategory {
        match arg_str(args, "target").as_deref().map(classify) {
            Ok(Ok(Target::Url(_))) => ToolCategory::Network,
            _ => ToolCategory::Execute,
        }
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        match arg_str(args, "target").as_deref().map(classify) {
            // Link não toca em arquivo nenhum; quem cuida dele é a categoria.
            Ok(Ok(Target::Url(_))) => true,
            Ok(Ok(Target::Path(rel))) => ctx.resolve(&rel).is_ok(),
            // Argumento inválido não é "fora do projeto": o erro certo, com a
            // explicação certa, aparece na execução.
            _ => true,
        }
    }

    /// Abrir não altera arquivo: não há nada para o checkpoint guardar.
    fn files_at_risk(&self, _args: &Value, _ctx: &ToolContext) -> Vec<String> {
        Vec::new()
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let raw = arg_str(args, "target").ok()?;
        let body = match plan(&raw, ctx) {
            Ok(Opening::Link(url)) => {
                format!("Abrir no navegador padrão do sistema (acesso à internet): {url}")
            }
            Ok(Opening::File { abs, .. }) => format!(
                "Abrir no aplicativo padrão do sistema: {}",
                abs.to_string_lossy()
            ),
            Err(e) => e.to_model_message(),
        };
        Some(ToolPreview::Text { body })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        let raw = arg_str(&args, "target")?;
        match plan(&raw, ctx)? {
            Opening::Link(url) => {
                self.host.open(&url).map_err(|e| open_failed(&url, e))?;
                Ok(ToolOutput::text(format!(
                    "Link aberto no navegador padrão do usuário: {url}. Não sei o que ele mostra \
                     — se precisar do conteúdo, use `web_fetch`."
                )))
            }
            Opening::File { rel, abs } => {
                let alvo = abs.to_string_lossy().into_owned();
                self.host.open(&alvo).map_err(|e| open_failed(&rel, e))?;
                Ok(ToolOutput::text(format!(
                    "`{rel}` foi aberto no aplicativo padrão do sistema; a janela apareceu na tela \
                     do usuário. Nada no projeto mudou."
                )))
            }
        }
    }
}

/// Falha do sistema ao abrir, com o que fazer em seguida.
fn open_failed(alvo: &str, detalhe: String) -> ToolError {
    ToolError::Other(format!(
        "Não consegui abrir `{alvo}`: {detalhe}. Pode não haver aplicativo associado a esse tipo \
         neste computador — diga ao usuário onde está o arquivo (ou o endereço) para ele abrir do \
         jeito dele."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Bench, FakeHost};

    fn tool(bench: &Bench) -> OpenPath {
        OpenPath::new(bench.shared())
    }

    #[tokio::test]
    async fn opening_a_file_hands_the_host_an_absolute_path() {
        let bench = Bench::new();
        let esperado = bench.write("docs/relatorio.md", "# vendas\n");

        let out = tool(&bench)
            .execute(json!({"target": "docs/relatorio.md"}), &bench.ctx)
            .await
            .expect("abrir");

        assert_eq!(
            bench.host.opened(),
            vec![esperado.to_string_lossy().into_owned()]
        );
        assert!(out.content.contains("docs/relatorio.md"), "{}", out.content);
        // Abrir não altera arquivo.
        assert!(out.changed_files.is_empty());
        assert!(
            tool(&bench)
                .files_at_risk(&json!({"target": "docs/relatorio.md"}), &bench.ctx)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_folder_of_the_project_can_be_opened() {
        let bench = Bench::new();
        bench.write("saida/a.txt", "x");
        tool(&bench)
            .execute(json!({"target": "saida"}), &bench.ctx)
            .await
            .expect("abrir a pasta");
        assert_eq!(bench.host.opened().len(), 1);
        assert!(bench.host.opened()[0].ends_with("saida"));
    }

    #[tokio::test]
    async fn paths_that_leave_the_project_are_refused() {
        let bench = Bench::new();
        for fora in ["../fora.txt", "/etc/passwd", "a/../../fora.txt"] {
            let err = tool(&bench)
                .execute(json!({ "target": fora }), &bench.ctx)
                .await
                .expect_err("deveria recusar");
            assert!(
                matches!(err, ToolError::OutsideWorkspace(_)),
                "{fora}: {err:?}"
            );
            assert!(!tool(&bench).within_workspace(&json!({ "target": fora }), &bench.ctx));
        }
        assert!(bench.host.opened().is_empty(), "nada pode ter sido aberto");
    }

    #[tokio::test]
    async fn https_links_are_accepted() {
        let bench = Bench::new();
        for link in ["https://exemplo.com/doc", "http://localhost:1234/painel"] {
            tool(&bench)
                .execute(json!({ "target": link }), &bench.ctx)
                .await
                .unwrap_or_else(|e| panic!("{link} deveria abrir: {e:?}"));
        }
        assert_eq!(
            bench.host.opened(),
            vec!["https://exemplo.com/doc", "http://localhost:1234/painel"]
        );
    }

    #[tokio::test]
    async fn other_url_schemes_are_refused_saying_what_works() {
        let bench = Bench::new();
        for ruim in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "steam://run/440",
            "ms-settings:privacy",
            "data:text/html,<b>x</b>",
        ] {
            let err = tool(&bench)
                .execute(json!({ "target": ruim }), &bench.ctx)
                .await
                .expect_err("deveria recusar");
            let msg = err.to_model_message();
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{ruim}: {err:?}");
            assert!(msg.contains("https://"), "diga o que é aceito: {msg}");
        }
        assert!(bench.host.opened().is_empty());
    }

    #[tokio::test]
    async fn a_url_without_an_address_is_refused() {
        let bench = Bench::new();
        for ruim in ["https://", "http:///caminho"] {
            let err = tool(&bench)
                .execute(json!({ "target": ruim }), &bench.ctx)
                .await
                .expect_err("deveria recusar");
            assert!(err.to_model_message().contains("URL completa"), "{err:?}");
        }
    }

    #[tokio::test]
    async fn opening_an_executable_is_refused_with_a_way_out() {
        let bench = Bench::new();
        for perigoso in [
            "instalador.exe",
            "script.ps1",
            "OUTRO.EXE",
            "nota.txt.exe",
            "bin/limpar.sh",
            "tools/build.py",
            "atalho.lnk",
        ] {
            bench.write(perigoso, "conteúdo qualquer");
            let err = tool(&bench)
                .execute(json!({ "target": perigoso }), &bench.ctx)
                .await
                .expect_err("deveria recusar");
            let msg = err.to_model_message();
            assert!(
                msg.contains("terminal_run"),
                "{perigoso}: a mensagem tem de ensinar o caminho certo: {msg}"
            );
        }
        assert!(bench.host.opened().is_empty(), "nada pode ter sido aberto");
    }

    #[tokio::test]
    async fn documents_and_images_can_be_opened() {
        let bench = Bench::new();
        for ok in ["leiame.md", "grafico.png", "manual.pdf", "notas.txt.md"] {
            bench.write(ok, "x");
            tool(&bench)
                .execute(json!({ "target": ok }), &bench.ctx)
                .await
                .unwrap_or_else(|e| panic!("{ok} deveria abrir: {e:?}"));
        }
        assert_eq!(bench.host.opened().len(), 4);
    }

    #[tokio::test]
    async fn a_file_that_does_not_exist_says_so_before_bothering_the_host() {
        let bench = Bench::new();
        let err = tool(&bench)
            .execute(json!({"target": "docs/sumiu.md"}), &bench.ctx)
            .await
            .expect_err("deveria recusar");
        assert!(matches!(err, ToolError::NotFound(_)), "{err:?}");
        assert!(bench.host.opened().is_empty());
    }

    #[tokio::test]
    async fn an_empty_target_is_refused_with_examples() {
        let bench = Bench::new();
        for args in [json!({"target": ""}), json!({"target": "   "}), json!({})] {
            let err = tool(&bench)
                .execute(args, &bench.ctx)
                .await
                .expect_err("deveria recusar");
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
            assert!(err.to_model_message().contains("target"), "{err:?}");
        }
    }

    #[tokio::test]
    async fn the_preview_says_what_will_be_opened_and_how() {
        let bench = Bench::new();
        let abs = bench.write("docs/relatorio.md", "# vendas\n");

        match tool(&bench)
            .preview(&json!({"target": "docs/relatorio.md"}), &bench.ctx)
            .await
            .expect("prévia")
        {
            ToolPreview::Text { body } => {
                assert!(body.contains("aplicativo padrão"), "{body}");
                assert!(body.contains(&*abs.to_string_lossy()), "{body}");
                assert_eq!(body.lines().count(), 1, "uma linha só: {body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }

        match tool(&bench)
            .preview(&json!({"target": "https://exemplo.com/x"}), &bench.ctx)
            .await
            .expect("prévia")
        {
            ToolPreview::Text { body } => {
                assert!(body.contains("https://exemplo.com/x"), "{body}");
                assert!(body.contains("internet"), "{body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }

        // Prévia não executa nada.
        assert!(bench.host.opened().is_empty());
    }

    #[tokio::test]
    async fn the_preview_of_a_refused_target_explains_the_refusal() {
        let bench = Bench::new();
        bench.write("instalador.exe", "x");
        match tool(&bench)
            .preview(&json!({"target": "instalador.exe"}), &bench.ctx)
            .await
            .expect("prévia")
        {
            ToolPreview::Text { body } => assert!(body.contains("terminal_run"), "{body}"),
            other => panic!("esperava texto, veio {other:?}"),
        }
    }

    /// Rede pede confirmação em qualquer modo; execução local, não.
    #[tokio::test]
    async fn a_link_counts_as_network_for_the_policy() {
        let bench = Bench::new();
        let t = tool(&bench);
        assert_eq!(
            t.category_for(&json!({"target": "https://exemplo.com"})),
            ToolCategory::Network
        );
        assert_eq!(
            t.category_for(&json!({"target": "docs/relatorio.md"})),
            ToolCategory::Execute
        );
        assert_eq!(t.category(), ToolCategory::Execute);
    }

    #[tokio::test]
    async fn a_failure_to_open_tells_the_user_where_the_file_is() {
        let bench = Bench::with_host(FakeHost::failing("nenhum aplicativo associado"));
        bench.write("manual.pdf", "x");
        let err = tool(&bench)
            .execute(json!({"target": "manual.pdf"}), &bench.ctx)
            .await
            .expect_err("host recusou");
        let msg = err.to_model_message();
        assert!(msg.contains("nenhum aplicativo associado"), "{msg}");
        assert!(msg.contains("manual.pdf"), "{msg}");
    }

    #[test]
    fn windows_drive_letters_are_paths_not_url_schemes() {
        // `C:` tem de cair na regra de caminho (e ser recusado por sair da
        // pasta), não na de esquema — a explicação certa é a do caminho.
        assert!(scheme_of(r"C:\Windows\notepad.exe").is_none());
        assert_eq!(scheme_of("https://x"), Some("https"));
        assert_eq!(scheme_of("ms-settings:x"), Some("ms-settings"));
        assert!(scheme_of("docs/relatorio.md").is_none());
    }

    #[test]
    fn the_last_extension_is_what_counts() {
        assert_eq!(last_extension("nota.txt.exe").as_deref(), Some("exe"));
        assert_eq!(last_extension("a/b/FOTO.PNG").as_deref(), Some("png"));
        assert_eq!(last_extension("pasta").as_deref(), None);
        assert_eq!(last_extension(".exe").as_deref(), Some("exe"));
        assert!(refuse_executable(".exe").is_err());
        assert!(refuse_executable("relatorio.md").is_ok());
        assert!(refuse_executable("comando.command").is_err());
    }
}
