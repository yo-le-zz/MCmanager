//! Push notifications via [ntfy](https://ntfy.sh) - a plain HTTP POST to a
//! topic URL, no bot account, no OAuth app, no webhook signing secret to
//! manage. Chosen over a Discord bot specifically for that simplicity: ntfy
//! needs nothing more than a topic name (and, for a self-hosted or
//! protected server, an optional auth token) to start receiving pushes on
//! a phone or desktop.
//!
//! Config lives in `ntfy_config.json`; the optional auth token is
//! encrypted at rest via `crate::secrets`, same mechanism and same shared
//! key file as the AI assistant's provider keys.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::secrets::{self, EncryptedBlob};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NtfyConfig {
    pub enabled: bool,
    /// e.g. "https://ntfy.sh" (default, public) or a self-hosted instance.
    #[serde(default)]
    pub server_url: String,
    pub topic: String,
    /// Plaintext in memory only - see `StoredNtfyConfig`.
    #[serde(skip)]
    pub auth_token: String,
    // Per-event toggles, all on by default once notifications are enabled
    // at all - a fresh setup should notify about everything until the user
    // decides to quiet specific ones down.
    #[serde(default = "default_true")]
    pub notify_crash: bool,
    #[serde(default = "default_true")]
    pub notify_backup: bool,
    #[serde(default = "default_true")]
    pub notify_scheduled_restart: bool,
    #[serde(default = "default_true")]
    pub notify_auto_stop: bool,
    #[serde(default)]
    pub notify_player_join_leave: bool,
}

fn default_true() -> bool { true }

impl NtfyConfig {
    fn base_url(&self) -> String {
        if self.server_url.trim().is_empty() {
            "https://ntfy.sh".to_string()
        } else {
            self.server_url.trim_end_matches('/').to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredNtfyConfig {
    enabled: bool,
    server_url: String,
    topic: String,
    #[serde(default = "default_true")]
    notify_crash: bool,
    #[serde(default = "default_true")]
    notify_backup: bool,
    #[serde(default = "default_true")]
    notify_scheduled_restart: bool,
    #[serde(default = "default_true")]
    notify_auto_stop: bool,
    #[serde(default)]
    notify_player_join_leave: bool,
    encrypted_token: Option<EncryptedBlob>,
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ntfy_config.json")
}

pub async fn load_config(data_dir: &Path) -> NtfyConfig {
    let Ok(content) = tokio::fs::read_to_string(config_path(data_dir)).await else {
        return NtfyConfig::default();
    };
    let Ok(stored) = serde_json::from_str::<StoredNtfyConfig>(&content) else {
        return NtfyConfig::default();
    };
    let auth_token = match &stored.encrypted_token {
        Some(blob) => match secrets::load_or_create_key(data_dir).await {
            Ok(key) => secrets::decrypt(&key, blob).unwrap_or_default(),
            Err(_) => String::new(),
        },
        None => String::new(),
    };
    NtfyConfig {
        enabled: stored.enabled,
        server_url: stored.server_url,
        topic: stored.topic,
        auth_token,
        notify_crash: stored.notify_crash,
        notify_backup: stored.notify_backup,
        notify_scheduled_restart: stored.notify_scheduled_restart,
        notify_auto_stop: stored.notify_auto_stop,
        notify_player_join_leave: stored.notify_player_join_leave,
    }
}

pub async fn save_config(data_dir: &Path, cfg: &NtfyConfig) -> anyhow::Result<()> {
    let encrypted_token = if cfg.auth_token.is_empty() {
        None
    } else {
        let key = secrets::load_or_create_key(data_dir).await?;
        Some(secrets::encrypt(&key, &cfg.auth_token)?)
    };
    let stored = StoredNtfyConfig {
        enabled: cfg.enabled,
        server_url: cfg.server_url.clone(),
        topic: cfg.topic.clone(),
        notify_crash: cfg.notify_crash,
        notify_backup: cfg.notify_backup,
        notify_scheduled_restart: cfg.notify_scheduled_restart,
        notify_auto_stop: cfg.notify_auto_stop,
        notify_player_join_leave: cfg.notify_player_join_leave,
        encrypted_token,
    };
    let path = config_path(data_dir);
    tokio::fs::write(&path, serde_json::to_string_pretty(&stored)?).await?;
    secrets::restrict_to_owner(&path).await;
    Ok(())
}

pub enum Event {
    Crash,
    Backup,
    ScheduledRestart,
    AutoStop,
    PlayerJoinLeave,
}

impl Event {
    fn allowed(&self, cfg: &NtfyConfig) -> bool {
        match self {
            Event::Crash => cfg.notify_crash,
            Event::Backup => cfg.notify_backup,
            Event::ScheduledRestart => cfg.notify_scheduled_restart,
            Event::AutoStop => cfg.notify_auto_stop,
            Event::PlayerJoinLeave => cfg.notify_player_join_leave,
        }
    }

    fn tags(&self) -> &'static str {
        match self {
            Event::Crash => "rotating_light",
            Event::Backup => "floppy_disk",
            Event::ScheduledRestart => "arrows_counterclockwise",
            Event::AutoStop => "pause_button",
            Event::PlayerJoinLeave => "busts_in_silhouette",
        }
    }

    fn priority(&self) -> &'static str {
        match self {
            Event::Crash => "high",
            _ => "default",
        }
    }
}

/// Sends a push notification if notifications are enabled and this event
/// type hasn't been muted. Failures are logged (via `tracing`) and
/// otherwise swallowed - a notification that doesn't arrive should never
/// be the reason a backup or restart itself fails.
pub async fn notify(http: &reqwest::Client, data_dir: &Path, event: Event, title: &str, message: &str) {
    let cfg = load_config(data_dir).await;
    if !cfg.enabled || cfg.topic.trim().is_empty() || !event.allowed(&cfg) {
        return;
    }
    if let Err(e) = send(http, &cfg, event.tags(), event.priority(), title, message).await {
        tracing::warn!("echec de l'envoi de la notification ntfy : {e}");
    }
}

async fn send(http: &reqwest::Client, cfg: &NtfyConfig, tags: &str, priority: &str, title: &str, message: &str) -> anyhow::Result<()> {
    let url = format!("{}/{}", cfg.base_url(), cfg.topic.trim());
    let mut req = http.post(&url)
        .header("Title", title)
        .header("Priority", priority)
        .header("Tags", tags)
        .body(message.to_string());
    if !cfg.auth_token.trim().is_empty() {
        req = req.bearer_auth(&cfg.auth_token);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("ntfy a repondu avec le statut {}", resp.status());
    }
    Ok(())
}

/// Sends a one-off test notification regardless of the enabled/per-event
/// toggles, so "Envoyer un test" in Settings works even before the user has
/// finished configuring everything.
pub async fn send_test(http: &reqwest::Client, cfg: &NtfyConfig) -> anyhow::Result<()> {
    if cfg.topic.trim().is_empty() {
        anyhow::bail!("indiquez un topic ntfy avant de tester");
    }
    send(http, cfg, "white_check_mark", "default", "MCManager", "Notification de test - la configuration fonctionne.").await
}
