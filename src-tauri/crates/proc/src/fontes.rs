//! Fontes COLRv1 e o WebKit que vai dentro do AppImage.
//!
//! O WebKitGTK do pacote vem do Ubuntu 22.04 (o runner da release) e foi
//! compilado contra o FreeType 2.11. Em execução ele usa o FreeType do
//! SISTEMA: o AppImage não empacota FreeType, fontconfig nem HarfBuzz. A
//! estrutura com que o FreeType descreve uma pintura COLRv1 mudou de layout
//! depois da 2.11 (API experimental até a 2.13), então o Skia do WebKit lê os
//! campos do gradiente no lugar errado: um emoji com gradiente sai deformado
//! ou derruba o processo da webview (`colrv1_configure_skpaint … Assertion
//! '__n < this->size()' failed`). Num Fedora 44 — FreeType 2.14 e o Noto Color
//! Emoji em COLRv1 — bastava o modelo terminar a resposta com um emoji de
//! rosto para o Chat cair, e recair a cada recarga.
//!
//! A saída é esconder do app as fontes COLRv1 quando o FreeType do sistema não
//! é o do WebKit: um `fonts.conf` que inclui a configuração da pessoa e só
//! rejeita esses arquivos. Os emojis caem na próxima fonte que os tem (a Noto
//! Emoji, monocromática, num Fedora); sem nenhuma, viram quadradinhos — ainda
//! melhor que a webview caindo. Programas do sistema abertos pelo app recebem
//! o `FONTCONFIG_FILE` da pessoa de volta (`appimage::correcoes`).
//!
//! O que fica de fora: uma webfont COLRv1 carregada por `@font-face` não passa
//! pelo fontconfig (o Skia a abre direto da memória). A interface não tem
//! webfont nenhuma, mas a prévia de HTML do chat (`PreviewPane`) roda no mesmo
//! processo e mostra o que o modelo gravou — uma página que traga a própria
//! fonte COLRv1 ainda derruba a webview.

use std::ffi::{CStr, OsStr, OsString, c_char, c_int, c_void};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Guarda o `FONTCONFIG_FILE` de antes da troca (vazio: não havia) — é daqui
/// que os filhos recebem o da pessoa de volta.
///
/// Contrato permanente, mesmo se a proteção um dia sair: o `app.restart()` de
/// uma atualização passa esta variável e o nosso `FONTCONFIG_FILE` para a
/// versão seguinte, que tem de devolver o da pessoa (`restaurar` aqui e
/// `appimage::correcoes` para os filhos).
pub(crate) const FONTCONFIG_ORIGINAL: &str = "OW_FONTCONFIG_FILE_ORIGINAL";

/// O FreeType com que o WebKitGTK do pacote foi compilado: o `libfreetype6`
/// do `ubuntu-22.04`, que o `release.yml` confere antes de empacotar. Se o
/// runner da release mudar, este número muda junto.
pub const FREETYPE_DO_WEBKIT: (i32, i32) = (2, 11);

/// O começo do nome do arquivo gerado. Uma configuração com este nome nunca
/// serve de base: seria incluir a si mesma.
const PREFIXO: &str = "fonts-sem-colrv1";

/// Esconde do WebKit as fontes COLRv1, se o app roda de um AppImage num
/// sistema cujo FreeType não é o do WebKit do pacote. Fora disso, nada.
///
/// # Safety
///
/// Troca variáveis do ambiente do processo (`std::env::set_var`): tem de
/// rodar no começo do `main`, antes de existir qualquer outra thread e antes
/// de o GTK carregar o fontconfig.
pub unsafe fn esconder_fontes_colrv1_do_webkit() {
    if crate::appimage::pacote().is_none() {
        return;
    }
    // Um processo aberto pelo próprio app (o `restart` de uma atualização)
    // herda a troca: o original é o que ficou guardado. Daqui em diante o
    // ambiente é o da pessoa — é o fontconfig dela que se lê abaixo, e é o
    // que fica se a proteção não se aplicar.
    let original = match std::env::var_os(FONTCONFIG_ORIGINAL) {
        Some(guardado) => {
            let original = (!guardado.is_empty()).then_some(guardado);
            // SAFETY: repassada de quem chamou (começo do `main`, uma thread).
            unsafe {
                match &original {
                    Some(v) => std::env::set_var("FONTCONFIG_FILE", v),
                    None => std::env::remove_var("FONTCONFIG_FILE"),
                }
                std::env::remove_var(FONTCONFIG_ORIGINAL);
            }
            original
        }
        None => std::env::var_os("FONTCONFIG_FILE"),
    };

    if std::env::var_os("FONTCONFIG_SYSROOT").is_some_and(|v| !v.is_empty()) {
        // Com sysroot, o fontconfig procura o FONTCONFIG_FILE dentro dele.
        log::info!("FONTCONFIG_SYSROOT definido: fontes COLRv1 ficam visíveis ao WebKit");
        return;
    }
    let freetype = versao_do_freetype();
    if !precisa_esconder(freetype) {
        return;
    }
    let versao = freetype.map_or("desconhecida".to_string(), |(a, b, c)| {
        format!("{a}.{b}.{c}")
    });
    let Some(coloridas) = fontes_coloridas() else {
        log::warn!(
            "FreeType {versao} no sistema, mas a libfontconfig não abriu: \
             fontes COLRv1 ficam visíveis ao WebKit"
        );
        return;
    };
    let colrv1 = fontes_colrv1(&coloridas);
    if colrv1.is_empty() {
        return;
    }
    let Some(base) = configuracao_base(original.as_deref()) else {
        log::warn!(
            "fontes COLRv1 ficam visíveis ao WebKit: a configuração do fontconfig \
             ({:?}) não foi encontrada",
            original
        );
        return;
    };
    let Some(conf) = fonts_conf(&base, &colrv1) else {
        log::warn!("fontes COLRv1 ficam visíveis ao WebKit: {base:?} não é UTF-8");
        return;
    };
    let arquivo = match escrever(&conf) {
        Ok(arquivo) => arquivo,
        Err(motivos) => {
            log::warn!(
                "fontes COLRv1 ficam visíveis ao WebKit: nenhuma pasta aceitou o arquivo ({motivos})"
            );
            return;
        }
    };
    // SAFETY: repassada de quem chamou (começo do `main`, uma thread).
    unsafe {
        std::env::set_var(FONTCONFIG_ORIGINAL, original.as_deref().unwrap_or_default());
        std::env::set_var("FONTCONFIG_FILE", &arquivo);
    }
    log::info!(
        "FreeType do sistema {versao}, WebKit do pacote compilado com {}.{}: \
         fontes COLRv1 escondidas da interface ({})",
        FREETYPE_DO_WEBKIT.0,
        FREETYPE_DO_WEBKIT.1,
        colrv1
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Sem saber o FreeType do sistema, esconder é o lado seguro: o pior caso
/// é um emoji monocromático, contra uma webview que cai.
fn precisa_esconder(freetype: Option<(i32, i32, i32)>) -> bool {
    freetype.is_none_or(|(a, b, _)| (a, b) != FREETYPE_DO_WEBKIT)
}

/// Uma biblioteca do sistema aberta com `dlopen` — a mesma busca do
/// carregador dinâmico, então a mesma que o WebKit vai usar (o pacote não
/// traz FreeType nem fontconfig).
struct Biblioteca(*mut c_void);

impl Biblioteca {
    fn abrir(nome: &CStr) -> Option<Self> {
        // SAFETY: `dlopen` com um nome terminado em zero; o handle é fechado
        // no `Drop`.
        let h = unsafe { libc::dlopen(nome.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
        (!h.is_null()).then_some(Self(h))
    }

    /// # Safety
    ///
    /// `F` tem de ser um ponteiro de função `extern "C"` com a assinatura
    /// exata do símbolo.
    unsafe fn simbolo<F: Copy>(&self, nome: &CStr) -> Option<F> {
        assert_eq!(size_of::<F>(), size_of::<*mut c_void>());
        // SAFETY: handle válido enquanto `self` vive.
        let p = unsafe { libc::dlsym(self.0, nome.as_ptr()) };
        // SAFETY: tamanhos iguais (conferido acima); a assinatura é
        // responsabilidade de quem chama.
        (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
    }
}

impl Drop for Biblioteca {
    fn drop(&mut self) {
        // SAFETY: o handle veio de um `dlopen` bem-sucedido.
        unsafe { libc::dlclose(self.0) };
    }
}

/// A versão do FreeType que o WebKit vai carregar.
fn versao_do_freetype() -> Option<(i32, i32, i32)> {
    type Iniciar = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
    type Versao = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int, *mut c_int);
    type Encerrar = unsafe extern "C" fn(*mut c_void) -> c_int;

    let ft = Biblioteca::abrir(c"libfreetype.so.6")?;
    // SAFETY: as assinaturas são as da API pública do FreeType 2
    // (`FT_Init_FreeType`, `FT_Library_Version`, `FT_Done_FreeType`), estáveis
    // desde a 2.0; a biblioteca criada aqui é encerrada aqui.
    unsafe {
        let iniciar: Iniciar = ft.simbolo(c"FT_Init_FreeType")?;
        let versao: Versao = ft.simbolo(c"FT_Library_Version")?;
        let encerrar: Encerrar = ft.simbolo(c"FT_Done_FreeType")?;
        let mut biblioteca = std::ptr::null_mut();
        if iniciar(&mut biblioteca) != 0 || biblioteca.is_null() {
            return None;
        }
        let (mut a, mut b, mut c) = (0, 0, 0);
        versao(biblioteca, &mut a, &mut b, &mut c);
        encerrar(biblioteca);
        Some((a, b, c))
    }
}

/// Os arquivos de fonte com tabela de cor que o fontconfig da pessoa vê,
/// pela mesma `libfontconfig` que o WebKit vai carregar (sem depender de um
/// `fc-list` no PATH). `None` quando a biblioteca não abre.
fn fontes_coloridas() -> Option<Vec<PathBuf>> {
    #[repr(C)]
    struct FcFontSet {
        nfont: c_int,
        sfont: c_int,
        fonts: *mut *mut c_void,
    }
    type Carregar = unsafe extern "C" fn() -> *mut c_void;
    type Criar = unsafe extern "C" fn() -> *mut c_void;
    type PadraoBool = unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_int;
    type Acrescentar = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
    type Listar = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut FcFontSet;
    type LerTexto = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut u8) -> c_int;
    type DestruirLista = unsafe extern "C" fn(*mut FcFontSet);
    type Destruir = unsafe extern "C" fn(*mut c_void);

    let fc = Biblioteca::abrir(c"libfontconfig.so.1")?;
    // SAFETY: assinaturas da API pública do fontconfig 2 (FcBool e FcResult
    // são `int`, FcChar8 é `unsigned char`, FcFontSet é {int, int, FcPattern**}
    // desde sempre). Tudo o que é criado aqui é destruído aqui; a configuração
    // é uma instância própria, que não vira a padrão do processo.
    unsafe {
        let carregar: Carregar = fc.simbolo(c"FcInitLoadConfigAndFonts")?;
        let criar_padrao: Criar = fc.simbolo(c"FcPatternCreate")?;
        let padrao_bool: PadraoBool = fc.simbolo(c"FcPatternAddBool")?;
        let criar_campos: Criar = fc.simbolo(c"FcObjectSetCreate")?;
        let acrescentar: Acrescentar = fc.simbolo(c"FcObjectSetAdd")?;
        let listar: Listar = fc.simbolo(c"FcFontList")?;
        let ler_texto: LerTexto = fc.simbolo(c"FcPatternGetString")?;
        let destruir_lista: DestruirLista = fc.simbolo(c"FcFontSetDestroy")?;
        let destruir_campos: Destruir = fc.simbolo(c"FcObjectSetDestroy")?;
        let destruir_padrao: Destruir = fc.simbolo(c"FcPatternDestroy")?;
        let destruir_config: Destruir = fc.simbolo(c"FcConfigDestroy")?;

        let config = carregar();
        if config.is_null() {
            return None;
        }
        let padrao = criar_padrao();
        let campos = criar_campos();
        let mut arquivos = Vec::new();
        if !padrao.is_null()
            && !campos.is_null()
            && padrao_bool(padrao, c"color".as_ptr(), 1) != 0
            && acrescentar(campos, c"file".as_ptr()) != 0
        {
            let lista = listar(config, padrao, campos);
            if !lista.is_null() {
                let n = usize::try_from((*lista).nfont).unwrap_or(0);
                for i in 0..n {
                    let fonte = *(*lista).fonts.add(i);
                    let mut texto: *mut u8 = std::ptr::null_mut();
                    // FcResultMatch = 0.
                    if ler_texto(fonte, c"file".as_ptr(), 0, &mut texto) == 0 && !texto.is_null() {
                        let bytes = CStr::from_ptr(texto.cast()).to_bytes();
                        arquivos.push(PathBuf::from(OsStr::from_bytes(bytes)));
                    }
                }
                destruir_lista(lista);
            }
        }
        if !campos.is_null() {
            destruir_campos(campos);
        }
        if !padrao.is_null() {
            destruir_padrao(padrao);
        }
        destruir_config(config);
        arquivos.sort();
        arquivos.dedup();
        Some(arquivos)
    }
}

fn fontes_colrv1(arquivos: &[PathBuf]) -> Vec<PathBuf> {
    arquivos
        .iter()
        .filter(|p| {
            std::fs::File::open(p)
                .ok()
                .is_some_and(|mut f| esconder_colorida(&mut f))
        })
        .cloned()
        .collect()
}

/// Uma fonte colorida (a lista já vem filtrada pelo fontconfig) que o WebKit
/// não deve ver: alguma face com `COLR` versão 1 ou maior. Formato que não dá
/// para ler sem descomprimir (WOFF, WOFF2) fica escondido — perder a cor de
/// uma fonte assim custa menos que uma webview que cai.
fn esconder_colorida<F: Read + Seek>(f: &mut F) -> bool {
    let Some(assinatura) = ler_u32(f, 0) else {
        return false;
    };
    let faces: Vec<u64> = match &assinatura.to_be_bytes() {
        b"ttcf" => {
            let Some(n) = ler_u32(f, 8) else {
                return false;
            };
            (0..n.min(256))
                .filter_map(|i| ler_u32(f, 12 + 4 * u64::from(i)).map(u64::from))
                .collect()
        }
        [0, 1, 0, 0] | b"OTTO" | b"true" => vec![0],
        _ => return true,
    };
    faces
        .into_iter()
        .any(|face| versao_do_colr(f, face).is_some_and(|v| v >= 1))
}

fn versao_do_colr<F: Read + Seek>(f: &mut F, face: u64) -> Option<u16> {
    let tabelas = ler_u16(f, face + 4)?;
    for i in 0..u64::from(tabelas.min(512)) {
        let registro = face + 12 + 16 * i;
        if ler_u32(f, registro)? == u32::from_be_bytes(*b"COLR") {
            let offset = ler_u32(f, registro + 8)?;
            return ler_u16(f, u64::from(offset));
        }
    }
    None
}

fn ler_u32<F: Read + Seek>(f: &mut F, pos: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    f.seek(SeekFrom::Start(pos)).ok()?;
    f.read_exact(&mut b).ok()?;
    Some(u32::from_be_bytes(b))
}

fn ler_u16<F: Read + Seek>(f: &mut F, pos: u64) -> Option<u16> {
    let mut b = [0u8; 2];
    f.seek(SeekFrom::Start(pos)).ok()?;
    f.read_exact(&mut b).ok()?;
    Some(u16::from_be_bytes(b))
}

/// O que o `<include>` do arquivo gerado deve carregar: a configuração que o
/// fontconfig da pessoa carregaria. O `FONTCONFIG_FILE` dela vai como está —
/// o fontconfig resolve `~` e nome relativo (pelo `FONTCONFIG_PATH`) no
/// `<include>` pela mesma regra —, desde que dê para conferir que existe.
/// `None` quando não dá: um `fonts.conf` que não carrega nada deixaria o app
/// sem fonte alguma, e trocar pela do sistema mudaria a configuração dela.
fn configuracao_base(original: Option<&OsStr>) -> Option<OsString> {
    let pastas_do_fontconfig = || {
        std::env::var_os("FONTCONFIG_PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain([PathBuf::from("/etc/fonts")])
    };
    let e_nosso = |p: &Path| {
        p.file_name()
            .is_some_and(|n| n.as_bytes().starts_with(PREFIXO.as_bytes()))
    };
    if let Some(original) = original.filter(|o| !o.is_empty() && !e_nosso(Path::new(o))) {
        let caminho = Path::new(original);
        let existe = if let Ok(resto) = caminho.strip_prefix("~") {
            std::env::var_os("HOME").is_some_and(|h| Path::new(&h).join(resto).is_file())
        } else if caminho.is_absolute() {
            caminho.is_file()
        } else {
            pastas_do_fontconfig().any(|d| d.join(caminho).is_file())
        };
        return existe.then(|| original.to_os_string());
    }
    pastas_do_fontconfig()
        .map(|d| d.join("fonts.conf"))
        .find(|p| p.is_absolute() && p.is_file())
        .map(PathBuf::into_os_string)
}

/// O `fonts.conf` gerado. `None` se a base não é UTF-8 (não dá para
/// escrevê-la no XML sem mudar o caminho).
fn fonts_conf(base: &OsStr, rejeitadas: &[PathBuf]) -> Option<String> {
    let base = xml(base.to_str()?);
    let mut s = String::from(
        "<?xml version=\"1.0\"?>\n\
         <!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
         <!-- Gerado pelo OpenWeights a cada abertura: a configuração da pessoa\n     \
         sem as fontes COLRv1, que o WebKit do AppImage não desenha com o\n     \
         FreeType deste sistema. Os programas abertos pelo app não a veem. -->\n\
         <fontconfig>\n",
    );
    s.push_str(&format!(
        "  <include ignore_missing=\"no\">{base}</include>\n  <selectfont>\n    <rejectfont>\n"
    ));
    for p in rejeitadas {
        s.push_str(&format!("      <glob>{}</glob>\n", glob(p.as_os_str())));
    }
    s.push_str("    </rejectfont>\n  </selectfont>\n</fontconfig>\n");
    Some(s)
}

fn xml(valor: &str) -> String {
    let mut s = String::with_capacity(valor.len());
    for c in valor.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            c => s.push(c),
        }
    }
    s
}

/// O caminho como `<glob>` que casa com ele. Os curingas do fontconfig (`*` e
/// `?`) e os bytes que não são UTF-8 viram `?`, que casa com exatamente um
/// byte: no pior caso se esconde também uma fonte vizinha que só difere
/// naquela posição.
fn glob(caminho: &OsStr) -> String {
    let mut s = String::new();
    for pedaco in caminho.as_bytes().utf8_chunks() {
        s.push_str(&xml(&pedaco.valid().replace(['*', '?'], "?")));
        s.extend(std::iter::repeat_n('?', pedaco.invalid().len()));
    }
    s
}

/// Onde gravar: a pasta de runtime da pessoa (dela, 0700, como a
/// especificação XDG exige — conferido), senão o cache. Em cada uma, uma
/// subpasta `openweights` só dela.
fn pastas_candidatas() -> Vec<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| {
            let aceita = p.is_absolute() && e_so_da_pessoa(p);
            if !aceita {
                log::info!(
                    "XDG_RUNTIME_DIR {} não é só desta conta (0700, dono, sem symlink): fica o cache",
                    p.display()
                );
            }
            aceita
        });
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"));
    [runtime, cache, home]
        .into_iter()
        .flatten()
        .filter(|p| p.is_absolute())
        .map(|p| p.join("openweights"))
        .collect()
}

/// Diretório de verdade (não symlink), do usuário do processo, sem acesso
/// para grupo e outros.
fn e_so_da_pessoa(p: &Path) -> bool {
    // SAFETY: `geteuid` não tem pré-condição.
    let euid = unsafe { libc::geteuid() };
    std::fs::symlink_metadata(p)
        .is_ok_and(|m| m.is_dir() && m.uid() == euid && m.mode() & 0o077 == 0)
}

/// Grava o `fonts.conf` na primeira pasta que aceitar. `Err` junta o motivo
/// de cada pasta recusada.
fn escrever(conteudo: &str) -> Result<PathBuf, String> {
    let mut motivos = Vec::new();
    for pasta in pastas_candidatas() {
        match gravar_em(&pasta, conteudo) {
            Ok(destino) => return Ok(destino),
            Err(e) => motivos.push(format!("{}: {e}", pasta.display())),
        }
    }
    Err(if motivos.is_empty() {
        "nenhuma pasta candidata".to_string()
    } else {
        motivos.join("; ")
    })
}

/// O nome do arquivo para um conteúdo: duas instâncias do app abertas com
/// configurações diferentes não trocam o arquivo uma da outra (os processos
/// novos do WebKit de cada uma releem o dela), e aberturas iguais caem no
/// mesmo arquivo.
fn nome_para(conteudo: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    conteudo.hash(&mut h);
    format!("{PREFIXO}-{:016x}.conf", h.finish())
}

/// Temporário criado novo (não segue symlink) e trocado de uma vez, para
/// uma abertura nunca ler o arquivo pela metade.
fn gravar_em(pasta: &Path, conteudo: &str) -> std::io::Result<PathBuf> {
    // O `~/.cache` de uma conta nova pode não existir: a mãe nasce com as
    // permissões de sempre, só a `openweights` é fechada.
    if let Some(mae) = pasta.parent() {
        std::fs::create_dir_all(mae)?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(pasta) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    if !e_so_da_pessoa(pasta) {
        // Nossa, mas aberta demais (uma umask larga): fecha e confere de novo.
        // SAFETY: `geteuid` não tem pré-condição.
        let euid = unsafe { libc::geteuid() };
        if std::fs::symlink_metadata(pasta).is_ok_and(|m| m.is_dir() && m.uid() == euid) {
            std::fs::set_permissions(pasta, std::fs::Permissions::from_mode(0o700))?;
        }
        if !e_so_da_pessoa(pasta) {
            return Err(std::io::Error::other(
                "pasta de outro usuário ou aberta a outros",
            ));
        }
    }
    let destino = pasta.join(nome_para(conteudo));
    let temporario = pasta.join(format!("{PREFIXO}-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temporario);
    let gravado = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporario)
        .and_then(|mut f| f.write_all(conteudo.as_bytes()))
        .and_then(|()| std::fs::rename(&temporario, &destino));
    if gravado.is_err() {
        let _ = std::fs::remove_file(&temporario);
    }
    gravado.map(|()| destino)
}

/// O `FONTCONFIG_FILE` que um programa do sistema aberto pelo app deve ver:
/// `Some(Some(v))` devolve o da pessoa, `Some(None)` remove (ela não tinha),
/// `None` quando o app não trocou nada.
pub(crate) fn fontconfig_da_pessoa(guardado: Option<OsString>) -> Option<Option<OsString>> {
    guardado.map(|v| (!v.is_empty()).then_some(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Uma fonte mínima: só o diretório de tabelas e, se pedido, uma `COLR`
    /// com a versão dada.
    fn fonte(colr: Option<u16>) -> Vec<u8> {
        let tabelas: Vec<(&[u8; 4], Vec<u8>)> = match colr {
            Some(v) => vec![(b"cmap", vec![0; 4]), (b"COLR", v.to_be_bytes().to_vec())],
            None => vec![(b"cmap", vec![0; 4])],
        };
        let mut d = Vec::new();
        d.extend(0x0001_0000u32.to_be_bytes());
        d.extend((tabelas.len() as u16).to_be_bytes());
        d.extend([0u8; 6]);
        let mut dados_em = 12 + 16 * tabelas.len();
        let mut corpo: Vec<u8> = Vec::new();
        for (tag, conteudo) in &tabelas {
            d.extend(*tag);
            d.extend(0u32.to_be_bytes());
            d.extend((dados_em as u32).to_be_bytes());
            d.extend((conteudo.len() as u32).to_be_bytes());
            dados_em += conteudo.len();
            corpo.extend(conteudo);
        }
        d.extend(corpo);
        d
    }

    /// Uma coleção `.ttc` com as faces dadas, uma atrás da outra.
    fn colecao(faces: &[Vec<u8>]) -> Vec<u8> {
        let cabecalho = 12 + 4 * faces.len();
        let mut d = Vec::new();
        d.extend(b"ttcf");
        d.extend(0x0001_0000u32.to_be_bytes());
        d.extend((faces.len() as u32).to_be_bytes());
        let mut pos = cabecalho;
        let mut corpo = Vec::new();
        for face in faces {
            d.extend((pos as u32).to_be_bytes());
            // Os offsets de uma face numa coleção contam do começo do arquivo.
            let mut f = face.clone();
            let n = u16::from_be_bytes([f[4], f[5]]) as usize;
            for i in 0..n {
                let r = 12 + 16 * i + 8;
                let off = u32::from_be_bytes(f[r..r + 4].try_into().unwrap()) + pos as u32;
                f[r..r + 4].copy_from_slice(&off.to_be_bytes());
            }
            pos += f.len();
            corpo.extend(f);
        }
        d.extend(corpo);
        d
    }

    #[test]
    fn so_a_colr_versao_1_conta() {
        assert!(esconder_colorida(&mut Cursor::new(fonte(Some(1)))));
        assert!(
            !esconder_colorida(&mut Cursor::new(fonte(Some(0)))),
            "COLRv0 o WebKit desenha"
        );
        assert!(!esconder_colorida(&mut Cursor::new(fonte(None))));
    }

    #[test]
    fn numa_colecao_basta_uma_face_com_colr_v1() {
        let c = colecao(&[fonte(None), fonte(Some(1))]);
        assert!(esconder_colorida(&mut Cursor::new(c)));
        let c = colecao(&[fonte(None), fonte(Some(0))]);
        assert!(!esconder_colorida(&mut Cursor::new(c)));
    }

    #[test]
    fn woff_colorida_fica_escondida_sem_ler() {
        for assinatura in [b"wOFF", b"wOF2"] {
            let mut d = assinatura.to_vec();
            d.extend([0u8; 40]);
            assert!(esconder_colorida(&mut Cursor::new(d)));
        }
    }

    #[test]
    fn arquivo_truncado_ou_estranho_nao_derruba() {
        let inteira = fonte(Some(1));
        for corte in 0..inteira.len() {
            let _ = esconder_colorida(&mut Cursor::new(inteira[..corte].to_vec()));
        }
        assert!(!esconder_colorida(&mut Cursor::new(Vec::new())));
        let mut ttc_mentirosa = b"ttcf\0\x01\0\0\xff\xff\xff\xff".to_vec();
        ttc_mentirosa.extend([0u8; 16]);
        assert!(!esconder_colorida(&mut Cursor::new(ttc_mentirosa)));
    }

    #[test]
    fn so_esconde_quando_o_freetype_do_sistema_nao_e_o_do_webkit() {
        let (a, b) = FREETYPE_DO_WEBKIT;
        assert!(!precisa_esconder(Some((a, b, 1))));
        assert!(
            !precisa_esconder(Some((a, b, 3))),
            "patch não muda o layout"
        );
        assert!(precisa_esconder(Some((2, 14, 3))), "Fedora 44");
        assert!(precisa_esconder(Some((2, 13, 2))), "Ubuntu 24.04");
        assert!(precisa_esconder(None), "sem saber, esconde");
    }

    #[test]
    fn o_fonts_conf_inclui_a_configuracao_da_pessoa_e_so_rejeita_as_colrv1() {
        let conf = fonts_conf(
            OsStr::new("~/.config/R&D/fonts.conf"),
            &[
                PathBuf::from("/usr/share/fonts/Noto-COLRv1.ttf"),
                PathBuf::from("/home/p/.fonts/R&D <x> 'a' \"b\" *?.ttf"),
            ],
        )
        .unwrap();
        assert!(
            conf.contains("<include ignore_missing=\"no\">~/.config/R&amp;D/fonts.conf</include>"),
            "a base vai como está, só com o escape de XML: {conf}"
        );
        assert!(conf.contains("<glob>/usr/share/fonts/Noto-COLRv1.ttf</glob>"));
        assert!(
            conf.contains(
                "<glob>/home/p/.fonts/R&amp;D &lt;x&gt; &apos;a&apos; &quot;b&quot; ??.ttf</glob>"
            ),
            "{conf}"
        );
        assert_eq!(conf.matches("<glob>").count(), 2);
    }

    #[test]
    fn caminho_que_nao_e_utf8_vira_curinga_de_um_byte() {
        let caminho = OsStr::from_bytes(b"/fontes/emo\xffji.ttf");
        assert_eq!(glob(caminho), "/fontes/emo?ji.ttf");
        assert_eq!(
            fonts_conf(OsStr::from_bytes(b"/etc/\xff.conf"), &[]),
            None,
            "base que não é UTF-8: nada de trocar"
        );
    }

    #[test]
    fn a_base_nunca_e_o_proprio_arquivo_gerado() {
        let dir = std::env::temp_dir().join(format!("ow-fontes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let nosso = dir.join(nome_para("x"));
        std::fs::write(&nosso, "x").unwrap();
        let base = configuracao_base(Some(nosso.as_os_str()));
        assert_ne!(base.as_deref(), Some(nosso.as_os_str()));
        let dela = dir.join("dela.conf");
        std::fs::write(&dela, "x").unwrap();
        assert_eq!(
            configuracao_base(Some(dela.as_os_str())).as_deref(),
            Some(dela.as_os_str())
        );
        assert_eq!(
            configuracao_base(Some(dir.join("sumiu.conf").as_os_str())),
            None,
            "a da pessoa não existe: nada de trocar pela do sistema"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grava_so_em_pasta_da_pessoa_e_sem_seguir_symlink() {
        let raiz = std::env::temp_dir().join(format!("ow-gravar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        std::fs::create_dir_all(&raiz).unwrap();
        let pasta = raiz.join("openweights");
        let destino = gravar_em(&pasta, "conteudo").unwrap();
        assert_eq!(std::fs::read_to_string(&destino).unwrap(), "conteudo");
        assert_eq!(std::fs::metadata(&pasta).unwrap().mode() & 0o777, 0o700);

        // Um temporário que já exista como symlink é removido, não seguido.
        let alvo = raiz.join("alvo");
        std::fs::write(&alvo, "intacto").unwrap();
        let temporario = pasta.join(format!("{PREFIXO}-{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(&alvo, &temporario).unwrap();
        let outro = gravar_em(&pasta, "de novo").unwrap();
        assert_eq!(std::fs::read_to_string(&alvo).unwrap(), "intacto");
        assert_eq!(std::fs::read_to_string(&outro).unwrap(), "de novo");

        // Outra instância com outra configuração não troca o arquivo desta;
        // a mesma configuração cai no mesmo arquivo.
        assert_ne!(outro, destino);
        assert_eq!(std::fs::read_to_string(&destino).unwrap(), "conteudo");
        assert_eq!(gravar_em(&pasta, "conteudo").unwrap(), destino);

        // Sem a pasta-mãe (o ~/.cache de uma conta nova), ela é criada.
        let funda = raiz.join("sem-cache").join("openweights");
        gravar_em(&funda, "x").unwrap();
        assert_eq!(std::fs::metadata(&funda).unwrap().mode() & 0o777, 0o700);

        // Pasta nossa aberta a outros é fechada antes de gravar.
        let aberta = raiz.join("aberta");
        std::fs::create_dir(&aberta).unwrap();
        std::fs::set_permissions(&aberta, std::fs::Permissions::from_mode(0o755)).unwrap();
        gravar_em(&aberta, "x").unwrap();
        assert_eq!(std::fs::metadata(&aberta).unwrap().mode() & 0o777, 0o700);

        // Symlink no lugar da pasta é recusado, mesmo apontando para uma nossa.
        let atalho = raiz.join("atalho");
        std::os::unix::fs::symlink(&pasta, &atalho).unwrap();
        assert!(gravar_em(&atalho, "x").is_err());
        std::fs::remove_dir_all(&raiz).unwrap();
    }

    #[test]
    fn o_fontconfig_do_sistema_responde_em_processo() {
        // Onde o GTK roda, a libfontconfig existe; a lista pode vir vazia.
        if Biblioteca::abrir(c"libfontconfig.so.1").is_some() {
            assert!(fontes_coloridas().is_some());
        }
    }

    #[test]
    fn o_filho_recebe_o_fontconfig_da_pessoa() {
        assert_eq!(fontconfig_da_pessoa(None), None, "o app não trocou nada");
        assert_eq!(fontconfig_da_pessoa(Some(OsString::new())), Some(None));
        assert_eq!(
            fontconfig_da_pessoa(Some(OsString::from("/home/p/fonts.conf"))),
            Some(Some(OsString::from("/home/p/fonts.conf")))
        );
    }
}
