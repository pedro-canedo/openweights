//! Escreve os arquivos do script e roda o Node com prazo.
//!
//! ## Onde o script mora
//!
//! Em `.openweights/scratch/`, dentro do projeto — a mesma pasta que o
//! `code_run` já usa, e pelo mesmo motivo: o processo roda com a raiz do
//! projeto como diretório atual, então caminho relativo dentro do script
//! aponta para o projeto, que é como o modelo escreve. A pasta da execução se
//! apaga sozinha por um guarda com `Drop`, inclusive quando o script falha,
//! estoura o tempo ou o Node nem existe.
//!
//! ## Por que dois arquivos
//!
//! `ow.mjs` é o SDK gerado e `main.mjs` é o programa do modelo. Separar os
//! dois deixa o traceback do Node apontar a linha do programa dele — que é
//! justamente o que o modelo precisa ler para se corrigir. Colar tudo num
//! arquivo só deslocaria todas as linhas pelo tamanho do prelúdio.
//!
//! O `main.mjs` derrama o SDK em `globalThis` antes do programa: assim
//! `await fs_read({path})` funciona sem `import`, que é como o modelo pequeno
//! escreve quando não é lembrado. Quem preferir `ow.fs_read` também tem.

use crate::sdk;
use lr_tools::spawner::{self, SpawnOutcome, SpawnRequest};
use lr_types::agent::ToolSpec;
use std::path::{Path, PathBuf};

/// Pasta dos arquivos temporários, dentro do projeto.
pub const SCRATCH_DIR: &str = ".openweights/scratch";

/// Prazo padrão de um script. Ele orquestra várias ferramentas: é mais que o
/// trecho solto do `code_run`, e menos que um build.
pub const DEFAULT_TIMEOUT_SECS: u64 = 180;

/// Teto do programa que aceitamos rodar.
pub const MAX_CODE_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error(
        "o Node não está disponível (`{0}`). O Code Mode precisa dele; \
         instale o Node 18+ ou desligue o Code Mode nas preferências."
    )]
    SemNode(String),
    #[error("o programa tem {0} bytes, acima do limite de {MAX_CODE_BYTES}")]
    GrandeDemais(usize),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
}

/// Tudo que uma execução precisa saber.
pub struct ScriptRequest {
    /// O programa escrito pelo modelo.
    pub code: String,
    /// Raiz do projeto: diretório atual do processo.
    pub workspace: PathBuf,
    /// Ferramentas que o script pode chamar (viram o `ow.mjs`).
    pub specs: Vec<ToolSpec>,
    /// Programa do Node (ver [`node_program`]).
    pub node: String,
    pub bridge_url: String,
    pub bridge_token: String,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    /// Sufixo do nome da pasta temporária (o `call_id`, para correlacionar
    /// com a trilha).
    pub call_id: String,
}

impl ScriptRequest {
    pub fn new(code: impl Into<String>, workspace: PathBuf, specs: Vec<ToolSpec>) -> Self {
        Self {
            code: code.into(),
            workspace,
            specs,
            node: node_program(None),
            bridge_url: String::new(),
            bridge_token: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_output_bytes: spawner::DEFAULT_MAX_OUTPUT_BYTES,
            call_id: String::new(),
        }
    }

    pub fn with_bridge(mut self, url: impl Into<String>, token: impl Into<String>) -> Self {
        self.bridge_url = url.into();
        self.bridge_token = token.into();
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Qual Node usar: o portátil que o app instalou, se houver; senão o do PATH.
///
/// O caminho do portátil chega pronto de quem chama — este crate não conhece
/// o instalador de runtimes, e não deve conhecer.
pub fn node_program(portable: Option<&Path>) -> String {
    match portable {
        Some(p) if p.exists() => p.to_string_lossy().into_owned(),
        _ => "node".to_string(),
    }
}

/// Roda o script. A saída vai chegando por `on_output` (para a UI mostrar em
/// tempo real) e volta inteira no [`SpawnOutcome`].
pub async fn run_script(
    req: ScriptRequest,
    on_output: impl FnMut(&str) + Send,
) -> Result<SpawnOutcome, ScriptError> {
    if req.code.len() > MAX_CODE_BYTES {
        return Err(ScriptError::GrandeDemais(req.code.len()));
    }

    let dir = req
        .workspace
        .join(SCRATCH_DIR)
        .join(format!("codemode_{}", sufixo(&req.call_id)));
    std::fs::create_dir_all(&dir)?;
    // A partir daqui, qualquer saída da função apaga a pasta.
    let _guarda = Scratch { dir: dir.clone() };

    std::fs::write(dir.join("ow.mjs"), sdk::render_module(&req.specs))?;
    let main = dir.join("main.mjs");
    std::fs::write(&main, montar_main(&req.code))?;

    let pedido = SpawnRequest::new(
        req.node.clone(),
        vec![main.to_string_lossy().into_owned()],
        req.workspace.clone(),
    )
    .with_timeout(req.timeout_secs)
    .with_max_output(req.max_output_bytes)
    // Ambiente, e não argumento: argumento aparece na lista de processos da
    // máquina, e o token é o que separa a ponte de qualquer outro programa.
    .with_env("OW_BRIDGE_URL", &req.bridge_url)
    .with_env("OW_BRIDGE_TOKEN", &req.bridge_token);

    match spawner::run(pedido, on_output).await {
        Ok(outcome) => Ok(outcome),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ScriptError::SemNode(req.node)),
        Err(e) => Err(ScriptError::Io(e)),
    }
}

/// O arquivo que o Node roda.
fn montar_main(code: &str) -> String {
    format!(
        "import * as ow from \"./ow.mjs\";\n\
         Object.assign(globalThis, ow);\n\
         // ——— programa do modelo ———\n{code}\n"
    )
}

/// Sufixo único e seguro para nome de pasta (o `call_id` vem de fora).
fn sufixo(call_id: &str) -> String {
    let limpo: String = call_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    if limpo.is_empty() {
        format!("anon_{nanos}")
    } else {
        format!("{limpo}_{nanos}")
    }
}

/// Pasta temporária que se apaga sozinha.
struct Scratch {
    dir: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            log::warn!("não consegui apagar {}: {e}", self.dir.display());
        }
        // `remove_dir` só apaga pasta vazia — é o que queremos: se outra
        // execução estiver rodando ao mesmo tempo, a pasta dela continua lá.
        if let Some(pai) = self.dir.parent() {
            let _ = std::fs::remove_dir(pai);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{Bridge, CallReply};
    use lr_types::agent::{ToolCategory, ToolOrigin, ToolSpec, ToolTier};
    use serde_json::json;
    use tempfile::TempDir;

    fn tem_node() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn eco() -> ToolSpec {
        ToolSpec {
            name: "fs_read".into(),
            description: "Lê um arquivo.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
            category: ToolCategory::Read,
            tier: ToolTier::Safe,
            origin: ToolOrigin::Builtin,
            read_only: true,
        }
    }

    /// Atende as chamadas da ponte com uma resposta fixa até o script acabar.
    fn hospedar(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::bridge::BridgeRequest>,
    ) -> tokio::task::JoinHandle<Vec<String>> {
        tokio::spawn(async move {
            let mut vistos = Vec::new();
            while let Some(pedido) = rx.recv().await {
                vistos.push(format!("{} {}", pedido.tool, pedido.args));
                let _ = pedido.reply.send(CallReply::ok("olá do harness"));
            }
            vistos
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_chama_a_ferramenta_pela_ponte_e_imprime_o_resultado() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let projeto = TempDir::new().unwrap();
        let (ponte, rx) = Bridge::start().unwrap();
        let hospedeiro = hospedar(rx);

        let req = ScriptRequest::new(
            "const texto = await fs_read({ path: \"a.txt\" });\nsay(texto);\n",
            projeto.path().to_path_buf(),
            vec![eco()],
        )
        .with_bridge(ponte.url(), ponte.token())
        .with_timeout(30);

        let outcome = run_script(req, |_| {}).await.expect("rodou");
        assert!(outcome.success(), "{outcome:?}");
        assert!(outcome.stdout.contains("olá do harness"), "{outcome:?}");

        drop(ponte);
        let vistos = tokio::time::timeout(std::time::Duration::from_secs(5), hospedeiro)
            .await
            .expect("o hospedeiro terminou")
            .unwrap();
        assert_eq!(vistos.len(), 1);
        assert!(vistos[0].starts_with("fs_read"), "{vistos:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn erro_de_ferramenta_vira_excecao_que_o_script_pode_tratar() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let projeto = TempDir::new().unwrap();
        let (ponte, mut rx) = Bridge::start().unwrap();
        tokio::spawn(async move {
            while let Some(pedido) = rx.recv().await {
                let _ = pedido.reply.send(CallReply::err("negado pela política"));
            }
        });

        let req = ScriptRequest::new(
            "try { await fs_read({ path: \"a.txt\" }); } catch (e) { say(\"peguei:\", e.message); }",
            projeto.path().to_path_buf(),
            vec![eco()],
        )
        .with_bridge(ponte.url(), ponte.token())
        .with_timeout(30);

        let outcome = run_script(req, |_| {}).await.expect("rodou");
        assert!(
            outcome.stdout.contains("peguei: negado pela política"),
            "{outcome:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_que_nao_termina_e_morto_no_prazo_e_a_pasta_some() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let projeto = TempDir::new().unwrap();
        let (ponte, _rx) = Bridge::start().unwrap();

        let req = ScriptRequest::new("while (true) {}", projeto.path().to_path_buf(), vec![eco()])
            .with_bridge(ponte.url(), ponte.token())
            .with_timeout(1);

        let outcome = run_script(req, |_| {}).await.expect("rodou");
        assert!(outcome.timed_out, "{outcome:?}");
        assert!(
            !projeto.path().join(SCRATCH_DIR).exists(),
            "a pasta temporária ficou para trás"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sem_node_o_erro_diz_o_que_fazer() {
        let projeto = TempDir::new().unwrap();
        let mut req = ScriptRequest::new("say(1)", projeto.path().to_path_buf(), vec![eco()]);
        req.node = "node_que_nao_existe_openweights".into();

        let erro = run_script(req, |_| {}).await.expect_err("devia falhar");
        assert!(matches!(erro, ScriptError::SemNode(_)), "{erro:?}");
        assert!(erro.to_string().contains("Code Mode"), "{erro}");
    }

    #[test]
    fn o_main_derrama_o_sdk_em_global_antes_do_programa() {
        let main = montar_main("say(1)");
        assert!(main.starts_with("import * as ow from \"./ow.mjs\";"));
        assert!(main.contains("Object.assign(globalThis, ow);"));
        assert!(main.trim_end().ends_with("say(1)"));
    }
}
