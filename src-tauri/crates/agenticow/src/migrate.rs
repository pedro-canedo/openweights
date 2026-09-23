//! Da era do DeepSeek Harness (pacote npm, `<data>/dsh-home`) para o
//! AgenticOw (`<data>/agenticow-home`), na primeira subida.
//!
//! O home antigo é COPIADO, nunca movido: fica intacto por pelo menos duas
//! versões, e um downgrade continua encontrando tudo. Vai tudo — sessões,
//! configurações (tema, idioma, permissões), credenciais que a pessoa digitou
//! na UI —, menos:
//! - `profiles/`: os profiles apontam, por links absolutos, para a árvore npm
//!   do pacote antigo; o runtime novo recria o dele;
//! - arquivos de lock (`*.lock`): de processos que não existem mais.
//!
//! As sessões gravadas pela 0.1.1 estão no formato 0; o runtime novo traz a
//! cadeia de migração `v0→v1→v2→v3` e as converte ao ler.

use std::path::Path;

/// O que a migração fez.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Migracao {
    /// O home novo já existe: nada a fazer (inclusive depois da primeira vez).
    JaMigrado,
    /// Não há home antigo.
    NadaParaMigrar,
    /// Copiado. `arquivos` conta o que foi copiado.
    Copiado { arquivos: usize },
}

/// Pastas do topo do home antigo que não vêm.
const PASTAS_QUE_NAO_VEM: [&str; 1] = ["profiles"];

pub fn migrar(origem: &Path, destino: &Path) -> std::io::Result<Migracao> {
    if destino.exists() {
        return Ok(Migracao::JaMigrado);
    }
    if !origem.is_dir() {
        return Ok(Migracao::NadaParaMigrar);
    }
    let nome = destino
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staging = destino.with_file_name(format!("{nome}.migrando"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let resultado = copiar(origem, &staging, true);
    match resultado {
        Ok(arquivos) => {
            std::fs::rename(&staging, destino)?;
            Ok(Migracao::Copiado { arquivos })
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

fn copiar(origem: &Path, destino: &Path, topo: bool) -> std::io::Result<usize> {
    std::fs::create_dir_all(destino)?;
    let mut arquivos = 0;
    for entrada in std::fs::read_dir(origem)? {
        let entrada = entrada?;
        let nome = entrada.file_name();
        let nome_str = nome.to_string_lossy();
        let tipo = entrada.file_type()?;
        // Links não vêm: no home antigo eles só apontam para a árvore npm.
        if tipo.is_symlink() {
            continue;
        }
        if tipo.is_dir() {
            if topo && PASTAS_QUE_NAO_VEM.contains(&nome_str.as_ref()) {
                continue;
            }
            arquivos += copiar(&entrada.path(), &destino.join(&nome), false)?;
        } else if tipo.is_file() {
            if nome_str.ends_with(".lock") {
                continue;
            }
            std::fs::copy(entrada.path(), destino.join(&nome))?;
            arquivos += 1;
        }
    }
    Ok(arquivos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_antigo(raiz: &Path) -> std::path::PathBuf {
        let h = raiz.join("dsh-home");
        std::fs::create_dir_all(h.join("sessions").join("2026-09")).unwrap();
        std::fs::write(
            h.join("sessions").join("2026-09").join("s1.jsonl"),
            "{\"v\":0}\n",
        )
        .unwrap();
        std::fs::write(h.join("settings.yaml"), "ui-theme:\n  preference: dark\n").unwrap();
        std::fs::write(h.join(".credentials.yaml"), "deepseek: x\n").unwrap();
        std::fs::write(h.join("settings.yaml.lock"), "4242").unwrap();
        std::fs::create_dir_all(h.join("profiles").join("web")).unwrap();
        std::fs::write(h.join("profiles").join("web").join("package.json"), "{}").unwrap();
        h
    }

    #[test]
    fn copia_tudo_menos_profiles_e_locks_e_deixa_o_original() {
        let tmp = tempfile::tempdir().unwrap();
        let origem = home_antigo(tmp.path());
        let destino = tmp.path().join("agenticow-home");
        assert_eq!(
            migrar(&origem, &destino).unwrap(),
            Migracao::Copiado { arquivos: 3 }
        );
        assert!(
            destino
                .join("sessions")
                .join("2026-09")
                .join("s1.jsonl")
                .is_file()
        );
        assert!(destino.join("settings.yaml").is_file());
        assert!(destino.join(".credentials.yaml").is_file());
        assert!(!destino.join("settings.yaml.lock").exists());
        assert!(!destino.join("profiles").exists());
        // O original fica intacto, inclusive o que não veio.
        assert!(
            origem
                .join("profiles")
                .join("web")
                .join("package.json")
                .is_file()
        );
        assert!(origem.join("settings.yaml.lock").is_file());
        assert!(!tmp.path().join("agenticow-home.migrando").exists());
    }

    #[test]
    fn so_na_primeira_vez() {
        let tmp = tempfile::tempdir().unwrap();
        let origem = home_antigo(tmp.path());
        let destino = tmp.path().join("agenticow-home");
        migrar(&origem, &destino).unwrap();
        std::fs::write(origem.join("sessions").join("nova.jsonl"), "{}").unwrap();
        assert_eq!(migrar(&origem, &destino).unwrap(), Migracao::JaMigrado);
        assert!(!destino.join("sessions").join("nova.jsonl").exists());
    }

    #[test]
    fn sem_home_antigo_nao_faz_nada() {
        let tmp = tempfile::tempdir().unwrap();
        let destino = tmp.path().join("agenticow-home");
        assert_eq!(
            migrar(&tmp.path().join("dsh-home"), &destino).unwrap(),
            Migracao::NadaParaMigrar
        );
        assert!(!destino.exists(), "o runtime cria o home dele");
    }

    #[cfg(unix)]
    #[test]
    fn links_nao_vem() {
        let tmp = tempfile::tempdir().unwrap();
        let origem = home_antigo(tmp.path());
        std::os::unix::fs::symlink("/", origem.join("link-para-fora")).unwrap();
        let destino = tmp.path().join("agenticow-home");
        migrar(&origem, &destino).unwrap();
        assert!(!destino.join("link-para-fora").exists());
    }
}
