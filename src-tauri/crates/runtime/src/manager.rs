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
//!    O CUDA 12.8 do fork no Linux não tem cudart na release: as bibliotecas
//!    vêm dos wheels oficiais da NVIDIA no PyPI, fixados por SHA256 e
//!    tamanho em [`crate::prism`] — assim como o próprio tarball.
//! 6. Instalação atômica por `rename`; a sessão temporária se limpa sozinha
//!    (e a de um app fechado no meio sai na próxima sessão que abrir).
//! 7. Pacotes com prova por execução (o fork da PrismML): fora do Windows a
//!    prova roda numa pasta `<variante>.pending` que o estado não enxerga, e
//!    só o que passou vira instalado; uma falha do momento guarda os bytes
//!    conferidos para a próxima tentativa só refazer a prova.

use crate::BackendVariant;
use serde::Serialize;
use std::path::PathBuf;

/// User-Agent exigido pela API do GitHub (e boa educação nos downloads).
const USER_AGENT: &str = concat!("OpenWeights/", env!("CARGO_PKG_VERSION"));

/// Repositório de onde vêm os binários prebuilt do motor oficial.
const REPO: &str = crate::OFFICIAL_REPO;

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
    /// A prova por execução reprovou o pacote NESTA máquina: o binário não
    /// carregou, disse outra build, ou a placa não apareceu para o backend.
    /// É o único erro que diz algo permanente sobre a máquina — rede, disco,
    /// digest e placa ocupada não dizem — e por isso é o único que a cadeia
    /// do motor da PrismML registra como "esta variante não serve aqui".
    #[error("verificação falhou: {0}")]
    Execution(String),
    /// A placa não respondeu à prova do motor CUDA AGORA (sem memória para o
    /// contexto, ocupada, num estado ruim): não diz nada da máquina, e o
    /// pacote já conferido fica guardado para a próxima tentativa. Quem
    /// chama pode liberar a placa e tentar de novo.
    #[error("verificação falhou: {0}")]
    GpuUnavailable(String),
    #[error("espaço em disco insuficiente: {0}")]
    NoSpace(String),
    /// A pessoa cancelou. Não diz nada da máquina.
    #[error("instalação cancelada")]
    Cancelled,
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
    /// `ggml-rpc-server`, irmão do servidor no mesmo pacote.
    pub rpc_exe: Option<PathBuf>,
    /// O host consegue `--rpc` e o worker existe. Sem isto o cluster recusa.
    pub rpc_ready: bool,
}

/// Gerencia a instalação dos runtimes em `<data_dir>/runtimes/`.
///
/// `Clone` barato (o lock é compartilhado): uma instalação em segundo plano
/// — o motor da PrismML baixado junto de um modelo — precisa de um handle
/// que sobreviva ao comando que a disparou.
#[derive(Clone)]
pub struct RuntimeManager {
    pub(crate) data_dir: PathBuf,
    /// Serializa instalações concorrentes (reentrada do comando, remount do
    /// webview): duas extrações no mesmo destino se atropelariam.
    pub(crate) install_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Serializa a CADEIA do motor da PrismML inteira, não só cada variante:
    /// quem espera relê o marcador e o disco depois que o outro terminou, em
    /// vez de repetir um download de 727 MB que acabou de reprovar.
    pub(crate) prism_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Acorda quem estiver instalando o motor da PrismML para desistir.
    pub(crate) prism_cancel: std::sync::Arc<tokio::sync::Notify>,
}

impl RuntimeManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            install_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            prism_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            prism_cancel: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn state(&self, variant: BackendVariant) -> RuntimeState {
        self.state_for(crate::PINNED_TAG, variant)
    }

    /// O estado de um pacote qualquer em `runtimes/<tag>/<variante>` — o
    /// oficial e o da PrismML têm o mesmo layout, então a leitura é uma só.
    pub(crate) fn state_for(&self, tag: &str, variant: BackendVariant) -> RuntimeState {
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

    /// Garante que o runtime oficial está instalado, baixando/extraindo se
    /// preciso.
    pub async fn ensure(
        &self,
        variant: BackendVariant,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        self.ensure_package(REPO, crate::PINNED_TAG, variant, None, on_event)
            .await
    }

    /// Garante um pacote de um repositório/tag em `runtimes/<tag>/<variante>`.
    ///
    /// `build_esperada` liga a prova por execução: depois de extrair, o
    /// `llama-server --version` do staging tem de reportar essa build — é o
    /// que separa "os arquivos estão lá" de "roda com o cudart ao lado". O
    /// oficial passa `None` porque a verificação dele acontece na tela do
    /// motor, a pedido; o fork passa a build porque ninguém mais o confere.
    pub(crate) async fn ensure_package(
        &self,
        repo: &str,
        tag: &str,
        variant: BackendVariant,
        build_esperada: Option<u64>,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        // Quem esperar na fila re-checa e sai cedo se o outro já instalou.
        let _install_guard = self.install_lock.lock().await;

        let state = self.state_for(tag, variant);
        if state.installed {
            return Ok(state);
        }

        // Todo o trabalho temporário vive na sessão, que se limpa no Drop —
        // inclusive quando `install` sai por `?` no meio.
        let session = lr_fetch::Session::new(&self.data_dir.join("runtimes"))?;
        let result = self
            .install(
                repo,
                tag,
                variant,
                build_esperada,
                session.path(),
                &on_event,
            )
            .await;
        drop(session);

        match result {
            Ok(()) => {
                on_event(RuntimeEvent::Ready);
                Ok(self.state_for(tag, variant))
            }
            Err(e) => {
                on_event(RuntimeEvent::Failed {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Baixa, verifica, extrai e instala o runtime no destino final.
    /// Trabalha inteiramente dentro de `session`; publicar é um `rename`.
    ///
    /// Com `build_esperada`, o pacote só vale depois da prova por execução.
    /// Fora do Windows ela roda numa pasta ao lado que o estado não enxerga
    /// (`<variante>.pending`, ver [`crate::pending_dir`]) e só o que passou
    /// vira a pasta da variante: a prova do CUDA 12.8 leva até 50 s, e um
    /// "instalado" nesse meio-tempo já faria o motor reiniciar num pacote que
    /// ainda pode reprovar — e ficar para sempre, sem prova, se o app fechasse
    /// ali. Se a prova falhar pelo MOMENTO (placa cheia, sonda sem resposta),
    /// os bytes conferidos ficam nessa pasta e a próxima tentativa só refaz a
    /// prova, sem baixar 727 MB de novo.
    async fn install<F>(
        &self,
        repo: &str,
        tag: &str,
        variant: BackendVariant,
        build_esperada: Option<u64>,
        session: &std::path::Path,
        on_event: &F,
    ) -> Result<(), RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let fixado = crate::prism::pacote_fixado(std::env::consts::OS, tag, variant);
        let final_dir = crate::runtime_dir(&self.data_dir, tag, variant);
        let provar_antes = build_esperada.is_some() && !cfg!(windows);
        let pendente = crate::pending_dir(&final_dir);
        let destino = if provar_antes { &pendente } else { &final_dir };

        if provar_antes && pendente.join(crate::server_exe_name()).is_file() {
            log::info!(
                "reaproveitando {}: baixado e conferido numa tentativa anterior, falta a prova",
                pendente.display()
            );
        } else {
            let staging = self
                .montar(repo, tag, variant, fixado, session, on_event)
                .await?;
            lr_fetch::install_atomically(&staging, destino)?;
        }

        if let Some(esperada) = build_esperada {
            // No Windows a prova roda na pasta FINAL: executar o binário no
            // staging e mover a pasta em seguida falhava com "Acesso negado"
            // (o executável recém-rodado, ou o Defender escaneando-o, ainda
            // segura a pasta por instantes, e o `rename` bate nela). Lá, se a
            // build não for a esperada, a pasta inteira sai — com repetição,
            // pelo mesmo motivo.
            if let Err(e) = provar(destino, tag, variant, esperada, fixado).await {
                // Só a reprovação da MÁQUINA joga fora o que foi conferido.
                let guardar = provar_antes && !matches!(e, RuntimeError::Execution(_));
                if !guardar && let Err(rm) = lr_fetch::remove_dir_all_retrying(destino) {
                    log::warn!("pacote reprovado ficou em {}: {rm}", destino.display());
                }
                return Err(e);
            }
            if provar_antes {
                lr_fetch::install_atomically(&pendente, &final_dir)?;
            }
        }
        log::info!(
            "runtime {tag}/{variant:?} instalado em {}",
            final_dir.display()
        );
        Ok(())
    }

    /// Baixa, confere e monta o pacote no staging da sessão: o asset
    /// principal, o cudart (Windows CUDA) ou as bibliotecas da NVIDIA
    /// fixadas (Linux CUDA 12.8), e a sanidade do conjunto. Devolve o
    /// staging, pronto para ser publicado.
    async fn montar<F>(
        &self,
        repo: &str,
        tag: &str,
        variant: BackendVariant,
        fixado: Option<&crate::prism::PacoteFixado>,
        session: &std::path::Path,
        on_event: &F,
    ) -> Result<PathBuf, RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let client = lr_fetch::client(USER_AGENT)?;

        // O pico de disco do CUDA 12.8 (tarball + wheel do cuBLAS + tudo
        // extraído) passa de 1,6 GB: descobrir que não cabe depois de baixar
        // 600 MB é pior que dizer antes.
        if let Some(f) = fixado
            && let Some(livre) = lr_fetch::available_space(session)
            && livre < f.pico_em_disco
        {
            return Err(RuntimeError::NoSpace(format!(
                "o motor {tag} ({variant:?}) precisa de ~{:.1} GB livres em {} durante a \
                 instalação; há {:.1} GB",
                f.pico_em_disco as f64 / 1e9,
                self.data_dir.join("runtimes").display(),
                livre as f64 / 1e9
            )));
        }

        // Digests de todos os assets de uma vez. `None` se a API falhar. O
        // pacote fixado no código não depende da API: o pino é a verificação.
        let digests = if fixado.is_some() {
            None
        } else {
            lr_fetch::github_release_digests(&client, repo, tag).await
        };

        // ---- Asset principal -------------------------------------------
        let asset = crate::asset_name(tag, variant).ok_or_else(|| {
            RuntimeError::Verification(format!(
                "a release {tag} não tem pacote {variant:?} para {}",
                std::env::consts::OS
            ))
        })?;
        // Um pacote fixado é UMA barra: o tarball e os wheels somam o total
        // que a tela prometeu antes do clique (~727 MB), em vez de três
        // barras que vão de 0 a 100% uma depois da outra.
        let mut faixa = Faixa {
            antes: 0,
            total: fixado.map_or(0, crate::prism::PacoteFixado::total_download_bytes),
            rotulo: &asset,
        };
        let part = match fixado {
            Some(f) => {
                let p = self
                    .baixar_fixado(
                        &client,
                        session,
                        &crate::asset_url(repo, tag, &asset),
                        &asset,
                        f.sha256,
                        f.bytes,
                        &faixa,
                        on_event,
                    )
                    .await?;
                faixa.antes += f.bytes;
                p
            }
            None => {
                self.baixar_asset(
                    &client,
                    session,
                    repo,
                    tag,
                    &asset,
                    digests.as_ref(),
                    on_event,
                )
                .await?
            }
        };

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
                .baixar_asset(
                    &client,
                    session,
                    repo,
                    tag,
                    &cudart,
                    digests.as_ref(),
                    on_event,
                )
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

        // ---- Bibliotecas da NVIDIA fixadas (CUDA 12.8 do fork no Linux) --
        // Um wheel por vez: baixa, confere, tira SÓ os membros fixados para
        // o lado do binário e apaga o `.part` antes do próximo — o do cuBLAS
        // pesa 594 MB e não precisa ficar no disco junto do que já extraiu.
        for roda in fixado.map(|f| f.bibliotecas).unwrap_or_default() {
            let wpart = self
                .baixar_fixado(
                    &client,
                    session,
                    roda.url,
                    roda.arquivo,
                    roda.sha256,
                    roda.bytes,
                    &faixa,
                    on_event,
                )
                .await?;
            faixa.antes += roda.bytes;
            on_event(RuntimeEvent::Extracting {
                asset: roda.arquivo.to_string(),
            });
            let membros = roda
                .membros
                .iter()
                .map(|(m, d)| (m.to_string(), d.to_string()))
                .collect();
            let extraido =
                lr_fetch::extract_zip_members_async(wpart.clone(), membros, staging.clone()).await;
            let _ = std::fs::remove_file(&wpart);
            extraido?;
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
        Ok(staging)
    }

    /// Baixa um asset da release para o `.part` da sessão e confere o SHA256.
    /// Traduz o progresso cru do `lr_fetch` no evento que a UI consome.
    #[allow(clippy::too_many_arguments)]
    async fn baixar_asset<F>(
        &self,
        client: &reqwest::Client,
        session: &std::path::Path,
        repo: &str,
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
            &crate::asset_url(repo, tag, asset),
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

    /// Baixa um arquivo FIXADO no código (URL, SHA256 e tamanho) para o
    /// `.part` da sessão. Sem pino não há instalação: divergência de tamanho
    /// ou de digest é erro, nunca aviso. O progresso sai na `faixa` do
    /// pacote inteiro.
    #[allow(clippy::too_many_arguments)]
    async fn baixar_fixado<F>(
        &self,
        client: &reqwest::Client,
        session: &std::path::Path,
        url: &str,
        nome: &str,
        sha256: &str,
        bytes: u64,
        faixa: &Faixa<'_>,
        on_event: &F,
    ) -> Result<PathBuf, RuntimeError>
    where
        F: Fn(RuntimeEvent) + Send + Sync,
    {
        let part = session.join(format!("{nome}.part"));
        let rotulo = faixa.rotulo.to_string();
        let (antes, total) = (faixa.antes, faixa.total);
        lr_fetch::download_to(client, url, &part, &|received_bytes, _| {
            on_event(RuntimeEvent::Progress {
                asset: rotulo.clone(),
                received_bytes: antes + received_bytes,
                total_bytes: total,
            });
        })
        .await?;
        let recebido = std::fs::metadata(&part)?.len();
        if recebido != bytes {
            let _ = std::fs::remove_file(&part);
            return Err(RuntimeError::Verification(format!(
                "{nome}: tamanho {recebido} B, o pino diz {bytes} B"
            )));
        }
        if let Err(e) = lr_fetch::verify_sha256_strict(&part, nome, sha256).await {
            let _ = std::fs::remove_file(&part);
            return Err(e.into());
        }
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

    /// Garante o runtime — que já traz o worker RPC.
    ///
    /// Existiu aqui um caminho que baixava um "overlay" com `GGML_RPC=ON` de
    /// um release nosso, na crença de que os pacotes oficiais vinham sem RPC.
    /// Vinham com: o `CMAKE_ARGS` do workflow de release do llama.cpp tem
    /// `-DGGML_RPC=ON`, e `ggml-rpc-server` está no índice dos zips de b10441
    /// (CUDA, Vulkan e macOS arm64). Manter o overlay significava compilar o
    /// llama.cpp na nossa conta do GitHub para reembalar o que já vinha
    /// pronto — e o app checar a pasta errada, que foi o que apareceu na tela
    /// como "este motor ainda não traz RPC".
    pub async fn ensure_rpc(
        &self,
        profile: &lr_types::HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let state = self.ensure_best(profile, &on_event).await?;
        if state.rpc_ready {
            return Ok(state);
        }
        Err(RuntimeError::Verification(format!(
            "o pacote {} instalado não traz o {} — atualize o motor de IA",
            state.tag,
            crate::rpc_exe_name()
        )))
    }
}

/// Onde um arquivo de um pacote fixado cai na barra de progresso: o que os
/// anteriores já baixaram, o total do pacote e o nome que a tela mostra.
struct Faixa<'a> {
    antes: u64,
    total: u64,
    rotulo: &'a str,
}

/// A prova por execução de um pacote: o `--version` tem de reportar a build
/// esperada e, onde o pino pede, a placa tem de aparecer no
/// `--list-devices`.
///
/// O que reprova a MÁQUINA (o binário não carrega, diz outra build, a CUDA
/// não sobe) vira [`RuntimeError::Execution`] — o único erro que a cadeia da
/// PrismML grava como "esta variante não serve aqui". O que é do momento vira
/// [`RuntimeError::Verification`] (a sonda não respondeu, o processo não
/// abriu) ou [`RuntimeError::GpuUnavailable`] (a placa cheia ou ocupada).
async fn provar(
    dir: &std::path::Path,
    tag: &str,
    variant: BackendVariant,
    esperada: u64,
    fixado: Option<&crate::prism::PacoteFixado>,
) -> Result<(), RuntimeError> {
    use crate::check::{DispositivosCuda, FalhaDaSonda};
    let (build, _) = crate::check::probe_build(dir).await.map_err(|e| match e {
        FalhaDaSonda::Transitoria(m) => RuntimeError::Verification(m),
        FalhaDaSonda::Recusou(m) => RuntimeError::Execution(m),
    })?;
    if build != esperada {
        return Err(RuntimeError::Execution(format!(
            "o pacote {tag} se diz build {build}, esperava {esperada}"
        )));
    }
    // O `--version` não inicializa a CUDA: um pacote CUDA sem placa
    // utilizável passa nele e roda tudo na CPU, calado. Onde o pino pede, a
    // placa tem de aparecer na lista de dispositivos.
    if !fixado.is_some_and(|f| f.exige_dispositivo_cuda) {
        return Ok(());
    }
    match crate::check::probe_cuda_devices(dir).await {
        DispositivosCuda::Presente(linha) => {
            log::info!("motor {tag}/{variant:?} vê a placa: {linha}");
            Ok(())
        }
        DispositivosCuda::Transitoria(causa) => Err(RuntimeError::GpuUnavailable(format!(
            "a placa não respondeu à prova do motor CUDA agora ({causa}); feche o que \
             estiver usando a GPU — um modelo carregado, um jogo — e tente de novo"
        ))),
        DispositivosCuda::Ausente(causa) => Err(RuntimeError::Execution(format!(
            "o motor CUDA não achou a placa: {causa}"
        ))),
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

    /// Teste live da instalação no Linux, de ponta a ponta: baixa o pacote
    /// Vulkan da release pinada, confere o digest, extrai (as `.so` chegam
    /// por symlink de versão, `libllama.so.0 -> libllama.so.0.1.0`, e o
    /// executável acha as vizinhas pelo `RUNPATH=$ORIGIN`) e executa o
    /// `llama-server --version` na pasta final. Roda só com `--ignored`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "rede: baixa ~33 MB da release pinada do llama.cpp"]
    async fn live_linux_vulkan_package_installs_and_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().to_path_buf());
        let state = mgr
            .ensure(BackendVariant::Vulkan, |_| {})
            .await
            .expect("instalação do pacote Vulkan");
        assert!(state.installed);
        assert!(state.rpc_ready, "o pacote Linux traz o ggml-rpc-server");
        let dir = state.dir.expect("pasta instalada");
        assert!(dir.join(crate::exe_name("llama-fit-params")).is_file());
        assert!(dir.join(crate::exe_name("llama-bench")).is_file());
        let (build, _) = crate::check::probe_build(&dir)
            .await
            .expect("o motor instalado executa");
        assert_eq!(Some(build), crate::build_number(crate::PINNED_TAG));
    }

    /// Teste live contra a API do GitHub — roda só com `--ignored`. Cobre
    /// todo pacote que algum sistema pode pedir, não só o deste.
    #[tokio::test]
    #[ignore = "rede: consulta a API de releases do GitHub"]
    async fn live_release_digests_cover_pinned_assets() {
        let client = lr_fetch::client(USER_AGENT).unwrap();
        let digests = lr_fetch::github_release_digests(&client, REPO, crate::PINNED_TAG)
            .await
            .expect("API do GitHub indisponível ou rate-limited");
        for (os, v) in [
            ("windows", BackendVariant::Cuda13),
            ("windows", BackendVariant::Cuda12),
            ("windows", BackendVariant::Vulkan),
            ("windows", BackendVariant::Cpu),
            ("linux", BackendVariant::Vulkan),
            ("linux", BackendVariant::Cpu),
            ("macos", BackendVariant::MacosArm64),
            ("macos", BackendVariant::MacosX64),
        ] {
            let asset = crate::asset_name_for(os, crate::PINNED_TAG, v).expect("publicada");
            assert!(
                digests.contains_key(&asset),
                "release pinada deve ter digest para {asset}"
            );
        }
    }
}
