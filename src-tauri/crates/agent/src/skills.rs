//! Skills padrão do agente local.
//!
//! A UI ainda mostra "Habilidades" como em breve: o modelo pequeno não pode
//! depender de a pessoa lembrar de anexar um guia. Os trilhos vêm **embutidos
//! no binário** e entram no prompt por fase (planejar vs executar). A pasta
//! `.openweights/skills/<nome>/SKILL.md` no projeto, se existir, substitui o
//! built-in de mesmo `name` — extra sem nome conhecido é ignorada (não inflar
//! a janela).

use std::path::Path;

/// Fase do laço que escolhe qual skill cabe no prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillPhase {
    #[default]
    None,
    Plan,
    Build,
}

/// Onde skills extra do projeto moram.
pub const SKILLS_DIR: &str = ".openweights/skills";

const BUILTIN_PLANNING: &str = include_str!("../skills/planning/SKILL.md");
const BUILTIN_IMPLEMENTATION: &str = include_str!("../skills/implementation/SKILL.md");
const BUILTIN_VERIFICATION: &str = include_str!("../skills/verification/SKILL.md");
const BUILTIN_CONTEXT: &str = include_str!("../skills/context/SKILL.md");

/// Teto da seção inteira: skill longa empurra o pedido para fora da janela.
const MAX_SECTION_CHARS: usize = 1_400;

struct Packed {
    #[allow(dead_code)]
    name: String,
    body: String,
}

/// Texto que entra no prompt de sistema nesta fase. Vazio no chat.
pub fn section(phase: SkillPhase, workspace: Option<&Path>) -> String {
    if phase == SkillPhase::None {
        return String::new();
    }
    let wanted: &[&str] = match phase {
        SkillPhase::Plan => &["planning", "context"],
        SkillPhase::Build => &["implementation", "verification", "context"],
        SkillPhase::None => return String::new(),
    };
    let mut out = String::from("\n## Trilhos desta etapa\n");
    for name in wanted {
        let body = body_for(name, workspace);
        if body.is_empty() {
            continue;
        }
        let next = format!("{body}\n");
        if out.chars().count() + next.chars().count() > MAX_SECTION_CHARS {
            break;
        }
        out.push_str(&next);
    }
    out
}

fn body_for(name: &str, workspace: Option<&Path>) -> String {
    if let Some(ws) = workspace
        && let Some(over) = read_override(ws, name)
    {
        return over;
    }
    builtin(name)
        .map(|raw| parse_skill(raw).map(|p| p.body).unwrap_or_default())
        .unwrap_or_default()
}

fn builtin(name: &str) -> Option<&'static str> {
    match name {
        "planning" => Some(BUILTIN_PLANNING),
        "implementation" => Some(BUILTIN_IMPLEMENTATION),
        "verification" => Some(BUILTIN_VERIFICATION),
        "context" => Some(BUILTIN_CONTEXT),
        _ => None,
    }
}

fn read_override(workspace: &Path, name: &str) -> Option<String> {
    let path = workspace.join(SKILLS_DIR).join(name).join("SKILL.md");
    let raw = std::fs::read_to_string(path).ok()?;
    let packed = parse_skill(&raw)?;
    Some(packed.body)
}

/// Frontmatter YAML raso (`name`, `phase`) + corpo. Sem parser YAML.
fn parse_skill(raw: &str) -> Option<Packed> {
    let text = raw.trim();
    let rest = text.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let head = &rest[..end];
    let body = rest[end + 4..].trim();
    if body.is_empty() {
        return None;
    }
    let mut name = String::new();
    for line in head.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(Packed {
        name,
        body: body.to_string(),
    })
}

impl SkillPhase {
    pub fn for_work_mode(mode: lr_types::scout::WorkMode) -> Self {
        match mode {
            lr_types::scout::WorkMode::Plan => Self::Plan,
            lr_types::scout::WorkMode::Agent | lr_types::scout::WorkMode::Loop => Self::Build,
            lr_types::scout::WorkMode::Chat => Self::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::scout::WorkMode;

    #[test]
    fn builtins_parse_and_keep_checklists() {
        for (nome, raw) in [
            ("planning", BUILTIN_PLANNING),
            ("implementation", BUILTIN_IMPLEMENTATION),
            ("verification", BUILTIN_VERIFICATION),
            ("context", BUILTIN_CONTEXT),
        ] {
            let p = parse_skill(raw).unwrap_or_else(|| panic!("{nome} inválida"));
            assert_eq!(p.name, nome);
            assert!(p.body.contains('#'), "{nome} sem título");
            assert!(p.body.lines().count() < 40, "{nome} longa demais");
        }
    }

    #[test]
    fn plan_phase_does_not_dump_implementation() {
        let s = section(SkillPhase::Plan, None);
        assert!(s.contains("Trilhos"));
        assert!(s.contains("uma entrega por fase") || s.contains("1 de 1"));
        assert!(s.contains("progress.md"));
        assert!(!s.contains("fs_write"), "planejar não pede escrita");
    }

    #[test]
    fn build_phase_requires_files_and_real_checks() {
        let s = section(SkillPhase::Build, None);
        assert!(s.contains("disco") || s.contains("fs_write"));
        assert!(s.contains("node -v"));
        assert!(s.contains("progress.md"));
    }

    #[test]
    fn chat_gets_no_skills() {
        assert!(section(SkillPhase::None, None).is_empty());
        assert_eq!(SkillPhase::for_work_mode(WorkMode::Chat), SkillPhase::None);
        assert_eq!(SkillPhase::for_work_mode(WorkMode::Plan), SkillPhase::Plan);
        assert_eq!(SkillPhase::for_work_mode(WorkMode::Loop), SkillPhase::Build);
    }

    #[test]
    fn workspace_skill_overrides_the_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join(SKILLS_DIR).join("planning");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: planning\nphase: plan\n---\n# Override\n- só isto\n",
        )
        .unwrap();
        let s = section(SkillPhase::Plan, Some(dir.path()));
        assert!(s.contains("só isto"));
        assert!(!s.contains("uma entrega por fase"));
    }

    #[test]
    fn section_stays_short_for_small_windows() {
        let s = section(SkillPhase::Build, None);
        assert!(
            s.chars().count() <= MAX_SECTION_CHARS,
            "seção grande demais: {}",
            s.chars().count()
        );
    }
}
