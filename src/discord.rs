//! Bot Discord : notifications d'évènements ET pilotage des serveurs par
//! commandes slash (`/mc liste`, `/mc statut`, `/mc demarrer`...).
//!
//! ## Fonctionnement
//!
//! - **Notifications** : un message (embed) dans un salon quand un serveur
//!   démarre, devient prêt, s'arrête, plante, finit une sauvegarde, redémarre,
//!   quand un joueur se connecte... Chaque évènement s'active séparément.
//!   Envoi par le bot (token + salon) ou, sans bot, par un simple **webhook**.
//! - **Commandes** : le bot ouvre une connexion sortante vers la *passerelle*
//!   (gateway) de Discord et reçoit les interactions par ce canal. Aucun port
//!   à ouvrir, aucune URL publique, aucun nom de domaine requis : ça marche
//!   derrière une box, sur un Raspberry Pi, dans Docker.
//!
//! ## Sécurité
//!
//! - Le token du bot et l'URL du webhook sont chiffrés sur disque (mêmes
//!   mécanismes que les clés de l'assistant IA et le jeton ntfy).
//! - Les commandes ne sont enregistrées que dans **un** serveur Discord (la
//!   guilde configurée) et toute interaction venant d'ailleurs est refusée.
//! - **Refus par défaut** : tant qu'aucun opérateur (utilisateur ou rôle) n'est
//!   configuré, personne ne peut démarrer/arrêter/envoyer une commande. Les
//!   commandes de lecture (`liste`, `statut`, `joueurs`) sont ouvertes à tout
//!   membre de la guilde, ou limitées à des rôles si `viewer_role_ids` est rempli.
//! - Chaque action de pilotage est journalisée avec l'identifiant Discord.
//!
//! Les serveurs d'instances distantes jumelées (voir `hub.rs`) se pilotent de
//! la même façon : la commande est relayée par le tunnel chiffré.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio_tungstenite_discord::tungstenite::protocol::Message as WsMessage;
use uuid::Uuid;

use crate::secrets::{self, EncryptedBlob};
use crate::state::AppState;

pub const API_BASE: &str = "https://discord.com/api/v10";
pub const GATEWAY_URL: &str = "wss://gateway.discord.gg";
const MAX_CHOICES: usize = 25;

// ───────────────────────── configuration ─────────────────────────

fn tru() -> bool {
    true
}

#[derive(Debug, Clone, Default)]
pub struct DiscordConfig {
    pub enabled: bool,
    /// Texte clair en mémoire uniquement (chiffré sur disque).
    pub bot_token: String,
    pub guild_id: String,
    /// Salon des notifications (utilisé avec le bot).
    pub channel_id: String,
    /// Alternative sans bot : URL de webhook (chiffrée sur disque).
    pub webhook_url: String,
    pub operator_user_ids: Vec<String>,
    pub operator_role_ids: Vec<String>,
    pub viewer_role_ids: Vec<String>,
    pub notify_started: bool,
    pub notify_ready: bool,
    pub notify_stopped: bool,
    pub notify_crash: bool,
    pub notify_backup: bool,
    pub notify_scheduled_restart: bool,
    pub notify_auto_stop: bool,
    pub notify_player_join_leave: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDiscordConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    guild_id: String,
    #[serde(default)]
    channel_id: String,
    #[serde(default)]
    operator_user_ids: Vec<String>,
    #[serde(default)]
    operator_role_ids: Vec<String>,
    #[serde(default)]
    viewer_role_ids: Vec<String>,
    #[serde(default = "tru")]
    notify_started: bool,
    #[serde(default = "tru")]
    notify_ready: bool,
    #[serde(default = "tru")]
    notify_stopped: bool,
    #[serde(default = "tru")]
    notify_crash: bool,
    #[serde(default = "tru")]
    notify_backup: bool,
    #[serde(default = "tru")]
    notify_scheduled_restart: bool,
    #[serde(default = "tru")]
    notify_auto_stop: bool,
    #[serde(default)]
    notify_player_join_leave: bool,
    encrypted_token: Option<EncryptedBlob>,
    encrypted_webhook: Option<EncryptedBlob>,
}

impl Default for StoredDiscordConfig {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults")
    }
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("discord_config.json")
}

pub async fn load_config(data_dir: &Path) -> DiscordConfig {
    let stored: StoredDiscordConfig = match tokio::fs::read_to_string(config_path(data_dir)).await {
        Ok(c) => serde_json::from_str(&c).unwrap_or_default(),
        Err(_) => StoredDiscordConfig::default(),
    };
    let key = secrets::load_or_create_key(data_dir).await.ok();
    let dec = |blob: &Option<EncryptedBlob>| match (blob, &key) {
        (Some(b), Some(k)) => secrets::decrypt(k, b).unwrap_or_default(),
        _ => String::new(),
    };
    DiscordConfig {
        enabled: stored.enabled,
        bot_token: dec(&stored.encrypted_token),
        webhook_url: dec(&stored.encrypted_webhook),
        guild_id: stored.guild_id,
        channel_id: stored.channel_id,
        operator_user_ids: stored.operator_user_ids,
        operator_role_ids: stored.operator_role_ids,
        viewer_role_ids: stored.viewer_role_ids,
        notify_started: stored.notify_started,
        notify_ready: stored.notify_ready,
        notify_stopped: stored.notify_stopped,
        notify_crash: stored.notify_crash,
        notify_backup: stored.notify_backup,
        notify_scheduled_restart: stored.notify_scheduled_restart,
        notify_auto_stop: stored.notify_auto_stop,
        notify_player_join_leave: stored.notify_player_join_leave,
    }
}

pub async fn save_config(data_dir: &Path, cfg: &DiscordConfig) -> anyhow::Result<()> {
    let key = secrets::load_or_create_key(data_dir).await?;
    let enc = |plain: &str| -> anyhow::Result<Option<EncryptedBlob>> {
        if plain.is_empty() {
            Ok(None)
        } else {
            Ok(Some(secrets::encrypt(&key, plain)?))
        }
    };
    let stored = StoredDiscordConfig {
        enabled: cfg.enabled,
        guild_id: cfg.guild_id.clone(),
        channel_id: cfg.channel_id.clone(),
        operator_user_ids: cfg.operator_user_ids.clone(),
        operator_role_ids: cfg.operator_role_ids.clone(),
        viewer_role_ids: cfg.viewer_role_ids.clone(),
        notify_started: cfg.notify_started,
        notify_ready: cfg.notify_ready,
        notify_stopped: cfg.notify_stopped,
        notify_crash: cfg.notify_crash,
        notify_backup: cfg.notify_backup,
        notify_scheduled_restart: cfg.notify_scheduled_restart,
        notify_auto_stop: cfg.notify_auto_stop,
        notify_player_join_leave: cfg.notify_player_join_leave,
        encrypted_token: enc(&cfg.bot_token)?,
        encrypted_webhook: enc(&cfg.webhook_url)?,
    };
    let path = config_path(data_dir);
    tokio::fs::write(&path, serde_json::to_string_pretty(&stored)?).await?;
    secrets::restrict_to_owner(&path).await;
    // Le bot se reconfigure tout de suite (nouveau token, désactivation...).
    let signal = restart_signal(data_dir);
    signal.notify_waiters();
    signal.notify_one();
    Ok(())
}

/// Un signal par dossier de données (et non un global de processus) : plusieurs
/// instances dans un même processus - comme les tests - ne se dérangent pas.
fn restart_signal(data_dir: &Path) -> std::sync::Arc<Notify> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    static N: OnceLock<Mutex<HashMap<PathBuf, Arc<Notify>>>> = OnceLock::new();
    let mut map = N.get_or_init(|| Mutex::new(HashMap::new())).lock().expect("verrou");
    map.entry(data_dir.to_path_buf()).or_insert_with(|| Arc::new(Notify::new())).clone()
}

// ───────────────────────── état de la connexion ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BotStatus {
    /// disabled | connecting | connected | error
    pub state: String,
    pub detail: String,
    pub bot_name: Option<String>,
    pub commands_registered: bool,
}

fn status_cell() -> &'static std::sync::RwLock<BotStatus> {
    static S: OnceLock<std::sync::RwLock<BotStatus>> = OnceLock::new();
    S.get_or_init(|| std::sync::RwLock::new(BotStatus { state: "disabled".into(), detail: String::new(), bot_name: None, commands_registered: false }))
}

pub fn status() -> BotStatus {
    status_cell().read().map(|s| s.clone()).unwrap_or(BotStatus { state: "error".into(), detail: "etat illisible".into(), bot_name: None, commands_registered: false })
}

fn set_status(state: &str, detail: impl Into<String>) {
    if let Ok(mut s) = status_cell().write() {
        s.state = state.to_string();
        s.detail = detail.into();
        if state != "connected" {
            s.commands_registered = false;
            if state == "disabled" {
                s.bot_name = None;
            }
        }
    }
}

// ───────────────────────── notifications ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Started,
    Ready,
    Stopped,
    Crash,
    Backup,
    ScheduledRestart,
    AutoStop,
    PlayerJoin,
    PlayerLeave,
}

impl Event {
    fn allowed(&self, c: &DiscordConfig) -> bool {
        match self {
            Event::Started => c.notify_started,
            Event::Ready => c.notify_ready,
            Event::Stopped => c.notify_stopped,
            Event::Crash => c.notify_crash,
            Event::Backup => c.notify_backup,
            Event::ScheduledRestart => c.notify_scheduled_restart,
            Event::AutoStop => c.notify_auto_stop,
            Event::PlayerJoin | Event::PlayerLeave => c.notify_player_join_leave,
        }
    }

    fn color(&self) -> u32 {
        match self {
            Event::Started | Event::ScheduledRestart => 0xF9E2AF,
            Event::Ready | Event::PlayerJoin => 0xA6E3A1,
            Event::Stopped | Event::AutoStop | Event::PlayerLeave => 0x9399B2,
            Event::Crash => 0xF38BA8,
            Event::Backup => 0x89B4FA,
        }
    }

    fn title(&self, server: &str) -> String {
        match self {
            Event::Started => format!("🟡 {server} démarre…"),
            Event::Ready => format!("🟢 {server} est en ligne"),
            Event::Stopped => format!("⚫ {server} est arrêté"),
            Event::Crash => format!("🔴 {server} a planté"),
            Event::Backup => format!("💾 Sauvegarde de {server}"),
            Event::ScheduledRestart => format!("🔄 Redémarrage programmé de {server}"),
            Event::AutoStop => format!("⏸ {server} arrêté (inactif)"),
            Event::PlayerJoin => format!("➕ Connexion sur {server}"),
            Event::PlayerLeave => format!("➖ Déconnexion de {server}"),
        }
    }
}

fn embed(title: &str, description: &str, color: u32) -> Value {
    json!({
        "title": title.chars().take(256).collect::<String>(),
        "description": description.chars().take(4000).collect::<String>(),
        "color": color,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "footer": { "text": "MCManager" },
    })
}

/// Envoie la notification si Discord est activé et que cet évènement n'est pas
/// coupé. Les échecs sont journalisés puis ignorés : une notification perdue ne
/// doit jamais faire échouer un démarrage ou une sauvegarde.
pub async fn notify(state: &AppState, event: Event, server_name: &str, message: &str) {
    notify_at(&state.http, &state.data_dir, API_BASE, event, server_name, message).await
}

pub async fn notify_at(http: &reqwest::Client, data_dir: &Path, api_base: &str, event: Event, server_name: &str, message: &str) {
    let cfg = load_config(data_dir).await;
    if !cfg.enabled || !event.allowed(&cfg) {
        return;
    }
    let body = json!({ "embeds": [embed(&event.title(server_name), message, event.color())] });
    if let Err(e) = post_message(http, &cfg, api_base, &body).await {
        tracing::warn!("echec de l'envoi de la notification Discord : {e}");
    }
}

/// Bot + salon si configurés, sinon webhook.
async fn post_message(http: &reqwest::Client, cfg: &DiscordConfig, api_base: &str, body: &Value) -> anyhow::Result<()> {
    let req = if !cfg.bot_token.is_empty() && !cfg.channel_id.trim().is_empty() {
        http.post(format!("{api_base}/channels/{}/messages", cfg.channel_id.trim()))
            .header("Authorization", format!("Bot {}", cfg.bot_token))
    } else if !cfg.webhook_url.trim().is_empty() {
        http.post(cfg.webhook_url.trim())
    } else {
        anyhow::bail!("aucun salon (avec le bot) ni webhook configuré");
    };
    let resp = req.json(body).timeout(Duration::from_secs(15)).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord a répondu {status} : {}", text.chars().take(300).collect::<String>());
    }
    Ok(())
}

/// Message de test (bouton « Envoyer un test »), indépendant des interrupteurs.
pub async fn send_test(http: &reqwest::Client, cfg: &DiscordConfig) -> anyhow::Result<()> {
    send_test_at(http, cfg, API_BASE).await
}

pub async fn send_test_at(http: &reqwest::Client, cfg: &DiscordConfig, api_base: &str) -> anyhow::Result<()> {
    let body = json!({ "embeds": [embed("✅ MCManager", "Notification de test : la configuration Discord fonctionne.", 0xA6E3A1)] });
    post_message(http, cfg, api_base, &body).await
}

// ───────────────────────── serveurs (locaux et distants) ─────────────────────────

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub id: Uuid,
    pub name: String,
    pub running: bool,
    pub remote: Option<String>,
}

impl ServerInfo {
    fn label(&self) -> String {
        match &self.remote {
            Some(r) => format!("{} · 🖧 {r}", self.name),
            None => self.name.clone(),
        }
    }
}

async fn gather_servers(state: &AppState) -> Vec<ServerInfo> {
    let mut out = Vec::new();
    {
        let servers = state.servers.read().await;
        let runtime = state.runtime.read().await;
        for (id, s) in servers.iter() {
            out.push(ServerInfo { id: *id, name: s.name.clone(), running: runtime.get(id).map(|r| r.running).unwrap_or(false), remote: None });
        }
    }
    // Serveurs des instances jumelées : on interroge chaque instance (état « en marche » inclus).
    if let Ok(identity) = crate::hub::identity(state).await {
        for target in crate::remote::load_targets(&state.data_dir).await {
            let call = crate::remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "list" }), Duration::from_secs(8));
            if let Ok(Ok(v)) = tokio::time::timeout(Duration::from_secs(10), call).await {
                for s in v["servers"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = s["id"].as_str().and_then(|x| Uuid::parse_str(x).ok()) {
                        out.push(ServerInfo { id, name: s["name"].as_str().unwrap_or("?").to_string(), running: s["running"].as_bool().unwrap_or(false), remote: Some(target.label.clone()) });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn resolve<'a>(servers: &'a [ServerInfo], query: &str) -> Result<&'a ServerInfo, String> {
    let q = query.trim();
    if let Ok(id) = Uuid::parse_str(q) {
        return servers.iter().find(|s| s.id == id).ok_or_else(|| "Serveur introuvable.".to_string());
    }
    let matches: Vec<&ServerInfo> = servers.iter().filter(|s| s.name.eq_ignore_ascii_case(q)).collect();
    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("Aucun serveur nommé « {q} ».")),
        _ => Err(format!("Plusieurs serveurs s'appellent « {q} » : choisissez-le dans la liste d'autocomplétion.")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Start,
    Stop,
    Restart,
}

impl Action {
    fn remote_name(&self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Restart => "restart",
        }
    }
}

async fn control(state: &AppState, server: &ServerInfo, action: Action) -> anyhow::Result<()> {
    if let Some(label) = &server.remote {
        let target = crate::hub::target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("instance « {label} » inconnue"))?;
        let identity = crate::hub::identity(state).await?;
        crate::remote::client_call_checked(&state.http, &identity, &target, json!({ "action": action.remote_name(), "server_id": server.id }), Duration::from_secs(30)).await?;
        return Ok(());
    }
    match action {
        Action::Start => crate::process::start_server(state, server.id).await,
        Action::Stop => crate::process::stop_server(state, server.id, false).await,
        Action::Restart => {
            crate::process::stop_server(state, server.id, false).await?;
            // L'arrêt est asynchrone : on attend la fin du processus (60 s max) avant de relancer.
            for _ in 0..120 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let running = state.runtime.read().await.get(&server.id).map(|r| r.running).unwrap_or(false);
                if !running {
                    break;
                }
            }
            crate::process::start_server(state, server.id).await
        }
    }
}

async fn send_console_command(state: &AppState, server: &ServerInfo, command: &str) -> anyhow::Result<()> {
    if let Some(label) = &server.remote {
        let target = crate::hub::target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("instance « {label} » inconnue"))?;
        let identity = crate::hub::identity(state).await?;
        crate::remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "send", "server_id": server.id, "command": command }), Duration::from_secs(15)).await?;
        return Ok(());
    }
    crate::process::send_command(state, server.id, command).await
}

/// (en ligne, max) via le ping du protocole Minecraft.
async fn player_counts(state: &AppState, server: &ServerInfo) -> Option<(u32, u32)> {
    if let Some(label) = &server.remote {
        let target = crate::hub::target_by_label(state, label).await?;
        let identity = crate::hub::identity(state).await.ok()?;
        let v = crate::remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "status", "server_id": server.id }), Duration::from_secs(10)).await.ok()?;
        return Some((v["players"]["online"].as_u64()? as u32, v["players"]["max"].as_u64()? as u32));
    }
    let port = state.servers.read().await.get(&server.id)?.port;
    let (online, max, _) = crate::stats::ping_server(port).await.ok()?;
    Some((online?, max?))
}

// ───────────────────────── permissions ─────────────────────────

struct Caller {
    user_id: String,
    user_name: String,
    roles: Vec<String>,
}

impl Caller {
    fn from_interaction(i: &Value) -> Option<Self> {
        let member = i.get("member")?;
        let user = member.get("user")?;
        Some(Caller {
            user_id: user["id"].as_str()?.to_string(),
            user_name: user["username"].as_str().unwrap_or("?").to_string(),
            roles: member["roles"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
        })
    }

    fn is_operator(&self, cfg: &DiscordConfig) -> bool {
        cfg.operator_user_ids.iter().any(|u| u == &self.user_id) || self.roles.iter().any(|r| cfg.operator_role_ids.contains(r))
    }

    fn can_view(&self, cfg: &DiscordConfig) -> bool {
        self.is_operator(cfg) || cfg.viewer_role_ids.is_empty() || self.roles.iter().any(|r| cfg.viewer_role_ids.contains(r))
    }
}

// ───────────────────────── commandes slash ─────────────────────────

fn command_definitions() -> Value {
    let server_opt = json!({ "type": 3, "name": "serveur", "description": "Nom du serveur", "required": true, "autocomplete": true });
    json!([{
        "name": "mc",
        "description": "Gérer les serveurs Minecraft (MCManager)",
        "type": 1,
        "dm_permission": false,
        "options": [
            { "type": 1, "name": "liste", "description": "Liste des serveurs et leur état" },
            { "type": 1, "name": "statut", "description": "État détaillé d'un serveur", "options": [server_opt.clone()] },
            { "type": 1, "name": "joueurs", "description": "Joueurs connectés", "options": [server_opt.clone()] },
            { "type": 1, "name": "demarrer", "description": "Démarrer un serveur (opérateurs)", "options": [server_opt.clone()] },
            { "type": 1, "name": "arreter", "description": "Arrêter un serveur proprement (opérateurs)", "options": [server_opt.clone()] },
            { "type": 1, "name": "redemarrer", "description": "Redémarrer un serveur (opérateurs)", "options": [server_opt.clone()] },
            { "type": 1, "name": "commande", "description": "Envoyer une commande à la console (opérateurs)", "options": [
                server_opt,
                { "type": 3, "name": "texte", "description": "Commande, sans le /", "required": true, "max_length": 200 }
            ]},
        ]
    }])
}

async fn register_commands(http: &reqwest::Client, api_base: &str, token: &str, app_id: &str, guild_id: &str) -> anyhow::Result<()> {
    let resp = http
        .put(format!("{api_base}/applications/{app_id}/guilds/{guild_id}/commands"))
        .header("Authorization", format!("Bot {token}"))
        .json(&command_definitions())
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("enregistrement des commandes refusé ({status}) : {}", text.chars().take(300).collect::<String>());
    }
    Ok(())
}

/// Réponse (callback) à une interaction. `ephemeral` = visible seulement par l'auteur.
async fn respond(http: &reqwest::Client, api_base: &str, interaction: &Value, content: &str, ephemeral: bool) {
    let (Some(id), Some(token)) = (interaction["id"].as_str(), interaction["token"].as_str()) else { return };
    let mut data = json!({ "content": content.chars().take(1900).collect::<String>() });
    if ephemeral {
        data["flags"] = json!(64);
    }
    post_callback(http, api_base, id, token, json!({ "type": 4, "data": data })).await;
}

async fn post_callback(http: &reqwest::Client, api_base: &str, id: &str, token: &str, body: Value) {
    let r = http.post(format!("{api_base}/interactions/{id}/{token}/callback")).json(&body).timeout(Duration::from_secs(10)).send().await;
    match r {
        Ok(resp) if !resp.status().is_success() => tracing::warn!("[discord] réponse d'interaction refusée : {}", resp.status()),
        Err(e) => tracing::warn!("[discord] réponse d'interaction impossible : {e}"),
        _ => {}
    }
}

fn option_value(sub: &Value, name: &str) -> Option<String> {
    sub["options"].as_array()?.iter().find(|o| o["name"] == name).and_then(|o| o["value"].as_str().map(|s| s.to_string()))
}

pub async fn handle_interaction(state: AppState, api_base: String, interaction: Value) {
    let cfg = load_config(&state.data_dir).await;
    let http = state.http.clone();

    if !cfg.enabled || interaction["guild_id"].as_str() != Some(cfg.guild_id.as_str()) {
        respond(&http, &api_base, &interaction, "⛔ Ce serveur Discord n'est pas autorisé à utiliser ce bot.", true).await;
        return;
    }
    let Some(caller) = Caller::from_interaction(&interaction) else {
        respond(&http, &api_base, &interaction, "⛔ Utilisateur non identifiable.", true).await;
        return;
    };
    if !caller.can_view(&cfg) {
        respond(&http, &api_base, &interaction, "⛔ Vous n'avez pas la permission d'utiliser cette commande.", true).await;
        return;
    }

    match interaction["type"].as_u64() {
        // Autocomplétion du nom de serveur
        Some(4) => {
            let sub = interaction["data"]["options"].get(0).cloned().unwrap_or(Value::Null);
            let typed = sub["options"].as_array().and_then(|o| o.iter().find(|x| x["focused"] == true)).and_then(|o| o["value"].as_str()).unwrap_or("").to_lowercase();
            let servers = gather_servers(&state).await;
            let choices: Vec<Value> = servers
                .iter()
                .filter(|s| typed.is_empty() || s.name.to_lowercase().contains(&typed))
                .take(MAX_CHOICES)
                .map(|s| json!({ "name": format!("{}{}", if s.running { "🟢 " } else { "⚫ " }, s.label()).chars().take(100).collect::<String>(), "value": s.id.to_string() }))
                .collect();
            if let (Some(id), Some(token)) = (interaction["id"].as_str(), interaction["token"].as_str()) {
                post_callback(&http, &api_base, id, token, json!({ "type": 8, "data": { "choices": choices } })).await;
            }
        }
        // Commande
        Some(2) => {
            if interaction["data"]["name"] != "mc" {
                return;
            }
            let sub = interaction["data"]["options"].get(0).cloned().unwrap_or(Value::Null);
            let sub_name = sub["name"].as_str().unwrap_or("").to_string();
            let servers = gather_servers(&state).await;

            if sub_name == "liste" {
                let text = if servers.is_empty() {
                    "Aucun serveur.".to_string()
                } else {
                    servers.iter().map(|s| format!("{} **{}**{}", if s.running { "🟢" } else { "⚫" }, s.name, s.remote.as_ref().map(|r| format!(" · 🖧 {r}")).unwrap_or_default())).collect::<Vec<_>>().join("\n")
                };
                respond(&http, &api_base, &interaction, &text, false).await;
                return;
            }

            let query = option_value(&sub, "serveur").unwrap_or_default();
            let server = match resolve(&servers, &query) {
                Ok(s) => s.clone(),
                Err(msg) => {
                    respond(&http, &api_base, &interaction, &format!("❌ {msg}"), true).await;
                    return;
                }
            };

            match sub_name.as_str() {
                "statut" => {
                    let players = if server.running { player_counts(&state, &server).await } else { None };
                    let line = match (server.running, players) {
                        (true, Some((o, m))) => format!("🟢 **{}** est en ligne — {o}/{m} joueur(s).", server.label()),
                        (true, None) => format!("🟡 **{}** est démarré (joueurs : indisponible, démarrage en cours ?).", server.label()),
                        (false, _) => format!("⚫ **{}** est arrêté.", server.label()),
                    };
                    respond(&http, &api_base, &interaction, &line, false).await;
                }
                "joueurs" => {
                    let line = if !server.running {
                        format!("⚫ **{}** est arrêté.", server.label())
                    } else {
                        match player_counts(&state, &server).await {
                            Some((o, m)) => format!("👥 **{}** : {o}/{m} joueur(s) connecté(s).", server.label()),
                            None => format!("❓ Impossible de lire les joueurs de **{}** pour l'instant.", server.label()),
                        }
                    };
                    respond(&http, &api_base, &interaction, &line, false).await;
                }
                "demarrer" | "arreter" | "redemarrer" | "commande" => {
                    if !caller.is_operator(&cfg) {
                        tracing::warn!("[discord] refus : {} ({}) a tenté /mc {sub_name} sur {}", caller.user_name, caller.user_id, server.name);
                        respond(&http, &api_base, &interaction, "⛔ Cette action est réservée aux opérateurs (voir les réglages Discord de MCManager).", true).await;
                        return;
                    }
                    if sub_name == "commande" {
                        let text = option_value(&sub, "texte").unwrap_or_default();
                        let text = text.trim().trim_start_matches('/').to_string();
                        if text.is_empty() || text.contains('\n') || text.contains('\r') {
                            respond(&http, &api_base, &interaction, "❌ Commande vide ou invalide.", true).await;
                            return;
                        }
                        tracing::info!("[discord] {} ({}) : commande « {text} » sur {}", caller.user_name, caller.user_id, server.name);
                        let reply = match send_console_command(&state, &server, &text).await {
                            Ok(()) => format!("✅ Commande envoyée à **{}** : `{}`", server.label(), text.replace('`', "'")),
                            Err(e) => format!("❌ {e}"),
                        };
                        respond(&http, &api_base, &interaction, &reply, false).await;
                        return;
                    }
                    let (action, doing) = match sub_name.as_str() {
                        "demarrer" => (Action::Start, "Démarrage"),
                        "arreter" => (Action::Stop, "Arrêt"),
                        _ => (Action::Restart, "Redémarrage"),
                    };
                    tracing::info!("[discord] {} ({}) : {sub_name} {}", caller.user_name, caller.user_id, server.name);
                    if action == Action::Start && server.running {
                        respond(&http, &api_base, &interaction, &format!("ℹ️ **{}** est déjà en ligne.", server.label()), true).await;
                        return;
                    }
                    if action == Action::Stop && !server.running {
                        respond(&http, &api_base, &interaction, &format!("ℹ️ **{}** est déjà arrêté.", server.label()), true).await;
                        return;
                    }
                    // Démarrage/arrêt immédiats ; le redémarrage attend l'arrêt, donc en tâche de fond.
                    if action == Action::Restart {
                        respond(&http, &api_base, &interaction, &format!("🔄 {doing} de **{}** en cours… (une notification suivra)", server.label()), false).await;
                        let (st, sv) = (state.clone(), server.clone());
                        tokio::spawn(async move {
                            if let Err(e) = control(&st, &sv, Action::Restart).await {
                                notify(&st, Event::Crash, &sv.name, &format!("Le redémarrage demandé depuis Discord a échoué : {e}")).await;
                            }
                        });
                        return;
                    }
                    let reply = match control(&state, &server, action).await {
                        Ok(()) => format!("✅ {doing} de **{}** demandé par {}.", server.label(), caller.user_name),
                        Err(e) => format!("❌ {e}"),
                    };
                    respond(&http, &api_base, &interaction, &reply, false).await;
                }
                _ => respond(&http, &api_base, &interaction, "❌ Commande inconnue.", true).await,
            }
        }
        _ => {}
    }
}

// ───────────────────────── connexion à la passerelle ─────────────────────────

#[derive(Default, Clone)]
struct GatewaySession {
    session_id: Option<String>,
    resume_url: Option<String>,
    seq: Option<u64>,
}

enum SessionEnd {
    /// Reconnexion avec reprise de session possible.
    Reconnect,
    /// La configuration a changé : repartir de zéro.
    Reconfigure,
    /// Token refusé / intentions non autorisées : inutile de réessayer tel quel.
    Fatal(String),
}

/// Démarre le bot en tâche de fond (à appeler une fois par processus).
pub fn spawn(state: AppState) {
    tokio::spawn(async move { run_forever(state, GATEWAY_URL.to_string(), API_BASE.to_string()).await });
}

pub async fn run_forever(state: AppState, gateway_url: String, api_base: String) {
    let mut backoff = 2u64;
    let mut session = GatewaySession::default();
    let signal = restart_signal(&state.data_dir);
    loop {
        // Un signal de reconfiguration déjà en attente est périmé : on relit la configuration
        // juste après, donc il ne faut pas qu'il fasse fermer la connexion qu'on va ouvrir.
        {
            use futures_util::FutureExt;
            let _ = signal.notified().now_or_never();
        }
        let cfg = load_config(&state.data_dir).await;
        if !cfg.enabled || cfg.bot_token.is_empty() || cfg.guild_id.trim().is_empty() {
            set_status("disabled", if cfg.enabled { "token ou identifiant du serveur Discord manquant" } else { "" });
            session = GatewaySession::default();
            tokio::select! {
                _ = signal.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(20)) => {}
            }
            continue;
        }
        set_status("connecting", "");
        match run_session(&state, &cfg, &gateway_url, &api_base, &mut session).await {
            Ok(SessionEnd::Reconfigure) => {
                session = GatewaySession::default();
                backoff = 2;
            }
            Ok(SessionEnd::Reconnect) => {
                backoff = 2;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(SessionEnd::Fatal(msg)) => {
                set_status("error", msg);
                session = GatewaySession::default();
                // On attend une modification de la configuration plutôt que de marteler Discord.
                signal.notified().await;
            }
            Err(e) => {
                set_status("error", e.to_string());
                tokio::select! {
                    _ = signal.notified() => { backoff = 2; session = GatewaySession::default(); }
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => { backoff = (backoff * 2).min(60); }
                }
            }
        }
    }
}

async fn run_session(state: &AppState, cfg: &DiscordConfig, gateway_url: &str, api_base: &str, session: &mut GatewaySession) -> anyhow::Result<SessionEnd> {
    let base = session.resume_url.clone().unwrap_or_else(|| gateway_url.to_string());
    let url = format!("{}/?v=10&encoding=json", base.trim_end_matches('/'));
    let (ws, _) = tokio_tungstenite_discord::connect_async(url).await.map_err(|e| anyhow::anyhow!("connexion à la passerelle Discord impossible : {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // 1. Hello
    let hello = tokio::time::timeout(Duration::from_secs(20), stream.next()).await.map_err(|_| anyhow::anyhow!("pas de Hello de la passerelle"))?;
    let hello: Value = match hello {
        Some(Ok(WsMessage::Text(t))) => serde_json::from_str(&t)?,
        _ => anyhow::bail!("Hello de la passerelle illisible"),
    };
    let interval = Duration::from_millis(hello["d"]["heartbeat_interval"].as_u64().unwrap_or(41_250).max(50));

    // 2. Identify ou Resume
    let can_resume = session.session_id.is_some() && session.resume_url.is_some();
    let first = if can_resume {
        json!({ "op": 6, "d": { "token": cfg.bot_token, "session_id": session.session_id, "seq": session.seq } })
    } else {
        json!({ "op": 2, "d": { "token": cfg.bot_token, "intents": 0, "properties": { "os": std::env::consts::OS, "browser": "mcmanager", "device": "mcmanager" } } })
    };
    sink.send(WsMessage::Text(first.to_string())).await?;

    let signal = restart_signal(&state.data_dir);
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + interval / 2, interval);
    let mut acked = true;

    loop {
        tokio::select! {
            _ = signal.notified() => {
                let _ = sink.send(WsMessage::Close(None)).await;
                return Ok(SessionEnd::Reconfigure);
            }
            _ = heartbeat.tick() => {
                if !acked {
                    return Ok(SessionEnd::Reconnect); // connexion zombie
                }
                acked = false;
                sink.send(WsMessage::Text(json!({ "op": 1, "d": session.seq }).to_string())).await?;
            }
            msg = stream.next() => {
                let Some(msg) = msg else { return Ok(SessionEnd::Reconnect) };
                let text = match msg? {
                    WsMessage::Text(t) => t,
                    WsMessage::Close(frame) => {
                        let code = frame.map(|f| u16::from(f.code)).unwrap_or(0);
                        return Ok(match code {
                            4004 => SessionEnd::Fatal("token du bot refusé par Discord (4004) : vérifiez le token".into()),
                            4010 | 4011 | 4012 | 4013 | 4014 => SessionEnd::Fatal(format!("configuration du bot refusée par Discord (code {code})")),
                            4007 | 4009 => { *session = GatewaySession::default(); SessionEnd::Reconnect }
                            _ => SessionEnd::Reconnect,
                        });
                    }
                    WsMessage::Ping(p) => { sink.send(WsMessage::Pong(p)).await?; continue; }
                    _ => continue,
                };
                let payload: Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                if let Some(s) = payload["s"].as_u64() { session.seq = Some(s); }
                match payload["op"].as_u64() {
                    Some(0) => match payload["t"].as_str() {
                        Some("READY") => {
                            session.session_id = payload["d"]["session_id"].as_str().map(|s| s.to_string());
                            session.resume_url = payload["d"]["resume_gateway_url"].as_str().map(|s| s.to_string());
                            let name = payload["d"]["user"]["username"].as_str().unwrap_or("bot").to_string();
                            let app_id = payload["d"]["application"]["id"].as_str().unwrap_or("").to_string();
                            if let Ok(mut s) = status_cell().write() { s.bot_name = Some(name.clone()); }
                            set_status("connected", format!("connecté en tant que {name}"));
                            let (http, token, guild, base) = (state.http.clone(), cfg.bot_token.clone(), cfg.guild_id.trim().to_string(), api_base.to_string());
                            tokio::spawn(async move {
                                match register_commands(&http, &base, &token, &app_id, &guild).await {
                                    Ok(()) => { if let Ok(mut s) = status_cell().write() { s.commands_registered = true; } }
                                    Err(e) => set_status("error", format!("connecté, mais {e}")),
                                }
                            });
                        }
                        Some("RESUMED") => {
                            let name = status().bot_name.unwrap_or_default();
                            set_status("connected", format!("connecté en tant que {name}"));
                        }
                        Some("INTERACTION_CREATE") => {
                            let (st, base, interaction) = (state.clone(), api_base.to_string(), payload["d"].clone());
                            tokio::spawn(handle_interaction(st, base, interaction));
                        }
                        _ => {}
                    },
                    Some(1) => {
                        sink.send(WsMessage::Text(json!({ "op": 1, "d": session.seq }).to_string())).await?;
                    }
                    Some(7) => return Ok(SessionEnd::Reconnect),
                    Some(9) => {
                        if payload["d"] != true { *session = GatewaySession::default(); }
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        return Ok(SessionEnd::Reconnect);
                    }
                    Some(11) => acked = true,
                    _ => {}
                }
            }
        }
    }
}

// ───────────────────────── tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State as AxState;
    use axum::routing::post;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<(String, String, Value)>>>);

    impl Recorder {
        fn all(&self) -> Vec<(String, String, Value)> {
            self.0.lock().unwrap().clone()
        }
        fn find(&self, path_part: &str) -> Vec<Value> {
            self.all().into_iter().filter(|(_, p, _)| p.contains(path_part)).map(|(_, _, b)| b).collect()
        }
    }

    /// Faux Discord REST : enregistre tout ce qu'on lui envoie.
    async fn mock_rest() -> (String, Recorder) {
        let rec = Recorder::default();
        async fn catch(AxState(rec): AxState<Recorder>, method: axum::http::Method, uri: axum::http::Uri, body: axum::body::Bytes) -> axum::Json<Value> {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            rec.0.lock().unwrap().push((method.to_string(), uri.path().to_string(), v));
            axum::Json(json!({}))
        }
        let app = axum::Router::new().route("/*path", post(catch).put(catch)).with_state(rec.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok(); });
        (format!("http://{addr}"), rec)
    }

    async fn temp_state(tag: &str) -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mcmanager-discord-{tag}-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        (crate::state::build_state_for_test(&dir).await, dir)
    }

    fn base_cfg() -> DiscordConfig {
        DiscordConfig {
            enabled: true,
            bot_token: "tkn-secret".into(),
            guild_id: "G1".into(),
            channel_id: "C1".into(),
            operator_user_ids: vec!["U1".into()],
            notify_started: true, notify_ready: true, notify_stopped: true, notify_crash: true,
            notify_backup: true, notify_scheduled_restart: true, notify_auto_stop: true,
            ..Default::default()
        }
    }

    fn sample_server(id: Uuid, name: &str, folder: &Path) -> crate::models::ServerEntry {
        crate::models::ServerEntry {
            id, name: name.into(), loader: crate::models::Loader::Paper, mc_version: "1.21.4".into(), loader_version: None,
            folder: folder.to_string_lossy().to_string(), jar_name: String::new(), java_path: "java".into(),
            xms_mb: 512, xmx_mb: 1024, port: 25598, extra_args: vec![], eula_accepted: true, auto_backup_minutes: None,
            backup_retention: None, auto_restart: false, auto_restart_delay_secs: 5, scheduled_restart_minutes: None,
            stop_when_empty_minutes: None, dynamic_server: false, aikar_flags: false, managed_addons: vec![], created_at: chrono::Utc::now(),
        }
    }

    fn interaction(kind: u64, guild: &str, user: &str, roles: &[&str], options: Value) -> Value {
        json!({
            "id": "I1", "token": "TOK1", "type": kind, "guild_id": guild,
            "member": { "user": { "id": user, "username": format!("user-{user}") }, "roles": roles },
            "data": { "name": "mc", "options": options },
        })
    }

    #[test]
    fn config_roundtrip_encrypts_secrets() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dir = std::env::temp_dir().join(format!("mcmanager-discord-cfg-{}", Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir).await.unwrap();
            let mut cfg = base_cfg();
            cfg.webhook_url = "https://discord.com/api/webhooks/123/secret-webhook-token".into();
            save_config(&dir, &cfg).await.unwrap();
            let raw = tokio::fs::read_to_string(config_path(&dir)).await.unwrap();
            assert!(!raw.contains("tkn-secret"), "le token ne doit pas être en clair sur disque");
            assert!(!raw.contains("secret-webhook-token"), "le webhook ne doit pas être en clair sur disque");
            let back = load_config(&dir).await;
            assert_eq!(back.bot_token, "tkn-secret");
            assert_eq!(back.webhook_url, cfg.webhook_url);
            assert_eq!(back.operator_user_ids, vec!["U1".to_string()]);
            assert!(back.notify_crash && !back.notify_player_join_leave);
            tokio::fs::remove_dir_all(&dir).await.ok();
        });
    }

    #[test]
    fn permissions_default_to_deny_for_control() {
        let mut cfg = base_cfg();
        let stranger = Caller { user_id: "U9".into(), user_name: "x".into(), roles: vec!["R9".into()] };
        assert!(stranger.can_view(&cfg), "lecture ouverte si aucun rôle de lecture n'est défini");
        assert!(!stranger.is_operator(&cfg));
        cfg.operator_user_ids.clear();
        let would_be_op = Caller { user_id: "U1".into(), user_name: "x".into(), roles: vec![] };
        assert!(!would_be_op.is_operator(&cfg), "sans opérateur configuré, personne ne contrôle");
        cfg.operator_role_ids = vec!["ROLE_OP".into()];
        let by_role = Caller { user_id: "U5".into(), user_name: "x".into(), roles: vec!["ROLE_OP".into()] };
        assert!(by_role.is_operator(&cfg));
        cfg.viewer_role_ids = vec!["ROLE_VIEW".into()];
        assert!(!stranger.can_view(&cfg));
        assert!(by_role.can_view(&cfg), "un opérateur voit toujours");
    }

    #[test]
    fn server_resolution() {
        let a = ServerInfo { id: Uuid::new_v4(), name: "Survie".into(), running: true, remote: None };
        let b = ServerInfo { id: Uuid::new_v4(), name: "Creatif".into(), running: false, remote: Some("vps".into()) };
        let list = vec![a.clone(), b.clone()];
        assert_eq!(resolve(&list, "survie").unwrap().id, a.id);
        assert_eq!(resolve(&list, &b.id.to_string()).unwrap().id, b.id);
        assert!(resolve(&list, "inconnu").is_err());
        let dup = vec![a.clone(), ServerInfo { id: Uuid::new_v4(), ..a.clone() }];
        assert!(resolve(&dup, "Survie").is_err(), "un nom ambigu est refusé");
    }

    #[test]
    fn command_definition_is_valid_for_discord() {
        let d = command_definitions();
        let cmd = &d[0];
        assert_eq!(cmd["name"], "mc");
        let subs = cmd["options"].as_array().unwrap();
        assert_eq!(subs.len(), 7);
        for s in subs {
            let name = s["name"].as_str().unwrap();
            assert!(name.chars().all(|c| c.is_ascii_lowercase()), "noms de sous-commandes en minuscules sans accent : {name}");
            assert!(s["description"].as_str().unwrap().chars().count() <= 100);
        }
    }

    #[tokio::test]
    async fn notifications_respect_toggles_and_target() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("notify").await;
        let mut cfg = base_cfg();
        cfg.notify_backup = false;
        save_config(&dir, &cfg).await.unwrap();

        notify_at(&state.http, &dir, &rest, Event::Crash, "Survie", "boom").await;
        notify_at(&state.http, &dir, &rest, Event::Backup, "Survie", "coupé").await; // coupé : rien
        notify_at(&state.http, &dir, &rest, Event::PlayerJoin, "Survie", "Alex").await; // off par défaut
        let sent = rec.find("/channels/C1/messages");
        assert_eq!(sent.len(), 1, "seul le crash doit partir : {:?}", rec.all());
        assert!(sent[0]["embeds"][0]["title"].as_str().unwrap().contains("Survie"));
        assert_eq!(sent[0]["embeds"][0]["color"], 0xF38BA8);

        // Désactivé globalement : plus rien.
        cfg.enabled = false;
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Crash, "Survie", "boom").await;
        assert_eq!(rec.find("/channels/C1/messages").len(), 1);

        // Sans bot : webhook.
        cfg.enabled = true;
        cfg.bot_token.clear();
        cfg.webhook_url = format!("{rest}/webhooks/42/tok");
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Ready, "Survie", "ok").await;
        assert_eq!(rec.find("/webhooks/42/tok").len(), 1);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn interactions_enforce_guild_and_roles() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("inter").await;
        let srv_dir = dir.join("srv");
        tokio::fs::create_dir_all(&srv_dir).await.unwrap();
        let sid = Uuid::new_v4();
        state.servers.write().await.insert(sid, sample_server(sid, "Survie", &srv_dir));
        save_config(&dir, &base_cfg()).await.unwrap();

        let last_content = |rec: &Recorder| -> (u64, String, Option<u64>) {
            let cb = rec.find("/interactions/I1/TOK1/callback").pop().expect("callback");
            (cb["type"].as_u64().unwrap(), cb["data"]["content"].as_str().unwrap_or("").to_string(), cb["data"]["flags"].as_u64())
        };
        let sub = |name: &str, extra: Value| json!([{ "name": name, "type": 1, "options": extra }]);

        // Mauvaise guilde : refusée.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "AUTRE", "U1", &[], sub("liste", json!([])))).await;
        let (_, content, flags) = last_content(&rec);
        assert!(content.contains("n'est pas autorisé") && flags == Some(64));

        // /mc liste : ouvert à tout membre.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], sub("liste", json!([])))).await;
        let (t, content, flags) = last_content(&rec);
        assert_eq!(t, 4);
        assert!(content.contains("Survie") && content.contains("⚫"));
        assert_eq!(flags, None, "la liste est visible de tous");

        // /mc demarrer par un non-opérateur : refusé, et le serveur n'est PAS démarré.
        let opts = json!([{ "name": "serveur", "type": 3, "value": sid.to_string() }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], sub("demarrer", opts.clone()))).await;
        let (_, content, flags) = last_content(&rec);
        assert!(content.contains("réservée aux opérateurs") && flags == Some(64));
        assert!(!state.runtime.read().await.get(&sid).map(|r| r.running).unwrap_or(false));

        // /mc arreter par un opérateur sur un serveur déjà arrêté : message clair.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], sub("arreter", opts.clone()))).await;
        let (_, content, _) = last_content(&rec);
        assert!(content.contains("déjà arrêté"), "{content}");

        // /mc commande par un opérateur : autorisée mais échoue proprement (serveur non lancé).
        let cmd_opts = json!([{ "name": "serveur", "type": 3, "value": "survie" }, { "name": "texte", "type": 3, "value": "say bonjour" }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], sub("commande", cmd_opts))).await;
        let (_, content, _) = last_content(&rec);
        assert!(content.starts_with("❌"), "{content}");
        // Une commande multi-lignes est rejetée avant tout envoi.
        let evil = json!([{ "name": "serveur", "type": 3, "value": "survie" }, { "name": "texte", "type": 3, "value": "say a\nstop" }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], sub("commande", evil))).await;
        let (_, content, flags) = last_content(&rec);
        assert!(content.contains("invalide") && flags == Some(64));

        // Autocomplétion : type 8 avec le serveur.
        let ac = json!([{ "name": "statut", "type": 1, "options": [{ "name": "serveur", "type": 3, "value": "sur", "focused": true }] }]);
        handle_interaction(state.clone(), rest.clone(), interaction(4, "G1", "U9", &[], ac)).await;
        let cb = rec.find("/interactions/I1/TOK1/callback").pop().unwrap();
        assert_eq!(cb["type"], 8);
        assert_eq!(cb["data"]["choices"][0]["value"], sid.to_string());

        // Rôle de lecture requis : un inconnu n'obtient rien (même pas l'autocomplétion).
        let mut cfg = base_cfg();
        cfg.viewer_role_ids = vec!["R_VIEW".into()];
        save_config(&dir, &cfg).await.unwrap();
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], sub("liste", json!([])))).await;
        let (_, content, flags) = last_content(&rec);
        assert!(content.contains("pas la permission") && flags == Some(64));
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    /// Bout en bout contre un faux Discord (passerelle WebSocket + REST) :
    /// Hello -> Identify(token) -> READY -> enregistrement des commandes ->
    /// battements de coeur -> interaction -> réponse, puis reprise de session
    /// après une déconnexion.
    #[tokio::test]
    async fn gateway_session_end_to_end() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("gw").await;
        let srv_dir = dir.join("srv");
        tokio::fs::create_dir_all(&srv_dir).await.unwrap();
        let sid = Uuid::new_v4();
        state.servers.write().await.insert(sid, sample_server(sid, "Survie", &srv_dir));
        save_config(&dir, &base_cfg()).await.unwrap();

        let gw = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gw_url = format!("ws://{}", gw.local_addr().unwrap());
        let (report_tx, mut report_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Faux serveur de passerelle
        let gw_url_for_ready = gw_url.clone();
        tokio::spawn(async move {
            // 1re connexion : identify
            for round in 0..2 {
                let (sock, _) = gw.accept().await.unwrap();
                let ws = tokio_tungstenite_discord::accept_async(sock).await.unwrap();
                let (mut tx, mut rx) = ws.split();
                tx.send(WsMessage::Text(json!({ "op": 10, "d": { "heartbeat_interval": 200 } }).to_string())).await.unwrap();
                let first: Value = match rx.next().await.unwrap().unwrap() { WsMessage::Text(t) => serde_json::from_str(&t).unwrap(), m => panic!("{m:?}") };
                if round == 0 {
                    assert_eq!(first["op"], 2, "première connexion : Identify");
                    assert_eq!(first["d"]["token"], "tkn-secret");
                    report_tx.send("identify".into()).unwrap();
                    tx.send(WsMessage::Text(json!({ "op": 0, "s": 1, "t": "READY", "d": {
                        "session_id": "SESS1", "resume_gateway_url": gw_url_for_ready,
                        "user": { "username": "McBot" }, "application": { "id": "APP1" } } }).to_string())).await.unwrap();
                    // Un battement de coeur doit arriver, on l'acquitte.
                    loop {
                        match rx.next().await.unwrap().unwrap() {
                            WsMessage::Text(t) => {
                                let v: Value = serde_json::from_str(&t).unwrap();
                                if v["op"] == 1 { report_tx.send("heartbeat".into()).unwrap(); tx.send(WsMessage::Text(json!({ "op": 11 }).to_string())).await.unwrap(); break; }
                            }
                            _ => {}
                        }
                    }
                    // Une commande /mc liste d'un membre.
                    tx.send(WsMessage::Text(json!({ "op": 0, "s": 2, "t": "INTERACTION_CREATE", "d": interaction(2, "G1", "U9", &[], json!([{ "name": "liste", "type": 1 }])) }).to_string())).await.unwrap();
                    // On reste connecté 600 ms en acquittant les battements de coeur (sinon le bot,
                    // à raison, jugerait la connexion morte), puis on coupe : il doit REPRENDRE la session.
                    let until = tokio::time::Instant::now() + Duration::from_millis(600);
                    while let Ok(Some(Ok(WsMessage::Text(t)))) = tokio::time::timeout_at(until, rx.next()).await {
                        let v: Value = serde_json::from_str(&t).unwrap();
                        if v["op"] == 1 { tx.send(WsMessage::Text(json!({ "op": 11 }).to_string())).await.ok(); }
                    }
                    tx.send(WsMessage::Close(None)).await.ok();
                } else {
                    assert_eq!(first["op"], 6, "reconnexion : Resume, pas un nouvel Identify");
                    assert_eq!(first["d"]["session_id"], "SESS1");
                    assert_eq!(first["d"]["seq"], 2);
                    report_tx.send("resume".into()).unwrap();
                    tx.send(WsMessage::Text(json!({ "op": 0, "s": 3, "t": "RESUMED", "d": {} }).to_string())).await.unwrap();
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        });

        let st = state.clone();
        let task = tokio::spawn(async move { run_forever(st, gw_url, rest).await });

        let mut seen = Vec::new();
        for _ in 0..3 {
            let ev = tokio::time::timeout(Duration::from_secs(15), report_rx.recv()).await.expect("évènement de passerelle attendu").unwrap();
            seen.push(ev);
        }
        assert_eq!(seen, vec!["identify", "heartbeat", "resume"]);

        // Commandes enregistrées dans la bonne guilde, avec le bon token.
        let reg = rec.all().into_iter().find(|(m, p, _)| m == "PUT" && p == "/applications/APP1/guilds/G1/commands").expect("enregistrement des commandes");
        assert_eq!(reg.2[0]["name"], "mc");
        // L'interaction a reçu sa réponse.
        let cb = rec.find("/interactions/I1/TOK1/callback");
        assert!(!cb.is_empty() && cb[0]["data"]["content"].as_str().unwrap().contains("Survie"));
        // Après la reprise, l'état repasse à « connecté » dès que RESUMED est traité.
        for _ in 0..30 {
            if status().state == "connected" { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let st = status();
        assert_eq!(st.state, "connected");
        assert_eq!(st.bot_name.as_deref(), Some("McBot"));
        task.abort();
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
