use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::models::{AppConfig, ServerEntry};

pub const CONSOLE_BACKLOG: usize = 500;

pub struct ServerRuntime {
    pub child: Option<Child>,
    pub stdin: Option<ChildStdin>,
    pub tx: broadcast::Sender<String>,
    pub backlog: Vec<String>,
    pub running: bool,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub pid: Option<u32>,
    /// Set right before we deliberately stop/kill the process, so the crash
    /// watcher can tell "user asked for this" apart from "it just died" and
    /// only auto-restarts in the latter case.
    pub stopping: bool,
}

impl Default for ServerRuntime {
    fn default() -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Self {
            child: None,
            stdin: None,
            tx,
            backlog: Vec::new(),
            running: false,
            started_at: None,
            pid: None,
            stopping: false,
        }
    }
}

impl ServerRuntime {
    pub fn push_line(&mut self, line: String) {
        self.backlog.push(line.clone());
        if self.backlog.len() > CONSOLE_BACKLOG {
            let excess = self.backlog.len() - CONSOLE_BACKLOG;
            self.backlog.drain(0..excess);
        }
        let _ = self.tx.send(line);
    }

    pub async fn send_command(&mut self, cmd: &str) -> anyhow::Result<()> {
        if let Some(stdin) = self.stdin.as_mut() {
            stdin.write_all(cmd.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            Ok(())
        } else {
            anyhow::bail!("server is not running")
        }
    }
}

pub struct AppStateInner {
    pub data_dir: PathBuf,
    pub servers: RwLock<HashMap<Uuid, ServerEntry>>,
    pub runtime: RwLock<HashMap<Uuid, ServerRuntime>>,
    pub http: reqwest::Client,
    pub config: RwLock<AppConfig>,
    pub playit_child: RwLock<Option<Child>>,
    pub playit_tx: broadcast::Sender<String>,
    /// Replayed to every new `/api/playit/ws` subscriber on connect, same
    /// pattern as `ServerRuntime::backlog`. Without this, output printed by
    /// the agent between `start_agent()` returning and the frontend opening
    /// its WebSocket (which always happens - the UI awaits the REST call
    /// first) was silently dropped, even when the binary was already
    /// downloaded and started instantly.
    pub playit_backlog: RwLock<Vec<String>>,
    pub backup_progress: RwLock<HashMap<Uuid, BackupProgress>>,
}

/// Live progress of a running backup, polled by the UI. `done` is shared
/// with the blocking zip task via an Arc so updates are visible without
/// re-acquiring the map lock from the worker thread on every file.
pub struct BackupProgress {
    pub done: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub total: u64,
}

pub type AppState = Arc<AppStateInner>;

fn registry_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("servers.json")
}

fn config_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("config.json")
}

pub async fn build_state() -> anyhow::Result<AppState> {
    let data_dir = resolve_data_dir();
    tokio::fs::create_dir_all(&data_dir).await.ok();
    tokio::fs::create_dir_all(data_dir.join("servers")).await.ok();
    acquire_instance_lock(&data_dir)?;

    let servers: HashMap<Uuid, ServerEntry> = match tokio::fs::read_to_string(registry_path(&data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    let config: AppConfig = match tokio::fs::read_to_string(config_path(&data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| default_config(&data_dir)),
        Err(_) => default_config(&data_dir),
    };

    let http = reqwest::Client::builder()
        .user_agent("mcmanager/1.0.0 (+https://github.com/yolezz/mcmanager)")
        .build()?;

    let (playit_tx, _rx) = broadcast::channel(512);

    let state = Arc::new(AppStateInner {
        data_dir,
        servers: RwLock::new(servers),
        runtime: RwLock::new(HashMap::new()),
        http,
        config: RwLock::new(config),
        playit_child: RwLock::new(None),
        playit_tx,
        playit_backlog: RwLock::new(Vec::new()),
        backup_progress: RwLock::new(HashMap::new()),
    });

    Ok(state)
}

fn default_config(data_dir: &PathBuf) -> AppConfig {
    AppConfig {
        java_path: "java".to_string(),
        playit_path: None,
        update_repo: "yolezz/mcmanager".to_string(),
        check_updates_on_start: true,
        data_dir: data_dir.to_string_lossy().to_string(),
    }
}

fn resolve_data_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("MCMANAGER_DATA_DIR") {
        return PathBuf::from(custom);
    }
    if let Some(dir) = dirs::data_dir() {
        return dir.join("mcmanager");
    }
    PathBuf::from("./mcmanager-data")
}

/// Both binaries (`mcmanager`, the web app, and `mcmanager-headless`, the
/// CLI-only one) supervise server processes purely in-memory - nothing about
/// which PIDs belong to which server is shared between OS processes. Running
/// two instances against the same data directory at once is a real risk of
/// double-starting a server or corrupting `servers.json`, not just a
/// theoretical one, since it's now easy to end up with one of each running
/// (GUI on a desktop, headless on a server, both pointed at a synced/shared
/// data dir). A simple exclusive lock file catches the common case.
///
/// Not a full liveness check (no cross-platform PID-alive probe here) - if
/// a previous instance crashed hard enough to leave the file behind, the
/// error message explains how to clear it manually rather than silently
/// overwriting a lock that might still be valid.
fn acquire_instance_lock(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let lock_path = data_dir.join("mcmanager.lock");
    if lock_path.exists() {
        let prev_pid = std::fs::read_to_string(&lock_path).unwrap_or_default();
        anyhow::bail!(
            "Un verrou d'instance existe deja : {}\n\
             Cela signifie qu'une autre instance de MCManager (web ou headless, pid note : {}) \
             tourne peut-etre deja sur ce dossier de donnees. Lancer deux instances en meme temps \
             sur le meme dossier peut corrompre l'etat des serveurs (demarrages en double, fichier \
             servers.json ecrase par l'une pendant que l'autre le lit...).\n\
             Si vous etes sur qu'aucune autre instance ne tourne (ex: arret brutal precedent), \
             supprimez ce fichier puis relancez.",
            lock_path.display(),
            prev_pid.trim()
        );
    }
    std::fs::write(&lock_path, std::process::id().to_string())
        .map_err(|e| anyhow::anyhow!("impossible de creer le verrou d'instance {} : {e}", lock_path.display()))?;
    Ok(())
}

pub async fn save_servers(state: &AppState) -> anyhow::Result<()> {
    let servers = state.servers.read().await;
    let json = serde_json::to_string_pretty(&*servers)?;
    tokio::fs::write(registry_path(&state.data_dir), json).await?;
    Ok(())
}

pub async fn save_config(state: &AppState) -> anyhow::Result<()> {
    let cfg = state.config.read().await;
    let json = serde_json::to_string_pretty(&*cfg)?;
    tokio::fs::write(config_path(&state.data_dir), json).await?;
    Ok(())
}
