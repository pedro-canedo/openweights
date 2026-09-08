//! Exercise the app's real resumable download path outside the webview.
//! Usage: validation_download REPO ARTIFACT_NAME MODELS_DIRECTORY
use lr_models::{DownloadManager, DownloadRequest, DownloadState, HfClient, group_artifacts};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("expected REPO ARTIFACT_NAME MODELS_DIRECTORY".into());
    }
    let files = HfClient::new(None).repo_files(&args[0]).await?;
    let artifact = match group_artifacts(&files)
        .into_iter()
        .find(|a| a.name == args[1])
    {
        Some(artifact) => artifact,
        None => {
            eprintln!("artifact not found; available artifacts:");
            for artifact in group_artifacts(&files) {
                eprintln!("  {}", artifact.name);
            }
            return Err("use the exact complete artifact name".into());
        }
    };
    println!("{}: {} bytes", artifact.name, artifact.total_bytes);
    let manager = DownloadManager::new(args[2].clone().into());
    let id = manager
        .enqueue(DownloadRequest {
            repo_id: args[0].clone(),
            artifact_name: artifact.name,
            files: artifact.files,
            token: None,
        })
        .await?;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let status = manager
            .list()
            .await
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("missing job")?;
        println!(
            "{} / {} bytes ({}/s)",
            status.received_bytes, status.total_bytes, status.bytes_per_sec
        );
        match status.state {
            DownloadState::Done => return Ok(()),
            DownloadState::Error => return Err(status.error.unwrap_or_default().into()),
            _ => {}
        }
    }
}
