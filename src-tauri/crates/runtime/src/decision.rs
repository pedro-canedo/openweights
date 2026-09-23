//! O motor de decisão: um `llama-server` compilado do fork
//! `parallel-decision` (thecodacus/llama.cpp), que serve `POST /v1/decision`.
//!
//! O fork não publica releases, então o pacote é nosso: a CI
//! (`.github/workflows/decision-runtime.yml`) compila o commit pinado e
//! publica em `pedro-canedo/openweights` com um `runtime.json` de
//! identidade. Mesma disciplina do MoE-cache ([`crate::experimental`]):
//! sem digest na release não se instala nada (fail-closed), e a identidade
//! do manifesto tem de bater byte a byte com a que este código espera.
//!
//! Não é um `EngineSource`: nunca é o motor do chat, é um segundo processo
//! ao lado dele. Por isso a pasta (`runtimes/<TAG>/<os>/`) fica invisível ao
//! `check`/`prune` — que só enxergam tags `b<n>` e `prism-b<n>`.

use crate::{BackendVariant, RuntimeEvent, RuntimeManager, RuntimeState};
use lr_types::HardwareProfile;
use serde::{Deserialize, Serialize};

/// Commit do fork (ponta de `parallel-decision`, 2 commits sobre o upstream
/// `60b06ab9` de 2026-09-19).
pub const REVISION: &str = "14d04e755fa28653e87b9a07072892265bdc0fad";
pub const TAG: &str = "decision-runtime-14d04e75-v1";
pub const REPOSITORY: &str = "pedro-canedo/openweights";
/// A action de CUDA desse ponto do upstream só conhece 13.4 na série 13; roda
/// em qualquer driver >= 580 (compatibilidade menor do 13.x).
pub const BACKEND: &str = "cuda-13.4";
/// Para a tela dizer antes do clique (o zip traz cudart + cublas).
pub const TAMANHO_APROXIMADO_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionIdentity {
    pub source: String,
    pub revision: String,
    pub backend: String,
    pub platform: String,
}

/// O pacote é CUDA 13 para Windows e Linux x64: pergunta pela placa, não
/// pela variante do motor oficial — que no Linux é Vulkan.
pub fn supported(profile: &HardwareProfile) -> bool {
    matches!(profile.os.as_str(), "windows" | "linux")
        && profile.arch == "x86_64"
        && crate::cuda13_capable(profile)
}

pub fn identity() -> DecisionIdentity {
    DecisionIdentity {
        source: "decision".into(),
        revision: REVISION.into(),
        backend: BACKEND.into(),
        platform: std::env::consts::OS.into(),
    }
}

pub fn asset_name() -> String {
    format!(
        "openweights-{TAG}-{}-x64-cuda13.4.{}",
        std::env::consts::OS,
        if cfg!(windows) { "zip" } else { "tar.gz" }
    )
}

impl RuntimeManager {
    fn decision_dir(&self) -> std::path::PathBuf {
        self.data_dir
            .join("runtimes")
            .join(TAG)
            .join(std::env::consts::OS)
    }

    pub fn decision_state(&self) -> RuntimeState {
        let dir = self.decision_dir();
        let valid = std::fs::read(dir.join("runtime.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<DecisionIdentity>(&bytes).ok())
            .is_some_and(|manifest| manifest == identity());
        let exe = dir.join(crate::server_exe_name());
        let installed = valid && exe.is_file();
        RuntimeState {
            tag: TAG.into(),
            variant: BackendVariant::Cuda13,
            installed,
            server_exe: installed.then_some(exe),
            dir: installed.then_some(dir),
            rpc_exe: None,
            rpc_ready: false,
        }
    }

    /// Baixa, verifica e instala o motor de decisão. Erros são códigos
    /// estáveis (`decision-*`) que a tela traduz.
    pub async fn ensure_decision(
        &self,
        profile: &HardwareProfile,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, String> {
        if !supported(profile) {
            return Err("decision-unsupported".into());
        }
        let _guard = self.install_lock.lock().await;
        let state = self.decision_state();
        if state.installed {
            verify_capabilities(state.server_exe.as_ref().unwrap()).await?;
            on_event(RuntimeEvent::Ready);
            return Ok(state);
        }
        let resultado = self.instalar_decision(&on_event).await;
        match &resultado {
            Ok(_) => on_event(RuntimeEvent::Ready),
            Err(e) => on_event(RuntimeEvent::Failed { message: e.clone() }),
        }
        resultado
    }

    async fn instalar_decision(
        &self,
        on_event: &(impl Fn(RuntimeEvent) + Send + Sync),
    ) -> Result<RuntimeState, String> {
        let session =
            lr_fetch::Session::new(&self.data_dir.join("runtimes")).map_err(|e| e.to_string())?;
        let client = lr_fetch::client("OpenWeights-runtime").map_err(|e| e.to_string())?;
        let asset = asset_name();
        // Fail closed: release não publicada ou sem digest não é convite para
        // executar um binário sem verificação.
        let digests = lr_fetch::github_release_digests(&client, REPOSITORY, TAG)
            .await
            .filter(|d| d.contains_key(&asset))
            .ok_or("decision-package-unavailable")?;
        let archive = session.path().join("download.part");
        let url = crate::asset_url(REPOSITORY, TAG, &asset);
        let update = |received_bytes, total_bytes| {
            on_event(RuntimeEvent::Progress {
                asset: asset.clone(),
                received_bytes,
                total_bytes,
            })
        };
        lr_fetch::download_to(&client, &url, &archive, &update)
            .await
            .map_err(|e| e.to_string())?;
        lr_fetch::verify_sha256(&archive, &asset, Some(&digests))
            .await
            .map_err(|e| e.to_string())?;
        on_event(RuntimeEvent::Extracting {
            asset: asset.clone(),
        });
        let extracted = session.path().join("extracted");
        lr_fetch::extract_archive_async(archive, asset, extracted.clone())
            .await
            .map_err(|e| e.to_string())?;
        let root = lr_fetch::find_dir_containing(&extracted, crate::server_exe_name())
            .ok_or("decision-package-incomplete")?;
        let manifest: DecisionIdentity = serde_json::from_slice(
            &std::fs::read(root.join("runtime.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if manifest != identity() {
            return Err("decision-package-identity".into());
        }
        let dest = self.decision_dir();
        lr_fetch::install_atomically(&root, &dest).map_err(|e| e.to_string())?;
        // Prova por execução DEPOIS de mover: no Windows o antivírus ainda
        // segura o executável recém-extraído (ver o motor da PrismML).
        if let Err(e) = verify_capabilities(&dest.join(crate::server_exe_name())).await {
            let _ = lr_fetch::remove_dir_all_retrying(&dest);
            return Err(e);
        }
        Ok(self.decision_state())
    }
}

/// O binário é mesmo o fork: o `--help` tem de listar `--decision-seqs`.
pub async fn verify_capabilities(exe: &std::path::Path) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        lr_proc::no_window(&mut cmd)
            .arg("--help")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "decision-runtime-timeout")?
    .map_err(|e| e.to_string())?;
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !help.contains("--decision-seqs") {
        return Err("decision-runtime-incompatible".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foreign_or_incomplete_installation_is_not_ready() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(temp.path().into());
        let dir = mgr.decision_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::server_exe_name()), b"incomplete").unwrap();
        assert!(!mgr.decision_state().installed, "sem manifesto");
        let mut wrong = identity();
        wrong.revision = "foreign".into();
        std::fs::write(
            dir.join("runtime.json"),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        assert!(!mgr.decision_state().installed, "revisão estranha");
        std::fs::write(
            dir.join("runtime.json"),
            serde_json::to_vec(&identity()).unwrap(),
        )
        .unwrap();
        let st = mgr.decision_state();
        assert!(st.installed);
        assert_eq!(st.tag, TAG);
        assert!(st.server_exe.unwrap().ends_with(crate::server_exe_name()));
    }

    #[test]
    fn the_manifest_is_what_the_packaging_script_writes() {
        let v = serde_json::to_value(identity()).unwrap();
        assert_eq!(v["source"], "decision");
        assert_eq!(v["revision"], REVISION);
        assert_eq!(v["backend"], "cuda-13.4");
        assert!(asset_name().starts_with("openweights-decision-runtime-14d04e75-v1-"));
        assert!(asset_name().contains("-x64-cuda13.4."));
    }

    #[test]
    fn the_decision_runtime_is_invisible_to_prune() {
        // Sem `b<n>` na tag, o scan não a vê — logo o prune não a apaga.
        assert_eq!(crate::build_number(TAG), None);
        let temp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(temp.path().into());
        let dir = mgr.decision_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::server_exe_name()), b"x").unwrap();
        assert!(crate::scan_installed(temp.path()).is_empty());
        let r = crate::prune(temp.path(), BackendVariant::Cuda13);
        assert_eq!(r.freed_bytes, 0);
        assert!(dir.join(crate::server_exe_name()).is_file());
    }

    /// Só com rede: a release publicada precisa cobrir os assets que o app
    /// vai pedir.
    #[tokio::test]
    #[ignore = "rede: consulta a release do motor de decisão"]
    async fn live_decision_release_digests_cover_the_assets() {
        let client = lr_fetch::client("OpenWeights-teste").unwrap();
        let digests = lr_fetch::github_release_digests(&client, REPOSITORY, TAG)
            .await
            .expect("release publicada");
        for os in ["windows", "linux"] {
            let ext = if os == "windows" { "zip" } else { "tar.gz" };
            let asset = format!("openweights-{TAG}-{os}-x64-cuda13.4.{ext}");
            assert!(digests.contains_key(&asset), "sem digest para {asset}");
        }
    }
}
