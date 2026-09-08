//! Reproducible opt-in runtime. No branch-tip downloads or local toolchain.
use crate::{BackendVariant, RuntimeEvent, RuntimeManager, RuntimeState};
use lr_types::{HardwareProfile, tuning::EngineSource};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const REVISION: &str = "b46f7f7a436f990932d3da3ec53380e2b9effc89";
pub const TAG: &str = "moe-runtime-b46f7f7a436f-v1";
pub const REPOSITORY: &str = "pedro-canedo/openweights";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeIdentity {
    pub source: EngineSource,
    pub revision: String,
    pub backend: String,
    pub platform: String,
}

pub fn supported(profile: &HardwareProfile) -> bool {
    matches!(profile.os.as_str(), "windows" | "linux")
        && profile.arch == "x86_64"
        && matches!(crate::select_variant(profile), BackendVariant::Cuda13)
}

pub fn identity() -> RuntimeIdentity {
    RuntimeIdentity {
        source: EngineSource::MoeCache,
        revision: REVISION.into(),
        backend: "cuda-13.3".into(),
        platform: std::env::consts::OS.into(),
    }
}

/// Identity stored with measurements produced by the official package too.
/// The official runtime is pinned by release tag rather than a fork commit.
pub fn official_identity(profile: &HardwareProfile) -> RuntimeIdentity {
    let backend = match crate::select_variant(profile) {
        BackendVariant::Cuda13 => "cuda-13.3",
        BackendVariant::Cuda12 => "cuda-12.4",
        BackendVariant::Vulkan => "vulkan",
        BackendVariant::Cpu => "cpu",
        BackendVariant::MacosArm64 => "metal-arm64",
        BackendVariant::MacosX64 => "metal-x64",
    };
    RuntimeIdentity {
        source: EngineSource::Official,
        revision: crate::PINNED_TAG.into(),
        backend: backend.into(),
        platform: profile.os.clone(),
    }
}

pub fn asset_name() -> String {
    format!(
        "openweights-{TAG}-{}-x64-cuda13.3.{}",
        std::env::consts::OS,
        if cfg!(windows) { "zip" } else { "tar.gz" }
    )
}

impl RuntimeManager {
    pub fn experimental_state(&self) -> RuntimeState {
        let dir = self
            .data_dir
            .join("runtimes")
            .join(TAG)
            .join(std::env::consts::OS);
        let valid = std::fs::read(dir.join("runtime.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RuntimeIdentity>(&bytes).ok())
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

    pub async fn ensure_experimental(
        &self,
        profile: &HardwareProfile,
        cancelled: &AtomicBool,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<RuntimeState, String> {
        if !supported(profile) {
            return Err("optimization-unsupported".into());
        }
        let _guard = self.install_lock.lock().await;
        let state = self.experimental_state();
        if cancelled.load(Ordering::SeqCst) {
            return Err("comparison-cancelled".into());
        }
        if state.installed {
            verify_capabilities(state.server_exe.as_ref().unwrap()).await?;
            return Ok(state);
        }
        let session =
            lr_fetch::Session::new(&self.data_dir.join("runtimes")).map_err(|e| e.to_string())?;
        let client = lr_fetch::client("OpenWeights-runtime").map_err(|e| e.to_string())?;
        let asset = asset_name();
        // Fail closed: an unpublished release or absent digest is not an
        // invitation to run an unverified executable.
        let digests = lr_fetch::github_release_digests(&client, REPOSITORY, TAG)
            .await
            .filter(|d| d.contains_key(&asset))
            .ok_or("optimization-package-unavailable")?;
        let archive = session.path().join("download.part");
        let url = format!("https://github.com/{REPOSITORY}/releases/download/{TAG}/{asset}");
        let update = |received_bytes, total_bytes| {
            on_event(RuntimeEvent::Progress {
                asset: asset.clone(),
                received_bytes,
                total_bytes,
            })
        };
        {
            let download = lr_fetch::download_to(&client, &url, &archive, &update);
            tokio::pin!(download);
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    return Err("comparison-cancelled".into());
                }
                tokio::select! {
                    result = &mut download => { result.map_err(|e| e.to_string())?; break; },
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
            }
        }
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
        // Extraction uses a blocking worker. Wait for it before disposing of
        // the session, even if cancellation arrived while it was running.
        if cancelled.load(Ordering::SeqCst) {
            return Err("comparison-cancelled".into());
        }
        let root = lr_fetch::find_dir_containing(&extracted, crate::server_exe_name())
            .ok_or("optimization-package-incomplete")?;
        let manifest: RuntimeIdentity = serde_json::from_slice(
            &std::fs::read(root.join("runtime.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if manifest != identity() {
            return Err("optimization-package-identity".into());
        }
        for stem in ["llama-server", "llama-fit-params", "llama-bench"] {
            if !root.join(crate::exe_name(stem)).is_file() {
                return Err("optimization-package-incomplete".into());
            }
        }
        verify_capabilities(&root.join(crate::server_exe_name())).await?;
        if cancelled.load(Ordering::SeqCst) {
            return Err("comparison-cancelled".into());
        }
        let dest = self
            .data_dir
            .join("runtimes")
            .join(TAG)
            .join(std::env::consts::OS);
        lr_fetch::install_atomically(&root, &dest).map_err(|e| e.to_string())?;
        on_event(RuntimeEvent::Ready);
        Ok(self.experimental_state())
    }
}

pub async fn verify_capabilities(exe: &std::path::Path) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(exe);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        lr_proc::no_window(&mut cmd)
            .arg("--help")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "optimization-runtime-timeout")?
    .map_err(|e| e.to_string())?;
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success()
        || !help.contains("--moe-expert-cache-size")
        || !help.contains("--load-mode")
    {
        return Err("optimization-runtime-incompatible".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_or_foreign_installations_are_not_ready() {
        let temp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(temp.path().into());
        let dir = temp
            .path()
            .join("runtimes")
            .join(TAG)
            .join(std::env::consts::OS);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::server_exe_name()), b"incomplete").unwrap();
        assert!(!mgr.experimental_state().installed);
        let mut wrong = identity();
        wrong.revision = "foreign".into();
        std::fs::write(
            dir.join("runtime.json"),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        assert!(!mgr.experimental_state().installed);
    }
}
