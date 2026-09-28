//! O ambiente do sistema para quem o app sobe de dentro de um AppImage.
//!
//! O AppRun do AppImage (o `AppRun.c` do AppImageKit mais o hook do
//! `linuxdeploy-plugin-gtk`) põe os diretórios de dentro do `$APPDIR` na
//! FRENTE de `PATH`, `LD_LIBRARY_PATH`, `XDG_DATA_DIRS` e companhia, aponta
//! `PYTHONHOME`, `GTK_PATH`, `GDK_PIXBUF_MODULE_FILE`… para lá e roda o app de
//! dentro de `$APPDIR/usr`. É o que o próprio app precisa para achar a glib e
//! o WebKit que vêm embutidos — e é veneno para qualquer programa DO SISTEMA
//! que um filho nosso execute:
//!
//! - `zenity` carrega a `libjson-glib` do sistema contra a glib 2.72
//!   embutida e morre com `undefined symbol: g_once_init_leave_pointer` (o
//!   "Escolher workspace" do AgenticOw);
//! - `python3` procura a biblioteca-padrão em `$APPDIR/usr/` e morre com
//!   "Failed to import encodings module";
//! - `curl`, `gio` e o `xdg-open` de dentro do pacote quebram do mesmo jeito.
//!
//! A regra: programa que mora dentro do `$APPDIR` continua com o ambiente do
//! pacote; todo o resto recebe o ambiente que a pessoa tinha antes de abrir o
//! app. As listas perdem as entradas de dentro do pacote (o que sobra é o
//! valor original, que o AppRun só prefixou), as variáveis que só existem por
//! causa dele saem inteiras e o filho sem diretório próprio volta para onde a
//! pessoa estava. O processo do app não é tocado: o WebKit acha os helpers
//! dele por caminhos relativos a `$APPDIR/usr` e precisa daquele ambiente.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// Listas de diretórios que o AppRun prefixa com caminhos do `$APPDIR`.
const LISTAS: [&str; 12] = [
    "PATH",
    "LD_LIBRARY_PATH",
    "PYTHONPATH",
    "PERLLIB",
    "QT_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "XDG_DATA_DIRS",
    "GSETTINGS_SCHEMA_DIR",
    "GI_TYPELIB_PATH",
    "GTK_PATH",
    "GIO_EXTRA_MODULES",
];

/// Caminhos únicos que o AppRun e o hook do GTK apontam para o `$APPDIR`.
/// Saem só quando apontam para lá — valor da pessoa fica.
const CAMINHOS: [&str; 5] = [
    "PYTHONHOME",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
];

/// As marcas do runtime do AppImage. Um programa do sistema que as herde se
/// acha empacotado (e um AppImage aberto pelo agente se confunde com o nosso).
const MARCAS: [&str; 4] = ["APPDIR", "APPIMAGE", "ARGV0", "OWD"];

/// Onde o pacote está montado.
pub(crate) struct Pacote {
    /// O `$APPDIR` cru e o canônico, quando diferem: o hook do GTK escreve o
    /// valor que o runtime exportou; o `AppRun.c`, o `realpath` dele.
    raizes: Vec<PathBuf>,
}

impl Pacote {
    /// `p` é do nosso AppImage — ou de outro montado ao lado dele: o mount
    /// da versão anterior, cujas entradas o `app.restart()` de uma
    /// atualização carrega para o processo novo, ou o de um AppImage que
    /// abriu o app.
    fn contem(&self, p: &Path) -> bool {
        self.raizes.iter().any(|raiz| {
            if p.starts_with(raiz) {
                return true;
            }
            let (Some(pai), Some(nome)) = (raiz.parent(), raiz.file_name()) else {
                return false;
            };
            e_mount(nome)
                && p.strip_prefix(pai).is_ok_and(|resto| {
                    matches!(resto.components().next(), Some(Component::Normal(n)) if e_mount(n))
                })
        })
    }
}

/// O runtime do AppImage monta em `$TMPDIR/.mount_<nome><sufixo>`. Pacote
/// extraído (`squashfs-root`) não tem irmãos a considerar.
fn e_mount(nome: &OsStr) -> bool {
    nome.to_string_lossy().starts_with(".mount_")
}

/// O pacote quando o app roda de dentro de um AppImage; `None` fora dele.
///
/// Não basta o `APPDIR` existir: o executável atual tem de morar dentro
/// dele. Assim um `APPDIR` que a pessoa exporte por outro motivo não faz o
/// app sair podando o ambiente de ninguém.
fn pacote() -> Option<&'static Pacote> {
    static PACOTE: OnceLock<Option<Pacote>> = OnceLock::new();
    PACOTE
        .get_or_init(|| {
            let cru = PathBuf::from(std::env::var_os("APPDIR")?);
            if !cru.is_absolute() {
                return None;
            }
            let canonico = std::fs::canonicalize(&cru).ok()?;
            // `/proc/self/exe` já vem canônico.
            if !std::env::current_exe().ok()?.starts_with(&canonico) {
                return None;
            }
            let mut raizes = vec![cru];
            if !raizes.contains(&canonico) {
                raizes.push(canonico);
            }
            Some(Pacote { raizes })
        })
        .as_ref()
}

/// Devolve ao comando o ambiente do sistema, se o app for um AppImage.
///
/// Só mexe no que o filho HERDARIA: variável que o próprio comando define ou
/// remove é decisão de quem o montou (o Studio aponta o `PYTHONPATH` para os
/// recursos, que moram no pacote). Quem monta uma variável a partir da do app
/// usa [`host_var`]. Idempotente. Não usar depois de `env_clear` — aí quem
/// chamou já decidiu o ambiente inteiro.
pub(crate) fn aplicar(cmd: &mut std::process::Command) {
    let Some(pacote) = pacote() else {
        return;
    };
    aplicar_com(
        cmd,
        pacote,
        |n| std::env::var_os(n),
        std::env::current_dir().ok(),
    );
}

fn aplicar_com(
    cmd: &mut std::process::Command,
    pacote: &Pacote,
    herdado: impl Fn(&str) -> Option<OsString>,
    cwd: Option<PathBuf>,
) {
    if pacote.contem(Path::new(cmd.get_program())) {
        return;
    }
    let explicitos: Vec<OsString> = cmd.get_envs().map(|(k, _)| k.to_os_string()).collect();
    let ler = |nome: &str| {
        if explicitos.iter().any(|k| k == nome) {
            None
        } else {
            herdado(nome)
        }
    };
    for (nome, valor) in correcoes(pacote, &ler) {
        match valor {
            Some(v) => cmd.env(nome, v),
            None => cmd.env_remove(nome),
        };
    }
    if cmd.get_current_dir().is_none()
        && let Some(dir) = diretorio_de_trabalho(pacote, cwd, &herdado)
    {
        cmd.current_dir(dir);
    }
}

/// O valor que `nome` teria para um programa do sistema, visto do app.
///
/// Para quem MONTA uma variável de filho a partir da do app (um `PATH` com
/// um diretório a mais na frente). Fora de um AppImage é o `var_os` puro.
pub(crate) fn host_var(nome: &str) -> Option<OsString> {
    let valor = std::env::var_os(nome);
    let Some(pacote) = pacote() else {
        return valor;
    };
    match correcoes(pacote, &|n| std::env::var_os(n))
        .into_iter()
        .find(|(n, _)| *n == nome)
    {
        Some((_, corrigido)) => corrigido,
        None => valor,
    }
}

/// O que muda para um programa do sistema: `Some` define, `None` remove.
/// Variável que já está certa não aparece.
fn correcoes(
    pacote: &Pacote,
    ler: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, Option<OsString>)> {
    let mut saida = Vec::new();

    for nome in LISTAS {
        let Some(valor) = ler(nome) else { continue };
        if let Some(limpo) = lista_limpa(pacote, nome, &valor) {
            saida.push((nome, limpo));
        }
    }

    for nome in CAMINHOS {
        if ler(nome).is_some_and(|v| pacote.contem(Path::new(&v))) {
            saida.push((nome, None));
        }
    }

    for nome in MARCAS {
        if ler(nome).is_some() {
            saida.push((nome, None));
        }
    }

    // O AppRun e o hook do GTK forçam estes sem guardar o valor de antes; o
    // que dá para fazer é não repassar o que foi escrito por eles.
    // `GDK_BACKEND=x11` existe pelo WebKit do app (tauri#8541) e faria o
    // `zenity` de um desktop Wayland subir pelo XWayland.
    if ler("GDK_BACKEND").is_some_and(|v| v == "x11") {
        saida.push(("GDK_BACKEND", None));
    }
    if ler("PYTHONDONTWRITEBYTECODE").is_some_and(|v| v == "1") {
        saida.push(("PYTHONDONTWRITEBYTECODE", None));
    }
    if ler("GTK_THEME").is_some_and(|v| tema_do_hook(&v, ler("APPIMAGE_GTK_THEME"))) {
        saida.push(("GTK_THEME", None));
    }

    saida
}

/// O `GTK_THEME` que o hook escreve: o `APPIMAGE_GTK_THEME` da pessoa, ou
/// `Adwaita:dark|light` conforme o esquema de cores do sistema.
fn tema_do_hook(valor: &OsStr, appimage_gtk_theme: Option<OsString>) -> bool {
    match appimage_gtk_theme {
        Some(t) => valor == t,
        None => valor == "Adwaita:dark" || valor == "Adwaita:light",
    }
}

/// A lista sem o que veio do pacote. `None` quando nada veio dele (a lista
/// da pessoa fica exatamente como está); `Some(None)` quando não sobra nada.
///
/// Entradas vazias saem junto: o AppRun deixa um `:` sobrando quando a
/// variável não existia — e entrada vazia no `PATH` ou no `PYTHONPATH`
/// significa "o diretório atual".
fn lista_limpa(pacote: &Pacote, nome: &str, valor: &OsStr) -> Option<Option<OsString>> {
    let mut entradas: Vec<PathBuf> = Vec::new();
    let mut mudou = false;
    let mut anterior_e_do_hook = false;
    for entrada in std::env::split_paths(valor) {
        let do_pacote = pacote.contem(&entrada);
        // O hook do GTK escreve `XDG_DATA_DIRS="$APPDIR/usr/share:/usr/share:…"`:
        // esse `/usr/share` colado no `$APPDIR/usr/share` é dele, não da
        // pessoa. O AppRun.c escreve `$APPDIR/usr/share/` — COM barra —, e o
        // que vem depois dele já é a lista original (a de um AppImage que
        // abriu o app pode começar pelo `/usr/share` da pessoa).
        let do_hook = nome == "XDG_DATA_DIRS"
            && anterior_e_do_hook
            && entrada
                .components()
                .eq(Path::new("/usr/share").components());
        anterior_e_do_hook = do_pacote
            && entrada
                .as_os_str()
                .as_encoded_bytes()
                .ends_with(b"/usr/share");
        if do_pacote || do_hook {
            mudou = true;
            continue;
        }
        entradas.push(entrada);
    }
    if !mudou {
        return None;
    }
    entradas.retain(|e| !e.as_os_str().is_empty());
    if entradas.is_empty() {
        return Some(None);
    }
    Some(std::env::join_paths(entradas).ok())
}

/// Para onde vai o filho que não escolheu diretório. O AppRun roda o app de
/// dentro de `$APPDIR/usr`; um filho que herde isso segura o mount (um
/// gerenciador de arquivos aberto pelo `xdg-open` sobrevive ao app) ou fica
/// num diretório que some quando o app fecha. Volta para onde a pessoa
/// estava ao abrir o app (`OWD`), ou para a casa dela.
fn diretorio_de_trabalho(
    pacote: &Pacote,
    cwd: Option<PathBuf>,
    herdado: &dyn Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    if !pacote.contem(&cwd?) {
        return None;
    }
    [herdado("OWD"), herdado("HOME")]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|d| d.is_absolute() && !pacote.contem(d) && d.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const APPDIR: &str = "/tmp/.mount_OpenWedjFafF";

    fn pacote_de_teste() -> Pacote {
        Pacote {
            raizes: vec![PathBuf::from(APPDIR)],
        }
    }

    /// O ambiente de verdade do runtime do AgenticOw, copiado do
    /// `/proc/<pid>/environ` do app 0.24.1 num Fedora Silverblue 44.
    fn ambiente_do_appimage() -> HashMap<&'static str, String> {
        let a = APPDIR;
        HashMap::from([
            ("APPDIR", a.to_string()),
            (
                "APPIMAGE",
                "/var/home/pedro/.local/bin/OpenWeights.AppImage".to_string(),
            ),
            (
                "ARGV0",
                "/var/home/pedro/.local/bin/OpenWeights.AppImage".to_string(),
            ),
            ("OWD", "/var/home/pedro".to_string()),
            (
                "PATH",
                format!(
                    "{a}/usr/bin/:{a}/usr/sbin/:{a}/usr/games/:{a}/bin/:{a}/sbin/:\
                 /var/home/pedro/.local/bin:/usr/local/bin:/usr/bin"
                ),
            ),
            (
                "LD_LIBRARY_PATH",
                format!(
                    "{a}/usr/lib/:{a}/usr/lib/i386-linux-gnu/:{a}/usr/lib/x86_64-linux-gnu/:\
                 {a}/usr/lib32/:{a}/usr/lib64/:{a}/lib/:{a}/lib/i386-linux-gnu/:\
                 {a}/lib/x86_64-linux-gnu/:{a}/lib32/:{a}/lib64/:"
                ),
            ),
            ("PYTHONHOME", format!("{a}/usr/")),
            ("PYTHONPATH", format!("{a}/usr/share/pyshared/:")),
            (
                "PERLLIB",
                format!("{a}/usr/share/perl5/:{a}/usr/lib/perl5/:"),
            ),
            (
                "QT_PLUGIN_PATH",
                format!("{a}/usr/lib/qt4/plugins/:{a}/usr/lib/qt5/plugins/:"),
            ),
            ("GST_PLUGIN_SYSTEM_PATH", format!("{a}/usr/lib/gstreamer:")),
            (
                "GST_PLUGIN_SYSTEM_PATH_1_0",
                format!("{a}/usr/lib/gstreamer-1.0:"),
            ),
            (
                "XDG_DATA_DIRS",
                format!(
                    "{a}/usr/share/:{a}/usr/share:/usr/share:\
                 /var/home/pedro/.local/share/flatpak/exports/share:\
                 /var/lib/flatpak/exports/share:/usr/local/share/:/usr/share/"
                ),
            ),
            (
                "GSETTINGS_SCHEMA_DIR",
                format!("{a}/usr/share/glib-2.0/schemas/:{a}//usr/share/glib-2.0/schemas"),
            ),
            ("GI_TYPELIB_PATH", format!("{a}//usr/lib/girepository-1.0")),
            ("GTK_PATH", format!("{a}//usr/lib/gtk-3.0")),
            (
                "GIO_EXTRA_MODULES",
                format!("{a}/usr/lib/x86_64-linux-gnu/gio/modules"),
            ),
            ("GTK_DATA_PREFIX", a.to_string()),
            ("GTK_EXE_PREFIX", format!("{a}//usr")),
            (
                "GTK_IM_MODULE_FILE",
                format!("{a}//usr/lib/gtk-3.0/3.0.0/immodules.cache"),
            ),
            (
                "GDK_PIXBUF_MODULE_FILE",
                format!("{a}//usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"),
            ),
            ("GTK_THEME", "Adwaita:dark".to_string()),
            ("GDK_BACKEND", "x11".to_string()),
            ("PYTHONDONTWRITEBYTECODE", "1".to_string()),
            ("HOME", "/var/home/pedro".to_string()),
            ("WAYLAND_DISPLAY", "wayland-0".to_string()),
        ])
    }

    /// Aplica as correções sobre o mapa, como o `Command` faria no filho.
    fn corrigir(amb: &HashMap<&'static str, String>) -> HashMap<String, String> {
        let mut saida: HashMap<String, String> = amb
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        for (nome, valor) in correcoes(&pacote_de_teste(), &|n| amb.get(n).map(OsString::from)) {
            match valor {
                Some(v) => saida.insert(nome.to_string(), v.into_string().unwrap()),
                None => saida.remove(nome),
            };
        }
        saida
    }

    #[test]
    fn a_program_from_the_system_sees_the_environment_from_before_the_appimage() {
        let filho = corrigir(&ambiente_do_appimage());

        assert_eq!(
            filho["PATH"],
            "/var/home/pedro/.local/bin:/usr/local/bin:/usr/bin"
        );
        assert_eq!(
            filho["XDG_DATA_DIRS"],
            "/var/home/pedro/.local/share/flatpak/exports/share:\
             /var/lib/flatpak/exports/share:/usr/local/share/:/usr/share/",
            "o /usr/share do hook sai; a ordem da pessoa fica"
        );
        for so_do_pacote in [
            "LD_LIBRARY_PATH",
            "PYTHONHOME",
            "PYTHONPATH",
            "PERLLIB",
            "QT_PLUGIN_PATH",
            "GST_PLUGIN_SYSTEM_PATH",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GSETTINGS_SCHEMA_DIR",
            "GI_TYPELIB_PATH",
            "GTK_PATH",
            "GIO_EXTRA_MODULES",
            "GTK_DATA_PREFIX",
            "GTK_EXE_PREFIX",
            "GTK_IM_MODULE_FILE",
            "GDK_PIXBUF_MODULE_FILE",
            "GTK_THEME",
            "GDK_BACKEND",
            "PYTHONDONTWRITEBYTECODE",
            "APPDIR",
            "APPIMAGE",
            "ARGV0",
            "OWD",
        ] {
            assert!(
                !filho.contains_key(so_do_pacote),
                "{so_do_pacote} só existia por causa do AppImage: {:?}",
                filho.get(so_do_pacote)
            );
        }
        assert_eq!(filho["HOME"], "/var/home/pedro");
        assert_eq!(filho["WAYLAND_DISPLAY"], "wayland-0");
    }

    #[test]
    fn what_the_person_had_before_the_prefix_survives() {
        let mut amb = ambiente_do_appimage();
        amb.insert(
            "LD_LIBRARY_PATH",
            format!("{APPDIR}/usr/lib/:/opt/cuda/lib64"),
        );
        amb.insert("PYTHONHOME", "/opt/python".to_string());
        amb.insert("GTK_THEME", "Nordic".to_string());
        amb.insert("GDK_BACKEND", "wayland".to_string());
        amb.insert(
            "XDG_DATA_DIRS",
            format!("{APPDIR}/usr/share/:{APPDIR}/usr/share:/usr/share:/usr/share:/opt/share"),
        );

        let filho = corrigir(&amb);
        assert_eq!(filho["LD_LIBRARY_PATH"], "/opt/cuda/lib64");
        assert_eq!(filho["PYTHONHOME"], "/opt/python");
        assert_eq!(filho["GTK_THEME"], "Nordic");
        assert_eq!(filho["GDK_BACKEND"], "wayland");
        assert_eq!(
            filho["XDG_DATA_DIRS"], "/usr/share:/opt/share",
            "só o /usr/share do hook sai"
        );
    }

    #[test]
    fn a_theme_chosen_through_appimage_gtk_theme_is_the_hooks_too() {
        let mut amb = ambiente_do_appimage();
        amb.insert("APPIMAGE_GTK_THEME", "Yaru-dark".to_string());
        amb.insert("GTK_THEME", "Yaru-dark".to_string());
        assert!(!corrigir(&amb).contains_key("GTK_THEME"));
    }

    #[test]
    fn xdg_data_dirs_that_did_not_exist_goes_away_again() {
        let mut amb = ambiente_do_appimage();
        amb.insert(
            "XDG_DATA_DIRS",
            format!("{APPDIR}/usr/share/:{APPDIR}/usr/share:/usr/share:"),
        );
        assert!(!corrigir(&amb).contains_key("XDG_DATA_DIRS"));
    }

    #[test]
    fn entries_left_by_the_previous_mount_after_an_update_go_too() {
        // O `app.restart()` de uma atualização sobe o AppImage novo com o
        // ambiente do velho; o AppRun novo só prefixa o dele.
        let velho = "/tmp/.mount_OpenWeAbC123";
        let mut amb = ambiente_do_appimage();
        amb.insert(
            "PATH",
            format!("{APPDIR}/usr/bin/:{velho}/usr/bin/:{velho}/sbin/:/usr/bin"),
        );
        amb.insert(
            "XDG_DATA_DIRS",
            format!(
                "{APPDIR}/usr/share/:{APPDIR}/usr/share:/usr/share:\
                 {velho}/usr/share/:{velho}/usr/share:/usr/share:/usr/local/share/:/usr/share/"
            ),
        );
        let filho = corrigir(&amb);
        assert_eq!(filho["PATH"], "/usr/bin");
        assert_eq!(filho["XDG_DATA_DIRS"], "/usr/local/share/:/usr/share/");
    }

    #[test]
    fn the_persons_usr_share_survives_an_appimage_that_opened_the_app() {
        // Um terminal empacotado só com o AppRun.c prefixa `{outro}/usr/share/`
        // (com barra); o `/usr/share` seguinte é o da pessoa.
        let outro = "/tmp/.mount_CursorXyZ12";
        let mut amb = ambiente_do_appimage();
        amb.insert(
            "XDG_DATA_DIRS",
            format!(
                "{APPDIR}/usr/share/:{APPDIR}/usr/share:/usr/share:\
                 {outro}/usr/share/:/usr/share:/opt/share"
            ),
        );
        assert_eq!(corrigir(&amb)["XDG_DATA_DIRS"], "/usr/share:/opt/share");
    }

    #[test]
    fn an_extracted_package_has_no_siblings_and_a_prefix_is_not_a_parent() {
        let pacote = Pacote {
            raizes: vec![PathBuf::from("/opt/ow/squashfs-root")],
        };
        let amb = HashMap::from([(
            "PATH",
            "/opt/ow/squashfs-root/usr/bin:/opt/ow/squashfs-rootX/bin:/tmp/.mount_x/bin:/usr/bin"
                .to_string(),
        )]);
        let c = correcoes(&pacote, &|n| amb.get(n).map(OsString::from));
        assert_eq!(
            c,
            vec![(
                "PATH",
                Some(OsString::from(
                    "/opt/ow/squashfs-rootX/bin:/tmp/.mount_x/bin:/usr/bin"
                ))
            )]
        );
    }

    #[test]
    fn a_list_with_nothing_from_the_package_is_left_exactly_as_it_was() {
        let amb: HashMap<&str, String> = HashMap::from([
            ("PATH", "/usr/local/bin::/usr/bin:".to_string()),
            ("XDG_DATA_DIRS", "/usr/local/share/:/usr/share/".to_string()),
        ]);
        assert!(correcoes(&pacote_de_teste(), &|n| amb.get(n).map(OsString::from)).is_empty());
    }

    #[test]
    fn what_the_command_sets_itself_is_the_callers_decision() {
        let amb = ambiente_do_appimage();
        let mut cmd = std::process::Command::new("node");
        crate::no_window_std(&mut cmd);
        // O Studio aponta o PYTHONPATH para os recursos, que moram no pacote.
        cmd.env("PATH", format!("/data/node/bin:{APPDIR}/usr/bin"))
            .env(
                "PYTHONPATH",
                format!("{APPDIR}/usr/lib/openweights/studio-backend"),
            );

        aplicar_com(
            &mut cmd,
            &pacote_de_teste(),
            |n| amb.get(n).map(OsString::from),
            None,
        );

        let envs: HashMap<String, Option<String>> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into(),
                    v.map(|v| v.to_string_lossy().into()),
                )
            })
            .collect();
        assert_eq!(
            envs["PATH"].as_deref(),
            Some(&*format!("/data/node/bin:{APPDIR}/usr/bin"))
        );
        assert_eq!(
            envs["PYTHONPATH"].as_deref(),
            Some(&*format!("{APPDIR}/usr/lib/openweights/studio-backend"))
        );
        assert_eq!(envs["LD_LIBRARY_PATH"], None, "o herdado é removido");
        assert_eq!(envs["PYTHONHOME"], None);
    }

    #[test]
    fn a_program_inside_the_package_keeps_the_package_environment() {
        let amb = ambiente_do_appimage();
        let mut cmd = std::process::Command::new(format!("{APPDIR}/usr/bin/openweights"));
        crate::no_window_std(&mut cmd);
        aplicar_com(
            &mut cmd,
            &pacote_de_teste(),
            |n| amb.get(n).map(OsString::from),
            None,
        );
        assert_eq!(cmd.get_envs().count(), 0);
        assert!(cmd.get_current_dir().is_none());
    }

    #[test]
    fn a_child_without_a_directory_leaves_the_mount() {
        let casa = std::env::temp_dir();
        let mut amb = ambiente_do_appimage();
        amb.insert("OWD", casa.display().to_string());
        let dentro = Some(PathBuf::from(format!("{APPDIR}/usr")));

        let mut cmd = std::process::Command::new("xdg-open");
        crate::no_window_std(&mut cmd);
        aplicar_com(
            &mut cmd,
            &pacote_de_teste(),
            |n| amb.get(n).map(OsString::from),
            dentro.clone(),
        );
        assert_eq!(cmd.get_current_dir(), Some(casa.as_path()));

        // OWD que não existe mais: vai para a casa.
        amb.insert("OWD", "/nao/existe/mais".to_string());
        amb.insert("HOME", casa.display().to_string());
        let mut cmd = std::process::Command::new("xdg-open");
        crate::no_window_std(&mut cmd);
        aplicar_com(
            &mut cmd,
            &pacote_de_teste(),
            |n| amb.get(n).map(OsString::from),
            dentro.clone(),
        );
        assert_eq!(cmd.get_current_dir(), Some(casa.as_path()));

        // Quem escolheu diretório fica nele.
        let mut cmd = std::process::Command::new("xdg-open");
        crate::no_window_std(&mut cmd);
        cmd.current_dir("/");
        aplicar_com(
            &mut cmd,
            &pacote_de_teste(),
            |n| amb.get(n).map(OsString::from),
            dentro,
        );
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/")));
    }

    #[test]
    fn outside_an_appimage_nothing_changes() {
        // O teste roda do `target/`, nunca de dentro de um AppImage.
        assert!(pacote().is_none());
        let mut cmd = std::process::Command::new("true");
        crate::host_env_std(crate::no_window_std(&mut cmd));
        assert_eq!(cmd.get_envs().count(), 0);
        assert!(cmd.get_current_dir().is_none());
        assert_eq!(host_var("PATH"), std::env::var_os("PATH"));
    }
}
