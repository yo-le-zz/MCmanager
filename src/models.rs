use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    // ── Java Edition ──
    Vanilla,
    Paper,
    Purpur,
    Spigot,
    Fabric,
    Quilt,
    Forge,
    Neoforge,
    // ── Bedrock Edition (ajoutés en 1.1.3 ; les anciens serveurs.json restent lisibles tels quels) ──
    /// Serveur dédié officiel Mojang (BDS), version stable.
    Bedrock,
    /// Serveur dédié officiel Mojang, version preview (bêta).
    Bedrockpreview,
    /// PocketMine-MP : serveur communautaire en PHP, avec plugins (.phar). Tourne aussi sur ARM (Raspberry Pi).
    Pocketmine,
    /// Nukkit (CloudburstMC) : serveur communautaire en Java, avec plugins (.jar). Tourne aussi sur ARM.
    Nukkit,
}

/// Comment le serveur est lancé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// `java -jar` (mémoire réglable, drapeaux JVM).
    Java,
    /// Binaire natif (serveur officiel Bedrock).
    Native,
    /// Interpréteur PHP (PocketMine-MP).
    Php,
}

impl Loader {
    pub const ALL_BEDROCK: [Loader; 4] = [Loader::Bedrock, Loader::Bedrockpreview, Loader::Pocketmine, Loader::Nukkit];

    pub fn as_str(&self) -> &'static str {
        match self {
            Loader::Vanilla => "vanilla",
            Loader::Paper => "paper",
            Loader::Purpur => "purpur",
            Loader::Spigot => "spigot",
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::Forge => "forge",
            Loader::Neoforge => "neoforge",
            Loader::Bedrock => "bedrock",
            Loader::Bedrockpreview => "bedrockpreview",
            Loader::Pocketmine => "pocketmine",
            Loader::Nukkit => "nukkit",
        }
    }

    /// Édition de Minecraft : `"java"` ou `"bedrock"`.
    pub fn edition(&self) -> &'static str {
        if self.is_bedrock() { "bedrock" } else { "java" }
    }

    pub fn is_bedrock(&self) -> bool {
        matches!(self, Loader::Bedrock | Loader::Bedrockpreview | Loader::Pocketmine | Loader::Nukkit)
    }

    /// Serveur dédié officiel de Mojang (binaire natif, packs de comportement/ressources, pas de plugins).
    pub fn is_official_bedrock(&self) -> bool {
        matches!(self, Loader::Bedrock | Loader::Bedrockpreview)
    }

    pub fn runtime(&self) -> Runtime {
        match self {
            Loader::Bedrock | Loader::Bedrockpreview => Runtime::Native,
            Loader::Pocketmine => Runtime::Php,
            _ => Runtime::Java,
        }
    }

    /// Port par défaut : 25565 (TCP) en Java, 19132 (UDP) en Bedrock.
    pub fn default_port(&self) -> u16 {
        if self.is_bedrock() { 19132 } else { 25565 }
    }

    /// Where third-party addons live for this loader: "plugins" for Bukkit-family
    /// servers, "mods" for Fabric/Quilt/Forge/NeoForge, "behavior_packs" for the
    /// official Bedrock server (resource packs sit in "resource_packs").
    pub fn addon_dir(&self) -> &'static str {
        match self {
            Loader::Vanilla => "mods", // vanilla has no addons, default harmless
            Loader::Paper | Loader::Purpur | Loader::Spigot | Loader::Pocketmine | Loader::Nukkit => "plugins",
            Loader::Fabric | Loader::Quilt | Loader::Forge | Loader::Neoforge => "mods",
            Loader::Bedrock | Loader::Bedrockpreview => "behavior_packs",
        }
    }

    /// Extension des fichiers d'extension reconnus pour ce serveur (plugins/mods) ; `None` = dossiers (packs Bedrock).
    pub fn addon_extension(&self) -> Option<&'static str> {
        match self {
            Loader::Pocketmine => Some("phar"),
            Loader::Bedrock | Loader::Bedrockpreview => None,
            _ => Some("jar"),
        }
    }

    /// Matches Modrinth's `project_type` + `loaders` facet vocabulary (Java uniquement ; vide pour Bedrock).
    pub fn modrinth_loader(&self) -> &'static str {
        match self {
            Loader::Vanilla => "minecraft",
            Loader::Paper => "paper",
            Loader::Purpur => "purpur",
            Loader::Spigot => "spigot",
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::Forge => "forge",
            Loader::Neoforge => "neoforge",
            Loader::Bedrock | Loader::Bedrockpreview | Loader::Pocketmine | Loader::Nukkit => "",
        }
    }

    pub fn is_plugin_based(&self) -> bool {
        matches!(self, Loader::Paper | Loader::Purpur | Loader::Spigot | Loader::Pocketmine | Loader::Nukkit)
    }

    pub fn is_modded(&self) -> bool {
        matches!(self, Loader::Fabric | Loader::Quilt | Loader::Forge | Loader::Neoforge)
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
    /// How many backups to keep for this server; older ones are deleted
    /// right after a new one finishes. `None` = unlimited (the previous,
    /// only, behavior) - this stays opt-in so existing backup sets are
    /// never touched unless the user explicitly sets a limit.
    #[serde(default)]
    pub backup_retention: Option<u32>,
    #[serde(default)]
    pub auto_restart: bool,
    /// Delay before an automatic crash-restart, in seconds. Configurable
    /// (rather than the previous hardcoded 5s) and editable after creation
    /// via `UpdateServerRequest`.
    #[serde(default = "default_restart_delay")]
    pub auto_restart_delay_secs: u32,
    /// If set, gracefully restart the server every N minutes regardless of
    /// crashes (memory-leak hygiene). `None` = disabled.
    #[serde(default)]
    pub scheduled_restart_minutes: Option<u32>,
    /// If set, gracefully stop the server after it has had zero players
    /// online for N consecutive minutes. `None` = never auto-stop from
    /// inactivity (the previous, only, behavior).
    #[serde(default)]
    pub stop_when_empty_minutes: Option<u32>,
    /// "Serveur dynamique" (sleep/wake): once `stop_when_empty_minutes`
    /// auto-stops the server, instead of leaving the port dead, a
    /// lightweight listener takes it over - answers server-list pings with
    /// a "sleeping" MOTD, and on a real join attempt, starts the real
    /// server and tells the player to retry in a few seconds. Trades a
    /// slower first join (real server boot time) for not running an idle
    /// JVM 24/7. Requires `stop_when_empty_minutes` to be set - it's what
    /// decides *when* to go to sleep in the first place.
    #[serde(default)]
    pub dynamic_server: bool,
    #[serde(default)]
    pub aikar_flags: bool,
    /// Mods/plugins the user wants MCManager to keep on the correct
    /// loader+MC-version build. Populated via the Marketplace ("suivre")
    /// or Addons tab; actually downloading/updating them happens on demand
    /// via `POST /servers/:id/managed-addons/sync`, not automatically on
    /// every boot (the user asked to control *when* this runs).
    #[serde(default)]
    pub managed_addons: Vec<ManagedAddon>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAddon {
    pub project_id: String,
    pub label: String,
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
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub aikar_flags: bool,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_restart_delay")]
    pub auto_restart_delay_secs: u32,
    pub scheduled_restart_minutes: Option<u32>,
    pub stop_when_empty_minutes: Option<u32>,
    pub auto_backup_minutes: Option<u32>,
    #[serde(default)]
    pub backup_retention: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerRequest {
    pub name: Option<String>,
    pub xms_mb: Option<u32>,
    pub xmx_mb: Option<u32>,
    pub port: Option<u16>,
    pub extra_args: Option<Vec<String>>,
    pub aikar_flags: Option<bool>,
    pub auto_restart: Option<bool>,
    pub auto_restart_delay_secs: Option<u32>,
    pub scheduled_restart_minutes: Option<Option<u32>>,
    pub stop_when_empty_minutes: Option<Option<u32>>,
    pub dynamic_server: Option<bool>,
    pub auto_backup_minutes: Option<u32>,
    pub backup_retention: Option<Option<u32>>,
    pub java_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportServerRequest {
    pub name: String,
    pub loader: Loader,
    pub mc_version: String,
    pub folder_path: String,
    pub jar_name: Option<String>,
    #[serde(default)]
    pub auto_restart: bool,
}

fn default_xms() -> u32 { 1024 }
fn default_xmx() -> u32 { 2048 }
fn default_port() -> u16 { 25565 }
fn default_restart_delay() -> u32 { 5 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub java_path: String,
    pub playit_path: Option<String>,
    pub update_repo: String, // "owner/repo" used for self-update checks
    pub check_updates_on_start: bool,
    pub data_dir: String,
    /// Relancer au démarrage les serveurs qui tournaient quand MCManager s'est arrêté (mise à jour, redémarrage, coupure).
    #[serde(default = "default_true")]
    pub resume_servers: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
impl ServerEntry {
    /// Entrée minimale pour les tests.
    pub fn test_entry(id: Uuid, name: &str, folder: &std::path::Path) -> Self {
        ServerEntry {
            id, name: name.into(), loader: Loader::Paper, mc_version: "1.21.4".into(), loader_version: None, folder: folder.to_string_lossy().to_string(),
            jar_name: String::new(), java_path: "java".into(), xms_mb: 512, xmx_mb: 1024, port: 25565, extra_args: vec![], eula_accepted: true, auto_backup_minutes: None,
            backup_retention: None, auto_restart: false, auto_restart_delay_secs: 5, scheduled_restart_minutes: None, stop_when_empty_minutes: None, dynamic_server: false,
            aikar_flags: false, managed_addons: vec![], created_at: chrono::Utc::now(),
        }
    }
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
