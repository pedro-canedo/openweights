//! Decide, para cada chamada de ferramenta, se ela roda direto, pede
//! confirmação ou é recusada.
//!
//! Princípios (nesta ordem de precedência):
//! 1. Um `never` do usuário sempre vence.
//! 2. Fora da pasta do projeto, sempre pergunta.
//! 3. Comando que não conseguimos analisar (`Opaque`), sempre pergunta.
//! 4. Rede sempre pergunta (a não ser que o usuário tenha marcado
//!    "sempre permitir" naquela ferramenta).
//! 5. Aí sim entram o modo do run e os atalhos de leitura.
//!
//! O objetivo é proteger a pessoa de um erro do modelo — apagar a pasta
//! errada, rodar um script que ninguém leu — mantendo o fluxo agradável
//! quando a ação é claramente inofensiva.

use lr_types::agent::{
    ApprovalSource, CommandClass, PolicyScope, RunMode, ToolCategory, ToolPolicy,
};

pub mod command;
pub use command::{CommandAnalysis, classify};

/// O que fazer com uma chamada de ferramenta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Pode executar; `source` explica por quê (para a auditoria).
    Allow { source: ApprovalSource },
    /// Precisa de confirmação; `reason` é exibido na UI.
    Ask { reason: String },
    /// Recusada pela configuração do usuário.
    Deny { reason: String },
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow { .. })
    }
}

/// Tudo que a política precisa saber sobre uma chamada.
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub tool_name: String,
    pub category: ToolCategory,
    /// `true` quando todos os caminhos tocados estão dentro do workspace.
    /// Ferramentas que não tocam arquivos devem informar `true`.
    pub within_workspace: bool,
    /// Presente apenas para `terminal_run`.
    pub command_class: Option<CommandClass>,
    /// Esta chamada é mais sensível do que o uso normal da ferramenta.
    /// `sql_query` consulta por padrão, mas altera o banco quando recebe
    /// `allow_write` — quem marcou "sempre permitir" vendo consultas não
    /// concordou com isso.
    pub escalated: bool,
}

impl ToolRequest {
    pub fn new(tool_name: impl Into<String>, category: ToolCategory) -> Self {
        Self {
            tool_name: tool_name.into(),
            category,
            within_workspace: true,
            command_class: None,
            escalated: false,
        }
    }

    pub fn outside_workspace(mut self) -> Self {
        self.within_workspace = false;
        self
    }

    pub fn with_command(mut self, class: CommandClass) -> Self {
        self.command_class = Some(class);
        self
    }

    pub fn escalated(mut self) -> Self {
        self.escalated = true;
        self
    }
}

/// Override configurado pelo usuário para uma ferramenta.
#[derive(Debug, Clone)]
pub struct PermissionOverride {
    pub tool_name: String,
    pub policy: ToolPolicy,
    pub scope: PolicyScope,
}

/// Avalia chamadas contra o modo do run e os overrides do usuário.
#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    overrides: Vec<PermissionOverride>,
}

impl PolicyEngine {
    pub fn new(overrides: Vec<PermissionOverride>) -> Self {
        Self { overrides }
    }

    /// Override aplicável (workspace tem precedência sobre global).
    fn override_for(&self, tool: &str) -> Option<&PermissionOverride> {
        let ws = self
            .overrides
            .iter()
            .find(|o| o.tool_name == tool && o.scope == PolicyScope::Workspace);
        ws.or_else(|| {
            self.overrides
                .iter()
                .find(|o| o.tool_name == tool && o.scope == PolicyScope::Global)
        })
    }

    pub fn decide(&self, req: &ToolRequest, mode: RunMode) -> Decision {
        // (1) Recusa explícita vence tudo.
        let ovr = self.override_for(&req.tool_name).map(|o| o.policy);
        if ovr == Some(ToolPolicy::Never) {
            return Decision::Deny {
                reason: format!("`{}` está marcada como nunca permitir.", req.tool_name),
            };
        }

        // Modo conversa não executa nada.
        if mode == RunMode::Chat {
            return Decision::Deny {
                reason: "O modo agente está desligado nesta conversa.".into(),
            };
        }

        // (2) Comando que não conseguimos analisar: sempre confirma, mesmo
        // com "sempre permitir" ou modo automático. É o caso em que a
        // classificação não significa nada.
        if req.command_class == Some(CommandClass::Opaque) {
            return Decision::Ask {
                reason: "Não foi possível analisar este comando por completo.".into(),
            };
        }

        // (3) Fora do projeto: confirma sempre (inclusive leitura — pode ser
        // um arquivo pessoal fora da pasta de trabalho).
        if !req.within_workspace {
            return Decision::Ask {
                reason: "Esta ação sai da pasta do projeto.".into(),
            };
        }

        // (4) "Sempre permitir" do usuário, uma vez passados os itens acima.
        //
        // Uma chamada fora do feitio da ferramenta descarta esse consentimento
        // e cai na matriz: quem liberou `sql_query` vendo consultas não
        // liberou apagar a tabela. Na matriz o modo automático ainda passa —
        // lá a foto do checkpoint é garantida, e foi isso que a pessoa
        // escolheu.
        if ovr == Some(ToolPolicy::AlwaysAllow) && !req.escalated {
            return Decision::Allow {
                source: ApprovalSource::Policy,
            };
        }
        if ovr == Some(ToolPolicy::Ask) {
            return Decision::Ask {
                reason: format!("`{}` está configurada para perguntar.", req.tool_name),
            };
        }

        // (5) Rede pede confirmação em qualquer modo (só o override acima
        // libera) — é o caminho por onde dados sairiam da máquina.
        if req.category == ToolCategory::Network {
            return Decision::Ask {
                reason: "Esta ação acessa a rede.".into(),
            };
        }

        match (req.category, mode) {
            // Coisas internas do app não têm efeito externo.
            (ToolCategory::Meta, _) => Decision::Allow {
                source: ApprovalSource::Policy,
            },
            // Leitura dentro do projeto é liberada em todos os modos ativos.
            (ToolCategory::Read, _) => Decision::Allow {
                source: ApprovalSource::Policy,
            },
            // Alterar/executar: só o modo automático dispensa confirmação.
            (ToolCategory::Edit | ToolCategory::Execute | ToolCategory::Mcp, RunMode::Yolo) => {
                Decision::Allow {
                    source: ApprovalSource::Yolo,
                }
            }
            (ToolCategory::Edit, _) => Decision::Ask {
                reason: "Esta ação altera arquivos do projeto.".into(),
            },
            (ToolCategory::Execute, _) => Decision::Ask {
                reason: "Esta ação executa um comando.".into(),
            },
            (ToolCategory::Mcp, _) => Decision::Ask {
                reason: "Esta ação usa um conector externo.".into(),
            },
            (ToolCategory::Network, _) => unreachable!("tratado acima"),
        }
    }

    /// Modo automático precisa de checkpoint antes da primeira alteração.
    pub fn needs_checkpoint(req: &ToolRequest) -> bool {
        matches!(req.category, ToolCategory::Edit | ToolCategory::Execute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> PolicyEngine {
        PolicyEngine::default()
    }

    fn read() -> ToolRequest {
        ToolRequest::new("fs_read", ToolCategory::Read)
    }
    fn edit() -> ToolRequest {
        ToolRequest::new("fs_write", ToolCategory::Edit)
    }
    fn exec(class: CommandClass) -> ToolRequest {
        ToolRequest::new("terminal_run", ToolCategory::Execute).with_command(class)
    }

    #[test]
    fn reading_inside_workspace_never_interrupts() {
        for mode in [RunMode::Approve, RunMode::Smart, RunMode::Yolo] {
            assert!(engine().decide(&read(), mode).is_allow(), "{mode:?}");
        }
    }

    #[test]
    fn editing_asks_unless_automatic() {
        assert!(matches!(
            engine().decide(&edit(), RunMode::Approve),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            engine().decide(&edit(), RunMode::Smart),
            Decision::Ask { .. }
        ));
        assert_eq!(
            engine().decide(&edit(), RunMode::Yolo),
            Decision::Allow {
                source: ApprovalSource::Yolo
            }
        );
    }

    #[test]
    fn opaque_command_always_asks_even_in_automatic_mode() {
        assert!(matches!(
            engine().decide(&exec(CommandClass::Opaque), RunMode::Yolo),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn opaque_command_asks_even_with_always_allow() {
        let e = PolicyEngine::new(vec![PermissionOverride {
            tool_name: "terminal_run".into(),
            policy: ToolPolicy::AlwaysAllow,
            scope: PolicyScope::Global,
        }]);
        assert!(matches!(
            e.decide(&exec(CommandClass::Opaque), RunMode::Yolo),
            Decision::Ask { .. }
        ));
        // Mas um comando analisável passa com o override.
        assert!(
            e.decide(&exec(CommandClass::Mutating), RunMode::Smart)
                .is_allow()
        );
    }

    /// "Sempre permitir" foi dado vendo consultas; a mesma ferramenta pedindo
    /// para ALTERAR o banco é outra conversa e volta a perguntar.
    #[test]
    fn an_escalated_call_ignores_always_allow() {
        let e = PolicyEngine::new(vec![PermissionOverride {
            tool_name: "sql_query".into(),
            policy: ToolPolicy::AlwaysAllow,
            scope: PolicyScope::Global,
        }]);
        let consulta = ToolRequest::new("sql_query", ToolCategory::Read);
        let escrita = ToolRequest::new("sql_query", ToolCategory::Edit).escalated();

        assert!(e.decide(&consulta, RunMode::Smart).is_allow());
        assert!(matches!(
            e.decide(&escrita, RunMode::Smart),
            Decision::Ask { .. }
        ));
        // No modo automático segue liberado — lá a foto do checkpoint é
        // garantida, e foi isso que a pessoa escolheu.
        assert!(e.decide(&escrita, RunMode::Yolo).is_allow());
        assert!(PolicyEngine::needs_checkpoint(&escrita));
    }

    #[test]
    fn outside_workspace_always_asks() {
        assert!(matches!(
            engine().decide(&read().outside_workspace(), RunMode::Yolo),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            engine().decide(&edit().outside_workspace(), RunMode::Yolo),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn never_override_wins_over_everything() {
        let e = PolicyEngine::new(vec![PermissionOverride {
            tool_name: "fs_read".into(),
            policy: ToolPolicy::Never,
            scope: PolicyScope::Global,
        }]);
        assert!(matches!(
            e.decide(&read(), RunMode::Yolo),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn workspace_override_beats_global() {
        let e = PolicyEngine::new(vec![
            PermissionOverride {
                tool_name: "fs_write".into(),
                policy: ToolPolicy::AlwaysAllow,
                scope: PolicyScope::Global,
            },
            PermissionOverride {
                tool_name: "fs_write".into(),
                policy: ToolPolicy::Ask,
                scope: PolicyScope::Workspace,
            },
        ]);
        assert!(matches!(
            e.decide(&edit(), RunMode::Smart),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn network_asks_by_default_in_every_mode() {
        let net = ToolRequest::new("web_fetch", ToolCategory::Network);
        for mode in [RunMode::Approve, RunMode::Smart, RunMode::Yolo] {
            assert!(
                matches!(engine().decide(&net, mode), Decision::Ask { .. }),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn chat_mode_blocks_all_tools() {
        assert!(matches!(
            engine().decide(&read(), RunMode::Chat),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn meta_tools_are_free() {
        let todo = ToolRequest::new("todo_update", ToolCategory::Meta);
        assert!(engine().decide(&todo, RunMode::Approve).is_allow());
    }

    #[test]
    fn checkpoint_needed_only_for_side_effects() {
        assert!(PolicyEngine::needs_checkpoint(&edit()));
        assert!(PolicyEngine::needs_checkpoint(&exec(
            CommandClass::Mutating
        )));
        assert!(!PolicyEngine::needs_checkpoint(&read()));
    }
}
