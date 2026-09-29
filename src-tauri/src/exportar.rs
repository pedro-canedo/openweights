//! Salvar um texto que a pessoa vai levar para fora do app (os logs, a
//! configuração). Sem seletor de arquivo: a interface entrega o nome e o
//! conteúdo, o texto vai para `<dados>/exports/` e o app mostra a pasta. É o
//! mesmo resultado em qualquer sistema, sem depender de portal ou de diálogo
//! (que no Linux é um caminho à parte).

use std::path::{Path, PathBuf};

use tauri::State;

use crate::state::AppState;

const PASTA: &str = "exports";
/// O que a interface pode gravar por aqui: um texto, não um arquivo qualquer.
const MAX_BYTES: usize = 32 * 1024 * 1024;

/// O nome vira só um nome: sem pasta, sem `..`, sem caractere que o Windows
/// recusa. Vazio (ou só pontos) vira `export.txt`.
pub fn nome_seguro(nome: &str) -> String {
    let base = nome.rsplit(['/', '\\']).next().unwrap_or("");
    let limpo: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let limpo = limpo
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' ']);
    if limpo.is_empty() {
        return "export.txt".to_string();
    }
    let nome: String = limpo.chars().take(120).collect();
    // Nomes de dispositivo do Windows (`CON`, `NUL`, `COM1`…): abrir um deles
    // "dá certo" e o texto se perde.
    let antes_do_ponto = nome.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reservado = matches!(antes_do_ponto.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (antes_do_ponto.len() == 4
            && (antes_do_ponto.starts_with("COM") || antes_do_ponto.starts_with("LPT"))
            && antes_do_ponto.as_bytes()[3].is_ascii_digit());
    if reservado { format!("_{nome}") } else { nome }
}

pub fn pasta(data_dir: &Path) -> PathBuf {
    data_dir.join(PASTA)
}

/// Grava e devolve o caminho. Um nome que já existe ganha ` (2)`, ` (3)`…:
/// nunca sobrescreve o que a pessoa guardou.
pub fn salvar(data_dir: &Path, nome: &str, conteudo: &str) -> Result<PathBuf, String> {
    if conteudo.len() > MAX_BYTES {
        return Err("o texto passa de 32 MB".into());
    }
    let dir = pasta(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let nome = nome_seguro(nome);
    let (raiz, ext) = match nome.rsplit_once('.') {
        Some((r, e)) if !r.is_empty() => (r.to_string(), format!(".{e}")),
        _ => (nome.clone(), String::new()),
    };
    for n in 1..1000 {
        let candidato = if n == 1 {
            format!("{raiz}{ext}")
        } else {
            format!("{raiz} ({n}){ext}")
        };
        let destino = dir.join(&candidato);
        // `create_new`: se outro salvar pegou o nome entre a checagem e a
        // escrita, tenta o próximo em vez de sobrescrever.
        let mut abrir = std::fs::OpenOptions::new();
        abrir.write(true).create_new(true);
        // Só o dono lê: um backup com chaves não pode ficar aberto aos outros
        // usuários da máquina.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            abrir.mode(0o600);
        }
        match abrir.open(&destino) {
            Ok(mut f) => {
                use std::io::Write as _;
                f.write_all(conteudo.as_bytes())
                    .map_err(|e| format!("{}: {e}", destino.display()))?;
                return Ok(destino);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", destino.display())),
        }
    }
    Err("nomes demais iguais em exports/".into())
}

#[tauri::command]
pub fn export_save(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<String, String> {
    salvar(&state.data_dir, &name, &content).map(|p| p.to_string_lossy().into_owned())
}

/// Mostra o arquivo salvo (ou a pasta) no gerenciador de arquivos. Só o que
/// está dentro de `exports/`.
#[tauri::command]
pub fn export_reveal(state: State<'_, AppState>, path: Option<String>) -> Result<(), String> {
    let dir = pasta(&state.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let rel = path
        .as_deref()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned());
    crate::workspace::reveal(&dir.to_string_lossy(), rel.as_deref()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_nome_nunca_sai_da_pasta() {
        assert_eq!(nome_seguro("../../etc/passwd"), "passwd");
        assert_eq!(nome_seguro("C:\\Users\\x\\log.txt"), "log.txt");
        assert_eq!(nome_seguro("a:b*c?.txt"), "a-b-c-.txt");
        assert_eq!(nome_seguro("..."), "export.txt");
        assert_eq!(nome_seguro(""), "export.txt");
        assert_eq!(nome_seguro(".oculto"), "oculto");
        assert_eq!(nome_seguro("NUL"), "_NUL");
        assert_eq!(nome_seguro("com1.txt"), "_com1.txt");
        assert_eq!(nome_seguro("lpt9"), "_lpt9");
        assert_eq!(nome_seguro("CONFIG.txt"), "CONFIG.txt");
        assert_eq!(nome_seguro("a . "), "a");
    }

    #[test]
    fn salvar_nao_sobrescreve() {
        let dir = std::env::temp_dir().join(format!("ow-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = salvar(&dir, "logs.txt", "um").unwrap();
        let b = salvar(&dir, "logs.txt", "dois").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "um");
        assert_eq!(b.file_name().unwrap(), "logs (2).txt");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "dois");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
