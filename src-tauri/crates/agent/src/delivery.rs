//! Conferência de entrega: o pedido foi atendido, ou o run só parou de agir?
//!
//! O laço tratava "o modelo respondeu texto" como "a tarefa acabou" — e é
//! assim que um pedido de dezenas de requisitos fechava como **Concluído** em
//! quatro passos, tendo apenas listado a pasta. A frase final não serve de
//! prova: `anuncio_sem_acao` procura promessa ("vou criar…") e não pega a
//! afirmação falsa ("pronto, criei os três arquivos"), que é justamente o que
//! modelo pequeno mais escreve.
//!
//! O que não mente é o disco. Este módulo pega o plano do run — as entregas em
//! que o pedido foi dividido, cada uma com os arquivos que ela mesma disse que
//! produziria — e confere **em disco** quais existem. Sem modelo, sem rede,
//! sem interpretar texto: mesma entrada, mesma resposta.
//!
//! O que sobra vira cobrança: a lista das entregas que faltam volta para a
//! conversa e o laço continua de onde parou, até entregar ou até o orçamento
//! de passos acabar. Nenhuma verificação nova de segurança acontece aqui — a
//! cobrança é uma mensagem de usuário como outra qualquer, e tudo que o modelo
//! fizer depois dela passa pela mesma política e pelas mesmas confirmações.

use std::path::Path;

use lr_types::scout::{TaskPlan, TaskStatus};

/// Uma entrega que o plano prometeu e o disco não confirma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pendencia {
    pub titulo: String,
    /// Instrução original da etapa — é ela que diz ao modelo o que fazer.
    pub instrucao: String,
    /// Arquivos que a etapa declarou e não existem.
    pub faltando: Vec<String>,
}

/// Quantas pendências entram na cobrança. Listar doze de uma vez enche a
/// janela do modelo pequeno com trabalho que ele não vai fazer neste passo;
/// três é o bastante para ele voltar a agir.
const MAX_NA_COBRANCA: usize = 3;

/// Entregas que o plano prometeu e o disco não confirma.
///
/// Uma etapa é cobrada quando declara arquivos e algum deles não está em
/// disco. Etapa sem arquivo declarado, ou run sem pasta de projeto, não são
/// cobrados: cobrar sem poder verificar seria chutar, e o preço do chute é
/// mandar refazer trabalho pronto.
pub fn pendencias(plan: &TaskPlan, workspace: Option<&Path>) -> Vec<Pendencia> {
    let Some(root) = workspace else {
        return Vec::new();
    };
    plan.tasks
        .iter()
        .filter(|t| !matches!(t.status, TaskStatus::Skipped))
        .filter_map(|t| {
            let faltando: Vec<String> = t
                .files
                .iter()
                .filter(|rel| !root.join(rel).exists())
                .cloned()
                .collect();
            // Etapa que não declara arquivo (rodar a suíte, revisar algo) não
            // tem evidência em disco. A tentação é usar a marcação do modelo
            // como prova — mas no modo agente ele nem recebe a ferramenta de
            // marcar, então TODA etapa dessas ficaria pendente para sempre e
            // o run inteiro terminaria escalando. Cobrar sem poder verificar
            // é chutar; aqui o silêncio é a resposta certa.
            if t.files.is_empty() {
                return None;
            }
            (!faltando.is_empty()).then(|| Pendencia {
                titulo: t.title.clone(),
                instrucao: t.instruction.clone(),
                faltando,
            })
        })
        .collect()
}

/// A cobrança que volta para a conversa.
///
/// Escrita como instrução de trabalho, não como reclamação: o modelo pequeno
/// responde melhor a "faça isto agora" do que a "você falhou". Cita arquivo
/// por nome porque é o que ele precisa criar — e é o que a próxima conferência
/// vai procurar.
pub fn cobranca(pendentes: &[Pendencia]) -> String {
    let mut texto = String::from(
        "A tarefa NÃO está concluída. Confiro os arquivos em disco, e estas entregas do \
         plano ainda não existem:\n",
    );
    for p in pendentes.iter().take(MAX_NA_COBRANCA) {
        texto.push_str(&format!("\n- {}", p.titulo));
        if !p.faltando.is_empty() {
            texto.push_str(&format!(" — falta criar: {}", p.faltando.join(", ")));
        }
        if !p.instrucao.trim().is_empty() {
            texto.push_str(&format!("\n  {}", p.instrucao.trim()));
        }
    }
    let restantes = pendentes.len().saturating_sub(MAX_NA_COBRANCA);
    if restantes > 0 {
        texto.push_str(&format!("\n\n(e mais {restantes} depois destas)"));
    }
    texto.push_str(
        "\n\nExecute AGORA a primeira delas, criando os arquivos com as ferramentas. \
         Não descreva o que faria: faça. Se algo impedir de verdade, diga em uma frase o que é.",
    );
    texto
}

/// Resumo honesto para quem lê o chat quando o orçamento acabou antes das
/// entregas. Nada de "Concluído" — o que ficou de fora fica escrito.
pub fn resumo_do_que_faltou(pendentes: &[Pendencia]) -> String {
    let lista = pendentes
        .iter()
        .take(MAX_NA_COBRANCA)
        .map(|p| format!("• {}", p.titulo))
        .collect::<Vec<_>>()
        .join("\n");
    let restantes = pendentes.len().saturating_sub(MAX_NA_COBRANCA);
    let cauda = if restantes > 0 {
        format!("\n• (e mais {restantes})")
    } else {
        String::new()
    };
    format!("Parei antes de terminar. Ficou faltando:\n{lista}{cauda}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::scout::Task;

    fn tarefa(title: &str, files: &[&str], status: TaskStatus) -> Task {
        Task {
            id: title.into(),
            title: title.into(),
            instruction: format!("faça {title}"),
            done_when: "existe".into(),
            status,
            handoff: None,
            files: files.iter().map(|s| s.to_string()).collect(),
            depends_on: Vec::new(),
            est_tokens: 0,
            error: None,
        }
    }

    fn plano(tasks: Vec<Task>) -> TaskPlan {
        TaskPlan {
            goal: "objetivo".into(),
            tasks,
            ..Default::default()
        }
    }

    #[test]
    fn a_task_whose_files_are_on_disk_is_not_charged_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pronto.js"), "x").unwrap();
        let plan = plano(vec![tarefa("feita", &["pronto.js"], TaskStatus::Pending)]);

        // O status ainda é "pendente" — o modelo esqueceu de marcar —, mas o
        // arquivo existe. Cobrar de novo mandaria refazer trabalho pronto.
        assert!(pendencias(&plan, Some(dir.path())).is_empty());
    }

    #[test]
    fn a_task_marked_done_without_its_file_is_still_pending() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plano(vec![tarefa(
            "mentira",
            &["nunca_criado.js"],
            TaskStatus::Done,
        )]);

        // É o caso que motivou o módulo: o run afirma ter feito e o disco
        // discorda. Quem manda é o disco.
        let p = pendencias(&plan, Some(dir.path()));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].faltando, vec!["nunca_criado.js".to_string()]);
    }

    #[test]
    fn a_task_without_declared_files_is_never_charged() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plano(vec![
            tarefa("rodar os testes", &[], TaskStatus::Done),
            tarefa("rodar o lint", &[], TaskStatus::Pending),
        ]);

        // Nenhuma das duas produz arquivo, então não há como conferir. Se
        // isto voltasse a devolver pendência, todo run do modo agente
        // terminaria escalando — o plano de fallback tem uma entrega só e
        // ela não declara arquivo nenhum.
        assert!(pendencias(&plan, Some(dir.path())).is_empty());
    }

    #[test]
    fn a_skipped_task_is_not_a_pending_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plano(vec![tarefa("pulada", &["x.js"], TaskStatus::Skipped)]);
        assert!(pendencias(&plan, Some(dir.path())).is_empty());
    }

    #[test]
    fn without_a_project_folder_there_is_nothing_to_check() {
        let plan = plano(vec![tarefa("qualquer", &["x.js"], TaskStatus::Pending)]);
        // Cobrar sem poder conferir seria chutar.
        assert!(pendencias(&plan, None).is_empty());
    }

    #[test]
    fn the_charge_names_the_missing_files_and_demands_action() {
        let p = vec![Pendencia {
            titulo: "HUD".into(),
            instrucao: "criar a HUD".into(),
            faltando: vec!["src/ui/hud.js".into()],
        }];
        let texto = cobranca(&p);
        assert!(texto.contains("src/ui/hud.js"), "{texto}");
        assert!(texto.contains("NÃO está concluída"), "{texto}");
        assert!(texto.contains("faça"), "{texto}");
    }

    #[test]
    fn the_charge_caps_the_list_and_says_how_many_are_left() {
        let p: Vec<Pendencia> = (0..7)
            .map(|i| Pendencia {
                titulo: format!("entrega {i}"),
                instrucao: String::new(),
                faltando: vec![format!("f{i}.js")],
            })
            .collect();
        let texto = cobranca(&p);
        assert!(texto.contains("entrega 0"), "{texto}");
        assert!(!texto.contains("entrega 5"), "{texto}");
        assert!(texto.contains("e mais 4"), "{texto}");
    }
}
