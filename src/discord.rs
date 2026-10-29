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
/// GUILDS + GUILD_MESSAGES + MESSAGE_CONTENT (privilégiée : à activer dans le portail développeur).
const INTENTS_FULL: u64 = 1 | (1 << 9) | (1 << 15);
/// Sans lecture du contenu des messages : notifications et commandes slash seulement.
const INTENTS_BASIC: u64 = 1 | (1 << 9);

// ───────────────────────── configuration ─────────────────────────

fn tru() -> bool {
    true
}

/// Salons Discord d'UN serveur Minecraft : une catégorie contenant un salon de
/// statut, une console (sortie en direct + envoi de commandes) et un chat relié
/// au chat du jeu. Créés par le bot (« Créer les salons » dans les réglages).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerChannels {
    #[serde(default)]
    pub category_id: String,
    #[serde(default)]
    pub status_channel_id: String,
    #[serde(default)]
    pub console_channel_id: String,
    #[serde(default)]
    pub chat_channel_id: String,
    /// Diffuser la console du serveur dans le salon console.
    #[serde(default)]
    pub stream_console: bool,
    /// Relier le chat du jeu et le salon chat dans les deux sens.
    #[serde(default)]
    pub bridge_chat: bool,
    /// Rôles qui peuvent *envoyer des commandes* dans le salon console de ce serveur
    /// (en plus des opérateurs globaux).
    #[serde(default)]
    pub console_command_role_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DiscordConfig {
    pub enabled: bool,
    /// "fr" ou "en" : langue des notifications et des messages du bot dans les salons.
    /// Les réponses aux commandes slash suivent, elles, la langue de l'utilisateur Discord.
    pub language: String,
    /// Rôles qui voient (en lecture seule) les salons de statut et de console.
    pub console_view_role_ids: Vec<String>,
    /// Rôles qui voient et écrivent dans les salons de chat.
    pub chat_role_ids: Vec<String>,
    pub servers: std::collections::HashMap<Uuid, ServerChannels>,
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
    language: String,
    #[serde(default)]
    console_view_role_ids: Vec<String>,
    #[serde(default)]
    chat_role_ids: Vec<String>,
    #[serde(default)]
    servers: std::collections::HashMap<Uuid, ServerChannels>,
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
        language: if stored.language.is_empty() { "fr".into() } else { stored.language },
        console_view_role_ids: stored.console_view_role_ids,
        chat_role_ids: stored.chat_role_ids,
        servers: stored.servers,
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
        language: cfg.language.clone(),
        console_view_role_ids: cfg.console_view_role_ids.clone(),
        chat_role_ids: cfg.chat_role_ids.clone(),
        servers: cfg.servers.clone(),
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
    pub bot_user_id: Option<String>,
    pub commands_registered: bool,
    /// « Message Content Intent » non activé dans le portail développeur : les
    /// notifications et commandes slash marchent, mais pas la console ni le chat dans Discord.
    pub message_content_missing: bool,
}

fn status_cell() -> &'static std::sync::RwLock<BotStatus> {
    static S: OnceLock<std::sync::RwLock<BotStatus>> = OnceLock::new();
    S.get_or_init(|| std::sync::RwLock::new(BotStatus { state: "disabled".into(), detail: String::new(), bot_name: None, bot_user_id: None, commands_registered: false, message_content_missing: false }))
}

pub fn status() -> BotStatus {
    status_cell().read().map(|s| s.clone()).unwrap_or(BotStatus { state: "error".into(), detail: "etat illisible".into(), bot_name: None, bot_user_id: None, commands_registered: false, message_content_missing: false })
}

fn set_status(state: &str, detail: impl Into<String>) {
    if let Ok(mut s) = status_cell().write() {
        s.state = state.to_string();
        s.detail = detail.into();
        if state == "disabled" {
            s.message_content_missing = false;
        }
        if state != "connected" {
            s.commands_registered = false;
            if state == "disabled" {
                s.bot_name = None;
            }
        }
    }
}

// ───────────────────────── langue ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn from_config(s: &str) -> Lang {
        if s.eq_ignore_ascii_case("en") { Lang::En } else { Lang::Fr }
    }
    /// Locale Discord de l'utilisateur ("fr", "en-US", "en-GB", "de"...) : français si "fr*", anglais sinon.
    pub fn from_locale(loc: &str) -> Lang {
        if loc.to_ascii_lowercase().starts_with("fr") { Lang::Fr } else { Lang::En }
    }
}

pub fn pick<'a>(l: Lang, fr: &'a str, en: &'a str) -> &'a str {
    match l {
        Lang::Fr => fr,
        Lang::En => en,
    }
}

// ───────────────────────── REST ─────────────────────────

/// Appel REST Discord authentifié par le bot. Une réponse 429 (limite de débit) est
/// réessayée une fois après l'attente demandée par Discord (au plus 10 s).
pub(crate) async fn rest(http: &reqwest::Client, api_base: &str, token: &str, method: reqwest::Method, path: &str, body: Option<&Value>) -> anyhow::Result<Value> {
    for attempt in 0..2 {
        let mut req = http.request(method.clone(), format!("{api_base}{path}")).header("Authorization", format!("Bot {token}")).timeout(Duration::from_secs(20));
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.as_u16() == 429 && attempt == 0 {
            let v: Value = resp.json().await.unwrap_or(Value::Null);
            let wait = v["retry_after"].as_f64().unwrap_or(1.0).clamp(0.1, 10.0);
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
            continue;
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Discord {status} : {}", text.chars().take(300).collect::<String>());
        }
        return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
    }
    anyhow::bail!("Discord : trop de requêtes (limite de débit)")
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

    fn title(&self, l: Lang, server: &str) -> String {
        let (fr, en) = match self {
            Event::Started => ("🟡 {} démarre…", "🟡 {} is starting…"),
            Event::Ready => ("🟢 {} est en ligne", "🟢 {} is online"),
            Event::Stopped => ("⚫ {} est arrêté", "⚫ {} is stopped"),
            Event::Crash => ("🔴 {} a planté", "🔴 {} crashed"),
            Event::Backup => ("💾 Sauvegarde de {}", "💾 Backup of {}"),
            Event::ScheduledRestart => ("🔄 Redémarrage programmé de {}", "🔄 Scheduled restart of {}"),
            Event::AutoStop => ("⏸ {} arrêté (inactif)", "⏸ {} stopped (idle)"),
            Event::PlayerJoin => ("➕ Connexion sur {}", "➕ Player joined {}"),
            Event::PlayerLeave => ("➖ Déconnexion de {}", "➖ Player left {}"),
        };
        pick(l, fr, en).replacen("{}", server, 1)
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

/// Aucun message du bot ne doit jamais pinguer quelqu'un (@everyone, @here, rôles,
/// utilisateurs) : un nom de joueur ou un texte de chat ne peut pas déclencher de notification de masse.
pub(crate) fn no_mentions() -> Value {
    json!({ "parse": [] })
}

/// Envoie la notification si Discord est activé et que cet évènement n'est pas
/// coupé. Les échecs sont journalisés puis ignorés : une notification perdue ne
/// doit jamais faire échouer un démarrage ou une sauvegarde. `fr`/`en` : le même
/// message dans les deux langues, la langue des réglages décide.
pub async fn notify(state: &AppState, event: Event, server_id: Option<Uuid>, server_name: &str, fr: &str, en: &str) {
    notify_at(&state.http, &state.data_dir, API_BASE, event, server_id, server_name, fr, en).await
}

#[allow(clippy::too_many_arguments)]
pub async fn notify_at(http: &reqwest::Client, data_dir: &Path, api_base: &str, event: Event, server_id: Option<Uuid>, server_name: &str, fr: &str, en: &str) {
    let cfg = load_config(data_dir).await;
    if !cfg.enabled || !event.allowed(&cfg) {
        return;
    }
    let lang = Lang::from_config(&cfg.language);
    let body = json!({ "embeds": [embed(&event.title(lang, server_name), pick(lang, fr, en), event.color())], "allowed_mentions": no_mentions() });
    // Salon de statut propre au serveur s'il existe, sinon salon/webhook global.
    let dedicated = server_id.and_then(|id| cfg.servers.get(&id)).map(|sc| sc.status_channel_id.clone()).filter(|c| !c.is_empty());
    let result = match dedicated {
        Some(channel) if !cfg.bot_token.is_empty() => rest(http, api_base, &cfg.bot_token, reqwest::Method::POST, &format!("/channels/{channel}/messages"), Some(&body)).await.map(|_| ()),
        _ => post_message(http, &cfg, api_base, &body).await,
    };
    if let Err(e) = result {
        tracing::warn!("echec de l'envoi de la notification Discord : {e}");
    }
}

/// Bot + salon si configurés, sinon webhook.
async fn post_message(http: &reqwest::Client, cfg: &DiscordConfig, api_base: &str, body: &Value) -> anyhow::Result<()> {
    if !cfg.bot_token.is_empty() && !cfg.channel_id.trim().is_empty() {
        rest(http, api_base, &cfg.bot_token, reqwest::Method::POST, &format!("/channels/{}/messages", cfg.channel_id.trim()), Some(body)).await?;
        return Ok(());
    }
    if cfg.webhook_url.trim().is_empty() {
        anyhow::bail!("aucun salon (avec le bot) ni webhook configuré");
    }
    let resp = http.post(cfg.webhook_url.trim()).json(body).timeout(Duration::from_secs(15)).send().await?;
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
    let l = Lang::from_config(&cfg.language);
    let body = json!({
        "embeds": [embed("✅ MCManager", pick(l, "Notification de test : la configuration Discord fonctionne.", "Test notification: the Discord configuration works."), 0xA6E3A1)],
        "allowed_mentions": no_mentions(),
    });
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
    pub(crate) fn label(&self) -> String {
        match &self.remote {
            Some(r) => format!("{} · 🖧 {r}", self.name),
            None => self.name.clone(),
        }
    }
}

pub(crate) async fn gather_servers(state: &AppState) -> Vec<ServerInfo> {
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

pub(crate) fn resolve<'a>(servers: &'a [ServerInfo], query: &str, l: Lang) -> Result<&'a ServerInfo, String> {
    let q = query.trim();
    if let Ok(id) = Uuid::parse_str(q) {
        return servers.iter().find(|s| s.id == id).ok_or_else(|| pick(l, "Serveur introuvable.", "Server not found.").to_string());
    }
    let matches: Vec<&ServerInfo> = servers.iter().filter(|s| s.name.eq_ignore_ascii_case(q)).collect();
    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(pick(l, "Aucun serveur nommé « {q} ».", "No server named “{q}”.").replace("{q}", q)),
        _ => Err(pick(l, "Plusieurs serveurs s'appellent « {q} » : choisissez-le dans la liste d'autocomplétion.", "Several servers are named “{q}”: pick it from the autocomplete list.").replace("{q}", q)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
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

pub(crate) async fn control(state: &AppState, server: &ServerInfo, action: Action) -> anyhow::Result<()> {
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

pub(crate) async fn send_console_command(state: &AppState, server: &ServerInfo, command: &str) -> anyhow::Result<()> {
    if let Some(label) = &server.remote {
        let target = crate::hub::target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("instance « {label} » inconnue"))?;
        let identity = crate::hub::identity(state).await?;
        crate::remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "send", "server_id": server.id, "command": command }), Duration::from_secs(15)).await?;
        return Ok(());
    }
    crate::process::send_command(state, server.id, command).await
}

/// (en ligne, max) via le ping du protocole Minecraft.
pub(crate) async fn player_counts(state: &AppState, server: &ServerInfo) -> Option<(u32, u32)> {
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

pub(crate) struct Caller {
    pub(crate) user_id: String,
    pub(crate) user_name: String,
    pub(crate) roles: Vec<String>,
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

    pub(crate) fn is_operator(&self, cfg: &DiscordConfig) -> bool {
        cfg.operator_user_ids.iter().any(|u| u == &self.user_id) || self.roles.iter().any(|r| cfg.operator_role_ids.contains(r))
    }

    /// Auteur d'un message du salon (évènement MESSAGE_CREATE).
    pub(crate) fn from_message(m: &Value) -> Option<Self> {
        let user = m.get("author")?;
        let roles = m["member"]["roles"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        Some(Caller { user_id: user["id"].as_str()?.to_string(), user_name: user["username"].as_str().unwrap_or("?").to_string(), roles })
    }

    fn can_view(&self, cfg: &DiscordConfig) -> bool {
        self.is_operator(cfg) || cfg.viewer_role_ids.is_empty() || self.roles.iter().any(|r| cfg.viewer_role_ids.contains(r))
    }
}

// ───────────────────────── commandes slash ─────────────────────────

/// Commandes déclarées en anglais (nom canonique) avec traduction française :
/// chaque utilisateur voit les noms et descriptions dans la langue de son client Discord.
fn command_definitions() -> Value {
    let server_opt = json!({
        "type": 3, "name": "server", "name_localizations": { "fr": "serveur" },
        "description": "Server name", "description_localizations": { "fr": "Nom du serveur" },
        "required": true, "autocomplete": true
    });
    let sub = |name: &str, fr_name: &str, desc: &str, fr_desc: &str, options: Vec<Value>| {
        json!({
            "type": 1, "name": name, "name_localizations": { "fr": fr_name },
            "description": desc, "description_localizations": { "fr": fr_desc },
            "options": options,
        })
    };
    json!([{
        "name": "mc",
        "description": "Manage Minecraft servers (MCManager)",
        "description_localizations": { "fr": "Gérer les serveurs Minecraft (MCManager)" },
        "type": 1,
        "dm_permission": false,
        "options": [
            sub("list", "liste", "List servers and their state", "Liste des serveurs et leur état", vec![]),
            sub("status", "statut", "Detailed state of a server", "État détaillé d'un serveur", vec![server_opt.clone()]),
            sub("players", "joueurs", "Connected players", "Joueurs connectés", vec![server_opt.clone()]),
            sub("start", "demarrer", "Start a server (operators)", "Démarrer un serveur (opérateurs)", vec![server_opt.clone()]),
            sub("stop", "arreter", "Stop a server cleanly (operators)", "Arrêter un serveur proprement (opérateurs)", vec![server_opt.clone()]),
            sub("restart", "redemarrer", "Restart a server (operators)", "Redémarrer un serveur (opérateurs)", vec![server_opt.clone()]),
            sub("command", "commande", "Send a console command (operators)", "Envoyer une commande à la console (opérateurs)", vec![
                server_opt,
                json!({ "type": 3, "name": "text", "name_localizations": { "fr": "texte" },
                        "description": "Command, without the /", "description_localizations": { "fr": "Commande, sans le /" },
                        "required": true, "max_length": 200 }),
            ]),
        ]
    }])
}

async fn register_commands(http: &reqwest::Client, api_base: &str, token: &str, app_id: &str, guild_id: &str) -> anyhow::Result<()> {
    rest(http, api_base, token, reqwest::Method::PUT, &format!("/applications/{app_id}/guilds/{guild_id}/commands"), Some(&command_definitions()))
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("enregistrement des commandes refusé : {e}"))
}

/// Réponse (callback) à une interaction. `ephemeral` = visible seulement par l'auteur.
async fn respond(http: &reqwest::Client, api_base: &str, interaction: &Value, content: &str, ephemeral: bool) {
    let (Some(id), Some(token)) = (interaction["id"].as_str(), interaction["token"].as_str()) else { return };
    let mut data = json!({ "content": content.chars().take(1900).collect::<String>(), "allowed_mentions": no_mentions() });
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
    // Réponses dans la langue du client Discord de la personne ; langue des réglages si inconnue.
    let l = match interaction["locale"].as_str() {
        Some(loc) => Lang::from_locale(loc),
        None => Lang::from_config(&cfg.language),
    };

    if !cfg.enabled || interaction["guild_id"].as_str() != Some(cfg.guild_id.as_str()) {
        respond(&http, &api_base, &interaction, pick(l, "⛔ Ce serveur Discord n'est pas autorisé à utiliser ce bot.", "⛔ This Discord server is not allowed to use this bot."), true).await;
        return;
    }
    let Some(caller) = Caller::from_interaction(&interaction) else {
        respond(&http, &api_base, &interaction, pick(l, "⛔ Utilisateur non identifiable.", "⛔ Could not identify the user."), true).await;
        return;
    };
    if !caller.can_view(&cfg) {
        respond(&http, &api_base, &interaction, pick(l, "⛔ Vous n'avez pas la permission d'utiliser cette commande.", "⛔ You do not have permission to use this command."), true).await;
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

            if sub_name == "list" {
                let text = if servers.is_empty() {
                    pick(l, "Aucun serveur.", "No servers.").to_string()
                } else {
                    servers.iter().map(|s| format!("{} **{}**{}", if s.running { "🟢" } else { "⚫" }, s.name, s.remote.as_ref().map(|r| format!(" · 🖧 {r}")).unwrap_or_default())).collect::<Vec<_>>().join("\n")
                };
                respond(&http, &api_base, &interaction, &text, false).await;
                return;
            }

            let query = option_value(&sub, "server").unwrap_or_default();
            let server = match resolve(&servers, &query, l) {
                Ok(s) => s.clone(),
                Err(msg) => {
                    respond(&http, &api_base, &interaction, &format!("❌ {msg}"), true).await;
                    return;
                }
            };

            match sub_name.as_str() {
                "status" => {
                    let players = if server.running { player_counts(&state, &server).await } else { None };
                    let line = match (server.running, players) {
                        (true, Some((o, m))) => pick(l, "🟢 **{n}** est en ligne — {o}/{m} joueur(s).", "🟢 **{n}** is online — {o}/{m} player(s).").replace("{n}", &server.label()).replace("{o}", &o.to_string()).replace("{m}", &m.to_string()),
                        (true, None) => pick(l, "🟡 **{n}** est démarré (joueurs : indisponible, démarrage en cours ?).", "🟡 **{n}** is running (players unavailable, still starting?).").replace("{n}", &server.label()),
                        (false, _) => pick(l, "⚫ **{n}** est arrêté.", "⚫ **{n}** is stopped.").replace("{n}", &server.label()),
                    };
                    respond(&http, &api_base, &interaction, &line, false).await;
                }
                "players" => {
                    let line = if !server.running {
                        pick(l, "⚫ **{n}** est arrêté.", "⚫ **{n}** is stopped.").replace("{n}", &server.label())
                    } else {
                        match player_counts(&state, &server).await {
                            Some((o, m)) => pick(l, "👥 **{n}** : {o}/{m} joueur(s) connecté(s).", "👥 **{n}**: {o}/{m} player(s) online.").replace("{n}", &server.label()).replace("{o}", &o.to_string()).replace("{m}", &m.to_string()),
                            None => pick(l, "❓ Impossible de lire les joueurs de **{n}** pour l'instant.", "❓ Cannot read the players of **{n}** right now.").replace("{n}", &server.label()),
                        }
                    };
                    respond(&http, &api_base, &interaction, &line, false).await;
                }
                "start" | "stop" | "restart" | "command" => {
                    if !caller.is_operator(&cfg) {
                        tracing::warn!("[discord] refus : {} ({}) a tenté /mc {sub_name} sur {}", caller.user_name, caller.user_id, server.name);
                        respond(&http, &api_base, &interaction, pick(l, "⛔ Cette action est réservée aux opérateurs (voir les réglages Discord de MCManager).", "⛔ This action is reserved to operators (see MCManager's Discord settings)."), true).await;
                        return;
                    }
                    if sub_name == "command" {
                        let text = option_value(&sub, "text").unwrap_or_default();
                        let text = text.trim().trim_start_matches('/').to_string();
                        if text.is_empty() || text.contains('\n') || text.contains('\r') {
                            respond(&http, &api_base, &interaction, pick(l, "❌ Commande vide ou invalide.", "❌ Empty or invalid command."), true).await;
                            return;
                        }
                        tracing::info!("[discord] {} ({}) : commande « {text} » sur {}", caller.user_name, caller.user_id, server.name);
                        let reply = match send_console_command(&state, &server, &text).await {
                            Ok(()) => pick(l, "✅ Commande envoyée à **{n}** : `{c}`", "✅ Command sent to **{n}**: `{c}`").replace("{n}", &server.label()).replace("{c}", &text.replace('`', "'")),
                            Err(e) => format!("❌ {e}"),
                        };
                        respond(&http, &api_base, &interaction, &reply, false).await;
                        return;
                    }
                    let (action, doing_fr, doing_en) = match sub_name.as_str() {
                        "start" => (Action::Start, "Démarrage", "Start"),
                        "stop" => (Action::Stop, "Arrêt", "Stop"),
                        _ => (Action::Restart, "Redémarrage", "Restart"),
                    };
                    tracing::info!("[discord] {} ({}) : {sub_name} {}", caller.user_name, caller.user_id, server.name);
                    if action == Action::Start && server.running {
                        respond(&http, &api_base, &interaction, &pick(l, "ℹ️ **{n}** est déjà en ligne.", "ℹ️ **{n}** is already online.").replace("{n}", &server.label()), true).await;
                        return;
                    }
                    if action == Action::Stop && !server.running {
                        respond(&http, &api_base, &interaction, &pick(l, "ℹ️ **{n}** est déjà arrêté.", "ℹ️ **{n}** is already stopped.").replace("{n}", &server.label()), true).await;
                        return;
                    }
                    // Démarrage/arrêt immédiats ; le redémarrage attend l'arrêt, donc en tâche de fond.
                    if action == Action::Restart {
                        respond(&http, &api_base, &interaction, &pick(l, "🔄 Redémarrage de **{n}** en cours… (une notification suivra)", "🔄 Restarting **{n}**… (a notification will follow)").replace("{n}", &server.label()), false).await;
                        let (st, sv) = (state.clone(), server.clone());
                        tokio::spawn(async move {
                            if let Err(e) = control(&st, &sv, Action::Restart).await {
                                notify(&st, Event::Crash, Some(sv.id), &sv.name, &format!("Le redémarrage demandé depuis Discord a échoué : {e}"), &format!("The restart requested from Discord failed: {e}")).await;
                            }
                        });
                        return;
                    }
                    let reply = match control(&state, &server, action).await {
                        Ok(()) => pick(l, "✅ {d} de **{n}** demandé par {u}.", "✅ {d} of **{n}** requested by {u}.")
                            .replace("{d}", pick(l, doing_fr, doing_en)).replace("{n}", &server.label()).replace("{u}", &caller.user_name),
                        Err(e) => format!("❌ {e}"),
                    };
                    respond(&http, &api_base, &interaction, &reply, false).await;
                }
                _ => respond(&http, &api_base, &interaction, pick(l, "❌ Commande inconnue.", "❌ Unknown command."), true).await,
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
    crate::discord_bridge::spawn_pumps(state.clone(), API_BASE.to_string());
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
        json!({ "op": 2, "d": { "token": cfg.bot_token, "intents": if status().message_content_missing { INTENTS_BASIC } else { INTENTS_FULL }, "properties": { "os": std::env::consts::OS, "browser": "mcmanager", "device": "mcmanager" } } })
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
                            // 4014 : intention privilégiée non activée. On se reconnecte sans elle (slash + notifications
                            // continuent de marcher) et on prévient l'interface plutôt que de tout bloquer.
                            4014 if !status().message_content_missing => {
                                if let Ok(mut st) = status_cell().write() { st.message_content_missing = true; }
                                *session = GatewaySession::default();
                                SessionEnd::Reconnect
                            }
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
                            if let Ok(mut s) = status_cell().write() {
                                s.bot_name = Some(name.clone());
                                s.bot_user_id = payload["d"]["user"]["id"].as_str().map(|x| x.to_string());
                            }
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
                        Some("MESSAGE_CREATE") => {
                            let (st, base, msg) = (state.clone(), api_base.to_string(), payload["d"].clone());
                            tokio::spawn(crate::discord_bridge::handle_message(st, base, msg));
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

/// Faux Discord (REST) partagé par les tests de `discord` et `discord_bridge`.
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    pub struct Recorder {
        pub calls: Arc<Mutex<Vec<(String, String, Value)>>>,
        /// Salons « supprimés à la main » : PATCH/GET répondent 404.
        pub missing: Arc<Mutex<std::collections::HashSet<String>>>,
        counter: Arc<Mutex<u32>>,
    }

    impl Recorder {
        pub fn all(&self) -> Vec<(String, String, Value)> {
            self.calls.lock().unwrap().clone()
        }
        pub fn find(&self, path_part: &str) -> Vec<Value> {
            self.all().into_iter().filter(|(_, p, _)| p.contains(path_part)).map(|(_, _, b)| b).collect()
        }
        pub fn count(&self, method: &str, path_part: &str) -> usize {
            self.all().into_iter().filter(|(m, p, _)| m == method && p.contains(path_part)).count()
        }
    }

    pub async fn mock_rest() -> (String, Recorder) {
        use axum::extract::State as AxState;
        let rec = Recorder::default();
        async fn handler(AxState(rec): AxState<Recorder>, method: axum::http::Method, uri: axum::http::Uri, body: axum::body::Bytes) -> axum::response::Response {
            use axum::response::IntoResponse;
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let (m, path) = (method.to_string(), uri.path().to_string());
            rec.calls.lock().unwrap().push((m.clone(), path.clone(), v));
            let json = |v: Value| axum::Json(v).into_response();
            match (m.as_str(), path.as_str()) {
                ("GET", "/users/@me") => json(json!({ "id": "BOT1", "username": "McBot" })),
                ("GET", "/users/@me/guilds") => json(json!([{ "id": "G1", "name": "Test Guild", "icon": null }])),
                ("GET", "/oauth2/applications/@me") => json(json!({ "id": "APP1" })),
                ("GET", "/guilds/G1/roles") => json(json!([
                    { "id": "G1", "name": "@everyone", "position": 0, "color": 0, "managed": false },
                    { "id": "R_ADMIN", "name": "Admin", "position": 10, "color": 0xE74C3C, "managed": false },
                    { "id": "R_MEMBER", "name": "Membre", "position": 2, "color": 0, "managed": false },
                    { "id": "R_BOT", "name": "McBot", "position": 5, "color": 0, "managed": true },
                ])),
                ("GET", "/guilds/G1/channels") => json(json!([
                    { "id": "T1", "name": "general", "type": 0, "parent_id": null },
                    { "id": "V1", "name": "Vocal", "type": 2, "parent_id": null },
                    { "id": "K1", "name": "Catégorie", "type": 4, "parent_id": null },
                ])),
                ("POST", "/guilds/G1/channels") => {
                    let mut c = rec.counter.lock().unwrap();
                    *c += 1;
                    json(json!({ "id": format!("CH{}", *c) }))
                }
                ("PATCH", p) if p.starts_with("/channels/") => {
                    if rec.missing.lock().unwrap().contains(p.trim_start_matches("/channels/")) {
                        (axum::http::StatusCode::NOT_FOUND, axum::Json(json!({ "message": "Unknown Channel", "code": 10003 }))).into_response()
                    } else {
                        json(json!({}))
                    }
                }
                _ => json(json!({})),
            }
        }
        let app = axum::Router::new().fallback(handler).with_state(rec.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok(); });
        (format!("http://{addr}"), rec)
    }

    pub async fn temp_state(tag: &str) -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mcmanager-discord-{tag}-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        (crate::state::build_state_for_test(&dir).await, dir)
    }

    pub fn base_cfg() -> DiscordConfig {
        DiscordConfig {
            enabled: true,
            language: "fr".into(),
            bot_token: "tkn-secret".into(),
            guild_id: "G1".into(),
            channel_id: "C1".into(),
            operator_user_ids: vec!["U1".into()],
            notify_started: true, notify_ready: true, notify_stopped: true, notify_crash: true,
            notify_backup: true, notify_scheduled_restart: true, notify_auto_stop: true,
            ..Default::default()
        }
    }

    pub fn sample_server(id: Uuid, name: &str, folder: &Path) -> crate::models::ServerEntry {
        crate::models::ServerEntry {
            id, name: name.into(), loader: crate::models::Loader::Paper, mc_version: "1.21.4".into(), loader_version: None,
            folder: folder.to_string_lossy().to_string(), jar_name: String::new(), java_path: "java".into(),
            xms_mb: 512, xmx_mb: 1024, port: 25598, extra_args: vec![], eula_accepted: true, auto_backup_minutes: None,
            backup_retention: None, auto_restart: false, auto_restart_delay_secs: 5, scheduled_restart_minutes: None,
            stop_when_empty_minutes: None, dynamic_server: false, aikar_flags: false, managed_addons: vec![], created_at: chrono::Utc::now(),
        }
    }

    /// Faux processus de serveur : tout ce qui est écrit sur son entrée standard atterrit dans `out_file`.
    pub async fn fake_running_server(state: &AppState, id: Uuid, out_file: &Path) {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c").arg(format!("cat > '{}'", out_file.display()))
            .stdin(std::process::Stdio::piped()).spawn().expect("sh");
        let mut runtime = state.runtime.write().await;
        let rt = runtime.entry(id).or_insert_with(crate::state::ServerRuntime::default);
        rt.stdin = child.stdin.take();
        rt.child = Some(child);
        rt.running = true;
    }

    pub fn interaction(kind: u64, guild: &str, user: &str, roles: &[&str], locale: &str, options: Value) -> Value {
        json!({
            "id": "I1", "token": "TOK1", "type": kind, "guild_id": guild, "locale": locale,
            "member": { "user": { "id": user, "username": format!("user-{user}") }, "roles": roles },
            "data": { "name": "mc", "options": options },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn config_roundtrip_encrypts_secrets() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dir = std::env::temp_dir().join(format!("mcmanager-discord-cfg-{}", Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir).await.unwrap();
            let mut cfg = base_cfg();
            cfg.webhook_url = "https://discord.com/api/webhooks/123/secret-webhook-token".into();
            cfg.language = "en".into();
            cfg.console_view_role_ids = vec!["R1".into()];
            cfg.chat_role_ids = vec!["R2".into()];
            let sid = Uuid::new_v4();
            cfg.servers.insert(sid, ServerChannels { console_channel_id: "CH9".into(), stream_console: true, ..Default::default() });
            save_config(&dir, &cfg).await.unwrap();
            let raw = tokio::fs::read_to_string(config_path(&dir)).await.unwrap();
            assert!(!raw.contains("tkn-secret"), "le token ne doit pas être en clair sur disque");
            assert!(!raw.contains("secret-webhook-token"), "le webhook ne doit pas être en clair sur disque");
            let back = load_config(&dir).await;
            assert_eq!(back.bot_token, "tkn-secret");
            assert_eq!(back.webhook_url, cfg.webhook_url);
            assert_eq!(back.operator_user_ids, vec!["U1".to_string()]);
            assert_eq!(back.language, "en");
            assert_eq!(back.console_view_role_ids, vec!["R1".to_string()]);
            assert_eq!(back.chat_role_ids, vec!["R2".to_string()]);
            assert_eq!(back.servers[&sid].console_channel_id, "CH9");
            assert!(back.servers[&sid].stream_console && !back.servers[&sid].bridge_chat);
            assert!(back.notify_crash && !back.notify_player_join_leave);
            tokio::fs::remove_dir_all(&dir).await.ok();
        });
    }

    #[tokio::test]
    async fn old_config_files_still_load() {
        // Fichier d'avant les salons par serveur : champs absents = valeurs par défaut sûres.
        let dir = std::env::temp_dir().join(format!("mcmanager-discord-old-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(config_path(&dir), r#"{"enabled":true,"guild_id":"G1","channel_id":"C1","operator_user_ids":["U1"]}"#).await.unwrap();
        let cfg = load_config(&dir).await;
        assert!(cfg.enabled && cfg.servers.is_empty() && cfg.chat_role_ids.is_empty());
        assert_eq!(cfg.language, "fr");
        tokio::fs::remove_dir_all(&dir).await.ok();
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
        assert_eq!(resolve(&list, "survie", Lang::Fr).unwrap().id, a.id);
        assert_eq!(resolve(&list, &b.id.to_string(), Lang::Fr).unwrap().id, b.id);
        assert!(resolve(&list, "inconnu", Lang::Fr).unwrap_err().contains("Aucun serveur"));
        assert!(resolve(&list, "inconnu", Lang::En).unwrap_err().contains("No server named"));
        let dup = vec![a.clone(), ServerInfo { id: Uuid::new_v4(), ..a.clone() }];
        assert!(resolve(&dup, "Survie", Lang::En).is_err(), "un nom ambigu est refusé");
    }

    #[test]
    fn commands_are_english_with_french_translations() {
        let d = command_definitions();
        let cmd = &d[0];
        assert_eq!(cmd["name"], "mc");
        assert_eq!(cmd["description_localizations"]["fr"], "Gérer les serveurs Minecraft (MCManager)");
        let subs = cmd["options"].as_array().unwrap();
        let names: Vec<&str> = subs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list", "status", "players", "start", "stop", "restart", "command"]);
        let fr: Vec<&str> = subs.iter().map(|s| s["name_localizations"]["fr"].as_str().unwrap()).collect();
        assert_eq!(fr, ["liste", "statut", "joueurs", "demarrer", "arreter", "redemarrer", "commande"]);
        for s in subs {
            for n in [s["name"].as_str().unwrap(), s["name_localizations"]["fr"].as_str().unwrap()] {
                assert!(n.chars().all(|c| c.is_ascii_lowercase()), "noms sans accent ni majuscule : {n}");
            }
            assert!(s["description"].as_str().unwrap().chars().count() <= 100);
            assert!(s["description_localizations"]["fr"].as_str().unwrap().chars().count() <= 100);
        }
        // L'option « server » est traduite et auto-complétée.
        let opt = &subs[1]["options"][0];
        assert_eq!(opt["name"], "server");
        assert_eq!(opt["name_localizations"]["fr"], "serveur");
        assert_eq!(opt["autocomplete"], true);
    }

    #[test]
    fn language_selection() {
        assert_eq!(Lang::from_locale("fr"), Lang::Fr);
        assert_eq!(Lang::from_locale("fr-CA"), Lang::Fr);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("de"), Lang::En, "langue inconnue : anglais");
        assert_eq!(Lang::from_config("EN"), Lang::En);
        assert_eq!(Lang::from_config(""), Lang::Fr);
        assert_eq!(Event::Crash.title(Lang::En, "Survie"), "🔴 Survie crashed");
        assert_eq!(Event::Crash.title(Lang::Fr, "Survie"), "🔴 Survie a planté");
    }

    #[tokio::test]
    async fn notifications_respect_toggles_language_and_target() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("notify").await;
        let mut cfg = base_cfg();
        cfg.notify_backup = false;
        let sid = Uuid::new_v4();
        save_config(&dir, &cfg).await.unwrap();

        notify_at(&state.http, &dir, &rest, Event::Crash, None, "Survie", "boom", "boom-en").await;
        notify_at(&state.http, &dir, &rest, Event::Backup, None, "Survie", "coupé", "off").await; // coupé : rien
        notify_at(&state.http, &dir, &rest, Event::PlayerJoin, None, "Survie", "Alex", "Alex").await; // off par défaut
        let sent = rec.find("/channels/C1/messages");
        assert_eq!(sent.len(), 1, "seul le crash doit partir : {:?}", rec.all());
        assert!(sent[0]["embeds"][0]["title"].as_str().unwrap().contains("a planté"));
        assert_eq!(sent[0]["embeds"][0]["description"], "boom");
        assert_eq!(sent[0]["embeds"][0]["color"], 0xF38BA8);
        assert_eq!(sent[0]["allowed_mentions"]["parse"], json!([]), "aucun ping possible");

        // Langue anglaise.
        cfg.language = "en".into();
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Crash, None, "Survie", "boom", "boom-en").await;
        let sent = rec.find("/channels/C1/messages");
        assert!(sent[1]["embeds"][0]["title"].as_str().unwrap().contains("crashed"));
        assert_eq!(sent[1]["embeds"][0]["description"], "boom-en");

        // Salon de statut dédié au serveur : les notifications de CE serveur y vont.
        cfg.servers.insert(sid, ServerChannels { status_channel_id: "STATUS9".into(), ..Default::default() });
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Ready, Some(sid), "Survie", "ok", "ok").await;
        assert_eq!(rec.find("/channels/STATUS9/messages").len(), 1);
        notify_at(&state.http, &dir, &rest, Event::Ready, Some(Uuid::new_v4()), "Autre", "ok", "ok").await;
        assert_eq!(rec.find("/channels/C1/messages").len(), 3, "un autre serveur retombe sur le salon global");

        // Désactivé globalement : plus rien.
        cfg.enabled = false;
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Crash, Some(sid), "Survie", "boom", "boom").await;
        assert_eq!(rec.find("/channels/STATUS9/messages").len(), 1);
        assert_eq!(rec.find("/channels/C1/messages").len(), 3);

        // Sans bot : webhook.
        cfg.enabled = true;
        cfg.bot_token.clear();
        cfg.webhook_url = format!("{rest}/webhooks/42/tok");
        save_config(&dir, &cfg).await.unwrap();
        notify_at(&state.http, &dir, &rest, Event::Ready, None, "Survie", "ok", "ok").await;
        assert_eq!(rec.find("/webhooks/42/tok").len(), 1);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn rest_retries_once_after_rate_limit() {
        use axum::response::IntoResponse;
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let h2 = hits.clone();
        let app = axum::Router::new().fallback(move || {
            let n = h2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if n == 0 {
                    (axum::http::StatusCode::TOO_MANY_REQUESTS, axum::Json(json!({ "retry_after": 0.1 }))).into_response()
                } else {
                    axum::Json(json!({ "ok": true })).into_response()
                }
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.ok(); });
        let v = rest(&reqwest::Client::new(), &base, "t", reqwest::Method::GET, "/x", None).await.unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn interactions_enforce_guild_roles_and_answer_in_the_users_language() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("inter").await;
        let srv_dir = dir.join("srv");
        tokio::fs::create_dir_all(&srv_dir).await.unwrap();
        let sid = Uuid::new_v4();
        state.servers.write().await.insert(sid, sample_server(sid, "Survie", &srv_dir));
        save_config(&dir, &base_cfg()).await.unwrap();

        let last = |rec: &Recorder| -> (u64, String, Option<u64>) {
            let cb = rec.find("/interactions/I1/TOK1/callback").pop().expect("callback");
            (cb["type"].as_u64().unwrap(), cb["data"]["content"].as_str().unwrap_or("").to_string(), cb["data"]["flags"].as_u64())
        };
        let sub = |name: &str, extra: Value| json!([{ "name": name, "type": 1, "options": extra }]);

        // Mauvaise guilde : refusée (langue du client).
        handle_interaction(state.clone(), rest.clone(), interaction(2, "AUTRE", "U1", &[], "fr", sub("list", json!([])))).await;
        let (_, content, flags) = last(&rec);
        assert!(content.contains("n'est pas autorisé") && flags == Some(64));
        handle_interaction(state.clone(), rest.clone(), interaction(2, "AUTRE", "U1", &[], "en-US", sub("list", json!([])))).await;
        assert!(last(&rec).1.contains("is not allowed"));

        // /mc list : ouvert à tout membre, en français puis en anglais.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], "fr", sub("list", json!([])))).await;
        let (t, content, flags) = last(&rec);
        assert_eq!(t, 4);
        assert!(content.contains("Survie") && content.contains("⚫"));
        assert_eq!(flags, None, "la liste est visible de tous");
        let opts = json!([{ "name": "server", "type": 3, "value": sid.to_string() }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], "fr", sub("status", opts.clone()))).await;
        assert!(last(&rec).1.contains("est arrêté"));
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], "en-GB", sub("status", opts.clone()))).await;
        assert!(last(&rec).1.contains("is stopped"));

        // /mc start par un non-opérateur : refusé, et le serveur n'est PAS démarré.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], "en", sub("start", opts.clone()))).await;
        let (_, content, flags) = last(&rec);
        assert!(content.contains("reserved to operators") && flags == Some(64));
        assert!(!state.runtime.read().await.get(&sid).map(|r| r.running).unwrap_or(false));

        // /mc stop par un opérateur sur un serveur déjà arrêté : message clair.
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], "fr", sub("stop", opts.clone()))).await;
        assert!(last(&rec).1.contains("déjà arrêté"));

        // /mc command : autorisée pour l'opérateur mais échoue proprement (serveur non lancé).
        let cmd_opts = json!([{ "name": "server", "type": 3, "value": "survie" }, { "name": "text", "type": 3, "value": "say bonjour" }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], "fr", sub("command", cmd_opts))).await;
        assert!(last(&rec).1.starts_with("❌"));
        let evil = json!([{ "name": "server", "type": 3, "value": "survie" }, { "name": "text", "type": 3, "value": "say a\nstop" }]);
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U1", &[], "fr", sub("command", evil))).await;
        let (_, content, flags) = last(&rec);
        assert!(content.contains("invalide") && flags == Some(64));

        // Autocomplétion : type 8 avec le serveur.
        let ac = json!([{ "name": "status", "type": 1, "options": [{ "name": "server", "type": 3, "value": "sur", "focused": true }] }]);
        handle_interaction(state.clone(), rest.clone(), interaction(4, "G1", "U9", &[], "fr", ac)).await;
        let cb = rec.find("/interactions/I1/TOK1/callback").pop().unwrap();
        assert_eq!(cb["type"], 8);
        assert_eq!(cb["data"]["choices"][0]["value"], sid.to_string());

        // Rôle de lecture requis : un inconnu n'obtient rien.
        let mut cfg = base_cfg();
        cfg.viewer_role_ids = vec!["R_VIEW".into()];
        save_config(&dir, &cfg).await.unwrap();
        handle_interaction(state.clone(), rest.clone(), interaction(2, "G1", "U9", &[], "fr", sub("list", json!([])))).await;
        let (_, content, flags) = last(&rec);
        assert!(content.contains("pas la permission") && flags == Some(64));
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    /// Bout en bout contre un faux Discord (passerelle WebSocket + REST).
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

        let gw_url_for_ready = gw_url.clone();
        tokio::spawn(async move {
            // Connexion 0 : la passerelle refuse l'intention privilégiée (4014) -> le bot doit se reconnecter sans elle.
            {
                let (sock, _) = gw.accept().await.unwrap();
                let ws = tokio_tungstenite_discord::accept_async(sock).await.unwrap();
                let (mut tx, mut rx) = ws.split();
                tx.send(WsMessage::Text(json!({ "op": 10, "d": { "heartbeat_interval": 200 } }).to_string())).await.unwrap();
                let first: Value = match rx.next().await.unwrap().unwrap() { WsMessage::Text(t) => serde_json::from_str(&t).unwrap(), m => panic!("{m:?}") };
                assert_eq!(first["op"], 2);
                let intents = first["d"]["intents"].as_u64().unwrap();
                assert_ne!(intents & (1 << 15), 0, "première tentative : avec la lecture du contenu des messages");
                report_tx.send("identify-full".into()).unwrap();
                tx.send(WsMessage::Close(Some(tokio_tungstenite_discord::tungstenite::protocol::CloseFrame {
                    code: tokio_tungstenite_discord::tungstenite::protocol::frame::coding::CloseCode::from(4014u16),
                    reason: "Disallowed intent(s)".into(),
                }))).await.ok();
            }
            for round in 0..2 {
                let (sock, _) = gw.accept().await.unwrap();
                let ws = tokio_tungstenite_discord::accept_async(sock).await.unwrap();
                let (mut tx, mut rx) = ws.split();
                tx.send(WsMessage::Text(json!({ "op": 10, "d": { "heartbeat_interval": 200 } }).to_string())).await.unwrap();
                let first: Value = match rx.next().await.unwrap().unwrap() { WsMessage::Text(t) => serde_json::from_str(&t).unwrap(), m => panic!("{m:?}") };
                if round == 0 {
                    assert_eq!(first["op"], 2, "nouvelle identification");
                    assert_eq!(first["d"]["token"], "tkn-secret");
                    assert_eq!(first["d"]["intents"].as_u64().unwrap() & (1 << 15), 0, "après 4014 : sans l'intention privilégiée");
                    report_tx.send("identify-basic".into()).unwrap();
                    tx.send(WsMessage::Text(json!({ "op": 0, "s": 1, "t": "READY", "d": {
                        "session_id": "SESS1", "resume_gateway_url": gw_url_for_ready,
                        "user": { "id": "BOT1", "username": "McBot" }, "application": { "id": "APP1" } } }).to_string())).await.unwrap();
                    loop {
                        if let WsMessage::Text(t) = rx.next().await.unwrap().unwrap() {
                            let v: Value = serde_json::from_str(&t).unwrap();
                            if v["op"] == 1 { report_tx.send("heartbeat".into()).unwrap(); tx.send(WsMessage::Text(json!({ "op": 11 }).to_string())).await.unwrap(); break; }
                        }
                    }
                    tx.send(WsMessage::Text(json!({ "op": 0, "s": 2, "t": "INTERACTION_CREATE", "d": interaction(2, "G1", "U9", &[], "en", json!([{ "name": "list", "type": 1 }])) }).to_string())).await.unwrap();
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
        for _ in 0..4 {
            seen.push(tokio::time::timeout(Duration::from_secs(20), report_rx.recv()).await.expect("évènement de passerelle attendu").unwrap());
        }
        assert_eq!(seen, vec!["identify-full", "identify-basic", "heartbeat", "resume"]);

        let reg = rec.all().into_iter().find(|(m, p, _)| m == "PUT" && p == "/applications/APP1/guilds/G1/commands").expect("enregistrement des commandes");
        assert_eq!(reg.2[0]["name"], "mc");
        let cb = rec.find("/interactions/I1/TOK1/callback");
        assert!(!cb.is_empty() && cb[0]["data"]["content"].as_str().unwrap().contains("Survie"));
        for _ in 0..30 {
            if status().state == "connected" { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let st = status();
        assert_eq!(st.state, "connected");
        assert_eq!(st.bot_name.as_deref(), Some("McBot"));
        assert_eq!(st.bot_user_id.as_deref(), Some("BOT1"));
        assert!(st.message_content_missing, "l'interface doit pouvoir expliquer pourquoi console/chat sont indisponibles");
        task.abort();
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
