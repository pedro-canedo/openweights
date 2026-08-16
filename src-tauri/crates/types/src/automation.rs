//! Automações: um pedido que roda sozinho, de tempos em tempos.
//!
//! A pessoa escreve uma vez ("toda sexta, resuma o que mudou no projeto") e o
//! app cuida do resto — inclusive de subir o motor, porque uma automação que
//! só funciona com o servidor já ligado não automatiza nada.
//!
//! O agendamento é deliberadamente simples: um intervalo em minutos e a hora
//! da próxima vez, em epoch. Quem calcula a primeira ocorrência é a interface,
//! que conhece o fuso e o relógio da pessoa; o Rust só soma o intervalo. Isso
//! evita matemática de fuso horário no backend ao custo de uma hora de desvio
//! quando o horário de verão muda — barato, e visível na tela.

use crate::agent::{RunMode, RunStatus};
use crate::scout::WorkMode;
use serde::{Deserialize, Serialize};

/// Uma tarefa agendada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    /// Nome curto, para a pessoa reconhecer na lista.
    pub name: String,
    /// O pedido, igual ao que ela escreveria no chat.
    pub prompt: String,
    pub workspace_dir: Option<String>,
    /// Modelo a usar. Vazio = o que estiver carregado no servidor.
    pub model: Option<String>,
    /// Nível de autorização. Ninguém está olhando quando isto roda: em
    /// `approve` a execução para no primeiro pedido de confirmação e fica
    /// esperando a pessoa — é isso que a tela precisa deixar claro.
    pub mode: RunMode,
    pub work_mode: WorkMode,
    /// De quanto em quanto tempo, em minutos. `0` = só quando ela mandar.
    pub every_minutes: u32,
    /// Quando roda de novo (epoch ms). `None` = não tem próxima.
    pub next_run_at: Option<i64>,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub last_run_id: Option<String>,
    pub last_status: Option<RunStatus>,
    /// Como terminou, em uma linha — ou o motivo de nem ter começado.
    pub last_summary: Option<String>,
    pub created_at: i64,
}

/// O que a interface manda ao criar ou editar (o resto é do backend).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskInput {
    /// Ausente = criar.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: RunMode,
    #[serde(default)]
    pub work_mode: WorkMode,
    #[serde(default)]
    pub every_minutes: u32,
    /// Primeira vez (epoch ms), calculada pela interface no fuso da pessoa.
    #[serde(default)]
    pub next_run_at: Option<i64>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl ScheduledTaskInput {
    /// Problemas que impedem a tarefa de existir, em linguagem de gente.
    pub fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("dê um nome à automação".into());
        }
        if self.prompt.trim().is_empty() {
            return Some("escreva o que a automação deve fazer".into());
        }
        // Um intervalo de poucos minutos com um modelo local vira fila de
        // execuções que nunca alcança o relógio.
        if self.every_minutes > 0 && self.every_minutes < 5 {
            return Some("o intervalo mínimo é de 5 minutos".into());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_automation_needs_a_name_and_something_to_do() {
        let base = ScheduledTaskInput {
            id: None,
            name: "Resumo semanal".into(),
            prompt: "resuma o que mudou".into(),
            workspace_dir: None,
            model: None,
            mode: RunMode::Approve,
            work_mode: WorkMode::Agent,
            every_minutes: 60,
            next_run_at: Some(1),
            enabled: true,
        };
        assert!(base.problem().is_none());

        let sem_nome = ScheduledTaskInput {
            name: "  ".into(),
            ..base.clone()
        };
        assert!(sem_nome.problem().unwrap().contains("nome"));

        let sem_pedido = ScheduledTaskInput {
            prompt: String::new(),
            ..base.clone()
        };
        assert!(sem_pedido.problem().unwrap().contains("fazer"));

        let apressada = ScheduledTaskInput {
            every_minutes: 2,
            ..base.clone()
        };
        assert!(apressada.problem().unwrap().contains("5 minutos"));

        // Sem intervalo é válido: roda só quando a pessoa mandar.
        let manual = ScheduledTaskInput {
            every_minutes: 0,
            ..base
        };
        assert!(manual.problem().is_none());
    }
}
