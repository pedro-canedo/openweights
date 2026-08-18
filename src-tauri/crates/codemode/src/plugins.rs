//! Peças que o agente escreve na hora.
//!
//! O DeepSeek Harness chama isso de "modo de criação": você descreve o que
//! falta e a ferramenta nasce dentro da mesma conversa. Aqui a versão honesta
//! em Rust — o laço é compilado e não vai virar plugin — é esta: uma peça
//! nova é um **arquivo JavaScript** em `.openweights/plugins/`, e ela vira
//! mais uma função dentro do programa.
//!
//! ## Por que não é uma ferramenta de verdade do registro
//!
//! Porque não precisa ser, e porque ser custaria as duas coisas que mais
//! importam. O agente já sabe escrever arquivo — com política, confirmação e
//! foto do projeto —, então criar a peça não exige mecanismo novo nenhum. E
//! como ela é carregada DENTRO do processo isolado do programa, ela herda o
//! mesmo cerco: sem acesso a arquivo, sem processo, só a ponte. Uma peça que
//! virasse ferramenta nativa rodaria fora desse cerco.
//!
//! ## O cabeçalho
//!
//! ```js
//! // @tool {"name":"resumo_de_log","description":"Conta níveis num .log","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}
//! export default async function ({ path }) {
//!   const texto = await fs_read({ path });
//!   return texto.split("\n").filter((l) => l.includes("ERROR")).length;
//! }
//! ```
//!
//! O cabeçalho é lido sem executar nada: é uma linha de comentário com JSON.
//! Executar o arquivo para descobrir o que ele é seria rodar código do modelo
//! antes de qualquer decisão sobre ele.

use lr_types::agent::{ToolCategory, ToolOrigin, ToolSpec, ToolTier};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Onde as peças moram, dentro do projeto.
///
/// Dentro do projeto de propósito: elas aparecem no `git status` da pessoa,
/// entram na foto do checkpoint e somem junto com o projeto. Uma pasta
/// escondida no sistema faria código escrito por um modelo sobreviver
/// invisível a todas as conversas seguintes.
pub const PLUGINS_DIR: &str = ".openweights/plugins";

/// O prefixo que separa uma peça criada na hora de uma ferramenta nativa.
pub const PREFIXO: &str = "plugin_";

/// Teto de peças carregadas: o cardápio do modelo não pode virar um depósito.
const MAX_PLUGINS: usize = 24;

#[derive(Debug, Clone)]
pub struct Plugin {
    /// Nome exposto ao programa, já com o prefixo (`plugin_resumo_de_log`).
    pub nome: String,
    pub descricao: String,
    pub parametros: Value,
    /// Arquivo de origem (copiado para a pasta do programa na execução).
    pub arquivo: PathBuf,
}

impl Plugin {
    /// A peça vista como ferramenta — é assim que ela entra nas assinaturas.
    pub fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.nome.clone(),
            description: self.descricao.clone(),
            parameters: self.parametros.clone(),
            category: ToolCategory::Meta,
            tier: ToolTier::Safe,
            origin: ToolOrigin::Builtin,
            read_only: false,
        }
    }
}

/// Lê as peças do projeto. Arquivo sem cabeçalho é ignorado em silêncio: a
/// pasta é do usuário, e um rascunho pela metade não pode derrubar um run.
pub fn carregar(workspace: &Path) -> Vec<Plugin> {
    let dir = workspace.join(PLUGINS_DIR);
    let Ok(entradas) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut arquivos: Vec<PathBuf> = entradas
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mjs" || e == "js"))
        .collect();
    // Ordem estável: dois runs com os mesmos arquivos veem o mesmo cardápio.
    arquivos.sort();

    let mut out = Vec::new();
    for arquivo in arquivos.into_iter().take(MAX_PLUGINS) {
        let Ok(texto) = std::fs::read_to_string(&arquivo) else {
            continue;
        };
        match ler_cabecalho(&texto) {
            Some(plugin) => out.push(Plugin { arquivo, ..plugin }),
            None => log::warn!(
                "{} não tem o cabeçalho `// @tool {{...}}` e foi ignorado",
                arquivo.display()
            ),
        }
    }
    out
}

/// O `// @tool {...}` do topo do arquivo.
fn ler_cabecalho(texto: &str) -> Option<Plugin> {
    let linha = texto
        .lines()
        .take(20)
        .map(str::trim)
        .find(|l| l.starts_with("// @tool"))?;
    let json = linha.trim_start_matches("// @tool").trim();
    let v: Value = serde_json::from_str(json).ok()?;

    let nome = v.get("name")?.as_str()?.trim().to_string();
    // Nome é identificador e nada mais: ele vira função no programa e chave
    // no cardápio. Um nome com ponto ou traço quebraria as duas coisas.
    if nome.is_empty()
        || !nome.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || nome.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    let descricao = v
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("Peça criada neste projeto.")
        .to_string();
    let parametros = v
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));

    Some(Plugin {
        nome: format!("{PREFIXO}{nome}"),
        descricao,
        parametros,
        arquivo: PathBuf::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn projeto_com(arquivos: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let plugins = dir.path().join(PLUGINS_DIR);
        std::fs::create_dir_all(&plugins).unwrap();
        for (nome, conteudo) in arquivos {
            std::fs::write(plugins.join(nome), conteudo).unwrap();
        }
        dir
    }

    const BOM: &str = r#"// @tool {"name":"resumo","description":"Resume um log.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}
export default async function ({ path }) { return await fs_read({ path }); }
"#;

    #[test]
    fn a_peca_com_cabecalho_vira_ferramenta_com_prefixo() {
        let dir = projeto_com(&[("resumo.mjs", BOM)]);
        let plugins = carregar(dir.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].nome, "plugin_resumo");
        let spec = plugins[0].spec();
        assert_eq!(spec.description, "Resume um log.");
        assert_eq!(spec.parameters["required"][0], "path");
    }

    #[test]
    fn arquivo_sem_cabecalho_ou_com_nome_ruim_e_ignorado() {
        let dir = projeto_com(&[
            ("rascunho.mjs", "export default () => 1;\n"),
            (
                "ruim.mjs",
                "// @tool {\"name\":\"nome-com-traco\"}\nexport default () => 1;\n",
            ),
            ("quebrado.mjs", "// @tool {não é json}\n"),
        ]);
        assert!(carregar(dir.path()).is_empty());
    }

    #[test]
    fn projeto_sem_a_pasta_nao_e_erro() {
        let dir = TempDir::new().unwrap();
        assert!(carregar(dir.path()).is_empty());
    }

    #[test]
    fn a_ordem_e_estavel_entre_execucoes() {
        let dir = projeto_com(&[
            ("b.mjs", &BOM.replace("\"resumo\"", "\"beta\"")),
            ("a.mjs", &BOM.replace("\"resumo\"", "\"alfa\"")),
        ]);
        let nomes: Vec<String> = carregar(dir.path()).into_iter().map(|p| p.nome).collect();
        assert_eq!(nomes, vec!["plugin_alfa", "plugin_beta"]);
    }
}
