//! O motor da PrismML: o fork do llama.cpp que abre os modelos Bonsai 2.
//!
//! Os arquivos `PTQ1_0`/`PQ2_0` usam tipos de tensor que o llama.cpp oficial
//! não conhece (ids 142 e 143, acima do `GGML_TYPE_COUNT` dele) e recusa com
//! "unknown type". O fork em `PrismML-Eng/llama.cpp` traz os kernels e
//! publica releases com o MESMO padrão de nome e o MESMO layout de pacote do
//! upstream (`llama-<tag>-bin-win-cuda-13.3-x64.zip`, launcher de 9 KB +
//! `llama-server-impl.dll`, cudart à parte; `…-bin-ubuntu-vulkan-x64.tar.gz`
//! no Linux) — por isso a instalação reaproveita o pipeline do motor oficial
//! e só troca repositório e tag.
//!
//! Pino explícito, como o oficial: a tag carrega a build do upstream em que o
//! fork se baseia, e a prova de instalação é o próprio binário reportar essa
//! build no `--version` com o cudart ao lado.
//!
//! **Linux com NVIDIA (desde a 0.24.3).** O `libggml-vulkan.so` da release
//! não tem shaders de `PQ2_0` (KNOWN_ISSUES do fork: "PQ2_0 on Vulkan
//! silently runs on the CPU") — o Ternary-Bonsai-2-27B `PQ2_0` media 0,6
//! tok/s de prompt numa RTX 3090. O asset `linux-cuda-12.8-x64` do fork, com
//! o cudart e o cuBLAS 12.8 ao lado, mede 1221 tok/s de prompt e 66 de
//! geração na mesma placa. O tarball NÃO traz as bibliotecas da NVIDIA: o
//! app as baixa dos wheels oficiais da NVIDIA no PyPI, fixados aqui por
//! SHA256 e tamanho (fail-closed), e extrai só os três `.so` e a licença.
//! Quem não pode ter o CUDA (AMD/Intel, driver < 570, sem NVML, RTX 50,
//! CPU sem AVX2) continua no Vulkan — e o `PTQ1_0` roda na GPU por ele.
//!
//! Windows e macOS não mudam: a variante do fork é a do motor oficial,
//! sem reserva e sem sonda nova.
//!
//! O que este motor NÃO faz: não é o motor padrão. Ele sobe quando o modelo
//! selecionado exige (ver `optimize_prepare_model` no app) e o oficial volta
//! quando outro modelo é escolhido. Sem RPC: o cluster continua no oficial.

use crate::experimental::RuntimeIdentity;
use crate::{BackendVariant, RuntimeError, RuntimeEvent, RuntimeManager, RuntimeState};
use lr_types::{GpuVendor, HardwareProfile, tuning::EngineSource};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Release do fork homologada para esta versão do app (2026-09-18).
pub const TAG: &str = "prism-b10709-9a9394a";
/// A build do llama.cpp em que essa release se baseia — é o que o binário
/// reporta em `--version`, e o que a instalação confere.
pub const UPSTREAM_BUILD: u64 = 10709;
pub const REPOSITORY: &str = "PrismML-Eng/llama.cpp";

/// Tamanho de reserva para uma variante sem número conferido — a tela
/// precisa dizer ALGUM tamanho antes do clique. As conferidas estão em
/// [`download_bytes`].
pub const TAMANHO_APROXIMADO_BYTES: u64 = 150 * 1024 * 1024;

/// O menor ramo de driver NVIDIA do CUDA 12.8 no Linux (o 12.8 GA pede
/// 570.26). Comparado como INTEIRO de ramo: "570.124.04" é ramo 570.
const DRIVER_MINIMO_CUDA128: u32 = 570;

/// O código que a instalação cancelada manda à tela (no evento `Failed` e
/// no erro do comando): ela o trata como "parou", não como erro.
pub const CANCELLED: &str = "prism-install-cancelled";

/// Uma biblioteca da NVIDIA fixada: o wheel oficial que a própria NVIDIA
/// publica no PyPI. O app baixa na máquina de quem usa — não redistribui —
/// e tira do wheel só os membros listados.
pub(crate) struct RodaNvidia {
    pub url: &'static str,
    /// Nome do arquivo, que também nomeia o progresso na tela.
    pub arquivo: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// `(caminho dentro do wheel, nome ao lado do binário)`. A licença do
    /// `dist-info` vai junto: a pasta instalada carrega os termos da NVIDIA.
    pub membros: &'static [(&'static str, &'static str)],
}

/// Um pacote do fork cujos bytes o código fixa, com o que ele precisa além
/// da release.
pub(crate) struct PacoteFixado {
    /// SHA256 e tamanho do asset principal — o pino é a verificação; o
    /// digest da API do GitHub não entra.
    pub sha256: &'static str,
    pub bytes: u64,
    /// Bibliotecas baixadas à parte para o lado do binário.
    pub bibliotecas: &'static [RodaNvidia],
    /// Pico de disco da instalação: pacote extraído + wheel do cuBLAS +
    /// bibliotecas extraídas, com folga. Conferido antes de baixar.
    pub pico_em_disco: u64,
    /// A prova por execução exige uma linha `CUDA<n>:` no `--list-devices`.
    pub exige_dispositivo_cuda: bool,
}

impl PacoteFixado {
    /// Tudo o que a instalação baixa: o asset principal mais as bibliotecas.
    pub fn total_download_bytes(&self) -> u64 {
        self.bytes + self.bibliotecas.iter().map(|r| r.bytes).sum::<u64>()
    }
}

/// `nvidia-cuda-runtime-cu12` 12.8.90 (conferido no JSON do PyPI em
/// 2026-09-28).
const RODA_CUDART: RodaNvidia = RodaNvidia {
    url: "https://files.pythonhosted.org/packages/0d/9b/a997b638fcd068ad6e4d53b8551a7d30fe8b404d6f1804abf1df69838932/nvidia_cuda_runtime_cu12-12.8.90-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
    arquivo: "nvidia_cuda_runtime_cu12-12.8.90-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
    bytes: 954_765,
    sha256: "adade8dcbd0edf427b7204d480d6066d33902cab2a4707dcfc48a2d0fd44ab90",
    membros: &[
        ("nvidia/cuda_runtime/lib/libcudart.so.12", "libcudart.so.12"),
        (
            "nvidia_cuda_runtime_cu12-12.8.90.dist-info/License.txt",
            "NVIDIA-LICENSE-cudart.txt",
        ),
    ],
};

/// `nvidia-cublas-cu12` 12.8.4.1 (conferido no JSON do PyPI em 2026-09-28).
/// O `libcublas.so.12` tem `RUNPATH=$ORIGIN` e acha o `libcublasLt.so.12`
/// ao lado.
const RODA_CUBLAS: RodaNvidia = RodaNvidia {
    url: "https://files.pythonhosted.org/packages/dc/61/e24b560ab2e2eaeb3c839129175fb330dfcfc29e5203196e5541a4c44682/nvidia_cublas_cu12-12.8.4.1-py3-none-manylinux_2_27_x86_64.whl",
    arquivo: "nvidia_cublas_cu12-12.8.4.1-py3-none-manylinux_2_27_x86_64.whl",
    bytes: 594_346_921,
    sha256: "8ac4e771d5a348c551b2a426eda6193c19aa630236b418086020df5ba9667142",
    membros: &[
        ("nvidia/cublas/lib/libcublas.so.12", "libcublas.so.12"),
        ("nvidia/cublas/lib/libcublasLt.so.12", "libcublasLt.so.12"),
        (
            "nvidia_cublas_cu12-12.8.4.1.dist-info/License.txt",
            "NVIDIA-LICENSE-cublas.txt",
        ),
    ],
};

/// `llama-prism-b10709-9a9394a-bin-linux-cuda-12.8-x64.tar.gz`: 167.241.119
/// B e o SHA256 abaixo, conferidos na release e no arquivo baixado.
/// Extraído, o pacote ocupa 206 MB; as bibliotecas, 869 MB.
static CUDA128: PacoteFixado = PacoteFixado {
    sha256: "8aec67eb023b251712c7e6490f367b5671bf587eced1436a9b85f4a90c3b7d3d",
    bytes: 167_241_119,
    bibliotecas: &[RODA_CUDART, RODA_CUBLAS],
    // 206 MB do pacote + 594 MB do wheel + 869 MB extraídos = 1,67 GB.
    pico_em_disco: 1_750_000_000,
    exige_dispositivo_cuda: true,
};

/// O pacote fixado para este sistema/tag/variante, quando o código fixa um.
pub(crate) fn pacote_fixado(
    os: &str,
    tag: &str,
    variant: BackendVariant,
) -> Option<&'static PacoteFixado> {
    (os == "linux" && tag == TAG && variant == BackendVariant::Cuda128).then_some(&CUDA128)
}

/// Quanto a instalação de cada variante do fork baixa — asset principal
/// mais o cudart (Windows) ou as bibliotecas da NVIDIA (Linux CUDA),
/// conferido na release e no PyPI em 2026-09-28. É o número que a tela
/// mostra antes do clique; era um "150 MB" fixo, que no Linux CUDA é 727 MB
/// e no Windows CUDA 13.3 é 511 MB.
pub fn download_bytes(os: &str, variant: BackendVariant) -> u64 {
    use BackendVariant::*;
    match (os, variant) {
        ("windows", Cuda13) => 145_031_500 + 390_970_417,
        ("windows", Cuda12) => 253_442_371 + 391_443_627,
        ("windows", Vulkan) => 29_908_460,
        ("windows", Cpu) => 18_785_841,
        ("linux", Cuda128) => CUDA128.total_download_bytes(),
        ("linux", Vulkan) => 34_248_149,
        ("linux", Cpu) => 17_108_139,
        ("macos", MacosArm64) => 11_500_187,
        ("macos", MacosX64) => 11_515_388,
        _ => TAMANHO_APROXIMADO_BYTES,
    }
}

/// Quanto a variante ocupa instalada, quando isso é bem maior que o
/// download (o CUDA 12.8: ~1,07 GB no disco para 727 MB baixados).
pub fn disk_bytes(os: &str, variant: BackendVariant) -> Option<u64> {
    (os == "linux" && variant == BackendVariant::Cuda128).then_some(1_074_957_745)
}

/// O fork publica para as mesmas plataformas que o app sabe instalar
/// (Windows x64, Linux x64 e macOS), com os mesmos nomes do upstream.
pub fn supported(profile: &HardwareProfile) -> bool {
    matches!(profile.os.as_str(), "windows" | "linux" | "macos")
}

/// Esta máquina pode rodar o CUDA 12.8 do fork, tudo o que dá para saber
/// sem baixar nada:
///
/// - Linux x86_64 com NVIDIA como placa principal;
/// - NVML respondeu (`cuda_compute` presente): sem ele não há userspace do
///   driver, logo não há `libcuda.so.1` — é o container sem `--nvidia`, e
///   baixar 727 MB para falhar na certa seria o pior dos mundos;
/// - compute capability < 12: a RTX 50 decodifica de forma errática com o
///   binário pronto do Linux (KNOWN_ISSUES do fork, #199);
/// - AVX2: o `libggml-cpu` do pacote CUDA é um só, AVX2, sem dispatch — numa
///   CPU sem ele a inferência morre com SIGILL, e nem o `--version` nem o
///   `--list-devices` pegam isso;
/// - driver do ramo 570 ou mais novo, comparado como inteiro.
fn cuda128_possivel(profile: &HardwareProfile) -> bool {
    profile.os == "linux"
        && profile.arch == "x86_64"
        && profile.avx2
        && profile.best_gpu().is_some_and(|gpu| {
            gpu.vendor == GpuVendor::Nvidia
                && gpu.cuda_compute.is_some_and(|(major, _)| major < 12)
                && gpu
                    .driver_version
                    .as_deref()
                    .and_then(crate::driver_major)
                    .is_some_and(|ramo| ramo >= DRIVER_MINIMO_CUDA128)
        })
}

/// A variante do fork que esta máquina pede: o CUDA 12.8 quando possível
/// (só no Linux), senão a mesma do motor oficial.
pub fn preferred_variant(profile: &HardwareProfile) -> BackendVariant {
    if cuda128_possivel(profile) {
        BackendVariant::Cuda128
    } else {
        crate::select_variant(profile)
    }
}

/// A ordem em que o fork é tentado: a preferida e, no Linux, as reservas
/// (CUDA 12.8 → Vulkan → CPU). No Windows e no macOS é a variante de
/// sempre, sozinha — lá nada mudou.
pub fn chain(profile: &HardwareProfile) -> Vec<BackendVariant> {
    let mut cadeia = vec![preferred_variant(profile)];
    if profile.os == "linux" {
        while let Some(proxima) = cadeia.last().and_then(|v| v.fallback()) {
            cadeia.push(proxima);
        }
    }
    cadeia
}

/// Pelo NOME do artefato — antes do download não há cabeçalho para ler.
/// Os Bonsai 2 publicam `…-PQ2_0.gguf`; o `PTQ1_0` não casa.
pub fn name_says_pq2(nome: &str) -> bool {
    nome.to_ascii_uppercase().contains("PQ2_0")
}

/// Identidade das medições feitas sob este motor, na variante que de fato
/// mediu — mesma forma da do oficial e da do MoE-cache, para o histórico de
/// desempenho distinguir. Um número do Vulkan não pode passar por número do
/// CUDA 12.8, nem o contrário.
pub fn identity(profile: &HardwareProfile, variant: BackendVariant) -> RuntimeIdentity {
    RuntimeIdentity {
        source: EngineSource::Prism,
        revision: TAG.into(),
        backend: crate::experimental::backend_name(variant).into(),
        platform: profile.os.clone(),
    }
}

/// O marcador de "esta variante reprovou nesta máquina":
/// `runtimes/<TAG>/<pasta>.failed`, ao lado da pasta que ela teria. É
/// arquivo, então o scan de pacotes não o vê.
fn marcador(data_dir: &Path, variant: BackendVariant) -> PathBuf {
    let dir = crate::runtime_dir(data_dir, TAG, variant);
    let nome = format!(
        "{}.failed",
        dir.file_name().unwrap_or_default().to_string_lossy()
    );
    dir.with_file_name(nome)
}

fn driver_da_placa(profile: &HardwareProfile) -> Option<String> {
    profile.best_gpu().and_then(|g| g.driver_version.clone())
}

/// O marcador vale enquanto o driver for o que falhou: a primeira linha
/// guarda a versão, e um driver novo é máquina nova para a CUDA.
pub(crate) fn marcador_vale(conteudo: &str, driver: Option<&str>) -> bool {
    let gravado = conteudo.lines().next().unwrap_or("").trim();
    driver.is_some_and(|d| !gravado.is_empty() && gravado == d.trim())
}

/// O que a tela precisa saber do motor da PrismML — para um arquivo
/// específico ou, sem arquivo, para a máquina.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrismStatus {
    /// O estado resolvido: instalado, ou a variante que a instalação vai
    /// buscar.
    #[serde(flatten)]
    pub state: RuntimeState,
    /// A variante que esta máquina pede ao fork.
    pub preferred: BackendVariant,
    /// A melhor variante do fork instalada, qualquer que seja o arquivo.
    pub installed_variant: Option<BackendVariant>,
    /// Quanto instalar `state.variant` baixa (0 quando já está instalada).
    pub download_bytes: u64,
    /// Quanto `state.variant` ocupa no disco, quando bem maior que o download.
    pub disk_bytes: Option<u64>,
    /// O CUDA 12.8 é possível aqui, não falhou e ainda não está instalado:
    /// é o que um arquivo `PQ2_0` baixaria junto.
    pub cuda_download_bytes: Option<u64>,
    /// O CUDA 12.8 reprovou nesta máquina com o driver de agora.
    pub cuda_failed: bool,
    /// O arquivo pedido tem tensores `PQ2_0`.
    pub pq2: bool,
    /// O arquivo é `PQ2_0` e o motor que vai abri-lo é o Vulkan, que roda
    /// esse formato na CPU. Não bloqueia: a tela sugere o `PTQ1_0`.
    pub pq2_on_cpu: bool,
}

impl RuntimeManager {
    /// O estado do motor da PrismML numa variante específica. Nunca oferece
    /// RPC: o cluster é do oficial.
    pub fn prism_state(&self, variant: BackendVariant) -> RuntimeState {
        let mut s = self.state_for(TAG, variant);
        s.rpc_exe = None;
        s.rpc_ready = false;
        s
    }

    /// O CUDA 12.8 reprovou na prova por execução nesta máquina, com o
    /// driver de agora.
    pub fn cuda128_failed(&self, profile: &HardwareProfile) -> bool {
        std::fs::read_to_string(marcador(&self.data_dir, BackendVariant::Cuda128))
            .is_ok_and(|c| marcador_vale(&c, driver_da_placa(profile).as_deref()))
    }

    /// A cadeia desta máquina sem o que já reprovou aqui (e não está
    /// instalado — o que está no disco e roda não se descarta por um
    /// marcador).
    fn cadeia_viva(&self, profile: &HardwareProfile) -> Vec<BackendVariant> {
        let falhou = self.cuda128_failed(profile);
        chain(profile)
            .into_iter()
            .filter(|v| {
                !(*v == BackendVariant::Cuda128 && falhou && !self.prism_state(*v).installed)
            })
            .collect()
    }

    /// O motor da PrismML desta máquina: a melhor variante INSTALADA da
    /// cadeia (o `cuda-12.8` quando está lá; senão a reserva — quem já tinha
    /// o Vulkan continua abrindo o `PTQ1_0` sem baixar nada). Sem nenhuma
    /// instalada, a variante que a instalação vai buscar primeiro.
    pub fn prism_state_resolved(&self, profile: &HardwareProfile) -> RuntimeState {
        for variant in chain(profile) {
            let s = self.prism_state(variant);
            if s.installed {
                return s;
            }
        }
        let alvo = self
            .cadeia_viva(profile)
            .first()
            .copied()
            .unwrap_or_else(|| preferred_variant(profile));
        self.prism_state(alvo)
    }

    /// O motor da PrismML para um arquivo. Igual ao da máquina, com uma
    /// exceção: um arquivo `PQ2_0` numa máquina que pode ter o CUDA 12.8,
    /// sem ele instalado e sem falha registrada, EXIGE o CUDA — no Vulkan
    /// esse formato roda na CPU a menos de 1 tok/s. O estado volta como não
    /// instalado, e o fluxo de "precisa do motor" instala o CUDA.
    pub fn prism_state_for_file(&self, profile: &HardwareProfile, pq2: bool) -> RuntimeState {
        let resolvido = self.prism_state_resolved(profile);
        if pq2
            && resolvido.variant != BackendVariant::Cuda128
            && preferred_variant(profile) == BackendVariant::Cuda128
            && !self.cuda128_failed(profile)
        {
            return self.prism_state(BackendVariant::Cuda128);
        }
        resolvido
    }

    /// O resumo para a tela (ver [`PrismStatus`]). `pq2` é o que o
    /// cabeçalho do arquivo diz; sem arquivo, `false`.
    pub fn prism_status(&self, profile: &HardwareProfile, pq2: bool) -> PrismStatus {
        let os = profile.os.as_str();
        let state = self.prism_state_for_file(profile, pq2);
        let preferida = preferred_variant(profile);
        let cuda_failed = self.cuda128_failed(profile);
        let cuda_pendente = preferida == BackendVariant::Cuda128
            && !cuda_failed
            && !self.prism_state(BackendVariant::Cuda128).installed;
        PrismStatus {
            preferred: preferida,
            installed_variant: chain(profile)
                .into_iter()
                .find(|v| self.prism_state(*v).installed),
            download_bytes: if state.installed {
                0
            } else {
                download_bytes(os, state.variant)
            },
            disk_bytes: disk_bytes(os, state.variant),
            cuda_download_bytes: cuda_pendente.then(|| download_bytes(os, BackendVariant::Cuda128)),
            cuda_failed,
            pq2,
            pq2_on_cpu: pq2 && state.installed && state.variant == BackendVariant::Vulkan,
            state,
        }
    }

    /// Instala o motor da PrismML numa variante só, sem cadeia. Serializado
    /// com a instalação do oficial pelo mesmo lock; a prova de instalação é
    /// o `--version` e, no CUDA 12.8, a placa no `--list-devices`.
    pub async fn ensure_prism(
        &self,
        variant: BackendVariant,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let mut s = self
            .ensure_package(REPOSITORY, TAG, variant, Some(UPSTREAM_BUILD), on_event)
            .await?;
        s.rpc_exe = None;
        s.rpc_ready = false;
        Ok(s)
    }

    /// Instala o melhor motor da PrismML que esta máquina aguenta: a
    /// preferida e, se a prova por execução a reprovar AQUI, as reservas.
    ///
    /// - Só a reprovação por execução (erro tipado) desce a cadeia e, no
    ///   CUDA 12.8, grava o marcador com o driver. Rede, disco, digest ou
    ///   placa ocupada voltam como erro na hora: não dizem nada da máquina,
    ///   e cair no Vulkan em silêncio deixaria o `PQ2_0` na CPU sem ninguém
    ///   saber por quê.
    /// - Já instalada, a variante volta na hora (sem baixar nada).
    /// - Instalar o CUDA apaga um marcador antigo.
    /// - Uma cadeia por vez: quem chega enquanto outra corre (dois `PQ2_0`
    ///   na fila de downloads, o botão do cartão) espera e relê o marcador
    ///   e o disco depois, em vez de repetir o que acabou de reprovar.
    /// - [`Self::cancel_prism_install`] desiste: o download e a sessão saem,
    ///   e nada é marcado.
    pub async fn ensure_prism_best(
        &self,
        profile: &HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, RuntimeError> {
        let on_event: &(dyn Fn(RuntimeEvent) + Send + Sync) = &on_event;
        let cadeia = self.instalar_cadeia(
            profile,
            |variant, eventos| async move {
                self.ensure_package(REPOSITORY, TAG, variant, Some(UPSTREAM_BUILD), eventos)
                    .await
            },
            on_event,
        );
        self.com_cancelamento(cadeia, on_event).await
    }

    /// Desiste da instalação do motor da PrismML em curso (e das que
    /// esperam na fila). Sem nenhuma, não faz nada.
    pub fn cancel_prism_install(&self) {
        self.prism_cancel.notify_waiters();
    }

    /// Esquece que o CUDA 12.8 reprovou nesta máquina: a próxima instalação
    /// o tenta de novo. É o "tentar de novo" da tela — "o motor não achou a
    /// placa" pode ter sido uma causa que a pessoa já resolveu.
    pub fn forget_cuda128_failure(&self) {
        let _ = std::fs::remove_file(marcador(&self.data_dir, BackendVariant::Cuda128));
    }

    /// `fut` até terminar, ou até [`Self::cancel_prism_install`]. Cancelar
    /// derruba o futuro — a sessão de download se limpa no `Drop`, o
    /// `llama-server` de uma prova morre com ele — e avisa a tela com o
    /// código [`CANCELLED`], que ela não mostra como erro.
    async fn com_cancelamento(
        &self,
        fut: impl std::future::Future<Output = Result<RuntimeState, RuntimeError>>,
        on_event: &(dyn Fn(RuntimeEvent) + Send + Sync),
    ) -> Result<RuntimeState, RuntimeError> {
        // Criado ANTES de esperar: o `notify_waiters` acorda todo `Notified`
        // que já exista, inclusive o de quem ainda espera a fila.
        let cancelado = self.prism_cancel.notified();
        tokio::select! {
            r = fut => r,
            () = cancelado => {
                log::info!("instalação do motor da PrismML cancelada");
                on_event(RuntimeEvent::Failed {
                    message: CANCELLED.to_string(),
                });
                Err(RuntimeError::Cancelled)
            }
        }
    }

    /// O miolo de [`Self::ensure_prism_best`], com a instalação de cada
    /// variante injetada — o que os testes trocam por um dublê.
    async fn instalar_cadeia<'a, T, Fut>(
        &'a self,
        profile: &HardwareProfile,
        tentar: T,
        on_event: &'a (dyn Fn(RuntimeEvent) + Send + Sync),
    ) -> Result<RuntimeState, RuntimeError>
    where
        T: FnMut(BackendVariant, Box<dyn Fn(RuntimeEvent) + Send + Sync + 'a>) -> Fut,
        Fut: std::future::Future<Output = Result<RuntimeState, RuntimeError>>,
    {
        let _fila = self.prism_lock.lock().await;
        // Relida DEPOIS da fila: o marcador e o disco são os de agora.
        let cadeia = self.cadeia_viva(profile);
        let mut s = percorrer_cadeia(
            &cadeia,
            tentar,
            on_event,
            |variant, erro| {
                if variant == BackendVariant::Cuda128 {
                    self.registrar_falha(profile, erro);
                }
            },
            |variant| {
                if variant == BackendVariant::Cuda128 {
                    self.forget_cuda128_failure();
                }
            },
        )
        .await?;
        s.rpc_exe = None;
        s.rpc_ready = false;
        Ok(s)
    }

    /// Baixar este arquivo leva o motor da PrismML junto? Pelo NOME (antes
    /// do download não há cabeçalho): o fork é suportado aqui e o estado
    /// dele para o arquivo não está instalado — um `PQ2_0` pede o CUDA 12.8
    /// no Linux mesmo com o Vulkan no disco. Quem chama já sabe que o
    /// arquivo exige o fork.
    pub fn installs_with_download(&self, profile: &HardwareProfile, artifact_name: &str) -> bool {
        supported(profile)
            && !self
                .prism_state_for_file(profile, name_says_pq2(artifact_name))
                .installed
    }

    /// As pastas do fork que a verificação e a limpeza preservam: a
    /// resolvida primeiro (a que o app usa) e, no Linux, o `cuda-12.8`
    /// instalado mesmo quando esta sessão não o resolve — com o NVML fora
    /// do ar (driver atualizado, reinício pendente) a preferida cai para o
    /// Vulkan, e a limpeza apagaria 1 GB que volta a valer no próximo boot.
    pub fn prism_dirs_to_keep(&self, profile: &HardwareProfile) -> Vec<PathBuf> {
        let resolvido = self.prism_state_resolved(profile);
        let mut pastas = vec![crate::runtime_dir(
            &self.data_dir,
            &resolvido.tag,
            resolvido.variant,
        )];
        let cuda = self.prism_state(BackendVariant::Cuda128);
        if let Some(dir) = cuda.dir
            && !pastas.contains(&dir)
        {
            pastas.push(dir);
        }
        pastas
    }

    fn registrar_falha(&self, profile: &HardwareProfile, erro: &str) {
        let caminho = marcador(&self.data_dir, BackendVariant::Cuda128);
        let conteudo = format!(
            "{}\n{}\n",
            driver_da_placa(profile).unwrap_or_default(),
            erro.lines().next().unwrap_or("")
        );
        let gravado = caminho
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&caminho, conteudo));
        if let Err(e) = gravado {
            log::warn!(
                "marcador de falha do CUDA não gravou em {}: {e}",
                caminho.display()
            );
        }
    }
}

/// A cadeia de instalação, separada do download para ser testável:
/// `tentar(variante, eventos)` instala uma variante e devolve o estado;
/// `eventos` é o callback da tela já filtrado.
///
/// O `Failed` de uma tentativa que ainda tem reserva não chega à tela: se a
/// reserva der certo, o painel ficaria para sempre com o erro de uma
/// variante que não está em uso. Quando a cadeia para num erro que não
/// desce (rede, disco…), o `Failed` sai uma vez, daqui.
pub(crate) async fn percorrer_cadeia<'a, T, Fut>(
    cadeia: &[BackendVariant],
    mut tentar: T,
    on_event: &'a (dyn Fn(RuntimeEvent) + Send + Sync),
    mut ao_reprovar: impl FnMut(BackendVariant, &str),
    mut ao_instalar: impl FnMut(BackendVariant),
) -> Result<RuntimeState, RuntimeError>
where
    T: FnMut(BackendVariant, Box<dyn Fn(RuntimeEvent) + Send + Sync + 'a>) -> Fut,
    Fut: std::future::Future<Output = Result<RuntimeState, RuntimeError>>,
{
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut desceu = false;
    for (i, &variant) in cadeia.iter().enumerate() {
        let ultima = i + 1 == cadeia.len();
        let pronto = Arc::new(AtomicBool::new(false));
        let visto = Arc::clone(&pronto);
        let eventos: Box<dyn Fn(RuntimeEvent) + Send + Sync + 'a> = Box::new(move |ev| {
            if matches!(ev, RuntimeEvent::Ready) {
                visto.store(true, Ordering::SeqCst);
            }
            if ultima || !matches!(ev, RuntimeEvent::Failed { .. }) {
                on_event(ev);
            }
        });
        match tentar(variant, eventos).await {
            Ok(s) => {
                ao_instalar(variant);
                // A reserva que já estava no disco volta sem evento nenhum;
                // depois de uma tentativa que baixou e reprovou, a tela
                // ficaria "instalando" para sempre sem este `Ready`.
                if desceu && !pronto.load(Ordering::SeqCst) {
                    on_event(RuntimeEvent::Ready);
                }
                return Ok(s);
            }
            Err(RuntimeError::Execution(motivo)) if !ultima => {
                log::warn!(
                    "motor da PrismML {variant:?} reprovou nesta máquina ({motivo}); \
                     tentando a reserva"
                );
                ao_reprovar(variant, &motivo);
                desceu = true;
            }
            Err(e) => {
                if !ultima {
                    on_event(RuntimeEvent::Failed {
                        message: e.to_string(),
                    });
                }
                return Err(e);
            }
        }
    }
    Err(RuntimeError::Verification(
        "nenhuma variante do motor da PrismML para esta máquina".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::GpuInfo;
    use std::sync::Mutex;

    fn perfil(os: &str, gpus: Vec<GpuInfo>) -> HardwareProfile {
        HardwareProfile {
            os: os.into(),
            arch: "x86_64".into(),
            cpu_name: "t".into(),
            cpu_cores: 8,
            avx2: true,
            avx512: false,
            ram_total_bytes: 64 << 30,
            ram_speed_mts: None,
            ram_channels: None,
            ram_bandwidth_bytes_s: None,
            gpus,
        }
    }

    fn nvidia(driver: &str, cc: Option<(u32, u32)>) -> GpuInfo {
        GpuInfo {
            name: "NVIDIA GeForce RTX 3090".into(),
            vendor: GpuVendor::Nvidia,
            vram_total_bytes: 24 << 30,
            is_integrated: false,
            driver_version: Some(driver.into()),
            cuda_compute: cc,
            bandwidth_bytes_s: None,
        }
    }

    fn amd() -> GpuInfo {
        GpuInfo {
            name: "Radeon RX 7800 XT".into(),
            vendor: GpuVendor::Amd,
            vram_total_bytes: 16 << 30,
            is_integrated: false,
            driver_version: None,
            cuda_compute: None,
            bandwidth_bytes_s: None,
        }
    }

    /// A máquina do Pedro: Bluefin, RTX 3090, driver 615.71.09.
    fn bluefin() -> HardwareProfile {
        perfil("linux", vec![nvidia("615.71.09", Some((8, 6)))])
    }

    fn instala(data_dir: &Path, variant: BackendVariant) -> PathBuf {
        let dir = crate::runtime_dir(data_dir, TAG, variant);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::server_exe_name()), b"x").unwrap();
        dir
    }

    fn marca_falha(data_dir: &Path, driver: &str) {
        let m = marcador(data_dir, BackendVariant::Cuda128);
        std::fs::create_dir_all(m.parent().unwrap()).unwrap();
        std::fs::write(m, format!("{driver}\no motor CUDA não achou a placa\n")).unwrap();
    }

    #[test]
    fn the_preferred_variant_is_cuda128_only_where_it_can_run() {
        use BackendVariant::*;
        let linux = |gpu: GpuInfo| perfil("linux", vec![gpu]);

        assert_eq!(preferred_variant(&bluefin()), Cuda128);
        // Minor de três dígitos: 570.124 é ramo 570, não "570,124 < 570,26".
        assert_eq!(
            preferred_variant(&linux(nvidia("570.124.04", Some((8, 9))))),
            Cuda128
        );
        assert_eq!(
            preferred_variant(&linux(nvidia("565.57.01", Some((8, 6))))),
            Vulkan
        );
        assert_eq!(preferred_variant(&linux(amd())), Vulkan);
        // Sem NVML (container sem `--nvidia`): sem `libcuda.so.1` também.
        assert_eq!(preferred_variant(&linux(nvidia("615.71.09", None))), Vulkan);
        // RTX 50: o binário pronto decodifica errado (KNOWN_ISSUES #199).
        assert_eq!(
            preferred_variant(&linux(nvidia("615.71.09", Some((12, 0))))),
            Vulkan
        );
        // Sem AVX2: o `libggml-cpu` do pacote CUDA morreria com SIGILL.
        let mut sem_avx2 = bluefin();
        sem_avx2.avx2 = false;
        assert_eq!(preferred_variant(&sem_avx2), Vulkan);
        // Não há pacote CUDA do fork para Linux ARM.
        let mut arm = bluefin();
        arm.arch = "aarch64".into();
        assert_eq!(preferred_variant(&arm), crate::select_variant(&arm));
        // Driver ilegível ou ausente: nada de CUDA.
        let mut sem_driver = bluefin();
        sem_driver.gpus[0].driver_version = None;
        assert_eq!(preferred_variant(&sem_driver), Vulkan);
        assert_eq!(preferred_variant(&perfil("linux", vec![])), Cpu);
    }

    /// Windows e macOS não mudam: a variante do fork é a do oficial.
    #[test]
    fn windows_and_macos_keep_the_official_variant() {
        for gpus in [
            vec![nvidia("581.42", Some((8, 6)))],
            vec![nvidia("545.00", Some((8, 9)))],
            vec![nvidia("615.71.09", Some((12, 0)))],
            vec![amd()],
            vec![],
        ] {
            let win = perfil("windows", gpus.clone());
            assert_eq!(preferred_variant(&win), crate::select_variant(&win));
            assert_eq!(chain(&win), vec![crate::select_variant(&win)]);
            let mut mac = perfil("macos", gpus);
            mac.arch = "aarch64".into();
            assert_eq!(preferred_variant(&mac), crate::select_variant(&mac));
            assert_eq!(chain(&mac).len(), 1);
        }
    }

    #[test]
    fn the_linux_chain_falls_back_to_vulkan_then_cpu() {
        use BackendVariant::*;
        assert_eq!(chain(&bluefin()), vec![Cuda128, Vulkan, Cpu]);
        assert_eq!(chain(&perfil("linux", vec![amd()])), vec![Vulkan, Cpu]);
        assert_eq!(chain(&perfil("linux", vec![])), vec![Cpu]);
    }

    /// Toda variante que a cadeia de algum sistema pode pedir tem asset na
    /// release do fork — com o nome `linux-cuda-12.8` (não `ubuntu-`).
    #[test]
    fn every_variant_in_a_chain_has_a_fork_asset() {
        let perfis = [
            bluefin(),
            perfil("linux", vec![amd()]),
            perfil("linux", vec![]),
            perfil("windows", vec![nvidia("581.42", Some((8, 6)))]),
            perfil("windows", vec![amd()]),
        ];
        for p in perfis {
            for v in chain(&p) {
                assert!(
                    crate::asset_name_for(&p.os, TAG, v).is_some(),
                    "{}/{v:?} sem pacote no fork",
                    p.os
                );
            }
        }
        assert_eq!(
            crate::asset_name_for("linux", TAG, BackendVariant::Cuda128).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-linux-cuda-12.8-x64.tar.gz")
        );
    }

    /// O padrão de nome do upstream, com a tag do fork, dá exatamente o
    /// asset publicado na release (conferido em 2026-09-20).
    #[test]
    fn the_fork_assets_follow_the_upstream_naming() {
        let nome = |os, v| crate::asset_name_for(os, TAG, v);
        assert_eq!(
            nome("windows", BackendVariant::Cuda13).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-win-cuda-13.3-x64.zip")
        );
        assert_eq!(
            nome("windows", BackendVariant::Cuda12).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-win-cuda-12.4-x64.zip")
        );
        assert_eq!(
            nome("windows", BackendVariant::Vulkan).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-win-vulkan-x64.zip")
        );
        assert_eq!(
            nome("windows", BackendVariant::Cpu).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-win-cpu-x64.zip")
        );
        assert_eq!(
            nome("linux", BackendVariant::Vulkan).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-ubuntu-vulkan-x64.tar.gz")
        );
        assert_eq!(
            nome("linux", BackendVariant::Cpu).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-ubuntu-x64.tar.gz")
        );
        assert_eq!(
            nome("linux", BackendVariant::Cuda128).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-linux-cuda-12.8-x64.tar.gz")
        );
        assert_eq!(
            nome("macos", BackendVariant::MacosArm64).as_deref(),
            Some("llama-prism-b10709-9a9394a-bin-macos-arm64.tar.gz")
        );
        assert_eq!(
            crate::cudart_asset_name_for("windows", TAG, BackendVariant::Cuda13).as_deref(),
            Some("cudart-llama-bin-win-cuda-13.3-x64.zip")
        );
        assert_eq!(
            crate::asset_url(REPOSITORY, TAG, "x.zip"),
            "https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10709-9a9394a/x.zip"
        );
    }

    /// Os pinos: só o CUDA 12.8 do fork no Linux é fixado; o tamanho
    /// mostrado é o soma do tarball com os dois wheels; e os wheels levam
    /// exatamente os três `.so` e as licenças.
    #[test]
    fn the_pinned_cuda128_package_is_complete() {
        use BackendVariant::*;
        let f = pacote_fixado("linux", TAG, Cuda128).expect("pino do CUDA 12.8");
        assert_eq!(f.bytes, 167_241_119);
        assert!(f.exige_dispositivo_cuda);
        assert!(pacote_fixado("windows", TAG, Cuda128).is_none());
        assert!(pacote_fixado("linux", crate::PINNED_TAG, Cuda128).is_none());
        assert!(pacote_fixado("linux", TAG, Vulkan).is_none());

        let hex = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
        assert!(hex(f.sha256));
        let mut libs = Vec::new();
        for roda in f.bibliotecas {
            assert!(hex(roda.sha256), "{}", roda.arquivo);
            assert!(roda.url.starts_with("https://files.pythonhosted.org/"));
            assert!(roda.url.ends_with(roda.arquivo));
            assert!(roda.arquivo.ends_with(".whl"));
            for (membro, destino) in roda.membros {
                assert!(!destino.contains('/'));
                if destino.starts_with("lib") {
                    assert!(membro.ends_with(destino), "{membro} -> {destino}");
                    libs.push(*destino);
                } else {
                    assert!(destino.starts_with("NVIDIA-LICENSE-"));
                    assert!(membro.ends_with(".dist-info/License.txt"));
                }
            }
        }
        libs.sort();
        assert_eq!(
            libs,
            ["libcublas.so.12", "libcublasLt.so.12", "libcudart.so.12"]
        );

        assert_eq!(download_bytes("linux", Cuda128), 762_542_805);
        assert!(
            f.pico_em_disco > f.bytes + f.bibliotecas.iter().map(|r| r.bytes).sum::<u64>(),
            "o pico inclui o que foi extraído, não só o download"
        );
        assert_eq!(download_bytes("linux", Vulkan), 34_248_149);
        assert_eq!(download_bytes("windows", Cuda13), 536_001_917);
        assert_eq!(disk_bytes("linux", Cuda128), Some(1_074_957_745));
        assert_eq!(disk_bytes("linux", Vulkan), None);
    }

    #[test]
    fn a_pq2_name_is_recognized_before_the_download() {
        assert!(name_says_pq2("Ternary-Bonsai-2-27B-PQ2_0.gguf"));
        assert!(name_says_pq2("bonsai-pq2_0"));
        assert!(!name_says_pq2("Ternary-Bonsai-2-27B-PTQ1_0.gguf"));
        assert!(!name_says_pq2("Qwen3-8B-Q4_K_M.gguf"));
    }

    #[test]
    fn the_prism_state_never_offers_rpc() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        assert!(!mgr.prism_state(BackendVariant::Cuda13).installed);

        let dir = crate::runtime_dir(tmp.path(), TAG, BackendVariant::Cuda13);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::server_exe_name()), b"x").unwrap();
        std::fs::write(dir.join(crate::rpc_exe_name()), b"x").unwrap();
        let s = mgr.prism_state(BackendVariant::Cuda13);
        assert!(s.installed);
        assert_eq!(s.tag, TAG);
        assert_eq!(s.server_exe, Some(dir.join(crate::server_exe_name())));
        assert!(!s.rpc_ready && s.rpc_exe.is_none());
        // O oficial continua não instalado: são pastas diferentes.
        assert!(!mgr.state(BackendVariant::Cuda13).installed);
    }

    /// A melhor INSTALADA vale; o Vulkan de quem já o tinha continua
    /// servindo; o `PQ2_0` exige o CUDA enquanto ele for possível e não
    /// tiver falhado com este driver.
    #[test]
    fn the_resolution_prefers_what_is_installed_and_pq2_asks_for_cuda() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mgr = RuntimeManager::new(d.into());
        let p = bluefin();

        // Nada instalado: a instalação vai buscar o CUDA 12.8.
        let s = mgr.prism_state_resolved(&p);
        assert!(!s.installed);
        assert_eq!(s.variant, Cuda128);

        // Só o Vulkan (quem vem da 0.24.2): ele vale para o PTQ1_0...
        instala(d, Vulkan);
        let s = mgr.prism_state_resolved(&p);
        assert!(s.installed);
        assert_eq!(s.variant, Vulkan);
        assert_eq!(mgr.prism_state_for_file(&p, false).variant, Vulkan);
        // ...e o PQ2_0 pede o CUDA.
        let s = mgr.prism_state_for_file(&p, true);
        assert!(!s.installed);
        assert_eq!(s.variant, Cuda128);

        // O CUDA reprovou com ESTE driver: o PQ2_0 fica no Vulkan.
        marca_falha(d, "615.71.09");
        assert!(mgr.cuda128_failed(&p));
        let s = mgr.prism_state_for_file(&p, true);
        assert!(s.installed);
        assert_eq!(s.variant, Vulkan);

        // Driver novo: o marcador não vale mais e o CUDA volta a ser pedido.
        let mut novo = p.clone();
        novo.gpus[0].driver_version = Some("620.10.02".into());
        assert!(!mgr.cuda128_failed(&novo));
        assert_eq!(mgr.prism_state_for_file(&novo, true).variant, Cuda128);

        // Com o CUDA instalado, ele vale para tudo.
        instala(d, Cuda128);
        for pq2 in [false, true] {
            let s = mgr.prism_state_for_file(&novo, pq2);
            assert!(s.installed);
            assert_eq!(s.variant, Cuda128);
        }
    }

    /// Com o marcador e nada instalado, a instalação vai direto ao Vulkan —
    /// e é o tamanho dele que a tela mostra.
    #[test]
    fn a_failed_cuda_is_skipped_by_the_install_target() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let p = bluefin();
        marca_falha(tmp.path(), "615.71.09");
        let s = mgr.prism_state_resolved(&p);
        assert!(!s.installed);
        assert_eq!(s.variant, Vulkan);

        let st = mgr.prism_status(&p, true);
        assert_eq!(st.state.variant, Vulkan);
        assert_eq!(st.download_bytes, download_bytes("linux", Vulkan));
        assert!(st.cuda_failed);
        assert_eq!(st.cuda_download_bytes, None);
        assert!(!st.pq2_on_cpu, "ainda não instalado: nada roda");
    }

    #[test]
    fn the_status_tells_the_screen_what_to_offer() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mgr = RuntimeManager::new(d.into());
        let p = bluefin();

        let st = mgr.prism_status(&p, false);
        assert!(!st.state.installed);
        assert_eq!(st.preferred, Cuda128);
        assert_eq!(st.installed_variant, None);
        assert_eq!(st.download_bytes, 762_542_805);
        assert_eq!(st.disk_bytes, Some(1_074_957_745));

        instala(d, Vulkan);
        let st = mgr.prism_status(&p, true);
        assert!(!st.state.installed, "PQ2_0 exige o CUDA");
        assert_eq!(st.installed_variant, Some(Vulkan));
        assert_eq!(st.cuda_download_bytes, Some(762_542_805));
        assert!(st.pq2 && !st.pq2_on_cpu);

        let st = mgr.prism_status(&p, false);
        assert!(st.state.installed);
        assert_eq!(st.download_bytes, 0);

        // AMD: o CUDA não existe aqui, e o PQ2_0 abre no Vulkan — na CPU.
        let radeon = perfil("linux", vec![amd()]);
        let st = mgr.prism_status(&radeon, true);
        assert!(st.state.installed);
        assert!(st.pq2_on_cpu);
        assert_eq!(st.cuda_download_bytes, None);
        assert!(!st.cuda_failed);
    }

    /// O Windows resolve só a variante de sempre: um Vulkan do fork que
    /// sobrou no disco não vira o motor de uma NVIDIA com CUDA 13.
    #[test]
    fn windows_resolves_only_its_single_variant() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mgr = RuntimeManager::new(d.into());
        let win = perfil("windows", vec![nvidia("581.42", Some((8, 6)))]);
        instala(d, Vulkan);
        let s = mgr.prism_state_resolved(&win);
        assert!(!s.installed);
        assert_eq!(s.variant, Cuda13);
        // O PQ2_0 não muda nada fora do Linux.
        assert_eq!(mgr.prism_state_for_file(&win, true).variant, Cuda13);
        instala(d, Cuda13);
        assert!(mgr.prism_state_for_file(&win, true).installed);
    }

    #[test]
    fn the_failure_marker_is_tied_to_the_driver() {
        assert!(marcador_vale("615.71.09\nerro\n", Some("615.71.09")));
        assert!(marcador_vale(" 615.71.09 \n", Some("615.71.09")));
        assert!(!marcador_vale("615.71.09\nerro\n", Some("620.10.02")));
        assert!(!marcador_vale("615.71.09\n", None));
        assert!(!marcador_vale("\nerro\n", Some("615.71.09")));
        assert!(!marcador_vale("", Some("")));
        let m = marcador(Path::new("/d"), BackendVariant::Cuda128);
        assert_eq!(
            m,
            Path::new("/d/runtimes").join(TAG).join("cuda-12.8.failed")
        );
    }

    /// A identidade carimba a variante que mediu: o histórico não mistura
    /// número do Vulkan com número do CUDA 12.8.
    #[test]
    fn the_identity_is_the_fork_on_the_variant_that_measured() {
        let windows = perfil("windows", vec![]);
        let id = identity(&windows, crate::select_variant(&windows));
        assert_eq!(id.source, EngineSource::Prism);
        assert_eq!(id.revision, TAG);
        assert_eq!(id.backend, "cpu");
        assert_eq!(id.platform, "windows");
        assert!(supported(&windows));
        assert!(supported(&bluefin()));

        assert_eq!(
            identity(&bluefin(), BackendVariant::Cuda128).backend,
            "cuda-12.8"
        );
        assert_eq!(
            identity(&bluefin(), BackendVariant::Vulkan).backend,
            "vulkan"
        );
    }

    fn estado(variant: BackendVariant) -> RuntimeState {
        RuntimeState {
            tag: TAG.into(),
            variant,
            installed: true,
            server_exe: None,
            dir: None,
            rpc_exe: None,
            rpc_ready: false,
        }
    }

    type Registro = Mutex<Vec<String>>;

    fn nome_do_evento(ev: &RuntimeEvent) -> String {
        match ev {
            RuntimeEvent::Progress { asset, .. } => format!("progress:{asset}"),
            RuntimeEvent::Extracting { asset } => format!("extracting:{asset}"),
            RuntimeEvent::Ready => "ready".into(),
            RuntimeEvent::Failed { message } => format!("failed:{message}"),
        }
    }

    /// A preferida reprova na execução: marca a falha, desce para a reserva,
    /// e a tela não vê o `Failed` intermediário.
    #[tokio::test]
    async fn an_execution_failure_falls_back_without_a_visible_failed_event() {
        use BackendVariant::*;
        let tela = Registro::default();
        let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
        let mut reprovadas = Vec::new();
        let mut instaladas = Vec::new();
        let r = percorrer_cadeia(
            &[Cuda128, Vulkan, Cpu],
            |v, eventos| async move {
                eventos(RuntimeEvent::Progress {
                    asset: format!("{v:?}"),
                    received_bytes: 1,
                    total_bytes: 2,
                });
                if v == Cuda128 {
                    eventos(RuntimeEvent::Failed {
                        message: "sem placa".into(),
                    });
                    Err(RuntimeError::Execution(
                        "o motor CUDA não achou a placa".into(),
                    ))
                } else {
                    eventos(RuntimeEvent::Ready);
                    Ok(estado(v))
                }
            },
            &on_event,
            |v, motivo| reprovadas.push((v, motivo.to_string())),
            |v| instaladas.push(v),
        )
        .await;
        assert_eq!(r.unwrap().variant, Vulkan);
        assert_eq!(
            reprovadas,
            vec![(Cuda128, "o motor CUDA não achou a placa".to_string())]
        );
        assert_eq!(instaladas, vec![Vulkan]);
        let vistos = tela.lock().unwrap().clone();
        assert_eq!(
            vistos,
            vec!["progress:Cuda128", "progress:Vulkan", "ready"],
            "nenhum Failed chega à tela quando a reserva deu certo"
        );
    }

    /// A reserva já estava no disco (volta sem evento): depois de o CUDA
    /// baixar e reprovar, a tela recebe um `Ready` — senão o painel ficaria
    /// "instalando" para sempre.
    #[tokio::test]
    async fn a_fallback_already_on_disk_still_closes_the_progress() {
        use BackendVariant::*;
        let tela = Registro::default();
        let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
        let r = percorrer_cadeia(
            &[Cuda128, Vulkan, Cpu],
            |v, eventos| async move {
                if v == Cuda128 {
                    eventos(RuntimeEvent::Progress {
                        asset: "cuda".into(),
                        received_bytes: 1,
                        total_bytes: 1,
                    });
                    Err(RuntimeError::Execution("sem placa".into()))
                } else {
                    Ok(estado(v))
                }
            },
            &on_event,
            |_, _| {},
            |_| {},
        )
        .await;
        assert_eq!(r.unwrap().variant, Vulkan);
        assert_eq!(*tela.lock().unwrap(), vec!["progress:cuda", "ready"]);

        // Sem descer a cadeia, nada a fechar: a preferida já instalada volta
        // calada, como sempre voltou.
        let tela = Registro::default();
        let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
        percorrer_cadeia(
            &[Cuda128, Vulkan],
            |v, _| async move { Ok(estado(v)) },
            &on_event,
            |_, _| {},
            |_| {},
        )
        .await
        .unwrap();
        assert!(tela.lock().unwrap().is_empty());
    }

    /// Rede, disco, digest, placa ocupada: a cadeia para, sem marcador e sem
    /// reserva, e o erro chega à tela UMA vez.
    #[tokio::test]
    async fn a_transient_failure_stops_the_chain_without_marking() {
        use BackendVariant::*;
        for erro in [
            RuntimeError::Verification("SHA256 divergente".into()),
            RuntimeError::NoSpace("precisa de ~1,8 GB".into()),
            RuntimeError::Io(std::io::Error::other("disco cheio")),
        ] {
            let tela = Registro::default();
            let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
            let mut reprovadas = Vec::new();
            let mut tentadas = Vec::new();
            let mensagem = erro.to_string();
            let mut erro = Some(erro);
            let r = percorrer_cadeia(
                &[Cuda128, Vulkan, Cpu],
                |v, eventos| {
                    tentadas.push(v);
                    let e = erro.take();
                    async move {
                        match e {
                            Some(e) => {
                                eventos(RuntimeEvent::Failed {
                                    message: e.to_string(),
                                });
                                Err(e)
                            }
                            None => Ok(estado(v)),
                        }
                    }
                },
                &on_event,
                |v, _| reprovadas.push(v),
                |_| {},
            )
            .await;
            assert!(r.is_err());
            assert_eq!(tentadas, vec![Cuda128], "{mensagem}: não desce a cadeia");
            assert!(reprovadas.is_empty(), "{mensagem}: não marca a máquina");
            assert_eq!(*tela.lock().unwrap(), vec![format!("failed:{mensagem}")]);
        }
    }

    /// Tudo reprova: o erro é o da última, e o `Failed` sai uma vez só.
    #[tokio::test]
    async fn when_every_variant_fails_the_last_error_is_shown_once() {
        use BackendVariant::*;
        let tela = Registro::default();
        let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
        let r = percorrer_cadeia(
            &[Cuda128, Vulkan],
            |v, eventos| async move {
                let msg = format!("{v:?} não executa");
                eventos(RuntimeEvent::Failed {
                    message: msg.clone(),
                });
                Err(RuntimeError::Execution(msg))
            },
            &on_event,
            |_, _| {},
            |_| {},
        )
        .await;
        assert!(matches!(r, Err(RuntimeError::Execution(m)) if m == "Vulkan não executa"));
        assert_eq!(*tela.lock().unwrap(), vec!["failed:Vulkan não executa"]);
    }

    /// Sem rede: o que já está no disco sai na hora. Com o marcador, o CUDA
    /// nem é tentado; com o CUDA instalado, um marcador de outro driver some.
    #[tokio::test]
    async fn ensure_best_returns_what_is_installed_without_downloading() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mgr = RuntimeManager::new(d.into());
        let p = bluefin();

        instala(d, Vulkan);
        marca_falha(d, "615.71.09");
        let s = mgr.ensure_prism_best(&p, |_| {}).await.unwrap();
        assert_eq!(s.variant, Vulkan);
        assert!(!s.rpc_ready);

        marca_falha(d, "600.00.01");
        instala(d, Cuda128);
        let s = mgr.ensure_prism_best(&p, |_| {}).await.unwrap();
        assert_eq!(s.variant, Cuda128);
        assert!(
            !marcador(d, Cuda128).exists(),
            "o CUDA instalado apaga o marcador velho"
        );
    }

    /// A cadeia de verdade (com o marcador em disco), com a instalação de
    /// cada variante trocada por um dublê: a reprovação por execução do
    /// CUDA grava o marcador com o driver na primeira linha; o que é do
    /// momento não grava nada.
    #[tokio::test]
    async fn only_an_execution_failure_of_the_cuda_marks_the_machine() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let p = bluefin();
        let r = mgr
            .instalar_cadeia(
                &p,
                |v, _| async move {
                    if v == Cuda128 {
                        Err(RuntimeError::Execution(
                            "o motor CUDA não achou a placa: no CUDA-capable device is detected"
                                .into(),
                        ))
                    } else {
                        Ok(estado(v))
                    }
                },
                &|_| {},
            )
            .await;
        assert_eq!(r.unwrap().variant, Vulkan);
        let gravado = std::fs::read_to_string(marcador(tmp.path(), Cuda128)).unwrap();
        assert_eq!(gravado.lines().next(), Some("615.71.09"));
        assert!(gravado.contains("no CUDA-capable device"));
        assert!(mgr.cuda128_failed(&p));

        // "Tentar de novo" esquece o marcador.
        mgr.forget_cuda128_failure();
        assert!(!mgr.cuda128_failed(&p));

        for erro in [
            RuntimeError::GpuUnavailable("out of memory".into()),
            RuntimeError::Verification("llama-server não respondeu em 25s".into()),
            RuntimeError::NoSpace("precisa de ~1,8 GB".into()),
            RuntimeError::Cancelled,
        ] {
            let mensagem = erro.to_string();
            let mut erro = Some(erro);
            let r = mgr
                .instalar_cadeia(
                    &p,
                    |v, _| {
                        let e = erro.take();
                        async move { e.map_or_else(|| Ok(estado(v)), Err) }
                    },
                    &|_| {},
                )
                .await;
            assert!(r.is_err(), "{mensagem}");
            assert!(
                !marcador(tmp.path(), Cuda128).exists(),
                "{mensagem}: não marca a máquina"
            );
        }
    }

    /// Dois `PQ2_0` na fila disparam duas cadeias: a segunda espera a
    /// primeira e relê o marcador — o CUDA que acabou de reprovar não é
    /// baixado de novo.
    #[tokio::test]
    async fn a_queued_chain_does_not_retry_what_just_failed() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let p = bluefin();
        let tentativas = Mutex::new(Vec::new());
        let tentar = |v: BackendVariant, _| {
            tentativas.lock().unwrap().push(v);
            async move {
                tokio::task::yield_now().await;
                if v == Cuda128 {
                    Err(RuntimeError::Execution("sem placa".into()))
                } else {
                    Ok(estado(v))
                }
            }
        };
        let (a, b) = tokio::join!(
            mgr.instalar_cadeia(&p, tentar, &|_| {}),
            mgr.instalar_cadeia(&p, tentar, &|_| {})
        );
        assert_eq!(a.unwrap().variant, Vulkan);
        assert_eq!(b.unwrap().variant, Vulkan);
        assert_eq!(*tentativas.lock().unwrap(), vec![Cuda128, Vulkan, Vulkan]);
    }

    /// Cancelar derruba a instalação em curso, avisa a tela com o código
    /// que ela não mostra como erro e não marca nada.
    #[tokio::test]
    async fn cancelling_stops_the_install_and_tells_the_screen() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let tela = Registro::default();
        let on_event = |ev: RuntimeEvent| tela.lock().unwrap().push(nome_do_evento(&ev));
        let (r, ()) = tokio::join!(
            mgr.com_cancelamento(std::future::pending(), &on_event),
            async {
                tokio::task::yield_now().await;
                mgr.cancel_prism_install();
            }
        );
        assert!(matches!(r, Err(RuntimeError::Cancelled)));
        assert_eq!(*tela.lock().unwrap(), vec![format!("failed:{CANCELLED}")]);
        assert!(!marcador(tmp.path(), BackendVariant::Cuda128).exists());
        // Sem ninguém instalando, cancelar não faz nada.
        mgr.cancel_prism_install();
    }

    /// Um pacote falso (um script no lugar do `llama-server`) na pasta de
    /// espera: a prova roda nele, sem rede. Placa cheia guarda o pacote
    /// conferido e ele não conta como instalado; a próxima tentativa só
    /// refaz a prova e publica. A reprovação da máquina joga o pacote fora.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_proof_runs_before_publishing_and_a_busy_card_keeps_the_bytes() {
        use BackendVariant::*;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let final_dir = crate::runtime_dir(tmp.path(), TAG, Cuda128);
        let pendente = crate::pending_dir(&final_dir);
        assert_eq!(pendente.file_name().unwrap(), "cuda-12.8.pending");
        std::fs::create_dir_all(&pendente).unwrap();
        let exe = pendente.join(crate::server_exe_name());
        let escreve = |lista: &str| {
            std::fs::write(
                &exe,
                format!(
                    "#!/bin/sh\ncase \"$1\" in\n--version) echo 'version: 0.1.0-dev (build 10709, commit 9a9394a)';;\n\
                     *) printf '%b' '{lista}';;\nesac\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        };

        escreve("CUDA error: out of memory\\nAvailable devices:\\n  (none)\\n");
        let r = mgr.ensure_prism(Cuda128, |_| {}).await;
        assert!(matches!(r, Err(RuntimeError::GpuUnavailable(_))), "{r:?}");
        assert!(exe.is_file(), "os bytes conferidos ficam para a próxima");
        assert!(
            !mgr.prism_state(Cuda128).installed,
            "sem prova, não instalado"
        );

        escreve(
            "Available devices:\\n  CUDA0: NVIDIA GeForce RTX 3090 (24173 MiB, 22820 MiB free)\\n",
        );
        let s = mgr.ensure_prism(Cuda128, |_| {}).await.unwrap();
        assert!(s.installed);
        assert_eq!(s.dir.as_deref(), Some(final_dir.as_path()));
        assert!(!pendente.exists());

        // A CUDA não sobe nesta máquina: o pacote sai.
        std::fs::remove_dir_all(&final_dir).unwrap();
        std::fs::create_dir_all(&pendente).unwrap();
        escreve(
            "E ggml_cuda_init: failed to initialize CUDA: no CUDA-capable device is detected\\n",
        );
        let r = mgr.ensure_prism(Cuda128, |_| {}).await;
        assert!(matches!(r, Err(RuntimeError::Execution(_))), "{r:?}");
        assert!(!pendente.exists());
        assert!(!final_dir.exists());
    }

    /// O download de um Bonsai leva o motor junto quando o estado do fork
    /// PARA AQUELE ARQUIVO não está instalado: o `PQ2_0` pede o CUDA mesmo
    /// com o Vulkan no disco; o `PTQ1_0`, não.
    #[test]
    fn the_download_takes_the_engine_along_when_the_file_needs_it() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let p = bluefin();
        let pq2 = "Ternary-Bonsai-2-27B-PQ2_0.gguf";
        let ptq1 = "Ternary-Bonsai-2-27B-PTQ1_0.gguf";
        assert!(mgr.installs_with_download(&p, ptq1));
        assert!(mgr.installs_with_download(&p, pq2));

        instala(tmp.path(), Vulkan);
        assert!(!mgr.installs_with_download(&p, ptq1));
        assert!(mgr.installs_with_download(&p, pq2));

        // CUDA reprovado com este driver: o PQ2_0 abre no Vulkan que está aí.
        marca_falha(tmp.path(), "615.71.09");
        assert!(!mgr.installs_with_download(&p, pq2));

        let mut sem_suporte = p.clone();
        sem_suporte.os = "freebsd".into();
        assert!(!mgr.installs_with_download(&sem_suporte, ptq1));
    }

    /// A limpeza preserva o `cuda-12.8` instalado mesmo numa sessão em que
    /// o NVML não respondeu e a resolução caiu para o Vulkan.
    #[test]
    fn the_installed_cuda_is_kept_even_when_this_session_cannot_resolve_it() {
        use BackendVariant::*;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mgr = RuntimeManager::new(d.into());
        let vulkan = instala(d, Vulkan);
        let cuda = instala(d, Cuda128);
        assert_eq!(mgr.prism_dirs_to_keep(&bluefin()), vec![cuda.clone()]);

        let mut sem_nvml = bluefin();
        sem_nvml.gpus[0].cuda_compute = None;
        assert_eq!(mgr.prism_dirs_to_keep(&sem_nvml), vec![vulkan, cuda]);
    }

    /// Teste live da prova na pasta de espera, sem rede: `LR_LIVE_DATA`
    /// aponta para um data_dir com `runtimes/<TAG>/cuda-12.8.pending` já
    /// montado (o tarball do fork com as bibliotecas da NVIDIA ao lado). A
    /// instalação reaproveita o pacote, prova `--version` e `--list-devices`
    /// com a placa de verdade — o `LD_LIBRARY_PATH` com a pasta de espera na
    /// frente vem do `host_env` — e só então publica. Rodar NO HOST.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "GPU NVIDIA + pacote montado à mão em LR_LIVE_DATA"]
    async fn live_linux_prism_cuda_pending_package_is_proved_then_published() {
        let data = std::env::var_os("LR_LIVE_DATA")
            .map(PathBuf::from)
            .expect("defina LR_LIVE_DATA");
        let mgr = RuntimeManager::new(data.clone());
        let final_dir = crate::runtime_dir(&data, TAG, BackendVariant::Cuda128);
        let pendente = crate::pending_dir(&final_dir);
        assert!(
            pendente.join(crate::server_exe_name()).is_file(),
            "monte o pacote em {}",
            pendente.display()
        );
        assert!(!mgr.prism_state(BackendVariant::Cuda128).installed);
        let s = mgr
            .ensure_prism(BackendVariant::Cuda128, |_| {})
            .await
            .expect("prova e publicação");
        assert!(s.installed);
        assert!(!pendente.exists());
        match crate::check::probe_cuda_devices(&final_dir).await {
            crate::check::DispositivosCuda::Presente(linha) => eprintln!("{linha}"),
            outro => panic!("a placa não apareceu: {outro:?}"),
        }
    }

    /// Teste live — confere que a release existe e cobre os assets que o app
    /// vai pedir, com digest; e que o pino do CUDA 12.8 bate com o digest e
    /// o tamanho publicados. Roda só com `--ignored`.
    #[tokio::test]
    #[ignore = "rede: consulta a release do fork da PrismML no GitHub"]
    async fn live_prism_release_digests_cover_the_assets() {
        let client = lr_fetch::client("OpenWeights-teste").unwrap();
        let digests = lr_fetch::github_release_digests(&client, REPOSITORY, TAG)
            .await
            .expect("release do fork indisponível");
        for (os, v) in [
            ("windows", BackendVariant::Cuda13),
            ("windows", BackendVariant::Cuda12),
            ("windows", BackendVariant::Vulkan),
            ("windows", BackendVariant::Cpu),
            ("linux", BackendVariant::Cuda128),
            ("linux", BackendVariant::Vulkan),
            ("linux", BackendVariant::Cpu),
            ("macos", BackendVariant::MacosArm64),
        ] {
            let a = crate::asset_name_for(os, TAG, v).expect("variante publicada");
            assert!(digests.contains_key(&a), "asset sem digest: {a}");
        }
        assert!(digests.contains_key("cudart-llama-bin-win-cuda-13.3-x64.zip"));

        let cuda = crate::asset_name_for("linux", TAG, BackendVariant::Cuda128).unwrap();
        let fixado = pacote_fixado("linux", TAG, BackendVariant::Cuda128).unwrap();
        assert_eq!(digests.get(&cuda).map(String::as_str), Some(fixado.sha256));
        #[derive(serde::Deserialize)]
        struct Asset {
            name: String,
            size: u64,
        }
        #[derive(serde::Deserialize)]
        struct Release {
            assets: Vec<Asset>,
        }
        let release: Release = client
            .get(format!(
                "https://api.github.com/repos/{REPOSITORY}/releases/tags/{TAG}"
            ))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let tamanho = release
            .assets
            .iter()
            .find(|a| a.name == cuda)
            .map(|a| a.size);
        assert_eq!(tamanho, Some(fixado.bytes));
    }

    /// Teste live — os pinos das bibliotecas da NVIDIA batem com o JSON do
    /// PyPI (URL, tamanho e SHA256). Roda só com `--ignored`.
    #[tokio::test]
    #[ignore = "rede: consulta o PyPI"]
    async fn live_nvidia_wheel_pins_match_pypi() {
        #[derive(serde::Deserialize)]
        struct Digests {
            sha256: String,
        }
        #[derive(serde::Deserialize)]
        struct Arquivo {
            filename: String,
            url: String,
            size: u64,
            digests: Digests,
        }
        #[derive(serde::Deserialize)]
        struct Versao {
            urls: Vec<Arquivo>,
        }
        let client = lr_fetch::client("OpenWeights-teste").unwrap();
        for (pacote, versao, roda) in [
            ("nvidia-cuda-runtime-cu12", "12.8.90", &RODA_CUDART),
            ("nvidia-cublas-cu12", "12.8.4.1", &RODA_CUBLAS),
        ] {
            let v: Versao = client
                .get(format!("https://pypi.org/pypi/{pacote}/{versao}/json"))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let a = v
                .urls
                .iter()
                .find(|a| a.filename == roda.arquivo)
                .unwrap_or_else(|| panic!("{} não está no PyPI", roda.arquivo));
            assert_eq!(a.url, roda.url);
            assert_eq!(a.size, roda.bytes);
            assert_eq!(a.digests.sha256, roda.sha256);
        }
    }

    /// Teste live da instalação do CUDA 12.8 de ponta a ponta, com os
    /// arquivos EXATOS fixados: baixa o tarball e os dois wheels (~727 MB),
    /// confere os pinos, extrai só os membros, prova `--version` == 10709 e
    /// `--list-devices` com uma linha `CUDA<n>:`. Não carrega modelo.
    ///
    /// Precisa de uma NVIDIA com driver >= 570 e do userspace do driver: no
    /// Bluefin, rodar o binário de teste NO HOST (o box `claude` não tem
    /// `libcuda.so.1`). `LR_LIVE_DIR` escolhe onde (padrão: o temp).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "rede + GPU NVIDIA: baixa ~727 MB e cria um contexto CUDA"]
    async fn live_linux_prism_cuda_installs_and_lists_the_gpu() {
        let base = std::env::var_os("LR_LIVE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let tmp = tempfile::tempdir_in(base).unwrap();
        let mgr = RuntimeManager::new(tmp.path().into());
        let eventos = Mutex::new(Vec::new());
        let s = mgr
            .ensure_prism(BackendVariant::Cuda128, |ev| {
                eventos.lock().unwrap().push(nome_do_evento(&ev));
            })
            .await
            .expect("instalação do CUDA 12.8 do fork");
        assert!(s.installed);
        let dir = s.dir.expect("pasta instalada");
        for f in [
            "libcudart.so.12",
            "libcublas.so.12",
            "libcublasLt.so.12",
            "NVIDIA-LICENSE-cudart.txt",
            "NVIDIA-LICENSE-cublas.txt",
            "llama-fit-params",
            "llama-bench",
        ] {
            assert!(dir.join(f).is_file(), "{f} ausente");
        }
        assert!(
            !dir.join("libnvblas.so.12").exists(),
            "só os membros fixados"
        );
        // Nada de `.part` nem de sessão sobrando.
        let sobras: Vec<_> = std::fs::read_dir(tmp.path().join("runtimes/.tmp"))
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        assert!(sobras.is_empty(), "sobrou {sobras:?}");
        let (build, _) = crate::check::probe_build(&dir).await.expect("executa");
        assert_eq!(build, UPSTREAM_BUILD);
        match crate::check::probe_cuda_devices(&dir).await {
            crate::check::DispositivosCuda::Presente(linha) => eprintln!("{linha}"),
            outro => panic!("a placa não apareceu: {outro:?}"),
        }
        let vistos = eventos.lock().unwrap().clone();
        assert!(vistos.iter().any(|e| e.contains("nvidia_cublas_cu12")));
        assert_eq!(vistos.last().map(String::as_str), Some("ready"));
        eprintln!(
            "instalado: {} MB em disco",
            lr_fetch::dir_size(&dir) / 1_000_000
        );
    }
}
