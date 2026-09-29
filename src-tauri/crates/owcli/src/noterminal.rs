//! O `owcli` no terminal do sistema (opt-in).
//!
//! O agente mora em `<data>/runtimes/owcli/<tag>/`, e a pasta muda a cada
//! versão do pin. Para o comando `owcli` valer em qualquer terminal, o app
//! mantém um ponto estável que aponta para o executável da versão instalada:
//!
//! - **Linux e macOS:** um link simbólico em `~/.local/bin/owcli`, trocado de
//!   uma vez (link temporário + `rename`) quando o runtime muda. Terminais já
//!   abertos continuam achando o comando.
//! - **Windows:** a pasta `bin` da versão instalada na entrada `Path` do
//!   usuário (`HKCU\Environment`), como `REG_EXPAND_SZ`, e um aviso de
//!   mudança de ambiente. A pasta tem a tag no caminho, então cada versão
//!   nova troca a nossa entrada; terminais abertos antes precisam ser
//!   reabertos. Nada de `.cmd` (Ctrl+C perguntaria "Terminate batch job?") e
//!   nada de tocar no `Path` da máquina.
//!
//! O app nunca mexe no que não é dele: um `~/.local/bin/owcli` que não aponta
//! para o runtime do app é um conflito, e no `Path` só sai a entrada que mora
//! dentro da pasta do runtime.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// Os erros têm contrato por prefixo, que a tela traduz.
#[derive(Debug, thiserror::Error)]
pub enum ErroDoTerminal {
    /// Já existe um `owcli` no lugar que não é do app.
    #[error("owcli-path-conflict:{0}")]
    Conflito(String),
    #[error("owcli-path-unsupported:{0}")]
    Indisponivel(String),
    #[error("owcli-path-failed:{0}")]
    Falhou(String),
}

impl From<std::io::Error> for ErroDoTerminal {
    fn from(e: std::io::Error) -> Self {
        Self::Falhou(e.to_string())
    }
}

/// Como está o `owcli` no terminal do sistema.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Estado {
    /// O app tem um ponto seu no terminal (link ou entrada no `Path`).
    pub ativo: bool,
    /// Ele aponta para o runtime instalado agora.
    pub aponta_para_o_atual: bool,
    /// Onde: o link (Linux e macOS) ou a pasta no `Path` (Windows).
    pub local: Option<String>,
    /// A pasta do link está no `PATH` deste processo (só Linux e macOS; um app
    /// aberto pelo Finder tem um `PATH` menor que o do terminal, então é uma
    /// dica, não uma prova).
    pub no_path: bool,
    /// Um arquivo que não é do app ocupa o lugar do link.
    pub conflito: Option<String>,
    /// O comando chega pelo `Path` do usuário (Windows), e não por um link.
    pub via_path: bool,
}

// ------------------------------------------------------------ o que é nosso ---

/// O caminho como o sistema o resolveria, mesmo que o fim dele já não exista:
/// o ancestral mais fundo que existe é canonicalizado (`/home` e `/var/home` são
/// o mesmo lugar no Silverblue) e o resto é acrescentado como está.
fn canonico_parcial(p: &Path) -> PathBuf {
    let mut resto = Vec::new();
    let mut atual = p.to_path_buf();
    loop {
        if let Ok(mut c) = std::fs::canonicalize(&atual) {
            c.extend(resto.iter().rev());
            return c;
        }
        match (atual.file_name().map(ToOwned::to_owned), atual.parent()) {
            (Some(nome), Some(pai)) => {
                resto.push(nome);
                atual = pai.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// O caminho está dentro da pasta de runtimes DESTE app? Nunca pelo nome: o link
/// de outra instalação (outro perfil, um build de desenvolvimento) tem o mesmo
/// formato e não é nosso. `..` e `.` no caminho o desqualificam.
pub fn e_do_runtime(caminho: &Path, raiz: &Path) -> bool {
    if caminho
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    caminho.starts_with(raiz) || canonico_parcial(caminho).starts_with(canonico_parcial(raiz))
}

/// Entradas de um `Path` do Windows separadas por `;`.
fn entradas(path: &str) -> impl Iterator<Item = &str> {
    path.split(';').filter(|e| !e.trim().is_empty())
}

/// `C:\A\B\` e `c:/a/b` são o mesmo caminho para comparar entradas do `Path`.
fn normalizar(entrada: &str) -> String {
    entrada
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// A entrada está dentro da pasta `raiz` (no limite de um componente: `…\owcli2`
/// não está dentro de `…\owcli`)? Ou é de uma versão do runtime de outro lugar
/// de dados (o app mudou de pasta, o perfil foi redirecionado): a entrada que
/// sobrou de uma instalação nossa, com o formato `…\runtimes\owcli\owcli-runtime-…`.
fn dentro_de(entrada: &str, raiz: &str) -> bool {
    let (e, r) = (normalizar(entrada), normalizar(raiz));
    let sob_a_raiz =
        !r.is_empty() && (e == r || (e.starts_with(&r) && e[r.len()..].starts_with('\\')));
    sob_a_raiz || e.contains("\\runtimes\\owcli\\owcli-runtime-")
}

/// O `Path` do Windows sem as entradas que moram em `raiz`, e com `nova` no fim
/// (se houver). As demais entradas ficam exatamente como estavam — inclusive
/// as que usam `%VARIÁVEL%` —, na mesma ordem. Idempotente.
pub fn editar_path(atual: &str, raiz: &str, nova: Option<&str>) -> String {
    let mut saida: Vec<&str> = entradas(atual).filter(|e| !dentro_de(e, raiz)).collect();
    if let Some(n) = nova {
        saida.push(n);
    }
    saida.join(";")
}

/// Um `Path` maior que isso quebra programas que leem o ambiente com buffer
/// fixo; melhor recusar do que deixar a pessoa sem terminal.
const LIMITE_DO_PATH: usize = 8000;

/// O `Path` gravado (unidades UTF-16, com o zero final) como texto. Recusa em
/// vez de degradar: um zero no meio ou um par substituto solto seriam
/// regravados errados, e o app nunca estraga o que não é dele.
pub fn decodificar_path(unidades: &[u16]) -> Result<String, String> {
    let fim = unidades.iter().rposition(|u| *u != 0).map_or(0, |i| i + 1);
    let util = &unidades[..fim];
    if util.contains(&0) {
        return Err("o Path do usuário tem um caractere nulo; não mexi nele".into());
    }
    String::from_utf16(util)
        .map_err(|_| "o Path do usuário tem texto inválido; não mexi nele".into())
}

/// O novo `Path` pode ser gravado? Sempre que cabe no limite, e também quando
/// passa dele mas não cresceu (tirar a nossa entrada, ou trocá-la por outra do
/// mesmo tamanho, nunca pode ficar preso). Conta em unidades UTF-16.
pub fn cabe_no_path(novo: &str, atual: &str) -> bool {
    let n = novo.encode_utf16().count();
    n <= LIMITE_DO_PATH || n <= atual.encode_utf16().count()
}

/// O `Path` tem alguma entrada dentro de `raiz`?
pub fn tem_entrada_em(path: &str, raiz: &str) -> bool {
    entradas(path).any(|e| dentro_de(e, raiz))
}

/// O `Path` tem exatamente esta pasta?
pub fn tem_pasta(path: &str, pasta: &str) -> bool {
    entradas(path).any(|e| normalizar(e) == normalizar(pasta))
}

// ------------------------------------------------------------ Linux e macOS ---

#[cfg(unix)]
mod sistema {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// `~/.local/bin`: onde o usuário costuma ter comandos próprios.
    pub fn pasta_dos_links() -> Option<PathBuf> {
        let casa = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())?;
        Some(casa.join(".local").join("bin"))
    }

    fn link() -> Result<PathBuf, ErroDoTerminal> {
        pasta_dos_links()
            .map(|p| p.join("owcli"))
            .ok_or_else(|| ErroDoTerminal::Indisponivel("sem pasta pessoal".into()))
    }

    /// O que o link aponta, se ele é um link nosso.
    fn alvo_nosso(link: &Path, raiz: &Path) -> Option<PathBuf> {
        let m = std::fs::symlink_metadata(link).ok()?;
        if !m.file_type().is_symlink() {
            return None;
        }
        let alvo = std::fs::read_link(link).ok()?;
        e_do_runtime(&alvo, raiz).then_some(alvo)
    }

    fn ocupado_por_outro(link: &Path, raiz: &Path) -> bool {
        std::fs::symlink_metadata(link).is_ok() && alvo_nosso(link, raiz).is_none()
    }

    /// Troca (ou cria) o link de uma vez: um link temporário ao lado, e o
    /// `rename` por cima. Quem está executando o comando nunca o vê sumir.
    fn trocar(link: &Path, exe: &Path) -> Result<(), ErroDoTerminal> {
        let pasta = link
            .parent()
            .ok_or_else(|| ErroDoTerminal::Falhou("link sem pasta".into()))?;
        std::fs::create_dir_all(pasta)?;
        // Único por chamada: duas trocas ao mesmo tempo não se atropelam.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let temporario = pasta.join(format!(
            ".owcli.{}.{}.tmp",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&temporario);
        std::os::unix::fs::symlink(exe, &temporario)?;
        if let Err(e) = std::fs::rename(&temporario, link) {
            let _ = std::fs::remove_file(&temporario);
            return Err(e.into());
        }
        Ok(())
    }

    pub fn ativar(raiz: &Path, exe: &Path) -> Result<(), ErroDoTerminal> {
        let link = link()?;
        if alvo_nosso(&link, raiz).is_some() {
            return trocar(&link, exe);
        }
        let conflito = || ErroDoTerminal::Conflito(link.display().to_string());
        if std::fs::symlink_metadata(&link).is_ok() {
            return Err(conflito());
        }
        // Sem nada no lugar: cria direto, sem passar pelo `rename`, que
        // sobrescreveria o que a pessoa puser ali no meio do caminho. O `EEXIST`
        // do kernel é a resposta.
        if let Some(pasta) = link.parent() {
            std::fs::create_dir_all(pasta)?;
        }
        match std::os::unix::fs::symlink(exe, &link) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(conflito()),
            r => r.map_err(Into::into),
        }
    }

    pub fn repontar(raiz: &Path, exe: &Path) -> Result<bool, ErroDoTerminal> {
        let link = link()?;
        match alvo_nosso(&link, raiz) {
            Some(alvo) if alvo != exe => trocar(&link, exe).map(|()| true),
            _ => Ok(false),
        }
    }

    pub fn desativar(raiz: &Path) -> Result<(), ErroDoTerminal> {
        let link = link()?;
        if alvo_nosso(&link, raiz).is_some() {
            std::fs::remove_file(&link)?;
        }
        Ok(())
    }

    pub fn estado(raiz: &Path, exe: Option<&Path>) -> Estado {
        let Ok(link) = link() else {
            return Estado {
                ativo: false,
                aponta_para_o_atual: false,
                local: None,
                no_path: false,
                conflito: None,
                via_path: false,
            };
        };
        let alvo = alvo_nosso(&link, raiz);
        let no_path = link.parent().is_some_and(|pasta| {
            let path = lr_proc::host_var("PATH").unwrap_or_default();
            let canonica = canonico_parcial(pasta);
            std::env::split_paths(&path).any(|p| p == pasta || canonico_parcial(&p) == canonica)
        });
        Estado {
            ativo: alvo.is_some(),
            aponta_para_o_atual: alvo.as_deref().is_some_and(|a| Some(a) == exe),
            local: Some(link.display().to_string()),
            no_path,
            conflito: ocupado_por_outro(&link, raiz).then(|| link.display().to_string()),
            via_path: false,
        }
    }
}

// ------------------------------------------------------------------ Windows ---

#[cfg(windows)]
mod sistema {
    use super::*;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE,
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use windows::core::w;

    /// Ler, editar e gravar o `Path` são três passos: duas operações do app ao
    /// mesmo tempo (o boot e um clique) se perderiam uma à outra.
    static TRAVA: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn trava() -> std::sync::MutexGuard<'static, ()> {
        TRAVA.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn erro(contexto: &str, codigo: u32) -> ErroDoTerminal {
        ErroDoTerminal::Falhou(format!("{contexto} (erro {codigo})"))
    }

    /// O `Path` do usuário como está gravado (sem expandir `%VARIÁVEIS%`).
    /// `None` quando o valor não existe.
    fn ler_path() -> Result<Option<String>, ErroDoTerminal> {
        let mut chave = HKEY::default();
        // SAFETY: `chave` é um destino válido; a chave é fechada abaixo.
        let r = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                None,
                KEY_READ,
                &mut chave,
            )
        };
        if r != ERROR_SUCCESS {
            return Err(erro("abrir HKCU\\Environment", r.0));
        }
        let resultado = (|| {
            let mut tipo = REG_VALUE_TYPE::default();
            let mut tamanho = 0u32;
            // SAFETY: primeira chamada, só para saber o tamanho.
            let r = unsafe {
                RegQueryValueExW(
                    chave,
                    w!("Path"),
                    None,
                    Some(&mut tipo),
                    None,
                    Some(&mut tamanho),
                )
            };
            if r == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if r != ERROR_SUCCESS {
                return Err(erro("ler o Path", r.0));
            }
            if tipo != REG_SZ && tipo != REG_EXPAND_SZ {
                return Err(ErroDoTerminal::Falhou(
                    "o Path do usuário não é um texto; não mexi nele".into(),
                ));
            }
            let mut buffer = vec![0u8; tamanho as usize + 2];
            let mut lido = tamanho + 2;
            // SAFETY: `buffer` tem o tamanho informado em `lido`.
            let r = unsafe {
                RegQueryValueExW(
                    chave,
                    w!("Path"),
                    None,
                    Some(&mut tipo),
                    Some(buffer.as_mut_ptr()),
                    Some(&mut lido),
                )
            };
            if r != ERROR_SUCCESS {
                return Err(erro("ler o Path", r.0));
            }
            let unidades: Vec<u16> = buffer[..lido as usize]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p))
                .collect();
            decodificar_path(&unidades)
                .map(Some)
                .map_err(ErroDoTerminal::Falhou)
        })();
        // SAFETY: a chave foi aberta acima.
        unsafe {
            let _ = RegCloseKey(chave);
        }
        resultado
    }

    /// Grava o `Path` como `REG_EXPAND_SZ` (o tipo do `Path` do Windows) e
    /// avisa os programas abertos de que o ambiente mudou.
    fn gravar_path(valor: &str, atual: &str) -> Result<(), ErroDoTerminal> {
        if !cabe_no_path(valor, atual) {
            return Err(ErroDoTerminal::Falhou(format!(
                "o Path ficaria com {} caracteres; não mexi nele",
                valor.encode_utf16().count()
            )));
        }
        let mut chave = HKEY::default();
        // SAFETY: como em `ler_path`.
        let r = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                None,
                KEY_WRITE,
                &mut chave,
            )
        };
        if r != ERROR_SUCCESS {
            return Err(erro("abrir HKCU\\Environment", r.0));
        }
        let bytes: Vec<u8> = valor
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        // SAFETY: `bytes` é o texto UTF-16 com o zero final, como o tipo exige.
        let r = unsafe { RegSetValueExW(chave, w!("Path"), None, REG_EXPAND_SZ, Some(&bytes)) };
        // SAFETY: a chave foi aberta acima.
        unsafe {
            let _ = RegCloseKey(chave);
        }
        if r != ERROR_SUCCESS {
            return Err(erro("gravar o Path", r.0));
        }
        // "Environment" avisa o Explorer e os terminais novos. O prazo é por
        // janela, não total (uma janela travada custa 5 s cada): fora da thread
        // de quem chamou, para o clique não esperar.
        std::thread::spawn(|| {
            let ambiente: Vec<u16> = "Environment\0".encode_utf16().collect();
            // SAFETY: `ambiente` vive até depois da chamada, e termina em zero.
            unsafe {
                let _ = SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    WPARAM(0),
                    LPARAM(ambiente.as_ptr() as isize),
                    SMTO_ABORTIFHUNG,
                    5000,
                    None,
                );
            }
        });
        Ok(())
    }

    fn pasta_do(exe: &Path) -> Result<String, ErroDoTerminal> {
        exe.parent()
            .map(|p| p.display().to_string())
            .ok_or_else(|| ErroDoTerminal::Falhou("executável sem pasta".into()))
    }

    /// Com a trava já pega. Sem nada a fazer (tirar o que não está lá), nem
    /// abre o registro para escrever.
    fn aplicar(raiz: &Path, nova: Option<&str>) -> Result<bool, ErroDoTerminal> {
        let atual = ler_path()?.unwrap_or_default();
        let raiz = raiz.display().to_string();
        if nova.is_none() && !tem_entrada_em(&atual, &raiz) {
            return Ok(false);
        }
        let novo = editar_path(&atual, &raiz, nova);
        if novo == atual {
            return Ok(false);
        }
        gravar_path(&novo, &atual).map(|()| true)
    }

    pub fn ativar(raiz: &Path, exe: &Path) -> Result<(), ErroDoTerminal> {
        let _g = trava();
        aplicar(raiz, Some(&pasta_do(exe)?)).map(|_| ())
    }

    pub fn repontar(raiz: &Path, exe: &Path) -> Result<bool, ErroDoTerminal> {
        let _g = trava();
        let atual = ler_path()?.unwrap_or_default();
        if !tem_entrada_em(&atual, &raiz.display().to_string()) {
            return Ok(false);
        }
        aplicar(raiz, Some(&pasta_do(exe)?))
    }

    pub fn desativar(raiz: &Path) -> Result<(), ErroDoTerminal> {
        let _g = trava();
        aplicar(raiz, None).map(|_| ())
    }

    pub fn estado(raiz: &Path, exe: Option<&Path>) -> Estado {
        let atual = ler_path().ok().flatten().unwrap_or_default();
        let ativo = tem_entrada_em(&atual, &raiz.display().to_string());
        let pasta = exe.and_then(|e| pasta_do(e).ok());
        Estado {
            ativo,
            aponta_para_o_atual: pasta.as_deref().is_some_and(|p| tem_pasta(&atual, p)),
            local: pasta,
            // O terminal novo já enxerga; um aberto antes precisa ser reaberto.
            no_path: ativo,
            conflito: None,
            via_path: true,
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod sistema {
    use super::*;

    pub fn ativar(_: &Path, _: &Path) -> Result<(), ErroDoTerminal> {
        Err(ErroDoTerminal::Indisponivel(std::env::consts::OS.into()))
    }
    pub fn repontar(_: &Path, _: &Path) -> Result<bool, ErroDoTerminal> {
        Ok(false)
    }
    pub fn desativar(_: &Path) -> Result<(), ErroDoTerminal> {
        Ok(())
    }
    pub fn estado(_: &Path, _: Option<&Path>) -> Estado {
        Estado {
            ativo: false,
            aponta_para_o_atual: false,
            local: None,
            no_path: false,
            conflito: None,
            via_path: false,
        }
    }
}

/// Deixa o `owcli` no terminal do sistema, apontando para `exe`.
pub fn ativar(raiz: &Path, exe: &Path) -> Result<(), ErroDoTerminal> {
    sistema::ativar(raiz, exe)
}

/// Se o app já tem seu ponto no terminal, aponta-o para `exe` (depois de uma
/// versão nova do runtime). `true` se mudou. Não cria nada por conta própria.
pub fn repontar(raiz: &Path, exe: &Path) -> Result<bool, ErroDoTerminal> {
    sistema::repontar(raiz, exe)
}

/// Tira o que o app pôs no terminal do sistema, e só isso.
pub fn desativar(raiz: &Path) -> Result<(), ErroDoTerminal> {
    sistema::desativar(raiz)
}

pub fn estado(raiz: &Path, exe: Option<&Path>) -> Estado {
    sistema::estado(raiz, exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAIZ: &str = r"C:\Users\p\AppData\Roaming\dev.openweights.app\runtimes\owcli";

    #[test]
    fn o_path_troca_so_a_nossa_entrada_e_deixa_as_outras_como_estavam() {
        let atual = format!(
            r"C:\Windows\system32;%USERPROFILE%\bin;{RAIZ}\owcli-runtime-aaaaaaaa-v1\bin;C:\Program Files\Git\cmd"
        );
        let novo = format!(r"{RAIZ}\owcli-runtime-bbbbbbbb-v1\bin");
        let saida = editar_path(&atual, RAIZ, Some(&novo));
        assert_eq!(
            saida,
            format!(r"C:\Windows\system32;%USERPROFILE%\bin;C:\Program Files\Git\cmd;{novo}")
        );
        // Idempotente.
        assert_eq!(editar_path(&saida, RAIZ, Some(&novo)), saida);
    }

    #[test]
    fn tirar_o_owcli_do_path_deixa_o_resto_intacto() {
        let atual = format!(r"C:\a;{RAIZ}\owcli-runtime-aaaaaaaa-v1\bin;%SystemRoot%;;C:\b");
        assert_eq!(editar_path(&atual, RAIZ, None), r"C:\a;%SystemRoot%;C:\b");
        assert_eq!(editar_path(r"C:\a;C:\b", RAIZ, None), r"C:\a;C:\b");
        assert_eq!(editar_path("", RAIZ, None), "");
    }

    #[test]
    fn a_comparacao_ignora_caixa_barra_e_aspas_mas_respeita_o_limite_do_componente() {
        assert!(dentro_de(
            &format!("{}\\X\\BIN\\", RAIZ.to_uppercase()),
            RAIZ
        ));
        assert!(dentro_de(
            &format!("\"{}/x/bin\"", RAIZ.replace('\\', "/")),
            RAIZ
        ));
        assert!(dentro_de(RAIZ, RAIZ));
        assert!(
            !dentro_de(&format!("{RAIZ}2\\bin"), RAIZ),
            "owcli2 não é owcli"
        );
        assert!(!dentro_de(
            r"C:\Users\p\AppData\Roaming\dev.openweights.app\runtimes",
            RAIZ
        ));
        assert!(
            !dentro_de(r"C:\qualquer", ""),
            "raiz vazia não casa com tudo"
        );
    }

    #[test]
    fn saber_se_o_path_ja_tem_a_pasta_ou_alguma_versao() {
        let pasta = format!(r"{RAIZ}\owcli-runtime-aaaaaaaa-v1\bin");
        let path = format!(r"C:\a;{pasta}");
        assert!(tem_entrada_em(&path, RAIZ));
        assert!(tem_pasta(&path, &pasta.to_lowercase()));
        assert!(!tem_pasta(
            &path,
            &format!(r"{RAIZ}\owcli-runtime-bbbbbbbb-v1\bin")
        ));
        assert!(!tem_entrada_em(r"C:\a;C:\b", RAIZ));
    }

    #[test]
    fn um_caminho_do_runtime_e_reconhecido_pela_pasta_e_nunca_pelo_nome() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real/dados/runtimes/owcli");
        std::fs::create_dir_all(&real).unwrap();
        // O mesmo lugar por outro nome (como /home e /var/home no Silverblue).
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("apelido"))
                .unwrap();
            let pelo_apelido = tmp
                .path()
                .join("apelido/dados/runtimes/owcli/owcli-runtime-6621daf3-v1/bin/owcli");
            assert!(
                e_do_runtime(&pelo_apelido, &real),
                "destino que já não existe"
            );
        }
        let dentro = real.join("owcli-runtime-6621daf3-v1/bin/owcli");
        assert!(e_do_runtime(&dentro, &real));
        // Outra instalação do app (outro perfil, um build de desenvolvimento) tem o
        // mesmo formato e NÃO é nossa.
        let outra = tmp
            .path()
            .join("outra/dados/runtimes/owcli/owcli-runtime-6621daf3-v1/bin/owcli");
        assert!(!e_do_runtime(&outra, &real));
        assert!(!e_do_runtime(Path::new("/usr/bin/owcli"), &real));
        // `..` para sair da pasta não vale, mesmo começando por ela.
        assert!(!e_do_runtime(
            &real.join("../../../../usr/bin/owcli"),
            &real
        ));
    }

    #[test]
    fn o_path_e_lido_sem_degradar_e_so_e_gravado_se_couber() {
        let unidades = |s: &str| -> Vec<u16> { s.encode_utf16().chain([0]).collect() };
        assert_eq!(
            decodificar_path(&unidades(r"C:\a;%X%\b")).unwrap(),
            r"C:\a;%X%\b"
        );
        assert_eq!(decodificar_path(&[0]).unwrap(), "");
        assert_eq!(decodificar_path(&[]).unwrap(), "");
        // Zeros do fim são o terminador; um no meio, não.
        let mut com_zeros = unidades("a;b");
        com_zeros.extend([0, 0]);
        assert_eq!(decodificar_path(&com_zeros).unwrap(), "a;b");
        assert!(decodificar_path(&[u16::from(b'a'), 0, u16::from(b'b'), 0]).is_err());
        // Um par substituto solto seria regravado como U+FFFD.
        assert!(decodificar_path(&[u16::from(b'a'), 0xD800, u16::from(b'b'), 0]).is_err());

        let grande = "x".repeat(LIMITE_DO_PATH + 500);
        assert!(cabe_no_path("curto", "curto"));
        assert!(!cabe_no_path(&grande, "curto"), "cresceu além do limite");
        assert!(
            cabe_no_path(&grande, &grande),
            "não cresceu: tirar a nossa entrada passa"
        );
        assert!(
            cabe_no_path(&"y".repeat(LIMITE_DO_PATH + 100), &grande),
            "encolheu"
        );
        // Caracteres, não bytes: acentos contam uma unidade.
        assert!(cabe_no_path(&"ã".repeat(LIMITE_DO_PATH), ""));
    }

    #[test]
    fn a_entrada_que_sobrou_de_outra_pasta_de_dados_tambem_sai() {
        let orfa = r"C:\Users\outro\AppData\Roaming\dev.openweights.app\runtimes\owcli\owcli-runtime-aaaaaaaa-v1\bin";
        let path = format!(r"C:\a;{orfa};C:\b");
        assert!(tem_entrada_em(&path, RAIZ));
        let saida = editar_path(&path, RAIZ, Some(r"C:\nova\bin"));
        assert_eq!(saida, r"C:\a;C:\b;C:\nova\bin");
        // Uma pasta qualquer chamada owcli não é nossa.
        assert!(!tem_entrada_em(r"C:\ferramentas\owcli\bin", RAIZ));
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::sync::Mutex;

        /// `HOME` é do processo: os testes que o trocam não podem se cruzar.
        static HOME: Mutex<()> = Mutex::new(());

        struct Ambiente {
            _guarda: std::sync::MutexGuard<'static, ()>,
            casa: tempfile::TempDir,
            raiz: PathBuf,
            anterior: Option<std::ffi::OsString>,
        }

        impl Ambiente {
            fn novo() -> Self {
                let guarda = HOME.lock().unwrap_or_else(|e| e.into_inner());
                let casa = tempfile::tempdir().unwrap();
                let raiz = casa.path().join("dados/runtimes/owcli");
                let anterior = std::env::var_os("HOME");
                // SAFETY: os testes que trocam o HOME se serializam em `HOME`.
                unsafe { std::env::set_var("HOME", casa.path()) };
                Self {
                    _guarda: guarda,
                    casa,
                    raiz,
                    anterior,
                }
            }

            fn exe(&self, tag: &str) -> PathBuf {
                let e = self.raiz.join(tag).join("bin/owcli");
                std::fs::create_dir_all(e.parent().unwrap()).unwrap();
                std::fs::write(&e, "#!/bin/sh\n").unwrap();
                e
            }

            fn link(&self) -> PathBuf {
                self.casa.path().join(".local/bin/owcli")
            }
        }

        impl Drop for Ambiente {
            fn drop(&mut self) {
                // SAFETY: idem.
                unsafe {
                    match &self.anterior {
                        Some(v) => std::env::set_var("HOME", v),
                        None => std::env::remove_var("HOME"),
                    }
                }
            }
        }

        #[test]
        fn ativar_cria_o_link_e_a_versao_nova_o_troca_sem_o_deixar_sumir() {
            let a = Ambiente::novo();
            let v1 = a.exe("owcli-runtime-aaaaaaaa-v1");
            ativar(&a.raiz, &v1).unwrap();
            assert_eq!(std::fs::read_link(a.link()).unwrap(), v1);
            let e = estado(&a.raiz, Some(&v1));
            assert!(e.ativo && e.aponta_para_o_atual && e.conflito.is_none());

            let v2 = a.exe("owcli-runtime-bbbbbbbb-v1");
            assert!(estado(&a.raiz, Some(&v2)).ativo);
            assert!(!estado(&a.raiz, Some(&v2)).aponta_para_o_atual);
            assert!(repontar(&a.raiz, &v2).unwrap());
            assert_eq!(std::fs::read_link(a.link()).unwrap(), v2);
            assert!(!repontar(&a.raiz, &v2).unwrap(), "já aponta: nada a fazer");
            // Nenhum link temporário fica para trás.
            let sobras: Vec<_> = std::fs::read_dir(a.link().parent().unwrap())
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(".owcli."))
                .collect();
            assert!(sobras.is_empty());
        }

        #[test]
        fn um_owcli_que_nao_e_nosso_e_conflito_e_nunca_e_tocado() {
            let a = Ambiente::novo();
            let v1 = a.exe("owcli-runtime-aaaaaaaa-v1");
            let link = a.link();
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();

            // Um arquivo de verdade.
            std::fs::write(&link, "meu script").unwrap();
            assert!(matches!(
                ativar(&a.raiz, &v1),
                Err(ErroDoTerminal::Conflito(_))
            ));
            assert!(!repontar(&a.raiz, &v1).unwrap());
            desativar(&a.raiz).unwrap();
            assert_eq!(std::fs::read_to_string(&link).unwrap(), "meu script");
            assert!(estado(&a.raiz, Some(&v1)).conflito.is_some());
            assert!(!estado(&a.raiz, Some(&v1)).ativo);

            // Um link para outro lugar.
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("/usr/bin/true", &link).unwrap();
            assert!(matches!(
                ativar(&a.raiz, &v1),
                Err(ErroDoTerminal::Conflito(_))
            ));
            desativar(&a.raiz).unwrap();
            assert!(
                std::fs::symlink_metadata(&link).is_ok(),
                "o link alheio continua"
            );
        }

        #[test]
        fn desativar_tira_o_nosso_link_mesmo_com_a_versao_ja_apagada() {
            let a = Ambiente::novo();
            let v1 = a.exe("owcli-runtime-aaaaaaaa-v1");
            ativar(&a.raiz, &v1).unwrap();
            std::fs::remove_dir_all(a.raiz.join("owcli-runtime-aaaaaaaa-v1")).unwrap();
            assert!(estado(&a.raiz, None).ativo, "link pendurado ainda é nosso");
            desativar(&a.raiz).unwrap();
            assert!(std::fs::symlink_metadata(a.link()).is_err());
            desativar(&a.raiz).unwrap(); // de novo: nada a fazer
        }

        /// Instala de mentira a versão pinada: o executável e a identidade.
        fn instalar_a_atual(layout: &crate::Layout) -> Option<PathBuf> {
            let alvo = crate::pins::alvo_atual()?;
            let dir = layout.atual();
            let entrada = crate::install::entrada_para(alvo);
            std::fs::create_dir_all(dir.join(entrada).parent().unwrap()).unwrap();
            std::fs::write(dir.join(entrada), "#!/bin/sh\n").unwrap();
            let id = crate::install::Identidade {
                name: crate::install::NOME_DO_RUNTIME.into(),
                format: crate::install::FORMATO_DO_RUNTIME,
                revision: crate::pins::pins().revision.clone(),
                upstream_tag: "rust-v0.157.1".into(),
                target: alvo.into(),
                entry: entrada.into(),
            };
            std::fs::write(
                dir.join("runtime.json"),
                serde_json::to_string(&id).unwrap(),
            )
            .unwrap();
            Some(dir.join(entrada))
        }

        #[test]
        fn atualizar_aponta_o_link_para_a_nova_antes_de_podar_e_nao_poda_sem_ela() {
            let a = Ambiente::novo();
            let layout = crate::Layout::new(&a.casa.path().join("dados"));
            assert_eq!(layout.raiz(), a.raiz);
            let velha = a.exe("owcli-runtime-00000000-v1");
            ativar(&a.raiz, &velha).unwrap();

            // A atual ainda não está instalada: a velha fica, e o link com ela.
            assert_eq!(crate::install::manter_versoes(&layout), 0);
            assert!(velha.is_file());
            assert_eq!(std::fs::read_link(a.link()).unwrap(), velha);

            // Instalada a atual: o link aponta para ela e a velha sai.
            let Some(nova) = instalar_a_atual(&layout) else {
                return; // sem pacote para este sistema
            };
            assert_eq!(crate::install::manter_versoes(&layout), 1);
            assert!(!velha.exists());
            assert_eq!(std::fs::read_link(a.link()).unwrap(), nova);
            assert!(estado(&a.raiz, Some(&nova)).aponta_para_o_atual);
        }

        #[test]
        fn quem_nunca_ligou_o_terminal_nao_ganha_link_ao_atualizar() {
            let a = Ambiente::novo();
            let layout = crate::Layout::new(&a.casa.path().join("dados"));
            a.exe("owcli-runtime-00000000-v1");
            if instalar_a_atual(&layout).is_none() {
                return;
            }
            crate::install::manter_versoes(&layout);
            assert!(std::fs::symlink_metadata(a.link()).is_err());
        }

        #[test]
        fn se_nao_da_para_apontar_a_versao_nova_a_velha_nao_e_apagada() {
            use std::os::unix::fs::PermissionsExt;
            let a = Ambiente::novo();
            let layout = crate::Layout::new(&a.casa.path().join("dados"));
            let velha = a.exe("owcli-runtime-00000000-v1");
            ativar(&a.raiz, &velha).unwrap();
            let Some(_nova) = instalar_a_atual(&layout) else {
                return;
            };
            // A pasta do link fica sem escrita: trocar o link falha.
            let pasta = a.link().parent().unwrap().to_path_buf();
            std::fs::set_permissions(&pasta, std::fs::Permissions::from_mode(0o500)).unwrap();
            let root = std::fs::write(pasta.join(".sonda"), "").is_ok();
            let podadas = crate::install::manter_versoes(&layout);
            std::fs::set_permissions(&pasta, std::fs::Permissions::from_mode(0o700)).unwrap();
            if root {
                return; // como root a permissão não impede nada: o caso não se aplica.
            }
            assert_eq!(podadas, 0);
            assert!(velha.is_file(), "o terminal ainda aponta para ela");
            assert_eq!(std::fs::read_link(a.link()).unwrap(), velha);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn a_poda_poupa_a_versao_com_uma_sessao_rodando() {
            let a = Ambiente::novo();
            let layout = crate::Layout::new(&a.casa.path().join("dados"));
            let velha = a.exe("owcli-runtime-00000000-v1");
            let sleep = ["/usr/bin/sleep", "/bin/sleep"]
                .into_iter()
                .find(|p| Path::new(p).is_file())
                .expect("sleep");
            std::fs::copy(sleep, &velha).unwrap();
            let mut cmd = std::process::Command::new(&velha);
            cmd.arg("30");
            // Como todo processo do app (a regra do lr_proc vale também aqui).
            lr_proc::host_env_std(&mut cmd);
            lr_proc::no_window_std(&mut cmd);
            let mut sessao = cmd.spawn().unwrap();
            assert_eq!(
                crate::install::podar(&layout),
                0,
                "com sessão rodando, fica"
            );
            assert!(velha.is_file());
            sessao.kill().unwrap();
            sessao.wait().unwrap();
            assert_eq!(crate::install::podar(&layout), 1, "sem sessão, sai");
        }

        #[test]
        fn duas_ativacoes_seguidas_e_o_arquivo_alheio_que_aparece_depois() {
            let a = Ambiente::novo();
            let v1 = a.exe("owcli-runtime-aaaaaaaa-v1");
            // Sem nada no lugar: cria direto. De novo: troca, e não é conflito.
            ativar(&a.raiz, &v1).unwrap();
            ativar(&a.raiz, &v1).unwrap();
            // Livre de novo, e a pessoa põe um arquivo dela: conflito.
            desativar(&a.raiz).unwrap();
            std::fs::write(a.link(), "meu").unwrap();
            assert!(matches!(
                ativar(&a.raiz, &v1),
                Err(ErroDoTerminal::Conflito(_))
            ));
            assert_eq!(std::fs::read_to_string(a.link()).unwrap(), "meu");
        }

        #[test]
        fn repontar_nao_cria_o_que_a_pessoa_nao_pediu() {
            let a = Ambiente::novo();
            let v1 = a.exe("owcli-runtime-aaaaaaaa-v1");
            assert!(!repontar(&a.raiz, &v1).unwrap());
            assert!(std::fs::symlink_metadata(a.link()).is_err());
        }
    }
}
