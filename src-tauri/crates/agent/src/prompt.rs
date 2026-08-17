//! Montagem do prompt de sistema do agente.
//!
//! O alvo são modelos de 7B a 30B rodando na máquina da pessoa: prompt longo
//! é veneno — eles perdem o pedido original no meio das instruções. Por isso
//! aqui cabem poucas centenas de tokens, em seções curtas e imperativas, e
//! nada é repetido em dois lugares.

use lr_types::agent::RunMode;

/// Tudo que o prompt precisa saber sobre a execução.
#[derive(Debug, Default, Clone)]
pub struct PromptContext<'a> {
    /// Pasta do projeto (raiz de todos os caminhos relativos).
    pub workspace: Option<&'a str>,
    /// Plano em markdown (Focus Chain), quando existe.
    pub focus_md: Option<&'a str>,
    /// Fatos da memória de longo prazo (H3 preenche; aqui só entram).
    pub memory: &'a [String],
    /// Nomes das ferramentas disponíveis nesta execução.
    pub tools: &'a [String],
    /// O cardápio é um recorte: existem outras ferramentas no catálogo que
    /// não couberam na janela deste modelo. Muda o texto para o modelo saber
    /// que pode pedir mais em vez de desistir do caminho certo.
    pub tools_partial: bool,
    /// Instruções que a pessoa configurou para a conversa.
    pub user_system: Option<&'a str>,
    pub mode: RunMode,
}

/// Quantos fatos de memória entram no prompt (o resto vira ruído).
const MAX_FACTS: usize = 12;

/// Monta o prompt de sistema em seções.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut out = String::with_capacity(1024);

    out.push_str(
        "Você é o agente do OpenWeights e trabalha no computador da pessoa.\n\
         Objetivo: resolver o pedido de verdade, com o menor número de passos, \
         e explicar em poucas linhas o que fez. Responda no mesmo idioma da pessoa.\n",
    );

    out.push_str("\n## Projeto\n");
    match ctx.workspace {
        Some(dir) if !dir.is_empty() => {
            out.push_str(&format!(
                "Pasta de trabalho: {dir}\nUse sempre caminhos relativos a ela.\n"
            ));
        }
        _ => out.push_str(
            "Nenhuma pasta foi escolhida: você não pode ler nem alterar arquivos. \
             Se o pedido exigir isso, diga que falta escolher a pasta do projeto.\n",
        ),
    }

    if let Some(plan) = ctx.focus_md.map(str::trim).filter(|p| !p.is_empty()) {
        out.push_str("\n## Plano atual\n");
        out.push_str(plan);
        out.push_str("\nMantenha o plano atualizado conforme concluir os itens.\n");
    }

    let facts: Vec<&String> = ctx
        .memory
        .iter()
        .filter(|f| !f.trim().is_empty())
        .take(MAX_FACTS)
        .collect();
    if !facts.is_empty() {
        out.push_str("\n## O que já sabemos deste projeto\n");
        for fact in facts {
            out.push_str(&format!("- {}\n", fact.trim()));
        }
    }

    out.push_str("\n## Ferramentas\n");
    if ctx.tools.is_empty() || ctx.mode == RunMode::Chat {
        out.push_str(
            "Nenhuma ferramenta está disponível nesta execução. Responda em texto, \
             com o que você já sabe, e diga o que precisaria para ir além.\n",
        );
    } else {
        out.push_str(&format!("Disponíveis: {}.\n", ctx.tools.join(", ")));
        out.push_str(
            "- Um passo por vez: chame uma ferramenta, leia o resultado e só então decida a próxima.\n\
             - Leia o arquivo antes de editar; nunca reescreva no escuro.\n\
             - Não repita uma chamada que já deu o mesmo resultado.\n\
             - Se a ferramenta falhar, leia a mensagem de erro, corrija os argumentos e tente outro caminho.\n\
             - Se a pessoa recusar uma ação, não insista: proponha outra abordagem.\n\
             - Não anuncie o que vai fazer: faça. Texto sem chamada de ferramenta ENCERRA \
             a execução, então só responda em texto quando a tarefa estiver pronta.\n\
             - Não rode servidor nem processo que não termina sozinho (`python -m http.server`, \
             `npm run dev`): o passo fica preso até estourar o tempo.\n",
        );
        if ctx.tools_partial {
            out.push_str(
                "Esta lista é um recorte do que existe. Se faltar uma ferramenta para o que                  você precisa fazer, chame `tools_find` descrevendo a tarefa e ela aparece.\n",
            );
        }
        if ctx.mode == RunMode::Approve {
            out.push_str(
                "Cada ação passa pela confirmação da pessoa: prefira poucas ações grandes a muitas pequenas.\n",
            );
        }
    }

    if let Some(extra) = ctx.user_system.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str("\n## Instruções da pessoa\n");
        out.push_str(extra);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<String> {
        vec!["fs_read".into(), "fs_write".into(), "terminal_run".into()]
    }

    #[test]
    fn prompt_has_the_expected_sections_and_stays_short() {
        let tools = tools();
        let memory = vec!["Usa pnpm, não npm.".to_string()];
        let p = build_system_prompt(&PromptContext {
            workspace: Some("/home/u/projeto"),
            focus_md: Some("- [ ] criar notas.md"),
            memory: &memory,
            tools: &tools,
            user_system: Some("Seja direto."),
            mode: RunMode::Smart,
            tools_partial: false,
        });

        for section in [
            "## Projeto",
            "## Plano atual",
            "## O que já sabemos",
            "## Ferramentas",
            "## Instruções da pessoa",
        ] {
            assert!(p.contains(section), "faltou {section}");
        }
        assert!(p.contains("/home/u/projeto"));
        assert!(p.contains("fs_read, fs_write, terminal_run"));
        assert!(p.contains("criar notas.md"));
        assert!(p.contains("Usa pnpm"));
        assert!(p.contains("Seja direto."));
        // A regra que define quando o run termina — e a contrapartida dela,
        // que impede o modelo de encerrar anunciando.
        assert!(p.contains("ENCERRA"));
        assert!(p.contains("Não anuncie o que vai fazer"));
        // Orçamento: algumas centenas de tokens (~4 caracteres por token).
        assert!(p.len() < 2600, "prompt longo demais: {} bytes", p.len());
    }

    /// Cardápio recortado: o modelo precisa saber que pode pedir mais, senão
    /// ele desiste do caminho certo por achar que a ferramenta não existe.
    #[test]
    fn a_partial_menu_tells_the_model_how_to_ask_for_more() {
        let tools = tools();
        let cheio = build_system_prompt(&PromptContext {
            workspace: Some("/ws"),
            tools: &tools,
            ..Default::default()
        });
        assert!(!cheio.contains("tools_find"));

        let recorte = build_system_prompt(&PromptContext {
            workspace: Some("/ws"),
            tools: &tools,
            tools_partial: true,
            ..Default::default()
        });
        assert!(recorte.contains("tools_find"));
    }

    #[test]
    fn without_workspace_the_model_is_told_it_cannot_touch_files() {
        let p = build_system_prompt(&PromptContext {
            tools: &[],
            ..Default::default()
        });
        assert!(p.contains("Nenhuma pasta foi escolhida"));
        assert!(p.contains("Nenhuma ferramenta está disponível"));
        assert!(!p.contains("## Plano atual"));
        assert!(!p.contains("## O que já sabemos"));
    }

    #[test]
    fn chat_mode_never_advertises_tools() {
        let tools = tools();
        let p = build_system_prompt(&PromptContext {
            workspace: Some("/ws"),
            tools: &tools,
            mode: RunMode::Chat,
            ..Default::default()
        });
        assert!(p.contains("Nenhuma ferramenta está disponível"));
        assert!(!p.contains("fs_write"));
    }

    #[test]
    fn memory_is_capped_and_ignores_blank_facts() {
        let facts: Vec<String> = (0..20).map(|i| format!("fato {i}")).collect();
        let mut facts = facts;
        facts.push("   ".into());
        let tools = tools();
        let p = build_system_prompt(&PromptContext {
            workspace: Some("/ws"),
            memory: &facts,
            tools: &tools,
            ..Default::default()
        });
        assert!(p.contains("- fato 0"));
        assert!(p.contains("- fato 11"));
        assert!(!p.contains("- fato 12"), "corta em {MAX_FACTS} fatos");
        assert!(!p.contains("-  \n"));
    }
}
