//! Erros dos conectores.
//!
//! Duas plateias diferentes leem estas mensagens, e por isso existem dois
//! formatos: [`McpError::to_user_message`] fala com a pessoa na tela de
//! configuração ("o comando `npx` não foi encontrado"), enquanto
//! [`McpError::to_model_message`] volta para o modelo dentro do resultado da
//! ferramenta e precisa dizer o que tentar em seguida — é assim que o agente
//! se corrige sozinho em vez de repetir a mesma chamada quebrada.

use lr_tools::ToolError;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// JSON malformado ou faltando campo obrigatório.
    #[error("configuração inválida: {0}")]
    Config(String),

    /// Não existe servidor com este id no banco.
    #[error("conector `{0}` não existe")]
    UnknownServer(String),

    /// Servidor existe mas está desligado pelo usuário.
    #[error("o conector `{0}` está desativado")]
    Disabled(String),

    /// As ferramentas mudaram (ou nunca foram revisadas) e o usuário ainda
    /// não aprovou. O gate contra "rug pull".
    #[error("as ferramentas do conector `{0}` aguardam revisão")]
    NeedsApproval(String),

    /// Falha ao subir o processo/abrir a conexão.
    #[error("não foi possível conectar em `{server}`: {reason}")]
    Connect { server: String, reason: String },

    /// Conexão de pé, mas o servidor respondeu errado (ou não respondeu).
    #[error("o conector `{server}` respondeu com erro: {reason}")]
    Protocol { server: String, reason: String },

    /// Estourou o tempo combinado.
    #[error("o conector `{server}` não respondeu em {secs}s")]
    Timeout { server: String, secs: u64 },

    /// A ferramenta não está no catálogo daquele servidor.
    #[error("o conector `{server}` não tem a ferramenta `{tool}`")]
    UnknownTool { server: String, tool: String },

    /// O servidor executou a ferramenta e ela falhou (`isError: true`).
    #[error("{0}")]
    ToolFailed(String),

    #[error("erro de banco: {0}")]
    Store(#[from] lr_store::StoreError),
}

impl McpError {
    /// Texto curto para a tela de conectores.
    pub fn to_user_message(&self) -> String {
        self.to_string()
    }

    /// Texto que volta para o modelo. Sempre diz o próximo passo possível —
    /// um erro sem saída faz o agente insistir no mesmo caminho.
    pub fn to_model_message(&self) -> String {
        match self {
            McpError::Disabled(id) => format!(
                "O conector `{id}` está desativado. Resolva a tarefa sem ele ou peça \
                 à pessoa para ativá-lo em Configurações › Conectores."
            ),
            McpError::NeedsApproval(id) => format!(
                "As ferramentas do conector `{id}` mudaram e ainda não foram revisadas. \
                 Elas ficam indisponíveis até a pessoa aprovar em Configurações › Conectores."
            ),
            McpError::Connect { server, reason } => format!(
                "O conector `{server}` não conectou ({reason}). Não insista nele: \
                 use outra ferramenta ou explique a limitação na resposta."
            ),
            McpError::Timeout { server, secs } => format!(
                "O conector `{server}` passou de {secs}s sem responder. \
                 Tente uma chamada mais simples ou siga sem ele."
            ),
            McpError::UnknownTool { server, tool } => format!(
                "`{tool}` não existe no conector `{server}`. Use apenas as ferramentas listadas."
            ),
            McpError::ToolFailed(msg) => format!(
                "A ferramenta do conector falhou: {msg}. Revise os argumentos antes de repetir."
            ),
            other => other.to_string(),
        }
    }
}

impl From<McpError> for ToolError {
    fn from(e: McpError) -> Self {
        // O texto já vem pronto para o modelo; `Other` preserva-o intacto
        // porque `ToolError::to_model_message` devolve `Other` sem enfeite.
        ToolError::Other(e.to_model_message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_messages_suggest_a_next_step() {
        let msg = McpError::NeedsApproval("gh".into()).to_model_message();
        assert!(msg.contains("gh"));
        assert!(msg.contains("aprovar"), "deve dizer como destravar: {msg}");

        let msg = McpError::UnknownTool {
            server: "gh".into(),
            tool: "voar".into(),
        }
        .to_model_message();
        assert!(msg.contains("voar") && msg.contains("listadas"));
    }

    #[test]
    fn converts_into_tool_error_without_losing_the_text() {
        let err: ToolError = McpError::Disabled("gh".into()).into();
        assert!(err.to_model_message().contains("desativado"));
    }
}
