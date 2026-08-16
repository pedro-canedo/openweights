//! Ferramentas que mexem no repositório: `git_add`, `git_commit`,
//! `git_restore` e `git_stash`.
//!
//! Todas são [`ToolCategory::Edit`], ou seja, passam pela tela de confirmação
//! (exceto no modo automático). O que cada prévia mostra foi escolhido pela
//! pergunta "o que a pessoa precisa ver para dizer sim com segurança?":
//!
//! - `git_add` mostra a lista de arquivos que serão preparados;
//! - `git_commit` mostra a mensagem **e** os arquivos que vão entrar — commit
//!   com arquivo a mais é o erro mais comum e mais chato de desfazer;
//! - `git_restore` mostra o diff do que será **perdido**, porque é a única
//!   ferramenta aqui que destrói trabalho não commitado;
//! - `git_stash` mostra o que sai da árvore (ou o que volta).
//!
//! Sobre [`Tool::files_at_risk`]: só `git_restore` devolve caminho. O
//! checkpoint existe para poder desfazer perda de conteúdo, e `add`/`commit`
//! não alteram nenhum arquivo da árvore de trabalho — pedir checkpoint neles
//! seria custo sem contrapartida. `git_stash` também não pede: ele guarda o
//! trabalho no próprio git, e a volta é um `git_stash {pop: true}`.
//!
//! Não implementamos `push`/`pull`/`fetch` aqui de propósito: são as únicas
//! operações que saem da máquina, e sair da máquina merece categoria de rede,
//! prévia de "para onde" e tratamento de credencial — assunto próprio.

use crate::run::{command_failed, git, repo_root};
use async_trait::async_trait;
use lr_tools::{Tool, ToolContext, ToolError, ToolOutput, ToolResult, arg_bool, arg_str};
use lr_types::agent::{ToolCategory, ToolPreview};
use serde_json::{Value, json};

/// Máximo de arquivos listados numa prévia ou num resultado.
const MAX_LISTED: usize = 40;

/// Teto de caracteres do diff mostrado na prévia do `git_restore`.
const PREVIEW_CHARS: usize = 6_000;

/// Lê o argumento `paths`.
///
/// Aceita tanto uma lista quanto um texto único: modelos pequenos mandam
/// `"paths": "src/app.rs"` o tempo todo, e recusar isso gasta um passo do
/// agente para não ganhar nada — a intenção é inequívoca.
fn read_paths(args: &Value) -> ToolResult<Vec<String>> {
    let raw = args.get("paths").ok_or_else(|| {
        ToolError::InvalidArgs(
            "faltou `paths` (lista de arquivos, ex.: [\"src/app.rs\"]; use [\".\"] para tudo)"
                .into(),
        )
    })?;

    let paths: Vec<String> = match raw {
        Value::String(one) => vec![one.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect(),
        _ => {
            return Err(ToolError::InvalidArgs(
                "`paths` precisa ser uma lista de textos, ex.: [\"src/app.rs\", \"README.md\"]"
                    .into(),
            ));
        }
    };

    let paths: Vec<String> = paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    if paths.is_empty() {
        return Err(ToolError::InvalidArgs(
            "`paths` está vazio — diga quais arquivos preparar (use [\".\"] para todos)".into(),
        ));
    }
    Ok(paths)
}

/// Confere que todos os caminhos ficam dentro da pasta do projeto.
fn check_inside(paths: &[String], ctx: &ToolContext) -> ToolResult<()> {
    for path in paths {
        ctx.resolve(path)?;
    }
    Ok(())
}

/// Lista de nomes em bloco, cortada quando é grande demais.
fn bullet_list(names: &[String]) -> String {
    let mut out = String::new();
    for name in names.iter().take(MAX_LISTED) {
        out.push_str(&format!("  {name}\n"));
    }
    if names.len() > MAX_LISTED {
        out.push_str(&format!(
            "  ... e mais {} arquivo(s)\n",
            names.len() - MAX_LISTED
        ));
    }
    out
}

/// Arquivos atualmente preparados para o commit.
async fn staged_files(ctx: &ToolContext) -> Vec<String> {
    match git(ctx, &["diff", "--cached", "--name-only"]).await {
        Ok(run) if run.ok() => run
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

// ------------------------------------------------------------------- add ---

/// Ferramenta `git_add`.
pub struct GitAdd;

#[async_trait]
impl Tool for GitAdd {
    fn name(&self) -> &str {
        "git_add"
    }

    fn description(&self) -> &str {
        "Prepara arquivos para o próximo commit (o `git add`). Nada é gravado no histórico e nada \
         sai da máquina — depois disso use `git_commit`. Passe [\".\"] para preparar tudo que \
         mudou."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arquivos ou pastas, relativos à raiz do projeto (ex.: [\"src/app.rs\", \"README.md\"]). Use [\".\"] para preparar tudo."
                }
            },
            "required": ["paths"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        match read_paths(args) {
            Ok(paths) => check_inside(&paths, ctx).is_ok(),
            // Argumento inválido não é "fora do projeto"; o erro certo aparece
            // na execução.
            Err(_) => true,
        }
    }

    async fn preview(&self, args: &Value, _ctx: &ToolContext) -> Option<ToolPreview> {
        let paths = read_paths(args).ok()?;
        Some(ToolPreview::Text {
            body: format!(
                "Preparar para o commit:\n{}\nNada é gravado no histórico ainda.",
                bullet_list(&paths)
            ),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let paths = read_paths(&args)?;
        check_inside(&paths, ctx)?;

        let mut cmd: Vec<&str> = vec!["add", "--"];
        cmd.extend(paths.iter().map(String::as_str));

        let run = git(ctx, &cmd).await?;
        if !run.ok() {
            return Err(ToolError::Other(format!(
                "O git não conseguiu preparar esses caminhos: {}. Confira os nomes com \
                 `git_status` — caminho que não casa com nada é recusado.",
                run.error_text()
            )));
        }

        let staged = staged_files(ctx).await;
        if staged.is_empty() {
            return Ok(ToolOutput::text(
                "Comando aceito, mas não há nada preparado para o commit — os caminhos indicados \
                 não tinham alterações. Veja `git_status`.",
            ));
        }
        Ok(ToolOutput::text(format!(
            "Preparado(s) para o commit ({}):\n{}",
            staged.len(),
            bullet_list(&staged)
        ))
        .truncated_to(ctx.max_output_bytes))
    }
}

// ---------------------------------------------------------------- commit ---

/// Ferramenta `git_commit`.
pub struct GitCommit;

/// Lê e valida a mensagem do commit.
fn read_message(args: &Value) -> ToolResult<String> {
    let message = arg_str(args, "message")?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err(ToolError::InvalidArgs(
            "a mensagem do commit está vazia. Escreva em uma linha o que mudou e por quê \
             (ex.: \"corrige cálculo do frete para pedidos sem CEP\")"
                .into(),
        ));
    }
    Ok(message)
}

#[async_trait]
impl Tool for GitCommit {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Grava no histórico do Git tudo que foi preparado com `git_add`, com a mensagem informada. \
         Commita apenas o que está preparado — arquivo mexido e não preparado fica de fora. O \
         commit é local: nada é enviado para um servidor."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Mensagem do commit: uma linha curta dizendo o que mudou e por quê. Não pode ser vazia."
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let message = read_message(args).ok()?;
        let staged = staged_files(ctx).await;
        let corpo = if staged.is_empty() {
            "Nada está preparado para o commit — use `git_add` antes.".to_string()
        } else {
            format!(
                "Vão entrar neste commit ({} arquivo(s)):\n{}",
                staged.len(),
                bullet_list(&staged)
            )
        };
        Some(ToolPreview::Text {
            body: format!("Mensagem:\n  {message}\n\n{corpo}"),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let message = read_message(&args)?;

        // Checagem antes de chamar o git: assim a explicação é nossa (e em
        // português) em vez do texto traduzido do "nothing to commit".
        if staged_files(ctx).await.is_empty() {
            return Err(ToolError::Other(
                "Não há nada preparado para o commit. Rode `git_add` com os arquivos que devem \
                 entrar e tente de novo (veja o que mudou com `git_status`)."
                    .into(),
            ));
        }

        let run = git(ctx, &["commit", "-m", &message]).await?;
        if !run.ok() {
            let texto = run.error_text();
            // `user.email` aparece literalmente mesmo com o git traduzido: é
            // nome de configuração, não frase.
            if texto.contains("user.email") || texto.contains("user.name") {
                return Err(ToolError::Other(format!(
                    "O Git ainda não sabe quem você é, então não pode assinar o commit. Rode no \
                     terminal: `git config --global user.name \"Seu Nome\"` e \
                     `git config --global user.email \"voce@exemplo.com\"`. (Git: {texto})"
                )));
            }
            return Err(ToolError::Other(format!(
                "O commit não foi gravado: {texto}. Se houver um hook de pré-commit no projeto, \
                 corrija o que ele apontou e tente de novo."
            )));
        }

        Ok(
            ToolOutput::text(format!("Commit gravado.\n{}", run.combined()))
                .truncated_to(ctx.max_output_bytes),
        )
    }
}

// --------------------------------------------------------------- restore ---

/// Ferramenta `git_restore`.
pub struct GitRestore;

impl GitRestore {
    /// O que existe hoje de diferente no arquivo — ou seja, o que se perde.
    async fn pending_diff(&self, rel: &str, ctx: &ToolContext) -> String {
        match git(ctx, &["diff", "--no-color", "--no-ext-diff", "--", rel]).await {
            Ok(run) if run.ok() => run.stdout,
            _ => String::new(),
        }
    }
}

#[async_trait]
impl Tool for GitRestore {
    fn name(&self) -> &str {
        "git_restore"
    }

    fn description(&self) -> &str {
        "Desfaz as alterações não commitadas de UM arquivo, voltando ao conteúdo da última versão \
         registrada no Git. As mudanças descartadas não voltam — leia a prévia antes de aceitar."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Arquivo a desfazer, relativo à raiz do projeto (ex.: src/app.rs). Se apontar uma pasta, desfaz tudo que mudou dentro dela — prefira um arquivo por vez."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn within_workspace(&self, args: &Value, ctx: &ToolContext) -> bool {
        match arg_str(args, "path") {
            Ok(rel) => ctx.resolve(&rel).is_ok(),
            Err(_) => true,
        }
    }

    /// O conteúdo atual do arquivo vai sumir: é exatamente o caso para o
    /// checkpoint automático antes da execução.
    fn files_at_risk(&self, args: &Value, ctx: &ToolContext) -> Vec<String> {
        match arg_str(args, "path") {
            Ok(rel) => match ctx.resolve(&rel) {
                Ok(path) => vec![ctx.relativize(&path)],
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        let rel = arg_str(args, "path").ok()?;
        ctx.resolve(&rel).ok()?;
        repo_root(ctx).await.ok()?;

        let diff = self.pending_diff(&rel, ctx).await;
        if diff.trim().is_empty() {
            return Some(ToolPreview::Text {
                body: format!(
                    "`{rel}` não tem alterações para desfazer (ou não está sendo rastreado pelo \
                     Git). Nada seria perdido."
                ),
            });
        }
        Some(ToolPreview::Diff {
            path: rel,
            // A prévia mostra o que existe hoje e vai embora.
            unified: diff.chars().take(PREVIEW_CHARS).collect(),
            created: false,
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let rel = arg_str(&args, "path")?;
        ctx.resolve(&rel)?;

        let havia = !self.pending_diff(&rel, ctx).await.trim().is_empty();

        let run = git(ctx, &["restore", "--worktree", "--", &rel]).await?;
        if !run.ok() {
            let rastreado = git(ctx, &["ls-files", "--error-unmatch", "--", &rel])
                .await
                .map(|r| r.ok())
                .unwrap_or(false);
            if !rastreado {
                return Err(ToolError::Other(format!(
                    "`{rel}` não está no Git (arquivo novo, nunca commitado), então não há versão \
                     anterior para restaurar. Se quiser removê-lo, apague o arquivo direto."
                )));
            }
            return Err(ToolError::Other(format!(
                "Não consegui desfazer `{rel}`: {}. (O `git restore` exige Git 2.23 ou mais novo.)",
                run.error_text()
            )));
        }

        if !havia {
            return Ok(ToolOutput::text(format!(
                "`{rel}` já estava igual à última versão commitada; nada foi alterado."
            )));
        }
        Ok(ToolOutput::text(format!(
            "Alterações de `{rel}` descartadas: o arquivo voltou ao conteúdo do último commit."
        ))
        .with_changed(vec![rel]))
    }
}

// ----------------------------------------------------------------- stash ---

/// Ferramenta `git_stash`.
pub struct GitStash;

impl GitStash {
    /// Nomes dos arquivos que seriam guardados (mexidos e/ou preparados).
    async fn dirty_files(&self, ctx: &ToolContext) -> Vec<String> {
        let mut nomes: Vec<String> = Vec::new();
        for cmd in [
            vec!["diff", "--name-only"],
            vec!["diff", "--cached", "--name-only"],
        ] {
            if let Ok(run) = git(ctx, &cmd).await
                && run.ok()
            {
                for linha in run.stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    if !nomes.iter().any(|n| n == linha) {
                        nomes.push(linha.to_string());
                    }
                }
            }
        }
        nomes.sort();
        nomes
    }

    /// Itens já guardados, um por linha.
    async fn stash_list(&self, ctx: &ToolContext) -> Vec<String> {
        match git(ctx, &["stash", "list"]).await {
            Ok(run) if run.ok() => run
                .stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[async_trait]
impl Tool for GitStash {
    fn name(&self) -> &str {
        "git_stash"
    }

    fn description(&self) -> &str {
        "Guarda temporariamente as alterações não commitadas para deixar a pasta limpa, ou as \
         devolve com `pop: true`. Guarda apenas arquivos que o Git já rastreia; arquivos novos \
         (não rastreados) continuam onde estão."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pop": {
                    "type": "boolean",
                    "description": "true devolve à pasta o último trabalho guardado; false (padrão) guarda o trabalho atual."
                }
            },
            "additionalProperties": false
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    async fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<ToolPreview> {
        repo_root(ctx).await.ok()?;
        let pop = arg_bool(args, "pop", false);
        let body = if pop {
            match self.stash_list(ctx).await.first() {
                Some(topo) => format!("Devolver à pasta o trabalho guardado:\n  {topo}"),
                None => "Não há nada guardado para devolver.".to_string(),
            }
        } else {
            let sujos = self.dirty_files(ctx).await;
            if sujos.is_empty() {
                "Não há alterações para guardar.".to_string()
            } else {
                format!(
                    "Sair da pasta e ficar guardado ({} arquivo(s)):\n{}\nPara trazer de volta: \
                     `git_stash` com `pop: true`.",
                    sujos.len(),
                    bullet_list(&sujos)
                )
            }
        };
        Some(ToolPreview::Text { body })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult<ToolOutput> {
        repo_root(ctx).await?;
        let pop = arg_bool(&args, "pop", false);

        if pop {
            if self.stash_list(ctx).await.is_empty() {
                return Ok(ToolOutput::text(
                    "Não há nada guardado: a pilha do `git stash` está vazia.",
                ));
            }
            let run = git(ctx, &["stash", "pop"]).await?;
            if !run.ok() {
                return Err(ToolError::Other(format!(
                    "Não consegui devolver o trabalho guardado: {}. Se houver conflito, resolva os \
                     arquivos apontados e depois use `git_add`.",
                    run.error_text()
                )));
            }
            return Ok(ToolOutput::text(format!(
                "Trabalho devolvido à pasta.\n{}",
                run.combined()
            ))
            .truncated_to(ctx.max_output_bytes));
        }

        let sujos = self.dirty_files(ctx).await;
        if sujos.is_empty() {
            return Ok(ToolOutput::text(
                "Não havia alterações rastreadas para guardar; a pasta já estava limpa. (Arquivos \
                 novos, nunca adicionados ao Git, não entram no stash.)",
            ));
        }

        let run = git(ctx, &["stash", "push"]).await?;
        if !run.ok() {
            return Err(command_failed("guardar as alterações", &run));
        }
        Ok(ToolOutput::text(format!(
            "Guardado ({} arquivo(s)); a pasta voltou ao estado do último commit. Use \
             `git_stash` com `pop: true` para trazer de volta.\n{}",
            sujos.len(),
            run.combined()
        ))
        .with_changed(sujos)
        .truncated_to(ctx.max_output_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::repo_or_skip;

    #[test]
    fn paths_accept_a_list_or_a_single_string() {
        assert_eq!(
            read_paths(&json!({"paths": ["a", "b"]})).unwrap(),
            ["a", "b"]
        );
        assert_eq!(read_paths(&json!({"paths": "a"})).unwrap(), ["a"]);
    }

    #[test]
    fn an_empty_paths_argument_explains_what_to_send() {
        for args in [json!({}), json!({"paths": []}), json!({"paths": ["  "]})] {
            let msg = read_paths(&args).unwrap_err().to_model_message();
            assert!(msg.contains("paths"), "{msg}");
        }
    }

    #[test]
    fn an_empty_commit_message_is_refused_before_touching_git() {
        for args in [json!({"message": ""}), json!({"message": "   \n"})] {
            let err = read_message(&args).unwrap_err();
            assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
            assert!(err.to_model_message().contains("vazia"));
        }
        assert!(read_message(&json!({})).is_err());
    }

    #[tokio::test]
    async fn add_stages_the_named_files() {
        let repo = repo_or_skip!();
        repo.write("novo.txt", "conteudo\n");
        repo.write("outro.txt", "outro\n");

        let out = GitAdd
            .execute(json!({"paths": ["novo.txt"]}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("novo.txt"), "{}", out.content);
        assert!(!out.content.contains("outro.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn add_refuses_paths_outside_the_project() {
        let repo = repo_or_skip!();
        let args = json!({"paths": ["../fora.txt"]});
        assert!(!GitAdd.within_workspace(&args, &repo.ctx));
        let err = GitAdd.execute(args, &repo.ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::OutsideWorkspace(_)), "{err:?}");
    }

    #[tokio::test]
    async fn add_preview_lists_what_will_be_staged() {
        let repo = repo_or_skip!();
        let preview = GitAdd
            .preview(&json!({"paths": ["a.txt", "b.txt"]}), &repo.ctx)
            .await
            .expect("prévia");
        match preview {
            ToolPreview::Text { body } => {
                assert!(body.contains("a.txt") && body.contains("b.txt"), "{body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn commit_records_what_was_staged() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        repo.git(&["add", "src/app.txt"]).await;

        let out = GitCommit
            .execute(json!({"message": "ajusta o beta"}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("Commit gravado"), "{}", out.content);
        assert_eq!(repo.subjects().await.first().unwrap(), "ajusta o beta");
    }

    #[tokio::test]
    async fn commit_without_anything_staged_says_to_add_first() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "mexido\n");
        let err = GitCommit
            .execute(json!({"message": "tentativa"}), &repo.ctx)
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("git_add"), "{msg}");
        // E não gravou nada.
        assert_eq!(repo.subjects().await.len(), 1);
    }

    #[tokio::test]
    async fn commit_preview_shows_the_message_and_the_files() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nBETA\ngama\n");
        repo.git(&["add", "src/app.txt"]).await;

        let preview = GitCommit
            .preview(&json!({"message": "ajusta o beta"}), &repo.ctx)
            .await
            .expect("prévia");
        match preview {
            ToolPreview::Text { body } => {
                assert!(body.contains("ajusta o beta"), "{body}");
                assert!(body.contains("src/app.txt"), "{body}");
            }
            other => panic!("esperava texto, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn restore_brings_the_file_back_to_the_last_commit() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "estragado\n");

        let out = GitRestore
            .execute(json!({"path": "src/app.txt"}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("descartadas"), "{}", out.content);
        assert_eq!(repo.read("src/app.txt"), "alfa\nbeta\ngama\n");
        assert_eq!(out.changed_files, vec!["src/app.txt"]);
    }

    #[tokio::test]
    async fn restore_preview_shows_exactly_what_is_lost() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "alfa\nestragado\ngama\n");
        let args = json!({"path": "src/app.txt"});

        assert_eq!(GitRestore.files_at_risk(&args, &repo.ctx), ["src/app.txt"]);
        match GitRestore.preview(&args, &repo.ctx).await.expect("prévia") {
            ToolPreview::Diff { path, unified, .. } => {
                assert_eq!(path, "src/app.txt");
                assert!(unified.contains("-beta"), "{unified}");
                assert!(unified.contains("+estragado"), "{unified}");
            }
            other => panic!("esperava diff, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn restoring_an_untracked_file_explains_why_it_cannot() {
        let repo = repo_or_skip!();
        repo.write("solto.txt", "nunca commitado\n");
        let err = GitRestore
            .execute(json!({"path": "solto.txt"}), &repo.ctx)
            .await
            .unwrap_err();
        let msg = err.to_model_message();
        assert!(msg.contains("não está no Git"), "{msg}");
        // O arquivo continua lá: nada foi apagado por engano.
        assert_eq!(repo.read("solto.txt"), "nunca commitado\n");
    }

    #[tokio::test]
    async fn stash_hides_and_pop_brings_back() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "trabalho em andamento\n");

        let guardou = GitStash.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(guardou.content.contains("Guardado"), "{}", guardou.content);
        assert_eq!(repo.read("src/app.txt"), "alfa\nbeta\ngama\n");

        let voltou = GitStash
            .execute(json!({"pop": true}), &repo.ctx)
            .await
            .unwrap();
        assert!(voltou.content.contains("devolvido"), "{}", voltou.content);
        assert_eq!(repo.read("src/app.txt"), "trabalho em andamento\n");
    }

    #[tokio::test]
    async fn stash_with_nothing_to_save_is_not_an_error() {
        let repo = repo_or_skip!();
        let out = GitStash.execute(json!({}), &repo.ctx).await.unwrap();
        assert!(out.content.contains("já estava limpa"), "{}", out.content);
    }

    #[tokio::test]
    async fn popping_an_empty_stash_says_so_instead_of_failing() {
        let repo = repo_or_skip!();
        let out = GitStash
            .execute(json!({"pop": true}), &repo.ctx)
            .await
            .unwrap();
        assert!(out.content.contains("está vazia"), "{}", out.content);
    }

    #[tokio::test]
    async fn stash_preview_lists_what_leaves_the_folder() {
        let repo = repo_or_skip!();
        repo.write("src/app.txt", "mexido\n");
        match GitStash
            .preview(&json!({}), &repo.ctx)
            .await
            .expect("prévia")
        {
            ToolPreview::Text { body } => assert!(body.contains("src/app.txt"), "{body}"),
            other => panic!("esperava texto, veio {other:?}"),
        }
    }
}
