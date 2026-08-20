//! Download, verificação e extração dos runtimes llama.cpp.
//!
//! A mecânica de baixar/verificar/extrair/instalar vive em `lr_fetch`, que
//! nasceu deste arquivo quando o Node portátil e o Traefik passaram a precisar
//! da mesma coisa. O que sobra aqui é o que só o llama.cpp sabe: qual asset
//! baixar para cada variante, que o cudart tem de ficar ao lado do executável,
//! e o que conta como instalação sã.
//!
//! Fluxo do `ensure`:
//! 1. Já instalado? Retorna na hora.
//! 2. Baixa o asset da release pinada para dentro do diretório de sessão.
//! 3. Verifica o SHA256 contra o `digest` da API do GitHub — API indisponível
//!    (rate limit) apenas loga e segue: bloquear a instalação por isso seria
//!    pior que o risco evitado.
//! 4. Extrai, localiza o `llama-server(.exe)` recursivamente (os pacotes têm
//!    estrutura variável) e "achata" o diretório que o contém.
//! 5. Variantes CUDA: baixa o cudart e espalha as DLLs ao lado do executável.
//! 6. Instalação atômica por `rename`; a sessão temporária se limpa sozinha.

use crate::BackendVariant;
use serde::Serialize;
use std::path::PathBuf;

/// User-Agent exigido pela API do GitHub (e boa educação nos downloads).
const USER_AGENT: &str = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));

/// Repositório de onde vêm os binários prebuilt.
const REPO: &str = "ggml-org/llama.cpp";

/// Sanidade final: o runtime completo (exe + DLLs) sempre passa disso.
/// ATENÇÃO: o llama-server.exe em si é um launcher de ~9 KB — o código real
/// mora em llama-server-impl.dll (~10 MB), então o tamanho do EXE sozinho
/// não diz nada; validamos o conjunto extraído.
const MIN_INSTALL_SIZE: u64 = 5 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("falha de rede: {0}")]
    Network(#[from] reqwest::Error),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("verificação falhou: {0}")]
    Verification(String),
}

/// Erros do `lr_fetch` viram erros de runtime preservando a categoria — a UI
/// distingue "sem rede" de "pacote corrompido".
impl From<lr_fetch::FetchError> for RuntimeError {
    fn from(e: lr_fetch::FetchError) -> Self {
        match e {
            lr_fetch::FetchError::Network(e) => Self::Network(e),
            lr_fetch::FetchError::Io(e) => Self::Io(e),
            lr_fetch::FetchError::Verification(m) => Self::Verification(m),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum RuntimeEvent {
    Progress {
        asset: String,
        received_bytes: u64,
        total_bytes: u64,
    },
    Extracting {
        asset: String,
    },
    Ready,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub tag: String,
    pub variant: BackendVariant,
    pub installed: bool,
    pub server_exe: Option<PathBuf>,
    /// Pasta onde o pacote foi extraído. É por ela que se alcançam os
    /// irmãos do servidor (`llama-fit-params`, `llama-bench`), que o
    /// otimizador usa para responder "cabe?" e "rende quanto?".
    pub dir: Option<PathBuf>,
    /// `ggml-rpc-server` ao lado do llama-server, quando o pacote traz RPC.
    pub rpc_exe: Option<PathBuf>,
    /// O host consegue `--rpc` e o worker existe. Sem isto o cluster recusa.
    pub rpc_ready: bool,
}

/// Gerencia a instalação dos runtimes em `<data_dir>/runtimes/`.
pub struct RuntimeManager {
    data_dir: PathBuf,
    /// Serializa instalações concorrentes (reentrada do comando, remount do
    /// webview): duas extrações no mesmo destino se atropelariam.
    install_lock: tokio::sync::Mutex<()>,
}

impl RuntimeManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            install_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn state(&self, variant: BackendVariant) -> RuntimeState {
        let tag = crate::PINNED_TAG;
        let dir = crate::runtime_dir(&self.data_dir, tag, variant);
        let exe = dir.join(crate::server_exe_name());
        // "Instalado" é a presença do executável: não há manifesto separado
        // que pudesse divergir do que está no disco.
        let installed = exe.is_file();
        let rpc = dir.join(crate::rpc_exe_name());
        let rpc_ready = installed && rpc.is_file();
        RuntimeState {
            tag: tag.to_string(),
            variant,
            installed,
            server_exe: installed.then_some(exe),
            dir: installed.then_some(dir),
            rpc_exe: rpc_ready.then_some(rpc),
            rpc_ready,
        }
    }

    /// Garante que o runtime está instalado, baixando/extraindo se preciso.
    pub async fn ensure(
        &self,
        variant: BackendVariant,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        // Quem esperar na fila re-checa e sai cedo se o outro já instalou.
        let _install_guard = self.install_lock.lock().await;

        let state = self.state(variant);
        if state.installed {
            return Ok(state);
        }

        // Todo o trabalho temporário vive na sessão, que se limpa no Drop —
        // inclusive quando `install` sai por `?` no meio.
        let session = lr_fetch::Session::new(&self.data_dir.join("runtimes"))?;
        let result = self.install(variant, session.path(), &on_event).await;
        drop(session);

        match result {
            Ok(()) => {
                on_event(RuntimeEvent::Ready);
                Ok(self.state(variant))
            }
            Err(e) => {
                on_event(RuntimeEvent::Failed {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Baixa, verifica, extrai e instala atomicamente o runtime no destino
    /// final. Trabalha inteiramente dentro de `session`; o último passo é um
    /// único `fs::rename`.
    async fn install<F>(
        &self,
        variant: BackendVariant,
        session: &std::path::Path,
        on_event: &F,
    ) -> Result<(), RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let tag = crate::PINNED_TAG;
        let client = lr_fetch::client(USER_AGENT)?;

        // Digests de todos os assets de uma vez. `None` se a API falhar.
        let digests = lr_fetch::github_release_digests(&client, REPO, tag).await;

        // ---- Asset principal -------------------------------------------
        let asset = crate::asset_name(tag, variant);
        let part = self
            .baixar_asset(&client, session, tag, &asset, digests.as_ref(), on_event)
            .await?;

        on_event(RuntimeEvent::Extracting {
            asset: asset.clone(),
        });
        let extract_dir = session.join("extract-main");
        lr_fetch::extract_archive_async(part.clone(), asset.clone(), extract_dir.clone()).await?;
        let _ = std::fs::remove_file(&part);

        // Estrutura variável nos pacotes: achamos o diretório que contém o
        // llama-server e usamos ele como raiz real do runtime.
        let root = lr_fetch::find_dir_containing(&extract_dir, crate::server_exe_name())
            .ok_or_else(|| {
                RuntimeError::Verification(format!(
                    "{} não encontrado dentro de {asset}",
                    crate::server_exe_name()
                ))
            })?;
        let staging = session.join("install");
        lr_fetch::move_dir_contents(&root, &staging)?;

        // ---- cudart (obrigatório para CUDA) -----------------------------
        if let Some(cudart) = crate::cudart_asset_name(tag, variant) {
            let cpart = self
                .baixar_asset(&client, session, tag, &cudart, digests.as_ref(), on_event)
                .await?;

            on_event(RuntimeEvent::Extracting {
                asset: cudart.clone(),
            });
            let cudart_dir = session.join("extract-cudart");
            lr_fetch::extract_archive_async(cpart.clone(), cudart.clone(), cudart_dir.clone())
                .await?;
            let _ = std::fs::remove_file(&cpart);
            // As DLLs precisam ficar AO LADO do llama-server.exe, então
            // achatamos qualquer subestrutura do pacote cudart.
            lr_fetch::move_files_flat(&cudart_dir, &staging)?;
        }

        // ---- Sanidade final ---------------------------------------------
        let exe = staging.join(crate::server_exe_name());
        let exe_ok = std::fs::metadata(&exe)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if !exe_ok {
            return Err(RuntimeError::Verification(format!(
                "{} ausente após a extração",
                crate::server_exe_name()
            )));
        }
        let install_size = lr_fetch::dir_size(&staging);
        if install_size < MIN_INSTALL_SIZE {
            return Err(RuntimeError::Verification(format!(
                "runtime extraído suspeito de incompleto ({install_size} bytes no total)"
            )));
        }

        // ---- Instalação atômica -----------------------------------------
        let final_dir = crate::runtime_dir(&self.data_dir, tag, variant);
        lr_fetch::install_atomically(&staging, &final_dir)?;
        log::info!(
            "runtime {tag}/{variant:?} instalado em {}",
            final_dir.display()
        );
        Ok(())
    }

    /// Baixa um asset da release para o `.part` da sessão e confere o SHA256.
    /// Traduz o progresso cru do `lr_fetch` no evento que a UI consome.
    async fn baixar_asset<F>(
        &self,
        client: &reqwest::Client,
        session: &std::path::Path,
        tag: &str,
        asset: &str,
        digests: Option<&std::collections::HashMap<String, String>>,
        on_event: &F,
    ) -> Result<PathBuf, RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let part = session.join(format!("{asset}.part"));
        let nome = asset.to_string();
        lr_fetch::download_to(
            client,
            &crate::asset_url(tag, asset),
            &part,
            &|received_bytes, total_bytes| {
                on_event(RuntimeEvent::Progress {
                    asset: nome.clone(),
                    received_bytes,
                    total_bytes,
                });
            },
        )
        .await?;
        lr_fetch::verify_sha256(&part, asset, digests).await?;
        Ok(part)
    }

    /// Seleciona a melhor variante para o perfil e tenta instalá-la,
    /// descendo a cadeia de fallback (CUDA → Vulkan → CPU) em caso de falha.
    pub async fn ensure_best(
        &self,
        profile: &lr_types::HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let mut variant = crate::select_variant(profile);
        loop {
            match self.ensure(variant, &on_event).await {
                Ok(state) => return Ok(state),
                Err(e) => match variant.fallback() {
                    Some(next) => {
                        log::warn!("runtime {variant:?} falhou ({e}); tentando {next:?}");
                        variant = next;
                    }
                    None => return Err(e),
                },
            }
        }
    }

    /// Garante o runtime E o worker RPC. Os zips oficiais quase nunca
    /// trazem `ggml-rpc-server`; tentamos o overlay publicado na tag
    /// `llama-rpc-<PINNED_TAG>` do OpenWeights. 404 = ainda não há build.
    pub async fn ensure_rpc(
        &self,
        profile: &lr_types::HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let state = self.ensure_best(profile, &on_event).await?;
        if state.rpc_ready {
            return Ok(state);
        }
        let variant = state.variant;
        let Some(asset) = crate::rpc_overlay_asset(crate::PINNED_TAG, variant) else {
            return Err(RuntimeError::Verification(
                "esta variante do motor não tem overlay RPC (CPU puro)".into(),
            ));
        };
        let _install_guard = self.install_lock.lock().await;
        if self.state(variant).rpc_ready {
            return Ok(self.state(variant));
        }
        let session = lr_fetch::Session::new(&self.data_dir.join("runtimes"))?;
        let result = self.install_rpc_overlay(variant, &asset, session.path(), &on_event).await;
        drop(session);
        match result {
            Ok(()) => {
                on_event(RuntimeEvent::Ready);
                let st = self.state(variant);
                if !st.rpc_ready {
                    return Err(RuntimeError::Verification(
                        "o overlay RPC não trouxe o ggml-rpc-server".into(),
                    ));
                }
                Ok(st)
            }
            Err(e) => {
                on_event(RuntimeEvent::Failed {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    async fn install_rpc_overlay<F>(
        &self,
        variant: BackendVariant,
        asset: &str,
        session: &std::path::Path,
        on_event: &F,
    ) -> Result<(), RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let tag = crate::PINNED_TAG;
        let client = lr_fetch::client(USER_AGENT)?;
        let url = crate::rpc_overlay_url(tag, asset);
        let part = session.join(format!("{asset}.part"));
        let nome = asset.to_string();
        let baixou = lr_fetch::download_to(
            &client,
            &url,
            &part,
            &|received_bytes, total_bytes| {
                on_event(RuntimeEvent::Progress {
                    asset: nome.clone(),
                    received_bytes,
                    total_bytes,
                });
            },
        )
        .await;
        if let Err(e) = baixou {
            return Err(RuntimeError::Verification(format!(
                "sem pacote RPC para {tag} ({e}). O workflow llama-rpc ainda não publicou este overlay"
            )));
        }
        on_event(RuntimeEvent::Extracting {
            asset: asset.to_string(),
        });
        let extract_dir = session.join("extract-rpc");
        lr_fetch::extract_archive_async(part.clone(), asset.to_string(), extract_dir.clone())
            .await?;
        let _ = std::fs::remove_file(&part);

        let final_dir = crate::runtime_dir(&self.data_dir, tag, variant);
        if !final_dir.is_dir() {
            return Err(RuntimeError::Verification(
                "instale o motor de IA antes do overlay RPC".into(),
            ));
        }
        // Copia o que o overlay trouxer (llama-server com RPC + ggml-rpc-server)
        // por cima do pacote oficial, na mesma pasta.
        if let Some(root) = lr_fetch::find_dir_containing(&extract_dir, crate::rpc_exe_name())
            .or_else(|| lr_fetch::find_dir_containing(&extract_dir, crate::server_exe_name()))
        {
            lr_fetch::move_files_flat(&root, &final_dir)?;
        } else {
            lr_fetch::move_files_flat(&extract_dir, &final_dir)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Testes
//
// A mecânica genérica (parsing de digest, traversal no zip, sha256) é testada
// em `lr_fetch`. Aqui ficam só as garantias específicas do llama.cpp.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Escreve um zip com estrutura aninhada (como os pacotes reais do
    /// llama.cpp) contendo um llama-server fake.
    fn write_nested_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        let exe = crate::server_exe_name();

        zip.add_directory("llama-b1-bin/build/bin/", opts).unwrap();
        zip.start_file(format!("llama-b1-bin/build/bin/{exe}"), opts)
            .unwrap();
        zip.write_all(b"fake-server-binary").unwrap();
        zip.start_file("llama-b1-bin/build/bin/ggml-base.dll", opts)
            .unwrap();
        zip.write_all(b"fake-dll").unwrap();
        // Arquivo fora da raiz do servidor: não deve ir para o destino.
        zip.start_file("llama-b1-bin/LICENSE", opts).unwrap();
        zip.write_all(b"mit").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn zip_extraction_finds_and_flattens_server_root() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("pkg.zip");
        write_nested_zip(&zip_path);

        let extract = tmp.path().join("extract");
        lr_fetch::extract_archive(&zip_path, "pkg.zip", &extract).unwrap();

        let root = lr_fetch::find_dir_containing(&extract, crate::server_exe_name())
            .expect("raiz com llama-server");
        assert!(root.ends_with("llama-b1-bin/build/bin"));

        let staging = tmp.path().join("install");
        lr_fetch::move_dir_contents(&root, &staging).unwrap();

        let exe = staging.join(crate::server_exe_name());
        assert!(exe.is_file(), "llama-server deve estar na raiz do destino");
        assert_eq!(std::fs::read(&exe).unwrap(), b"fake-server-binary");
        assert!(staging.join("ggml-base.dll").is_file());
        // O LICENSE ficou acima da raiz do servidor e não é movido.
        assert!(!staging.join("LICENSE").exists());
    }

    #[test]
    fn tar_gz_extraction_finds_server_root() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("pkg.tar.gz");
        let exe = crate::server_exe_name();

        let file = std::fs::File::create(&tar_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let data = b"fake-macos-server";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("llama-b1/bin/{exe}"), data.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let extract = tmp.path().join("extract");
        lr_fetch::extract_archive(&tar_path, "pkg.tar.gz", &extract).unwrap();

        let root = lr_fetch::find_dir_containing(&extract, exe).expect("raiz com llama-server");
        assert!(root.ends_with("llama-b1/bin"));
        assert_eq!(std::fs::read(root.join(exe)).unwrap(), data);
    }

    #[test]
    fn cudart_dlls_are_flattened_next_to_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("cudart");
        std::fs::create_dir_all(src.join("nested/deep")).unwrap();
        std::fs::write(src.join("cudart64_12.dll"), b"a").unwrap();
        std::fs::write(src.join("nested/deep/cublas64_12.dll"), b"b").unwrap();

        let dst = tmp.path().join("install");
        std::fs::create_dir_all(&dst).unwrap();
        lr_fetch::move_files_flat(&src, &dst).unwrap();

        assert!(dst.join("cudart64_12.dll").is_file());
        assert!(dst.join("cublas64_12.dll").is_file());
        assert!(!dst.join("nested").exists());
    }

    /// Regressão: nos pacotes reais o llama-server.exe é um launcher de ~9 KB
    /// (o código mora em llama-server-impl.dll). A sanidade valida o CONJUNTO
    /// extraído, nunca o tamanho do exe sozinho.
    #[test]
    fn sanity_accepts_tiny_launcher_exe_with_big_impl_dll() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("llama-server.exe"), vec![1u8; 9 * 1024]).unwrap();
        std::fs::write(
            dir.path().join("llama-server-impl.dll"),
            vec![2u8; 6 * 1024 * 1024],
        )
        .unwrap();
        assert!(lr_fetch::dir_size(dir.path()) >= MIN_INSTALL_SIZE);

        // Extração truncada (só o launcher) precisa continuar reprovando.
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join("llama-server.exe"), vec![1u8; 9 * 1024]).unwrap();
        assert!(lr_fetch::dir_size(broken.path()) < MIN_INSTALL_SIZE);
    }

    /// Teste live contra a API do GitHub — roda só com `--ignored`.
    #[tokio::test]
    #[ignore = "rede: consulta a API de releases do GitHub"]
    async fn live_release_digests_cover_pinned_assets() {
        let client = lr_fetch::client(USER_AGENT).unwrap();
        let digests = lr_fetch::github_release_digests(&client, REPO, crate::PINNED_TAG)
            .await
            .expect("API do GitHub indisponível ou rate-limited");
        let asset = crate::asset_name(crate::PINNED_TAG, BackendVariant::Cpu);
        assert!(
            digests.contains_key(&asset),
            "release pinada deve ter digest para {asset}"
        );
    }
}
