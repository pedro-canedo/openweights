//! Ferramentas que ligam o agente ao computador em volta do projeto:
//! `clipboard_read`, `clipboard_write`, `notify_user` e `open_path`.
//!
//! | Ferramenta         | Categoria | Para quê |
//! |--------------------|-----------|----------|
//! | `clipboard_read`   | Read      | ler o que a pessoa copiou |
//! | `clipboard_write`  | Edit      | deixar um texto pronto para colar |
//! | `notify_user`      | Meta      | chamar a pessoa de volta ao computador |
//! | `open_path`        | Execute   | mostrar um arquivo, uma pasta ou um link |
//!
//! # Por que existe o [`DesktopHost`]
//!
//! Área de transferência, notificação e "abrir no aplicativo padrão" são as
//! três coisas que **só o app sabe fazer**: dependem do `AppHandle` do Tauri,
//! dos plugins do sistema e da janela. Se este crate falasse com eles direto,
//! duas coisas ruins aconteceriam: a suíte precisaria de tela (e de área de
//! transferência de verdade) para rodar, e nenhum teste poderia provar as
//! recusas — que são justamente a parte que protege a pessoa.
//!
//! Então o crate define o contrato e o app implementa. Aqui dentro mora só o
//! que é decisão: validar o argumento, recusar o que não deve passar, montar
//! a prévia da confirmação e explicar o resultado ao modelo. Nos testes, o
//! contrato é cumprido por um host de mentira em memória.
//!
//! # A regra do `open_path`
//!
//! Abrir no aplicativo padrão **é executar**: um `.exe` roda, um `.ps1` pode
//! rodar. Por isso a ferramenta é [`ToolCategory::Execute`], recusa extensão
//! de programa e de script, e só aceita caminho dentro da pasta do projeto ou
//! link `http`/`https` — nunca `file:`, `javascript:` ou esquema de aplicativo
//! (`steam:`, `ms-settings:`), que entregam o comando a outro programa.
//!
//! [`ToolCategory::Execute`]: lr_types::agent::ToolCategory::Execute

use std::sync::Arc;

use lr_tools::SharedTool;

pub mod clipboard;
pub mod notify;
pub mod open;

#[cfg(test)]
pub(crate) mod testing;

pub use clipboard::{ClipboardRead, ClipboardWrite, MAX_CLIPBOARD_CHARS};
pub use notify::{MAX_NOTIFY_BODY_CHARS, MAX_NOTIFY_TITLE_CHARS, NotifyUser};
pub use open::OpenPath;

/// O que só o app sabe fazer (tem `AppHandle`, plugins do Tauri, janela).
///
/// Toda falha volta como texto pronto para a pessoa ler: o crate não conhece
/// os erros de cada plugin e não teria como traduzi-los melhor do que quem
/// os produziu.
pub trait DesktopHost: Send + Sync {
    fn clipboard_read(&self) -> Result<String, String>;
    fn clipboard_write(&self, text: &str) -> Result<(), String>;
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;
    /// Abre no aplicativo padrão do sistema. Recebe caminho absoluto ou URL
    /// http/https — a validação já aconteceu antes de chegar aqui.
    fn open(&self, target: &str) -> Result<(), String>;
}

/// As quatro ferramentas de área de trabalho, prontas para o registro.
pub fn desktop_tools(host: Arc<dyn DesktopHost>) -> Vec<SharedTool> {
    vec![
        Arc::new(ClipboardRead::new(host.clone())),
        Arc::new(ClipboardWrite::new(host.clone())),
        Arc::new(NotifyUser::new(host.clone())),
        Arc::new(OpenPath::new(host)),
    ]
}

/// Corta um texto no limite de **caracteres**, devolvendo `(texto, cortou?)`.
///
/// Caractere e não byte: um corte por byte parte um acentuado no meio e
/// produz UTF-8 inválido, que é justamente o que a área de transferência de
/// um usuário brasileiro tem em toda linha.
pub(crate) fn cut_chars(text: &str, max: usize) -> (String, bool) {
    let mut kept = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == max {
            return (kept, true);
        }
        kept.push(c);
    }
    (kept, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHost;
    use lr_types::agent::{ToolCategory, ToolTier};

    fn catalog() -> Vec<SharedTool> {
        desktop_tools(Arc::new(FakeHost::default()))
    }

    #[test]
    fn the_catalog_has_the_four_tools() {
        let mut names: Vec<String> = catalog().iter().map(|t| t.name().to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "clipboard_read",
                "clipboard_write",
                "notify_user",
                "open_path",
            ]
        );
    }

    /// A categoria é o que a política lê para decidir se interrompe a pessoa.
    /// Ler o que ela mesma copiou é livre; sobrescrever a área de
    /// transferência é perda de dado; abrir é executar.
    #[test]
    fn categories_are_the_ones_the_policy_expects() {
        let esperado = [
            ("clipboard_read", ToolCategory::Read, ToolTier::Safe, true),
            (
                "clipboard_write",
                ToolCategory::Edit,
                ToolTier::Caution,
                false,
            ),
            ("notify_user", ToolCategory::Meta, ToolTier::Safe, true),
            ("open_path", ToolCategory::Execute, ToolTier::Caution, false),
        ];
        for tool in catalog() {
            let spec = tool.spec();
            let (_, categoria, tier, somente_leitura) = esperado
                .iter()
                .find(|(nome, ..)| *nome == spec.name)
                .copied()
                .unwrap_or_else(|| panic!("ferramenta inesperada: {}", spec.name));
            assert_eq!(spec.category, categoria, "{}", spec.name);
            assert_eq!(spec.tier, tier, "{}", spec.name);
            assert_eq!(spec.read_only, somente_leitura, "{}", spec.name);
        }
    }

    /// Descrição e schema são o que o modelo lê antes de chamar: sem eles a
    /// ferramenta é chamada errado e a pessoa é interrompida à toa.
    #[test]
    fn every_tool_describes_itself_and_its_arguments() {
        for tool in catalog() {
            let spec = tool.spec();
            assert!(!spec.description.is_empty(), "{} sem descrição", spec.name);
            assert_eq!(spec.parameters["type"], "object", "{}", spec.name);
            let props = spec.parameters["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{} sem `properties`", spec.name));
            for (key, schema) in props {
                assert!(
                    schema.get("description").and_then(|d| d.as_str()).is_some(),
                    "{}.{key} sem descrição",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn cutting_counts_characters_and_never_breaks_utf8() {
        assert_eq!(cut_chars("ação", 10), ("ação".to_string(), false));
        let (curto, cortou) = cut_chars("ação", 2);
        assert_eq!(curto, "aç");
        assert!(cortou);
        assert_eq!(cut_chars("", 3), (String::new(), false));
    }
}
