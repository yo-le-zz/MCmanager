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
    if entry.aikar_flags {
        for arg in aikar_flags() {
            cmd.arg(arg);
        }
    }
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
        rt.stopping = false;
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
    spawn_idle_watcher(state.clone(), id);

    Ok(())
}

/// Sensible default JVM flags for a smoother-running Paper/Purpur/Spigot
/// server (Aikar's flags, https://docs.papermc.io/paper/aikars-flags) -
/// mainly G1GC tuning that reduces GC-related lag spikes on larger servers.
fn aikar_flags() -> Vec<&'static str> {
    vec![
        "-XX:+UseG1GC",
        "-XX:+ParallelRefProcEnabled",
        "-XX:MaxGCPauseMillis=200",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:+DisableExplicitGC",
        "-XX:+AlwaysPreTouch",
        "-XX:G1NewSizePercent=30",
        "-XX:G1MaxNewSizePercent=40",
        "-XX:G1HeapRegionSize=8M",
        "-XX:G1ReservePercent=20",
        "-XX:G1HeapWastePercent=5",
        "-XX:G1MixedGCCountTarget=4",
        "-XX:InitiatingHeapOccupancyPercent=15",
        "-XX:G1MixedGCLiveThresholdPercent=90",
        "-XX:G1RSetUpdatingPauseTimePercent=5",
        "-XX:SurvivorRatio=32",
        "-XX:MaxTenuringThreshold=1",
    ]
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
            let exited = {
                let mut runtime = state.runtime.write().await;
                let Some(rt) = runtime.get_mut(&id) else { break };
                let Some(child) = rt.child.as_mut() else { break };
                match child.try_wait() {
                    Ok(Some(status)) => {
                        rt.running = false;
                        rt.stdin = None;
                        rt.pid = None;
                        let intentional = rt.stopping;
                        rt.stopping = false;
                        rt.push_line(format!(
                            "[MCManager] Serveur arrete ({status}){}.",
                            if intentional { "" } else { " - arret inattendu" }
                        ));
                        Some(intentional)
                    }
                    Ok(None) => None,
                    Err(_) => break,
                }
            };

            let Some(intentional) = exited else { continue };
            if intentional {
                break;
            }

            // Unexpected exit (crash, OOM-killed, `kill -9` from outside...).
            // Auto-restart only if the user opted in for this server, after
            // a configurable delay (editable any time from Settings, even
            // after the server was created - not a fixed 5s anymore).
            let (auto_restart, delay_secs) = state.servers.read().await.get(&id)
                .map(|e| (e.auto_restart, e.auto_restart_delay_secs))
                .unwrap_or((false, 5));
            if auto_restart {
                {
                    let mut runtime = state.runtime.write().await;
                    if let Some(rt) = runtime.get_mut(&id) {
                        rt.push_line(format!("[MCManager] Redemarrage automatique dans {delay_secs}s (crash detecte)..."));
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs as u64)).await;
                if let Err(e) = start_server(&state, id).await {
                    let mut runtime = state.runtime.write().await;
                    if let Some(rt) = runtime.get_mut(&id) {
                        rt.push_line(format!("[MCManager] Echec du redemarrage automatique: {e}"));
                    }
                }
            }
            break;
        }
    });
}

/// Handles the two "hands-off maintenance" settings that run alongside a
/// live server: periodic scheduled restarts (memory-leak hygiene) and
/// auto-stop after a configurable stretch with zero players online. Both
/// are opt-in per server and editable from Settings at any time, including
/// after the server already exists.
///
/// Exits as soon as this start's session ends (server stopped, or a newer
/// start superseded it) - identified by comparing `started_at`, since a
/// fresh `start_server()` call always sets a new timestamp.
fn spawn_idle_watcher(state: AppState, id: Uuid) {
    tokio::spawn(async move {
        let session_started_at = {
            let runtime = state.runtime.read().await;
            match runtime.get(&id).and_then(|rt| rt.started_at) {
                Some(t) => t,
                None => return,
            }
        };
        let mut empty_since: Option<chrono::DateTime<chrono::Utc>> = None;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;

            let (still_this_session, entry) = {
                let runtime = state.runtime.read().await;
                let Some(rt) = runtime.get(&id) else { break };
                if !rt.running || rt.started_at != Some(session_started_at) {
                    (false, None)
                } else {
                    (true, state.servers.read().await.get(&id).cloned())
                }
            };
            if !still_this_session {
                break;
            }
            let Some(entry) = entry else { break };

            // Scheduled restart: give players a heads-up, then a graceful
            // `stop` (the crash-restart path in spawn_watcher picks it back
            // up automatically since this is a clean, non-"stopping" exit
            // only if auto_restart is also on - otherwise we restart it
            // ourselves directly here).
            if let Some(minutes) = entry.scheduled_restart_minutes {
                let elapsed = chrono::Utc::now().signed_duration_since(session_started_at);
                if elapsed.num_minutes() >= minutes as i64 {
                    {
                        let mut runtime = state.runtime.write().await;
                        if let Some(rt) = runtime.get_mut(&id) {
                            rt.push_line("[MCManager] Redemarrage programme du serveur...".to_string());
                        }
                    }
                    let _ = stop_server(&state, id, false).await;
                    // Wait for the clean shutdown to finish, then start a
                    // fresh session (bounded wait so a stuck shutdown can't
                    // hang this task forever).
                    for _ in 0..60 {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let running = state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false);
                        if !running {
                            break;
                        }
                    }
                    let _ = start_server(&state, id).await;
                    break; // the new start_server() call spawns its own idle watcher
                }
            }

            // Auto-stop when empty.
            if let Some(minutes) = entry.stop_when_empty_minutes {
                match crate::stats::ping_server(entry.port).await {
                    Ok((Some(0), _, _)) => {
                        let since = *empty_since.get_or_insert_with(chrono::Utc::now);
                        let idle_minutes = chrono::Utc::now().signed_duration_since(since).num_minutes();
                        if idle_minutes >= minutes as i64 {
                            {
                                let mut runtime = state.runtime.write().await;
                                if let Some(rt) = runtime.get_mut(&id) {
                                    rt.push_line(format!("[MCManager] Aucun joueur depuis {minutes} min - arret automatique."));
                                }
                            }
                            let _ = stop_server(&state, id, false).await;
                            break;
                        }
                    }
                    _ => {
                        // Players present, or we couldn't reach the status
                        // port (server still booting) - reset the timer
                        // either way, we only want *confirmed* emptiness.
                        empty_since = None;
                    }
                }
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
    rt.stopping = true;
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
    // Keep the intentional/crash distinction correct even when "stop" is
    // typed directly into the console instead of using the Stop button.
    let lowered = cmd.trim().to_ascii_lowercase();
    if matches!(lowered.as_str(), "stop" | "end" | "shutdown") {
        rt.stopping = true;
    }
    rt.push_line(format!("> {cmd}"));
    rt.send_command(cmd).await
}

pub async fn clear_console(state: &AppState, id: Uuid) -> Result<()> {
    let mut runtime = state.runtime.write().await;
    let rt = runtime.entry(id).or_insert_with(ServerRuntime::default);
    rt.backlog.clear();
    Ok(())
}
