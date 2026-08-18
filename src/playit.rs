use std::process::Stdio;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::state::AppState;

const RELEASES_API: &str = "https://api.github.com/repos/playit-cloud/playit-agent/releases/latest";

fn asset_name_for_platform() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    { "playit-linux-amd64" }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    { "playit-linux-aarch64" }
    #[cfg(target_os = "windows")]
    { "playit-windows-x86_64.exe" }
    #[cfg(target_os = "macos")]
    { "playit-darwin-universal" }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    { "playit-linux-amd64" }
}

/// Downloads the latest playit-agent binary for the current platform and makes it executable.
pub async fn download_agent(state: &AppState) -> Result<String> {
    let releases: Value = state.http.get(RELEASES_API).send().await?.json().await?;
    let assets = releases["assets"].as_array().context("aucune release playit-agent trouvee")?;
    let wanted = asset_name_for_platform();
    let asset = assets.iter().find(|a| a["name"].as_str().unwrap_or_default().contains(wanted))
        .or_else(|| assets.first())
        .context("aucun binaire playit-agent compatible trouve")?;
    let url = asset["browser_download_url"].as_str().context("url de telechargement manquante")?;
    let name = asset["name"].as_str().unwrap_or("playit-agent");

    let bin_dir = state.data_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    let dest = bin_dir.join(name);
    let bytes = state.http.get(url).send().await?.error_for_status()?.bytes().await?;
    tokio::fs::write(&dest, &bytes).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&dest).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&dest, perms).await?;
    }

    let path_str = dest.to_string_lossy().to_string();
    {
        let mut cfg = state.config.write().await;
        cfg.playit_path = Some(path_str.clone());
    }
    crate::state::save_config(state).await?;
    Ok(path_str)
}

pub async fn start_agent(state: &AppState) -> Result<()> {
    let path = {
        let cfg = state.config.read().await;
        cfg.playit_path.clone().context("playit-agent n'est pas installe (telechargez-le d'abord)")?
    };
    {
        let child_guard = state.playit_child.read().await;
        if child_guard.is_some() {
            anyhow::bail!("playit-agent tourne deja");
        }
    }
    let mut cmd = Command::new(&path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn().context("impossible de lancer playit-agent")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    {
        let mut guard = state.playit_child.write().await;
        *guard = Some(child);
    }

    if let Some(out) = stdout {
        let tx = state.playit_tx.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        });
    }
    if let Some(err) = stderr {
        let tx = state.playit_tx.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        });
    }

    Ok(())
}

pub async fn stop_agent(state: &AppState) -> Result<()> {
    let mut guard = state.playit_child.write().await;
    if let Some(child) = guard.as_mut() {
        child.start_kill().ok();
    }
    *guard = None;
    Ok(())
}

pub async fn status(state: &AppState) -> (bool, Option<String>) {
    let running = state.playit_child.read().await.is_some();
    let path = state.config.read().await.playit_path.clone();
    (running, path)
}
