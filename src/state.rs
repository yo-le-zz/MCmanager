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
        .user_agent("mcmanager/1.0.0 (+https://github.com/yo-le-zz/MCmanager)")
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

/// Test-only variant of `build_state()`: same construction, but skips
/// `acquire_instance_lock` (tests may run concurrently against their own
/// isolated temp directories, and shouldn't fight over - or leave behind -
/// a lock file meant for real, single-instance daemon usage).
#[cfg(test)]
pub async fn build_state_for_test(data_dir: &std::path::Path) -> AppState {
    let data_dir = data_dir.to_path_buf();
    tokio::fs::create_dir_all(&data_dir).await.ok();
    tokio::fs::create_dir_all(data_dir.join("servers")).await.ok();

    let http = reqwest::Client::builder().build().expect("client http de test");
    let (playit_tx, _rx) = broadcast::channel(512);

    Arc::new(AppStateInner {
        config: RwLock::new(default_config(&data_dir)),
        data_dir,
        servers: RwLock::new(HashMap::new()),
        runtime: RwLock::new(HashMap::new()),
        http,
        playit_child: RwLock::new(None),
        playit_tx,
        playit_backlog: RwLock::new(Vec::new()),
        backup_progress: RwLock::new(HashMap::new()),
    })
}

fn default_config(data_dir: &PathBuf) -> AppConfig {
    AppConfig {
        java_path: "java".to_string(),
        playit_path: None,
        update_repo: "yo-le-zz/MCmanager".to_string(),
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
/// What we found when inspecting an existing `mcmanager.lock` file.
enum LockState {
    /// No lock file at all - the common case.
    Free,
    /// The PID in the lock is alive and really is an MCManager process
    /// (`mcmanager` or `mcmanager-headless`, matched by executable name) -
    /// a real conflict, never auto-resolved.
    HeldByMcManager { pid: u32 },
    /// The PID in the lock isn't running at all anymore - a stale lock left
    /// behind by an unclean shutdown (`kill -9`, power loss, crash).
    Stale { pid: u32 },
    /// The PID in the lock *is* running, but it's some other program - most
    /// likely PID reuse by the OS after the original MCManager process
    /// exited (very possible on Linux, where PIDs cycle) rather than an
    /// actual second MCManager instance.
    ForeignProcess { pid: u32, name: String },
}

fn inspect_lock(lock_path: &std::path::Path) -> LockState {
    let Ok(content) = std::fs::read_to_string(lock_path) else { return LockState::Free };
    let Ok(pid) = content.trim().parse::<u32>() else {
        // Unreadable/corrupt content - treat like a stale lock rather than
        // refusing to start over a file we can't even interpret.
        return LockState::Stale { pid: 0 };
    };

    let mut sys = sysinfo::System::new();
    sys.refresh_process(sysinfo::Pid::from_u32(pid));
    match sys.process(sysinfo::Pid::from_u32(pid)) {
        None => LockState::Stale { pid },
        Some(proc_) => {
            let name = proc_.name().to_string();
            let is_mcmanager = name.eq_ignore_ascii_case("mcmanager")
                || name.eq_ignore_ascii_case("mcmanager.exe")
                || name.eq_ignore_ascii_case("mcmanager-headless")
                || name.eq_ignore_ascii_case("mcmanager-headless.exe");
            if is_mcmanager {
                LockState::HeldByMcManager { pid }
            } else {
                LockState::ForeignProcess { pid, name }
            }
        }
    }
}

/// Both binaries (`mcmanager`, the web app, and `mcmanager-headless`) supervise
/// server processes purely in-memory - nothing about which PIDs belong to
/// which server is shared between OS processes. Running two instances
/// against the same data directory at once is a real risk of double-starting
/// a server or corrupting `servers.json`, not just a theoretical one.
///
/// Unlike the earlier version of this function, a lock file present on disk
/// no longer means an automatic refusal: the PID it names is checked for
/// both liveness and identity (via `sysinfo`, already a dependency for the
/// CPU/RAM stats). A stale lock (process no longer running) or one that now
/// belongs to an unrelated program (PID reuse after MCManager exited) is
/// offered up for cleanup - interactively, over stdin, so an unattended
/// service (stdin closed/`/dev/null`, immediate EOF) always falls back to
/// the safe refusal rather than silently deleting a lock it can't actually
/// verify is safe to remove. A lock genuinely held by another live
/// MCManager process is never offered for deletion, attended or not.
fn acquire_instance_lock(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let lock_path = data_dir.join("mcmanager.lock");

    match inspect_lock(&lock_path) {
        LockState::Free => {}
        LockState::HeldByMcManager { pid } => {
            anyhow::bail!(
                "Une autre instance de MCManager tourne deja sur ce dossier de donnees (pid {pid}, verifie et confirme). \
                 Fermez-la avant de relancer, ou utilisez un dossier de donnees different (MCMANAGER_DATA_DIR)."
            );
        }
        LockState::Stale { pid } => {
            let prompt = if pid == 0 {
                "Le fichier de verrou d'instance existant est illisible/corrompu.".to_string()
            } else {
                format!("Le fichier de verrou d'instance existant reference le pid {pid}, qui ne tourne plus (arret brutal precedent ?).")
            };
            if !confirm_delete_lock(&prompt) {
                anyhow::bail!(
                    "{prompt}\nSupprimez {} manuellement puis relancez si vous etes sur qu'aucune autre instance ne tourne.",
                    lock_path.display()
                );
            }
            let _ = std::fs::remove_file(&lock_path);
        }
        LockState::ForeignProcess { pid, name } => {
            let prompt = format!(
                "Le fichier de verrou d'instance existant reference le pid {pid}, qui tourne bien mais correspond a \"{name}\" \
                 et non a MCManager - probablement un ancien pid reutilise par un autre programme apres l'arret de MCManager."
            );
            if !confirm_delete_lock(&prompt) {
                anyhow::bail!(
                    "{prompt}\nSupprimez {} manuellement puis relancez si vous etes sur qu'aucune instance MCManager ne tourne.",
                    lock_path.display()
                );
            }
            let _ = std::fs::remove_file(&lock_path);
        }
    }

    std::fs::write(&lock_path, std::process::id().to_string())
        .map_err(|e| anyhow::anyhow!("impossible de creer le verrou d'instance {} : {e}", lock_path.display()))?;
    Ok(())
}

/// Asks over stdin whether to delete and continue. Returns `false`
/// immediately (no deletion) if stdin isn't attended - an immediate EOF
/// (0 bytes read), as happens under systemd with stdin on `/dev/null` -
/// rather than blocking a service startup forever waiting for input nobody
/// will provide.
fn confirm_delete_lock(prompt: &str) -> bool {
    use std::io::Write;
    println!("{prompt}");
    print!("Supprimer ce verrou et continuer le demarrage ? [o/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).unwrap_or(0);
    if n == 0 {
        println!("(entree non-interactive - verrou conserve par securite)");
        return false;
    }
    matches!(line.trim().to_lowercase().as_str(), "o" | "oui" | "y" | "yes")
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
