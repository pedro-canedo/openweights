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
//! O que este motor NÃO faz: não é o motor padrão. Ele sobe quando o modelo
//! selecionado exige (ver `optimize_prepare_model` no app) e o oficial volta
//! quando outro modelo é escolhido. Sem RPC: o cluster continua no oficial.

use crate::experimental::RuntimeIdentity;
use crate::{BackendVariant, RuntimeError, RuntimeEvent, RuntimeManager, RuntimeState};
use lr_types::{HardwareProfile, tuning::EngineSource};

/// Release do fork homologada para esta versão do app (2026-09-18).
pub const TAG: &str = "prism-b10709-9a9394a";
/// A build do llama.cpp em que essa release se baseia — é o que o binário
/// reporta em `--version`, e o que a instalação confere.
pub const UPSTREAM_BUILD: u64 = 10709;
pub const REPOSITORY: &str = "PrismML-Eng/llama.cpp";

/// Tamanho aproximado do pacote CUDA 13.3 para Windows, para a tela dizer
/// quanto vai baixar antes de a pessoa clicar.
pub const TAMANHO_APROXIMADO_BYTES: u64 = 150 * 1024 * 1024;

/// O fork publica para as mesmas plataformas que o app sabe instalar
/// (Windows x64, Linux x64 e macOS), com os mesmos nomes do upstream.
pub fn supported(profile: &HardwareProfile) -> bool {
    matches!(profile.os.as_str(), "windows" | "linux" | "macos")
}

/// Identidade das medições feitas sob este motor — mesma forma da do
/// oficial e da do MoE-cache, para o histórico de desempenho distinguir.
pub fn identity(profile: &HardwareProfile) -> RuntimeIdentity {
    let mut id = crate::experimental::official_identity(profile);
    id.source = EngineSource::Prism;
    id.revision = TAG.into();
    id
}

impl RuntimeManager {
    /// O estado do motor da PrismML para a variante desta máquina. Nunca
    /// oferece RPC: o cluster é do oficial.
    pub fn prism_state(&self, variant: BackendVariant) -> RuntimeState {
        let mut s = self.state_for(TAG, variant);
        s.rpc_exe = None;
        s.rpc_ready = false;
        s
    }

    /// Instala o motor da PrismML se preciso. Serializado com a instalação
    /// do oficial pelo mesmo lock; a prova de instalação é o `--version`.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_identity_is_the_fork_on_this_machines_backend() {
        let profile = HardwareProfile {
            os: "windows".into(),
            arch: "x86_64".into(),
            cpu_name: "t".into(),
            cpu_cores: 4,
            avx2: true,
            avx512: false,
            ram_total_bytes: 16 << 30,
            ram_speed_mts: None,
            ram_channels: None,
            ram_bandwidth_bytes_s: None,
            gpus: vec![],
        };
        let id = identity(&profile);
        assert_eq!(id.source, EngineSource::Prism);
        assert_eq!(id.revision, TAG);
        assert_eq!(id.backend, "cpu");
        assert!(supported(&profile));
        assert!(supported(&HardwareProfile {
            os: "linux".into(),
            ..profile
        }));
    }

    /// Teste live — confere que a release existe e cobre os assets que o app
    /// vai pedir, com digest. Roda só com `--ignored`.
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
            ("linux", BackendVariant::Vulkan),
            ("linux", BackendVariant::Cpu),
            ("macos", BackendVariant::MacosArm64),
        ] {
            let a = crate::asset_name_for(os, TAG, v).expect("variante publicada");
            assert!(digests.contains_key(&a), "asset sem digest: {a}");
        }
        assert!(digests.contains_key("cudart-llama-bin-win-cuda-13.3-x64.zip"));
    }
}
