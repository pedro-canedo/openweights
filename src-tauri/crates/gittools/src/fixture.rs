//! Repositório de verdade em pasta temporária, para os testes.
//!
//! Duas regras que valem para toda a suíte:
//!
//! **Nunca a configuração global do usuário.** O `git init` é seguido de
//! `git config --local` para tudo que poderia atrapalhar: assinatura de commit
//! (`commit.gpgsign = true` faria todo commit falhar numa máquina que assina),
//! hooks (`core.hooksPath` global rodaria o hook pessoal de quem estiver
//! rodando os testes), conversão de fim de linha e identidade. O `--local`
//! grava em `.git/config`, que vence o `~/.gitconfig` — e vale também para as
//! chamadas feitas pelas ferramentas, que rodam no mesmo repositório. As
//! opções que não dá para fixar por config (cor, untracked, formato de data)
//! são passadas como argumento pelas próprias ferramentas.
//!
//! **Nada de nome de branch fixo.** `init.defaultBranch` varia de máquina para
//! máquina; os testes perguntam ao git qual é a branch atual em vez de supor
//! `main` ou `master`.
//!
//! Se o git não existir na máquina, [`repo`] devolve `None` e o teste se
//! encerra com um aviso — melhor pular do que reprovar por algo que não é do
//! projeto.

#![cfg(test)]

use crate::run::{GIT, GitRun, run_in};
use lr_tools::ToolContext;
use std::path::PathBuf;
use tempfile::TempDir;

/// Orçamento de saída dos comandos de apoio dos testes.
const BUDGET: usize = 64 * 1024;

/// O git existe nesta máquina?
pub async fn git_available() -> bool {
    matches!(
        run_in(GIT, std::env::temp_dir(), &["--version"], BUDGET).await,
        Ok(run) if run.ok()
    )
}

/// Repositório pronto: um commit inicial com `README.md` e `src/app.txt`.
pub struct TestRepo {
    _dir: TempDir,
    root: PathBuf,
    pub ctx: ToolContext,
}

impl TestRepo {
    /// Roda um git de apoio (montagem do cenário), não a ferramenta em teste.
    pub async fn git(&self, args: &[&str]) -> GitRun {
        let run = run_in(GIT, self.root.clone(), args, BUDGET)
            .await
            .unwrap_or_else(|e| panic!("git {args:?} não rodou: {e}"));
        assert!(
            run.ok(),
            "git {args:?} falhou ({:?}): {}",
            run.code,
            run.error_text()
        );
        run
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.root.join(rel)).unwrap();
    }

    /// Nome da branch atual, perguntado ao git (não suposto).
    pub async fn current_branch(&self) -> String {
        self.git(&["symbolic-ref", "--short", "HEAD"])
            .await
            .stdout
            .trim()
            .to_string()
    }

    /// Assuntos dos commits, do mais novo para o mais antigo.
    pub async fn subjects(&self) -> Vec<String> {
        self.git(&["log", "--pretty=format:%s"])
            .await
            .stdout
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// Monta o repositório. `None` quando não há git na máquina.
pub async fn repo() -> Option<TestRepo> {
    if !git_available().await {
        eprintln!("AVISO: git não encontrado nesta máquina — teste pulado.");
        return None;
    }

    let dir = tempfile::tempdir().unwrap();
    // Canonicalizado porque em alguns sistemas a pasta temporária é um link
    // (`/tmp` -> `/private/tmp`) e o git devolve sempre o caminho real; sem
    // isso as comparações de caminho ficariam falsamente diferentes.
    let root = dir
        .path()
        .canonicalize()
        .unwrap_or_else(|_| dir.path().into());
    let hooks = root.join("sem-hooks");
    std::fs::create_dir_all(&hooks).unwrap();

    let setup: Vec<Vec<String>> = vec![
        vec!["init".into()],
        vec![
            "config".into(),
            "--local".into(),
            "user.name".into(),
            "Teste".into(),
        ],
        vec![
            "config".into(),
            "--local".into(),
            "user.email".into(),
            "teste@exemplo.invalido".into(),
        ],
        // Sem assinatura: numa máquina que assina por padrão, todo commit do
        // teste falharia pedindo a chave.
        vec![
            "config".into(),
            "--local".into(),
            "commit.gpgsign".into(),
            "false".into(),
        ],
        vec![
            "config".into(),
            "--local".into(),
            "tag.gpgsign".into(),
            "false".into(),
        ],
        // Sem hooks do usuário e sem conversão de fim de linha.
        vec![
            "config".into(),
            "--local".into(),
            "core.hooksPath".into(),
            hooks.to_string_lossy().into_owned(),
        ],
        vec![
            "config".into(),
            "--local".into(),
            "core.autocrlf".into(),
            "false".into(),
        ],
        vec![
            "config".into(),
            "--local".into(),
            "gc.auto".into(),
            "0".into(),
        ],
    ];
    for args in &setup {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let run = run_in(GIT, root.clone(), &refs, BUDGET).await.ok()?;
        assert!(
            run.ok(),
            "montagem falhou em {refs:?}: {}",
            run.error_text()
        );
    }

    let ctx = ToolContext::new(Some(root.clone()), "call-teste");
    let repo = TestRepo {
        _dir: dir,
        root,
        ctx,
    };

    repo.write("README.md", "# Projeto\n\nlinha original\n");
    repo.write("src/app.txt", "alfa\nbeta\ngama\n");
    repo.git(&["add", "."]).await;
    repo.git(&["commit", "-m", "primeiro commit"]).await;

    Some(repo)
}

/// Pega o repositório de teste ou encerra o teste com aviso.
macro_rules! repo_or_skip {
    () => {
        match $crate::fixture::repo().await {
            Some(repo) => repo,
            None => return,
        }
    };
}

pub(crate) use repo_or_skip;
