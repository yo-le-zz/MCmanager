use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    Vanilla,
    Paper,
    Purpur,
    Spigot,
    Fabric,
    Forge,
    Neoforge,
}

impl Loader {
    pub fn as_str(&self) -> &'static str {
        match self {
            Loader::Vanilla => "vanilla",
            Loader::Paper => "paper",
            Loader::Purpur => "purpur",
            Loader::Spigot => "spigot",
            Loader::Fabric => "fabric",
            Loader::Forge => "forge",
            Loader::Neoforge => "neoforge",
        }
    }

    /// Where third-party addons live for this loader: "plugins" for Bukkit-family
    /// servers, "mods" for Fabric/Forge/NeoForge.
    pub fn addon_dir(&self) -> &'static str {
        match self {
            Loader::Vanilla => "mods", // vanilla has no addons, default harmless
            Loader::Paper | Loader::Purpur | Loader::Spigot => "plugins",
            Loader::Fabric | Loader::Forge | Loader::Neoforge => "mods",
        }
    }

    /// Matches Modrinth's `project_type` + `loaders` facet vocabulary.
    pub fn modrinth_loader(&self) -> &'static str {
        match self {
            Loader::Vanilla => "minecraft",
            Loader::Paper => "paper",
            Loader::Purpur => "purpur",
            Loader::Spigot => "spigot",
            Loader::Fabric => "fabric",
            Loader::Forge => "forge",
            Loader::Neoforge => "neoforge",
        }
    }

    pub fn is_plugin_based(&self) -> bool {
        matches!(self, Loader::Paper | Loader::Purpur | Loader::Spigot)
    }

    pub fn is_modded(&self) -> bool {
        matches!(self, Loader::Fabric | Loader::Forge | Loader::Neoforge)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: Uuid,
    pub name: String,
    pub loader: Loader,
    pub mc_version: String,
    /// Loader/installer version, e.g. Fabric loader version or Forge build.
    pub loader_version: Option<String>,
    pub folder: String,
    pub jar_name: String,
    pub java_path: String,
    pub xms_mb: u32,
    pub xmx_mb: u32,
    pub port: u16,
    pub extra_args: Vec<String>,
    pub eula_accepted: bool,
    pub auto_backup_minutes: Option<u32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub loader: Loader,
    pub mc_version: String,
    pub loader_version: Option<String>,
    #[serde(default = "default_xms")]
    pub xms_mb: u32,
    #[serde(default = "default_xmx")]
    pub xmx_mb: u32,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub eula_accepted: bool,
}

fn default_xms() -> u32 { 1024 }
fn default_xmx() -> u32 { 2048 }
fn default_port() -> u16 { 25565 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub java_path: String,
    pub playit_path: Option<String>,
    pub update_repo: String, // "owner/repo" used for self-update checks
    pub check_updates_on_start: bool,
    pub data_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub id: Uuid,
    pub running: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub cpu_percent: f32,
    pub mem_mb: f32,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub motd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonInfo {
    pub file_name: String,
    pub enabled: bool,
    pub size_bytes: u64,
    pub modrinth_project_id: Option<String>,
    pub modrinth_version_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub name: String,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetItem {
    pub key: String,
    pub label: String,
    pub description: String,
    pub category: String,
    pub modrinth_slug: String,
    pub loaders: Vec<String>,
}
