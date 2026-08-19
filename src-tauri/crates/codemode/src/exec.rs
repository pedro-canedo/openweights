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
//!
//! ## Por que o Node roda com `--permission`
//!
//! Sem isso o Code Mode seria uma porta dos fundos com outro nome: o programa
//! passa pela política a cada `await fs_write(...)`, mas nada o impediria de
//! fazer `require("node:fs").writeFileSync("/etc/qualquer-coisa")` e pular a
//! política inteira. O modo de permissões do Node 22 fecha isso — sem
//! `--allow-fs-write` o programa **não escreve arquivo nenhum**, sem
//! `--allow-child-process` não abre processo, e a única saída que resta é a
//! ponte, que é justamente onde a política mora.
//!
//! A leitura fica liberada só para a pasta temporária do próprio programa
//! (ele precisa importar o `ow.mjs`). Rede não entra no modelo de permissões
//! do Node, e é o que a ponte usa.
//!
//! Node mais antigo não conhece a flag: nesse caso o programa roda sem o
//! isolamento e o resultado **diz isso** em vez de fingir que está protegido.

use crate::plugins::Plugin;
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
    /// Peças que o projeto criou (viram funções a mais no programa).
    pub plugins: Vec<Plugin>,
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
            plugins: Vec::new(),
            node: node_program(None),
            bridge_url: String::new(),
            bridge_token: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_output_bytes: spawner::DEFAULT_MAX_OUTPUT_BYTES,
            call_id: String::new(),
        }
    }

    pub fn with_plugins(mut self, plugins: Vec<Plugin>) -> Self {
        self.plugins = plugins;
        self
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

/// O que a execução produziu.
#[derive(Debug)]
pub struct ScriptOutcome {
    pub spawn: SpawnOutcome,
    /// O programa rodou com o modo de permissões do Node (sem acesso a
    /// arquivo nem a processo por fora da ponte).
    pub isolado: bool,
}

/// Roda o script. A saída vai chegando por `on_output` (para a UI mostrar em
/// tempo real) e volta inteira no [`ScriptOutcome`].
pub async fn run_script(
    req: ScriptRequest,
    mut on_output: impl FnMut(&str) + Send,
) -> Result<ScriptOutcome, ScriptError> {
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
    // As peças do projeto são COPIADAS para a pasta da execução, em vez de
    // importadas de onde moram: assim a leitura liberada pelo modo de
    // permissões continua sendo um diretório só, o da execução.
    for plugin in &req.plugins {
        let destino = dir.join(format!("{}.mjs", plugin.nome));
        if let Err(e) = std::fs::copy(&plugin.arquivo, &destino) {
            log::warn!("não consegui copiar {}: {e}", plugin.arquivo.display());
        }
    }
    let main = dir.join("main.mjs");
    std::fs::write(&main, montar_main(&req.code, &req.plugins))?;

    let dir_real = caminho_para_node(&dir);
    let caminho_main = dir_real.join("main.mjs").to_string_lossy().into_owned();
    let monta = |args: Vec<String>| {
        SpawnRequest::new(req.node.clone(), args, req.workspace.clone())
            .with_timeout(req.timeout_secs)
            .with_max_output(req.max_output_bytes)
            // Ambiente, e não argumento: argumento aparece na lista de
            // processos da máquina, e o token é o que separa a ponte de
            // qualquer outro programa.
            .with_env("OW_BRIDGE_URL", &req.bridge_url)
            .with_env("OW_BRIDGE_TOKEN", &req.bridge_token)
    };

    let com_permissoes = vec![
        "--permission".to_string(),
        format!("--allow-fs-read={}", dir_real.to_string_lossy()),
        caminho_main.clone(),
    ];
    let primeiro = match spawner::run(monta(com_permissoes), &mut on_output).await {
        Ok(outcome) => outcome,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScriptError::SemNode(req.node));
        }
        Err(e) => return Err(ScriptError::Io(e)),
    };

    if !flag_desconhecida(&primeiro) {
        return Ok(ScriptOutcome {
            spawn: primeiro,
            isolado: true,
        });
    }

    // Node antigo: roda sem isolamento, e quem chamou fica sabendo.
    log::warn!(
        "`{}` não conhece `--permission`: o programa do Code Mode roda sem isolamento de arquivos",
        req.node
    );
    let spawn = spawner::run(monta(vec![caminho_main]), on_output)
        .await
        .map_err(ScriptError::Io)?;
    Ok(ScriptOutcome {
        spawn,
        isolado: false,
    })
}

/// O Node recusou a flag (versão anterior à 22)?
///
/// Ele sai com código 9 e escreve `bad option` — nada disso vem de um
/// programa que rodou, então não há como confundir com falha do modelo.
fn flag_desconhecida(outcome: &SpawnOutcome) -> bool {
    outcome.exit_code == Some(9)
        && (outcome.stderr.contains("bad option") || outcome.stderr.contains("--permission"))
}

/// O arquivo que o Node roda.
fn montar_main(code: &str, plugins: &[Plugin]) -> String {
    let mut out =
        String::from("import * as ow from \"./ow.mjs\";\nObject.assign(globalThis, ow);\n");
    // As peças entram como globais também, com o mesmo nome que aparece nas
    // assinaturas. Importadas ANTES do programa: um `import` no meio do
    // arquivo é içado pelo ESM, mas o `globalThis` não seria.
    for (i, plugin) in plugins.iter().enumerate() {
        out.push_str(&format!(
            "import __peca{i} from \"./{}.mjs\";\nglobalThis.{} = __peca{i};\n",
            plugin.nome, plugin.nome
        ));
    }
    out.push_str("// ——— programa do modelo ———\n");
    out.push_str(code);
    out.push('\n');
    out
}

/// O caminho da pasta da execução, na forma que o Node entende.
///
/// O Node resolve o caminho do entrypoint com `realpath` ANTES de aplicar as
/// permissões. Quando esse caminho atravessa um link simbólico, ele tenta ler
/// o alvo do link — que não está liberado — e o programa morre com
/// `ERR_ACCESS_DENIED` sem executar uma linha. No macOS isso não é caso
/// exótico: `/var` e `/tmp` são links para `/private/...`, que é onde a pasta
/// temporária mora; o Code Mode inteiro ficava indisponível na plataforma.
///
/// Canonizar resolve isso — mas no Windows o canônico volta com o prefixo
/// verbatim (`\\?\C:\...`), que o Node lê errado e derruba com
/// `lstat 'C:'`. Lá o prefixo sai; num UNC verbatim, onde tirá-lo mudaria o
/// caminho, fica o original.
fn caminho_para_node(dir: &Path) -> PathBuf {
    let Ok(real) = std::fs::canonicalize(dir) else {
        return dir.to_path_buf();
    };
    #[cfg(windows)]
    if let Some(sem_prefixo) = real.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return if sem_prefixo.starts_with("UNC\\") {
            dir.to_path_buf()
        } else {
            PathBuf::from(sem_prefixo)
        };
    }
    real
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

    /// O caminho do projeto passa por um link simbólico — o caso comum no
    /// macOS, onde `/var` e `/tmp` apontam para `/private/...` e é ali que
    /// mora a pasta temporária. Sem canonizar o caminho antes de entregá-lo
    /// ao Node, o `realpath` do carregador tenta ler o alvo do link, esbarra
    /// no modo de permissões e o programa morre com `ERR_ACCESS_DENIED` antes
    /// da primeira linha: o Code Mode inteiro ficava indisponível na
    /// plataforma.
    #[tokio::test(flavor = "multi_thread")]
    async fn o_programa_roda_quando_o_projeto_e_alcancado_por_um_link() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let real = TempDir::new().unwrap();
        let real_projeto = real.path().join("projeto");
        std::fs::create_dir(&real_projeto).unwrap();

        let casa = TempDir::new().unwrap();
        let link = casa.path().join("atalho");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_projeto, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&real_projeto, &link).is_err() {
            // Sem privilégio de criar link (padrão no Windows), não há o que
            // medir aqui — e o defeito que este teste cobre é de Unix.
            eprintln!("pulando: sem permissão para criar link simbólico");
            return;
        }

        let (ponte, rx) = Bridge::start().unwrap();
        let hospedeiro = hospedar(rx);

        let req = ScriptRequest::new(
            "const texto = await fs_read({ path: \"a.txt\" });\nsay(texto);\n",
            link.clone(),
            vec![eco()],
        )
        .with_bridge(ponte.url(), ponte.token())
        .with_timeout(30);

        let resultado = run_script(req, |_| {}).await.expect("rodou");
        let outcome = &resultado.spawn;
        assert!(
            outcome.success(),
            "o programa tinha que rodar mesmo alcançado por link: {outcome:?}"
        );
        assert!(outcome.stdout.contains("olá do harness"), "{outcome:?}");

        drop(ponte);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), hospedeiro).await;
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

        let resultado = run_script(req, |_| {}).await.expect("rodou");
        let outcome = &resultado.spawn;
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

        let outcome = run_script(req, |_| {}).await.expect("rodou").spawn;
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

        let outcome = run_script(req, |_| {}).await.expect("rodou").spawn;
        assert!(outcome.timed_out, "{outcome:?}");
        assert!(
            !projeto.path().join(SCRATCH_DIR).exists(),
            "a pasta temporária ficou para trás"
        );
    }

    /// O que separa o Code Mode de um `eval`: o programa não escreve arquivo
    /// por fora da ponte, nem abre processo. Se um dia alguém tirar a flag,
    /// este teste cai.
    #[tokio::test(flavor = "multi_thread")]
    async fn o_programa_nao_escreve_arquivo_por_fora_da_ponte() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let projeto = TempDir::new().unwrap();
        let (ponte, _rx) = Bridge::start().unwrap();
        let alvo = projeto.path().join("escapou.txt");

        let req = ScriptRequest::new(
            format!(
                "import fs from \"node:fs\";\n\
                 try {{ fs.writeFileSync({:?}, \"x\"); say(\"escreveu\"); }}\n\
                 catch (e) {{ say(\"bloqueado:\", e.code); }}\n\
                 try {{ const cp = await import(\"node:child_process\"); cp.execSync(\"echo oi\"); say(\"rodou comando\"); }}\n\
                 catch (e) {{ say(\"sem processo:\", e.code); }}\n",
                alvo.to_string_lossy()
            ),
            projeto.path().to_path_buf(),
            vec![eco()],
        )
        .with_bridge(ponte.url(), ponte.token())
        .with_timeout(30);

        let resultado = run_script(req, |_| {}).await.expect("rodou");
        if !resultado.isolado {
            eprintln!("pulando a régua: este `node` não conhece --permission");
            return;
        }
        let saida = &resultado.spawn.stdout;
        assert!(saida.contains("bloqueado: ERR_ACCESS_DENIED"), "{saida}");
        assert!(saida.contains("sem processo:"), "{saida}");
        assert!(!alvo.exists(), "o arquivo não podia ter sido criado");
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
        let main = montar_main("say(1)", &[]);
        assert!(main.starts_with("import * as ow from \"./ow.mjs\";"));
        assert!(main.contains("Object.assign(globalThis, ow);"));
        assert!(main.trim_end().ends_with("say(1)"));
    }

    /// A peça que o agente escreveu roda dentro do MESMO processo isolado, e
    /// pode usar as ferramentas como qualquer outro trecho do programa.
    #[tokio::test(flavor = "multi_thread")]
    async fn uma_peca_do_projeto_vira_funcao_do_programa() {
        if !tem_node() {
            eprintln!("pulando: `node` não está instalado");
            return;
        }
        let projeto = TempDir::new().unwrap();
        let dir_plugins = projeto.path().join(crate::plugins::PLUGINS_DIR);
        std::fs::create_dir_all(&dir_plugins).unwrap();
        std::fs::write(
            dir_plugins.join("dobro.mjs"),
            "// @tool {\"name\":\"dobro\",\"description\":\"Dobra e usa uma ferramenta.\"}\n\
             export default async function ({ n }) {\n\
               const lido = await fs_read({ path: \"x\" });\n\
               return `${n * 2} com ${lido}`;\n\
             }\n",
        )
        .unwrap();
        let plugins = crate::plugins::carregar(projeto.path());
        assert_eq!(plugins.len(), 1);

        let (ponte, rx) = Bridge::start().unwrap();
        let _hospedeiro = hospedar(rx);

        let req = ScriptRequest::new(
            "say(await plugin_dobro({ n: 21 }));",
            projeto.path().to_path_buf(),
            vec![eco()],
        )
        .with_plugins(plugins)
        .with_bridge(ponte.url(), ponte.token())
        .with_timeout(30);

        let saida = run_script(req, |_| {}).await.expect("rodou").spawn;
        assert!(saida.stdout.contains("42 com olá do harness"), "{saida:?}");
    }
}
