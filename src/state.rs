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
