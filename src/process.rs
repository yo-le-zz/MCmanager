use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::state::{AppState, ServerRuntime};

pub async fn start_server(state: &AppState, id: Uuid) -> Result<()> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().context("serveur introuvable")?
    };

    {
        let runtime = state.runtime.read().await;
        if let Some(rt) = runtime.get(&id) {
            if rt.running {
                anyhow::bail!("le serveur tourne deja");
            }
        }
    }

    let folder = PathBuf::from(&entry.folder);
    if entry.eula_accepted {
        tokio::fs::write(folder.join("eula.txt"), "eula=true\n").await.ok();
    }

    let mut cmd = Command::new(&entry.java_path);
    cmd.arg(format!("-Xms{}M", entry.xms_mb))
        .arg(format!("-Xmx{}M", entry.xmx_mb));
    for arg in &entry.extra_args {
        cmd.arg(arg);
    }
    cmd.arg("-jar").arg(&entry.jar_name).arg("nogui");
    cmd.current_dir(&folder);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(false);

    let mut child = cmd.spawn().context("impossible de demarrer Java (verifiez le chemin Java dans les parametres)")?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pid = child.id();

    {
        let mut runtime = state.runtime.write().await;
        let rt = runtime.entry(id).or_insert_with(ServerRuntime::default);
        rt.child = Some(child);
        rt.stdin = stdin;
        rt.running = true;
        rt.started_at = Some(chrono::Utc::now());
        rt.pid = pid;
        rt.push_line("[MCManager] Demarrage du serveur...".to_string());
    }

    if let Some(stdout) = stdout {
        spawn_reader(state.clone(), id, stdout);
    }
    if let Some(stderr) = stderr {
        spawn_reader(state.clone(), id, stderr);
    }

    spawn_watcher(state.clone(), id);

    Ok(())
}

fn spawn_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(state: AppState, id: Uuid, reader: R) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut runtime = state.runtime.write().await;
            if let Some(rt) = runtime.get_mut(&id) {
                rt.push_line(line);
            }
        }
    });
}

fn spawn_watcher(state: AppState, id: Uuid) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let mut runtime = state.runtime.write().await;
            if let Some(rt) = runtime.get_mut(&id) {
                if let Some(child) = rt.child.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            rt.running = false;
                            rt.stdin = None;
                            rt.pid = None;
                            rt.push_line(format!("[MCManager] Serveur arrete ({status})."));
                            break;
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    });
}

pub async fn stop_server(state: &AppState, id: Uuid, force: bool) -> Result<()> {
    let mut runtime = state.runtime.write().await;
    let rt = runtime.get_mut(&id).context("serveur non demarre")?;
    if !rt.running {
        anyhow::bail!("le serveur n'est pas en cours d'execution");
    }
    if force {
        if let Some(child) = rt.child.as_mut() {
            child.start_kill().ok();
        }
    } else {
        rt.send_command("stop").await?;
    }
    Ok(())
}

pub async fn send_command(state: &AppState, id: Uuid, cmd: &str) -> Result<()> {
    let mut runtime = state.runtime.write().await;
    let rt = runtime.get_mut(&id).context("serveur non demarre")?;
    rt.push_line(format!("> {cmd}"));
    rt.send_command(cmd).await
}
