//! Memória de longo prazo do agente.
//!
//! O problema: cada conversa começa do zero. A pessoa explica pela quinta vez
//! que aqui é `pnpm`, que o teste é `cargo test -p lr_x`, que ela quer
//! resposta curta. A solução preguiçosa seria jogar o histórico inteiro num
//! banco vetorial e recuperar trechos por similaridade — e é justamente o que
//! este crate **não** faz.
//!
//! Decisão de projeto: memória é **pouca, curta, curada e inspecionável**.
//! - poucos fatos, não conversas inteiras (o contexto de um modelo local é
//!   caro: o que entra no prompt entra em TODA execução seguinte);
//! - cada fato passa por [`facts::curate`] antes de existir;
//! - tudo aparece em `.openweights/memory/*.md` para a pessoa ler, corrigir
//!   e versionar ([`files`]);
//! - o trabalho pesado (ler episódios, extrair o que ficou) acontece em
//!   ocioso, não no meio do run ([`consolidate`]).
//!
//! As peças:
//!
//! | módulo | papel |
//! |---|---|
//! | [`facts`] | curadoria: normaliza, limita, deduplica, classifica escopo |
//! | [`files`] | `MEMORY.md` + um arquivo por assunto, sem destruir edição manual |
//! | [`consolidate`] | episódios pendentes → fatos duráveis, via modelo |
//! | [`tool`] | `memory_save`, como o modelo guarda um fato sozinho |
//!
//! O caminho de volta já existe fora daqui: `run_start` lê
//! `Store::list_memory_facts` e entrega em `StartRun::memory`, que
//! `lr_agent::prompt` transforma em seção do prompt de sistema. Ou seja:
//! **fato salvo aparece na próxima execução, sem mais nenhuma ligação.**

pub mod consolidate;
pub mod facts;
pub mod files;
pub mod tool;

pub use consolidate::ConsolidateReport;
pub use facts::{CuratedFact, CurationError, FactScope, MAX_FACT_CHARS};
pub use files::{MEMORY_SUBDIR, memory_dir};
pub use tool::{MemorySave, memory_tools};

use lr_store::Store;
use lr_store::memory::MemoryFact;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("erro ao guardar a memória: {0}")]
    Store(#[from] lr_store::StoreError),
    #[error("erro na pasta de memória: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Curation(#[from] facts::CurationError),
    #[error("o modelo não conseguiu organizar a memória: {0}")]
    Engine(#[from] lr_engine::EngineError),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

/// O que aconteceu ao guardar um fato.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFact {
    pub id: i64,
    pub content: String,
    pub scope: FactScope,
    pub topic: String,
    /// Arquivo legível que recebeu o fato (só para fatos de projeto).
    pub file: Option<PathBuf>,
}

/// Fachada da memória: banco + arquivos, sempre nessa ordem.
///
/// O banco é a verdade (é dele que o prompt é montado); os arquivos são a
/// cópia legível. Se a escrita do arquivo falhar, o fato continua valendo —
/// por isso a sincronização dos arquivos é registrada em log, não abortada.
pub struct MemoryStore {
    store: Arc<Store>,
    workspace: Option<PathBuf>,
}

impl MemoryStore {
    pub fn new(store: Arc<Store>, workspace: Option<PathBuf>) -> Self {
        Self { store, workspace }
    }

    pub fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }

    /// Chave de escopo usada no banco (`None` = global).
    fn workspace_key(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Pasta legível da memória — `None` sem projeto aberto.
    pub fn folder(&self) -> Option<PathBuf> {
        self.workspace.as_deref().map(files::memory_dir)
    }

    /// Fatos que valem aqui: os globais mais os deste projeto.
    pub fn facts(&self) -> MemoryResult<Vec<MemoryFact>> {
        Ok(self
            .store
            .list_memory_facts(self.workspace_key().as_deref())?)
    }

    /// Só o texto dos fatos — é o que entra no prompt.
    pub fn fact_texts(&self) -> MemoryResult<Vec<String>> {
        Ok(self.facts()?.into_iter().map(|f| f.content).collect())
    }

    /// Curadoria + gravação. É por aqui que TODO fato entra.
    pub fn save(
        &self,
        raw: &str,
        scope_hint: Option<FactScope>,
        source_run: Option<&str>,
    ) -> MemoryResult<SavedFact> {
        let existing = self.fact_texts()?;
        let fact = facts::curate(raw, &existing, scope_hint, self.workspace.is_some())?;
        self.save_curated(&fact, source_run)
    }

    /// Grava um fato que já passou pela curadoria (caminho da consolidação,
    /// que curou o lote inteiro de uma vez).
    pub fn save_curated(
        &self,
        fact: &CuratedFact,
        source_run: Option<&str>,
    ) -> MemoryResult<SavedFact> {
        let scope_key = match fact.scope {
            FactScope::Global => None,
            FactScope::Workspace => self.workspace_key(),
        };
        let id = self
            .store
            .add_memory_fact(scope_key.as_deref(), &fact.content, source_run)?;

        // Só fato de projeto vira arquivo: preferência da pessoa não pertence
        // ao repositório de ninguém.
        let file = match (&self.workspace, fact.scope) {
            (Some(dir), FactScope::Workspace) => {
                files::append_facts(dir, &fact.topic, std::slice::from_ref(&fact.content))?;
                files::write_index(dir)?;
                Some(files::topic_path(dir, &fact.topic))
            }
            _ => None,
        };

        Ok(SavedFact {
            id,
            content: fact.content.clone(),
            scope: fact.scope,
            topic: fact.topic.clone(),
            file,
        })
    }

    /// Esquece um fato: sai do banco e some do arquivo legível.
    ///
    /// O arquivo é atualizado antes do banco falhar de propósito? Não: banco
    /// primeiro (é a verdade), arquivo depois, e a falha de arquivo só vira
    /// log — a pessoa pediu para esquecer, e o fato já não está mais no
    /// prompt.
    pub fn forget(&self, id: i64) -> MemoryResult<()> {
        let content = self
            .facts()?
            .into_iter()
            .find(|f| f.id == id)
            .map(|f| f.content);
        self.store.delete_memory_fact(id)?;
        if let (Some(dir), Some(content)) = (&self.workspace, content) {
            if let Err(e) = files::remove_fact(dir, &content) {
                log::warn!("não deu para tirar o fato do arquivo de memória: {e}");
            }
            if let Err(e) = files::write_index(dir) {
                log::warn!("não deu para atualizar o MEMORY.md: {e}");
            }
        }
        Ok(())
    }

    /// Registra o que uma execução fez — entrada da consolidação.
    ///
    /// Episódio não é memória ainda: é matéria-prima. Fica pendente até o
    /// [`consolidate`] decidir o que dali sobrevive.
    pub fn record_episode(
        &self,
        chat_id: Option<i64>,
        run_id: Option<&str>,
        summary: &str,
    ) -> MemoryResult<i64> {
        Ok(self.store.add_memory_episode(
            self.workspace_key().as_deref(),
            chat_id,
            run_id,
            summary,
        )?)
    }

    /// Arrumação em ocioso. **Nunca chame com um run ativo**: usa o mesmo
    /// modelo do agente e roubaria contexto no meio do trabalho — quem chama
    /// é que verifica (a parte pura é [`consolidate::plan`], testável sem
    /// servidor).
    pub async fn consolidate_now(
        &self,
        client: &lr_engine::LlamaClient,
        model: &str,
    ) -> MemoryResult<ConsolidateReport> {
        consolidate::run(self, client, model).await
    }

    /// Reescreve o `MEMORY.md` a partir dos arquivos de assunto.
    pub fn refresh_files(&self) -> MemoryResult<bool> {
        match &self.workspace {
            Some(dir) => Ok(files::write_index(dir)?),
            None => Ok(false),
        }
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }
}

/// Quantos fatos cabem na seção do prompt. Igual ao teto de
/// `lr_agent::prompt` — acima disso o modelo pequeno para de ler.
pub const MAX_SECTION_FACTS: usize = 12;

/// Teto em caracteres da seção inteira (~300 tokens).
pub const MAX_SECTION_CHARS: usize = 1200;

/// O trecho de memória que entra no prompt de sistema.
///
/// Existe para que o teto viva num lugar só: a memória pode crescer sem
/// limite no banco, mas o que chega ao modelo é sempre este pedaço curto.
/// Vazio quando não há fato — seção vazia no prompt é ruído.
pub fn system_section(facts: &[String]) -> String {
    let header = "## O que já sabemos deste projeto\n";
    let mut out = String::new();
    let mut used = 0usize;

    for fact in facts.iter().filter(|f| !f.trim().is_empty()) {
        if out.matches('\n').count() >= MAX_SECTION_FACTS {
            break;
        }
        let line = format!("- {}\n", facts::clip(fact.trim()));
        if used + line.len() > MAX_SECTION_CHARS {
            break;
        }
        used += line.len();
        out.push_str(&line);
    }

    if out.is_empty() {
        return String::new();
    }
    format!("{header}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    pub(crate) fn memory(dir: &TempDir) -> MemoryStore {
        MemoryStore::new(
            Arc::new(Store::open_in_memory().unwrap()),
            Some(dir.path().to_path_buf()),
        )
    }

    #[test]
    fn saving_writes_the_database_and_the_readable_file() {
        let dir = TempDir::new().unwrap();
        let mem = memory(&dir);

        let saved = mem
            .save("este projeto usa pnpm, não npm", None, Some("run-1"))
            .unwrap();
        assert_eq!(saved.scope, FactScope::Workspace);
        assert_eq!(saved.topic, "build");

        assert_eq!(
            mem.fact_texts().unwrap(),
            vec!["este projeto usa pnpm, não npm"]
        );
        let file = std::fs::read_to_string(saved.file.unwrap()).unwrap();
        assert!(file.contains("- este projeto usa pnpm, não npm"));
        assert!(files::read_index(dir.path()).unwrap().contains("build.md"));
    }

    #[test]
    fn a_global_fact_never_lands_in_the_project_folder() {
        let dir = TempDir::new().unwrap();
        let mem = memory(&dir);

        let saved = mem.save("prefiro respostas curtas", None, None).unwrap();
        assert_eq!(saved.scope, FactScope::Global);
        assert!(saved.file.is_none());
        assert!(!memory_dir(dir.path()).exists());

        // Mas continua valendo aqui (e em qualquer outro projeto).
        assert_eq!(mem.fact_texts().unwrap().len(), 1);
    }

    #[test]
    fn duplicates_are_refused_with_a_message_for_the_model() {
        let dir = TempDir::new().unwrap();
        let mem = memory(&dir);
        mem.save("este projeto usa pnpm", None, None).unwrap();

        let err = mem.save("Este projeto usa pnpm!", None, None).unwrap_err();
        let MemoryError::Curation(err) = err else {
            panic!("esperava recusa da curadoria");
        };
        assert!(err.to_model_message().contains("já está na memória"));
        assert_eq!(mem.facts().unwrap().len(), 1);
    }

    #[test]
    fn forgetting_clears_the_prompt_and_the_file() {
        let dir = TempDir::new().unwrap();
        let mem = memory(&dir);
        let saved = mem.save("este projeto usa pnpm", None, None).unwrap();
        let path = saved.file.clone().unwrap();

        mem.forget(saved.id).unwrap();
        assert!(mem.fact_texts().unwrap().is_empty());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("pnpm"), "{body}");
    }

    #[test]
    fn without_a_project_everything_is_global_and_nothing_is_written() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mem = MemoryStore::new(store, None);
        let saved = mem
            .save("este projeto usa pnpm", Some(FactScope::Workspace), None)
            .unwrap();
        assert_eq!(saved.scope, FactScope::Global);
        assert!(saved.file.is_none());
        assert!(mem.folder().is_none());
        assert!(!mem.refresh_files().unwrap());
    }

    #[test]
    fn the_prompt_section_has_a_ceiling() {
        assert_eq!(system_section(&[]), "");
        assert_eq!(system_section(&["   ".to_string()]), "");

        let facts: Vec<String> = (0..40).map(|i| format!("fato número {i}")).collect();
        let section = system_section(&facts);
        assert!(section.starts_with("## O que já sabemos deste projeto\n"));
        assert!(section.contains("- fato número 0\n"));
        assert!(!section.contains("- fato número 12\n"), "corta em 12 fatos");
        assert!(section.len() <= MAX_SECTION_CHARS + 64);

        // Fatos gigantes cortam pelo orçamento de caracteres, não pelo de itens.
        let fat: Vec<String> = (0..12)
            .map(|i| format!("{i} {}", "x".repeat(230)))
            .collect();
        let section = system_section(&fat);
        assert!(section.len() <= MAX_SECTION_CHARS + 64, "{}", section.len());
        assert!(section.lines().count() < 12);
    }
}
