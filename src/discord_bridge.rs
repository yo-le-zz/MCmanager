//! Salons Discord par serveur Minecraft : catégorie + statut + console + chat.
//!
//! - **Console** : la sortie du serveur est diffusée en direct (par lots toutes les
//!   ~2 s, sous forme de blocs de code, pour respecter les limites de Discord) ;
//!   les messages tapés dans ce salon par les rôles autorisés sont envoyés comme
//!   commandes à la console du serveur.
//! - **Chat** : ce que les joueurs écrivent en jeu apparaît dans le salon, et ce
//!   qui est écrit dans le salon apparaît en jeu (`tellraw`).
//! - **Statut** : les notifications (démarrage, prêt, crash...) de ce serveur.
//! - **Rôles** : les salons sont créés invisibles pour tout le monde (`@everyone`)
//!   et visibles seulement pour les rôles choisis. Les droits d'écriture sont
//!   revérifiés côté MCManager à chaque message : ne pas se fier aux seules
//!   permissions de salon Discord pour autoriser une commande.
//!
//! Les serveurs d'instances distantes jumelées fonctionnent de la même façon
//! (la console arrive par le tunnel chiffré, les commandes repartent par lui).
//!
//! Lire le contenu des messages demande l'intention privilégiée « Message Content »
//! (portail développeur Discord). Sans elle, notifications et commandes slash
//! continuent de marcher ; seuls console et chat dans Discord sont indisponibles,
//! et l'interface l'indique.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::discord::{self, no_mentions, pick, rest, Caller, DiscordConfig, Lang, ServerChannels, ServerInfo};
use crate::state::AppState;

// Bits de permission Discord utilisés.
const MANAGE_CHANNELS: u64 = 1 << 4;
const ADD_REACTIONS: u64 = 1 << 6;
const VIEW_CHANNEL: u64 = 1 << 10;
const SEND_MESSAGES: u64 = 1 << 11;
const EMBED_LINKS: u64 = 1 << 14;
const READ_HISTORY: u64 = 1 << 16;
const MANAGE_ROLES: u64 = 1 << 28;

/// Permissions à donner au bot à l'invitation : de quoi créer les salons et leurs
/// droits, écrire, lire l'historique, réagir.
pub const INVITE_PERMISSIONS: u64 = MANAGE_CHANNELS | ADD_REACTIONS | VIEW_CHANNEL | SEND_MESSAGES | EMBED_LINKS | READ_HISTORY | MANAGE_ROLES;

pub fn invite_url(app_id: &str) -> String {
    format!("https://discord.com/oauth2/authorize?client_id={app_id}&scope=bot%20applications.commands&permissions={INVITE_PERMISSIONS}")
}

// ───────────────────────── fonctions pures (testées) ─────────────────────────

/// Retire les séquences d'échappement ANSI (couleurs de console) et les caractères de contrôle.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

/// Découpe des lignes de console en messages Discord (blocs de code de 1800 caractères au
/// plus, `max_messages` messages au plus pour tenir dans la limite de débit ; le surplus est
/// résumé en une ligne plutôt que de noyer le salon).
pub fn format_console_batch(lines: &[String], max_messages: usize, l: Lang) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut skipped = 0usize;
    for raw in lines {
        let line = truncate_chars(&strip_ansi(raw).replace("```", "'''"), 400);
        if line.trim().is_empty() {
            continue;
        }
        if blocks.len() >= max_messages {
            skipped += 1;
            continue;
        }
        if !current.is_empty() && current.len() + 1 + line.len() > 1800 {
            blocks.push(std::mem::take(&mut current));
            if blocks.len() >= max_messages {
                skipped += 1;
                continue;
            }
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&line);
    }
    if !current.is_empty() && blocks.len() < max_messages {
        blocks.push(current);
    }
    let mut out: Vec<String> = blocks.into_iter().map(|b| format!("```\n{b}\n```")).collect();
    if skipped > 0 {
        let note = pick(l, "… {n} ligne(s) ignorée(s) (trop de sortie à la fois)", "… {n} line(s) skipped (too much output at once)").replace("{n}", &skipped.to_string());
        match out.last_mut() {
            Some(last) => {
                last.push('\n');
                last.push_str(&note);
            }
            None => out.push(note),
        }
    }
    out
}

/// Extrait `(joueur, message)` d'une ligne de log de chat Minecraft :
/// `[12:34:56 INFO]: <Alex> bonjour` (Paper), `[12:34:56] [Server thread/INFO]: <Alex> bonjour`
/// (vanilla), avec ou sans le marqueur `[Not Secure]`. Les lignes de plugins, de commandes
/// ou de système ne ressemblent pas à `<nom> texte` et sont ignorées.
pub fn parse_chat(line: &str) -> Option<(String, String)> {
    let rest = line.split_once("]: ")?.1;
    let rest = rest.strip_prefix("[Not Secure] ").unwrap_or(rest);
    let rest = rest.strip_prefix('<')?;
    let (name, msg) = rest.split_once("> ")?;
    if name.is_empty() || name.len() > 32 || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '*')) {
        return None;
    }
    let msg = msg.trim();
    if msg.is_empty() {
        return None;
    }
    Some((name.to_string(), msg.to_string()))
}

/// Retire ce qui n'a rien à faire dans une ligne de chat : caractères de contrôle (une
/// nouvelle ligne enverrait une *seconde* commande à la console), `§` (codes de couleur
/// forgés), espaces multiples ; limite la longueur.
pub fn sanitize_chat(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).filter(|c| !c.is_control() && *c != '§').collect();
    truncate_chars(&cleaned.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// Commande `tellraw` affichant un message Discord à tous les joueurs. Le texte est placé
/// dans un composant JSON construit par `serde_json` (guillemets, antislashs et unicode
/// échappés correctement) : le contenu d'un message ne peut ni casser le JSON ni injecter
/// une autre commande.
pub fn build_tellraw(author: &str, message: &str) -> Option<String> {
    let msg = sanitize_chat(message, 240);
    if msg.is_empty() {
        return None;
    }
    let name = sanitize_chat(author, 32);
    let name = if name.is_empty() { "Discord".to_string() } else { name };
    let component = json!([
        { "text": "[Discord] ", "color": "blue" },
        { "text": format!("<{name}> "), "color": "gray" },
        { "text": msg }
    ]);
    Some(format!("tellraw @a {component}"))
}

/// Désamorce les mentions dans un texte venu du jeu (en plus de `allowed_mentions` vide).
fn defang_mentions(s: &str) -> String {
    s.replace('@', "@\u{200B}")
}

// ───────────────────────── droits des salons ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Category,
    Status,
    Console,
    Chat,
}

fn ow(id: &str, kind: u8, allow: u64, deny: u64) -> Value {
    json!({ "id": id, "type": kind, "allow": allow.to_string(), "deny": deny.to_string() })
}

/// Permissions d'un salon : invisible pour `@everyone`, visible/écrivable selon le rôle.
fn overwrites(kind: Kind, guild_id: &str, bot_id: &str, cfg: &DiscordConfig, command_roles: &[String]) -> Vec<Value> {
    let mut roles: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut give = |ids: &[String], allow: u64, deny: u64| {
        for id in ids {
            roles.insert(id.clone(), (allow, deny));
        }
    };
    let view = &cfg.console_view_role_ids;
    let chat = &cfg.chat_role_ids;
    let ops = &cfg.operator_role_ids;
    match kind {
        Kind::Category => {
            for ids in [view, chat, ops, command_roles] {
                give(ids, VIEW_CHANNEL, 0);
            }
        }
        Kind::Status => {
            for ids in [view, chat, ops, command_roles] {
                give(ids, VIEW_CHANNEL | READ_HISTORY, SEND_MESSAGES);
            }
        }
        Kind::Console => {
            give(view, VIEW_CHANNEL | READ_HISTORY, SEND_MESSAGES);
            // Plus fort que « lecture seule » si un rôle est dans les deux listes.
            give(ops, VIEW_CHANNEL | READ_HISTORY | SEND_MESSAGES, 0);
            give(command_roles, VIEW_CHANNEL | READ_HISTORY | SEND_MESSAGES, 0);
        }
        Kind::Chat => {
            for ids in [chat, ops, command_roles] {
                give(ids, VIEW_CHANNEL | READ_HISTORY | SEND_MESSAGES, 0);
            }
        }
    }
    let mut out = vec![ow(guild_id, 0, 0, VIEW_CHANNEL)];
    out.push(ow(bot_id, 1, VIEW_CHANNEL | SEND_MESSAGES | READ_HISTORY | ADD_REACTIONS | EMBED_LINKS | MANAGE_CHANNELS, 0));
    for (id, (allow, deny)) in roles {
        if id != guild_id {
            out.push(ow(&id, 0, allow, deny));
        }
    }
    out
}

// ───────────────────────── détection (menus plutôt que des identifiants à recopier) ─────────────────────────

pub async fn detect_guilds(http: &reqwest::Client, api_base: &str, token: &str) -> anyhow::Result<Vec<Value>> {
    let v = rest(http, api_base, token, reqwest::Method::GET, "/users/@me/guilds", None).await?;
    Ok(v.as_array().cloned().unwrap_or_default().into_iter().map(|g| json!({ "id": g["id"], "name": g["name"] })).collect())
}

/// Rôles (sans `@everyone` ni les rôles gérés par des bots) et salons textuels d'un serveur Discord.
pub async fn detect_guild(http: &reqwest::Client, api_base: &str, token: &str, guild_id: &str) -> anyhow::Result<Value> {
    let roles = rest(http, api_base, token, reqwest::Method::GET, &format!("/guilds/{guild_id}/roles"), None).await?;
    let channels = rest(http, api_base, token, reqwest::Method::GET, &format!("/guilds/{guild_id}/channels"), None).await?;
    let mut roles: Vec<Value> = roles
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r["id"].as_str() != Some(guild_id) && r["managed"] != true)
        .collect();
    roles.sort_by_key(|r| std::cmp::Reverse(r["position"].as_i64().unwrap_or(0)));
    let roles: Vec<Value> = roles.into_iter().map(|r| json!({ "id": r["id"], "name": r["name"], "color": format!("#{:06x}", r["color"].as_u64().unwrap_or(0)) })).collect();
    let text_channels: Vec<Value> = channels
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| matches!(c["type"].as_u64(), Some(0) | Some(5)))
        .map(|c| json!({ "id": c["id"], "name": c["name"], "parent_id": c["parent_id"] }))
        .collect();
    Ok(json!({ "roles": roles, "channels": text_channels }))
}

pub async fn application_id(http: &reqwest::Client, api_base: &str, token: &str) -> anyhow::Result<String> {
    let v = rest(http, api_base, token, reqwest::Method::GET, "/oauth2/applications/@me", None).await?;
    v["id"].as_str().map(|s| s.to_string()).ok_or_else(|| anyhow::anyhow!("identifiant d'application introuvable"))
}

// ───────────────────────── création / suppression des salons ─────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SetupOptions {
    #[serde(default)]
    pub stream_console: bool,
    #[serde(default)]
    pub bridge_chat: bool,
    /// Rôles pouvant envoyer des commandes dans la console de CE serveur (en plus des opérateurs globaux).
    #[serde(default)]
    pub command_role_ids: Vec<String>,
}

async fn ensure_channel(http: &reqwest::Client, api_base: &str, token: &str, guild_id: &str, existing: &str, create_body: Value) -> anyhow::Result<String> {
    if !existing.is_empty() {
        // Salon déjà créé : on remet à jour droits/emplacement. S'il a été supprimé à la main sur Discord, on le recrée.
        let mut patch = json!({ "permission_overwrites": create_body["permission_overwrites"] });
        if let Some(p) = create_body.get("parent_id") {
            patch["parent_id"] = p.clone();
        }
        if rest(http, api_base, token, reqwest::Method::PATCH, &format!("/channels/{existing}"), Some(&patch)).await.is_ok() {
            return Ok(existing.to_string());
        }
    }
    let created = rest(http, api_base, token, reqwest::Method::POST, &format!("/guilds/{guild_id}/channels"), Some(&create_body)).await?;
    created["id"].as_str().map(|s| s.to_string()).ok_or_else(|| anyhow::anyhow!("Discord n'a pas renvoyé d'identifiant de salon"))
}

/// Crée (ou remet à jour) la catégorie et les salons d'un serveur, et enregistre leurs identifiants.
pub async fn setup_server_channels(http: &reqwest::Client, api_base: &str, cfg: &mut DiscordConfig, id: Uuid, server_name: &str, opts: &SetupOptions) -> anyhow::Result<ServerChannels> {
    if cfg.bot_token.is_empty() || cfg.guild_id.is_empty() {
        anyhow::bail!("renseignez d'abord le token du bot et le serveur Discord");
    }
    let token = cfg.bot_token.clone();
    let guild = cfg.guild_id.clone();
    let bot_id = match discord::status().bot_user_id {
        Some(b) => b,
        None => rest(http, api_base, &token, reqwest::Method::GET, "/users/@me", None).await?["id"].as_str().unwrap_or_default().to_string(),
    };
    if bot_id.is_empty() {
        anyhow::bail!("identité du bot introuvable (token invalide ?)");
    }
    let mut sc = cfg.servers.get(&id).cloned().unwrap_or_default();
    sc.stream_console = opts.stream_console;
    sc.bridge_chat = opts.bridge_chat;
    sc.console_command_role_ids = opts.command_role_ids.clone();
    let cmd_roles = sc.console_command_role_ids.clone();

    let label = truncate_chars(&server_name.replace(['\n', '\r'], " "), 80);
    sc.category_id = ensure_channel(http, api_base, &token, &guild, &sc.category_id, json!({
        "name": format!("🎮 {label}"), "type": 4, "permission_overwrites": overwrites(Kind::Category, &guild, &bot_id, cfg, &cmd_roles),
    })).await?;
    let child = |name: &str, topic: &str, kind: Kind, parent: &str| json!({
        "name": name, "type": 0, "topic": topic, "parent_id": parent,
        "permission_overwrites": overwrites(kind, &guild, &bot_id, cfg, &cmd_roles),
    });
    sc.status_channel_id = ensure_channel(http, api_base, &token, &guild, &sc.status_channel_id, child("status", &format!("{label} — état du serveur (MCManager)"), Kind::Status, &sc.category_id)).await?;
    sc.console_channel_id = ensure_channel(http, api_base, &token, &guild, &sc.console_channel_id, child("console", &format!("{label} — console en direct ; les rôles autorisés peuvent taper des commandes"), Kind::Console, &sc.category_id)).await?;
    sc.chat_channel_id = ensure_channel(http, api_base, &token, &guild, &sc.chat_channel_id, child("chat", &format!("{label} — chat relié au jeu"), Kind::Chat, &sc.category_id)).await?;

    cfg.servers.insert(id, sc.clone());
    Ok(sc)
}

/// Retire le lien avec les salons ; `delete_channels` supprime aussi la catégorie et les salons sur Discord.
pub async fn teardown_server_channels(http: &reqwest::Client, api_base: &str, cfg: &mut DiscordConfig, id: Uuid, delete_channels: bool) -> anyhow::Result<()> {
    if let Some(sc) = cfg.servers.remove(&id) {
        if delete_channels && !cfg.bot_token.is_empty() {
            for ch in [&sc.status_channel_id, &sc.console_channel_id, &sc.chat_channel_id, &sc.category_id] {
                if !ch.is_empty() {
                    // Salon déjà supprimé à la main : sans importance.
                    let _ = rest(http, api_base, &cfg.bot_token, reqwest::Method::DELETE, &format!("/channels/{ch}"), None).await;
                }
            }
        }
    }
    Ok(())
}

// ───────────────────────── messages entrants (console / chat) ─────────────────────────

async fn react(http: &reqwest::Client, api_base: &str, token: &str, channel: &str, message: &str, emoji_encoded: &str) {
    let _ = rest(http, api_base, token, reqwest::Method::PUT, &format!("/channels/{channel}/messages/{message}/reactions/{emoji_encoded}/@me"), None).await;
}

const EMOJI_OK: &str = "%E2%9C%85"; // ✅
const EMOJI_DENIED: &str = "%E2%9B%94"; // ⛔
const EMOJI_FAIL: &str = "%E2%9D%8C"; // ❌
const EMOJI_WARN: &str = "%E2%9A%A0%EF%B8%8F"; // ⚠️

async fn server_info(state: &AppState, id: Uuid) -> Option<ServerInfo> {
    if let Some(label) = state.hub.label_for_server(id).await {
        let name = crate::hub::remote_servers_json(state).await.into_iter().find(|s| s["id"].as_str() == Some(&id.to_string())).and_then(|s| s["name"].as_str().map(|n| n.to_string())).unwrap_or_else(|| "?".into());
        return Some(ServerInfo { id, name, running: true, remote: Some(label) });
    }
    let name = state.servers.read().await.get(&id)?.name.clone();
    let running = state.runtime.read().await.get(&id).map(|r| r.running).unwrap_or(false);
    Some(ServerInfo { id, name, running, remote: None })
}

/// Évènement MESSAGE_CREATE : commande dans un salon console, ou message de chat à relayer en jeu.
pub async fn handle_message(state: AppState, api_base: String, msg: Value) {
    let cfg = discord::load_config(&state.data_dir).await;
    if !cfg.enabled || cfg.bot_token.is_empty() || msg["guild_id"].as_str() != Some(cfg.guild_id.as_str()) {
        return;
    }
    // Jamais de réaction aux bots, aux webhooks, ni à nous-mêmes (boucle infinie sinon).
    if msg["author"]["bot"] == true || msg.get("webhook_id").map(|w| !w.is_null()).unwrap_or(false) {
        return;
    }
    let (Some(channel), Some(message_id)) = (msg["channel_id"].as_str(), msg["id"].as_str()) else { return };
    let content = msg["content"].as_str().unwrap_or("").to_string();
    if content.trim().is_empty() {
        return; // pas d'intention « Message Content », ou message sans texte (image...)
    }
    let Some((&server_id, sc)) = cfg.servers.iter().find(|(_, sc)| (!sc.console_channel_id.is_empty() && sc.console_channel_id == channel) || (!sc.chat_channel_id.is_empty() && sc.chat_channel_id == channel)) else { return };
    let Some(caller) = Caller::from_message(&msg) else { return };
    let http = state.http.clone();
    let token = cfg.bot_token.clone();

    if sc.console_channel_id == channel {
        let allowed = caller.is_operator(&cfg) || caller.roles.iter().any(|r| sc.console_command_role_ids.contains(r));
        if !allowed {
            react(&http, &api_base, &token, channel, message_id, EMOJI_DENIED).await;
            return;
        }
        let command = content.trim().trim_start_matches('/').to_string();
        if command.is_empty() || command.contains('\n') || command.contains('\r') {
            react(&http, &api_base, &token, channel, message_id, EMOJI_WARN).await;
            return;
        }
        let Some(server) = server_info(&state, server_id).await else { return };
        tracing::info!("[discord] {} ({}) : console de {} : « {command} »", caller.user_name, caller.user_id, server.name);
        match discord::send_console_command(&state, &server, &command).await {
            Ok(()) => react(&http, &api_base, &token, channel, message_id, EMOJI_OK).await,
            Err(e) => {
                tracing::debug!("[discord] commande console refusée : {e}");
                react(&http, &api_base, &token, channel, message_id, EMOJI_FAIL).await;
            }
        }
        return;
    }

    // Salon de chat : relayé en jeu si le pont est actif.
    if sc.bridge_chat {
        let display = msg["member"]["nick"].as_str().or_else(|| msg["author"]["global_name"].as_str()).or_else(|| msg["author"]["username"].as_str()).unwrap_or("Discord");
        let Some(cmd) = build_tellraw(display, &content) else { return };
        let Some(server) = server_info(&state, server_id).await else { return };
        // Serveur éteint : rien à faire, le message reste simplement dans Discord.
        let _ = discord::send_console_command(&state, &server, &cmd).await;
    }
}

// ───────────────────────── diffusion de la console ─────────────────────────

/// Envoie un lot de lignes de console : chat vers le salon chat, tout vers le salon console.
pub async fn process_lines(state: &AppState, api_base: &str, id: Uuid, lines: &[String]) {
    let cfg = discord::load_config(&state.data_dir).await;
    let Some(sc) = cfg.servers.get(&id) else { return };
    if !cfg.enabled || cfg.bot_token.is_empty() {
        return;
    }
    let l = Lang::from_config(&cfg.language);
    let http = &state.http;
    let post = |channel: String, content: String| {
        let (http, token, base) = (http.clone(), cfg.bot_token.clone(), api_base.to_string());
        async move {
            let body = json!({ "content": content.chars().take(1990).collect::<String>(), "allowed_mentions": no_mentions() });
            if let Err(e) = rest(&http, &base, &token, reqwest::Method::POST, &format!("/channels/{channel}/messages"), Some(&body)).await {
                tracing::debug!("[discord] envoi vers le salon impossible : {e}");
            }
        }
    };
    if sc.bridge_chat && !sc.chat_channel_id.is_empty() {
        for line in lines {
            if let Some((name, msg)) = parse_chat(&strip_ansi(line)) {
                post(sc.chat_channel_id.clone(), format!("**{}** » {}", defang_mentions(&name), defang_mentions(&msg))).await;
            }
        }
    }
    if sc.stream_console && !sc.console_channel_id.is_empty() {
        for block in format_console_batch(lines, 3, l) {
            post(sc.console_channel_id.clone(), block).await;
        }
    }
}

pub fn spawn_pumps(state: AppState, api_base: String) {
    tokio::spawn(async move {
        let mut tasks: HashMap<Uuid, tokio::task::JoinHandle<()>> = HashMap::new();
        loop {
            let cfg = discord::load_config(&state.data_dir).await;
            let mut wanted: HashSet<Uuid> = HashSet::new();
            if cfg.enabled && !cfg.bot_token.is_empty() {
                for (id, sc) in &cfg.servers {
                    if (sc.stream_console && !sc.console_channel_id.is_empty()) || (sc.bridge_chat && !sc.chat_channel_id.is_empty()) {
                        wanted.insert(*id);
                    }
                }
            }
            tasks.retain(|id, handle| {
                if !wanted.contains(id) {
                    handle.abort();
                    false
                } else {
                    !handle.is_finished()
                }
            });
            for id in wanted {
                tasks.entry(id).or_insert_with(|| tokio::spawn(pump_server(state.clone(), api_base.clone(), id)));
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// Écoute la console d'UN serveur (local ou distant) et la transmet par lots toutes les 2 s.
async fn pump_server(state: AppState, api_base: String, id: Uuid) {
    loop {
        let mut rx = if let Some(label) = state.hub.label_for_server(id).await {
            crate::hub::console_subscribe(&state, id, &label).await.1
        } else {
            // Le canal de console n'existe qu'une fois le serveur démarré une première fois.
            loop {
                if let Some(r) = state.runtime.read().await.get(&id) {
                    break r.tx.subscribe();
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        };
        let mut buffer: Vec<String> = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                line = rx.recv() => match line {
                    Ok(l) => { buffer.push(l); if buffer.len() > 2000 { buffer.drain(0..1000); } }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                _ = tick.tick() => {
                    if !buffer.is_empty() {
                        let lines = std::mem::take(&mut buffer);
                        process_lines(&state, &api_base, id, &lines).await;
                    }
                }
            }
        }
        if !buffer.is_empty() {
            process_lines(&state, &api_base, id, &buffer).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[32mHello\u{1b}[0m world\u{1b}[1;31m!"), "Hello world!");
        assert_eq!(strip_ansi("a\u{7}b"), "ab");
    }

    #[test]
    fn console_batches_respect_limits() {
        let l = Lang::En;
        let lines: Vec<String> = (0..5).map(|i| format!("[12:00:0{i} INFO]: line {i}")).collect();
        let out = format_console_batch(&lines, 3, l);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("```\n") && out[0].ends_with("\n```"));
        assert!(out[0].contains("line 4"));
        // Un backtick triple dans la sortie ne referme pas le bloc de code.
        let evil = vec!["``` @everyone".to_string()];
        let out = format_console_batch(&evil, 3, l);
        assert_eq!(out[0].matches("```").count(), 2);
        // Beaucoup de sortie : au plus 3 messages, le reste est résumé.
        let many: Vec<String> = (0..400).map(|i| format!("{i:04} {}", "x".repeat(60))).collect();
        let out = format_console_batch(&many, 3, l);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|m| m.len() < 1990), "chaque message tient dans la limite de Discord");
        assert!(out.last().unwrap().contains("skipped"));
        assert!(format_console_batch(&["   ".to_string()], 3, l).is_empty());
    }

    #[test]
    fn chat_lines_are_recognised() {
        assert_eq!(parse_chat("[12:34:56 INFO]: <Alex> bonjour tout le monde"), Some(("Alex".into(), "bonjour tout le monde".into())));
        assert_eq!(parse_chat("[12:34:56] [Server thread/INFO]: <Steve_99> salut"), Some(("Steve_99".into(), "salut".into())));
        assert_eq!(parse_chat("[12:34:56 INFO]: [Not Secure] <Alex> signé"), Some(("Alex".into(), "signé".into())));
        assert_eq!(parse_chat("[12:34:56 INFO]: <.BedrockGuy> hi"), Some((".BedrockGuy".into(), "hi".into())));
        for not_chat in [
            "[12:34:56 INFO]: Alex joined the game",
            "[12:34:56 INFO]: [Server] annonce",
            "[12:34:56 INFO]: [Discord] <Bob> relayé",
            "> say hello",
            "[12:34:56 INFO]: <> vide",
            "[12:34:56 INFO]: <Alex>",
            "[12:34:56 INFO]: <bad name!> x",
            "Done (3.2s)! For help, type \"help\"",
        ] {
            assert_eq!(parse_chat(not_chat), None, "{not_chat}");
        }
    }

    #[test]
    fn tellraw_cannot_be_broken_or_used_for_injection() {
        let cmd = build_tellraw("Al\"ex", "hello \"world\" \\ end").unwrap();
        assert!(cmd.starts_with("tellraw @a ["));
        // Le JSON produit est valide et le texte est intact.
        let json_part = cmd.trim_start_matches("tellraw @a ");
        let v: Value = serde_json::from_str(json_part).expect("JSON valide");
        assert_eq!(v[2]["text"], "hello \"world\" \\ end");
        assert_eq!(v[1]["text"], "<Al\"ex> ");
        // Une nouvelle ligne enverrait une seconde commande à la console : elle est neutralisée.
        let cmd = build_tellraw("x", "salut\nstop").unwrap();
        assert!(!cmd.contains('\n') && !cmd.contains('\r'));
        let v: Value = serde_json::from_str(cmd.trim_start_matches("tellraw @a ")).unwrap();
        assert_eq!(v[2]["text"], "salut stop");
        // Codes de formatage forgés et contrôles supprimés ; longueur bornée ; vide refusé.
        let v: Value = serde_json::from_str(build_tellraw("x", "§4rouge\u{0007}").unwrap().trim_start_matches("tellraw @a ")).unwrap();
        assert_eq!(v[2]["text"], "4rouge");
        let long = build_tellraw("x", &"a".repeat(1000)).unwrap();
        assert!(long.len() < 500);
        assert_eq!(build_tellraw("x", "  \n\t "), None);
        assert_eq!(build_tellraw("x", "§§"), None);
    }

    #[test]
    fn mentions_are_defanged() {
        assert!(!defang_mentions("@everyone @here <@123>").contains("@e") && !defang_mentions("@everyone").starts_with("@e"));
    }

    fn cfg_with_roles() -> DiscordConfig {
        DiscordConfig {
            console_view_role_ids: vec!["R_VIEW".into(), "R_BOTH".into()],
            chat_role_ids: vec!["R_CHAT".into()],
            operator_role_ids: vec!["R_OP".into()],
            ..Default::default()
        }
    }

    fn find<'a>(list: &'a [Value], id: &str) -> Option<&'a Value> {
        list.iter().find(|o| o["id"] == id)
    }

    #[test]
    fn channel_permissions_are_private_by_default() {
        let cfg = cfg_with_roles();
        let cmd = vec!["R_BOTH".to_string(), "R_CMD".to_string()];
        for kind in [Kind::Category, Kind::Status, Kind::Console, Kind::Chat] {
            let o = overwrites(kind, "G1", "BOT", &cfg, &cmd);
            let everyone = find(&o, "G1").unwrap();
            assert_eq!(everyone["deny"], VIEW_CHANNEL.to_string(), "@everyone ne voit rien ({kind:?})");
            assert_eq!(everyone["allow"], "0");
            assert!(find(&o, "BOT").is_some(), "le bot garde l'accès");
        }
        // Console : lecture seule pour R_VIEW, écriture pour R_OP / R_CMD, et R_BOTH (dans les deux listes) écrit.
        let o = overwrites(Kind::Console, "G1", "BOT", &cfg, &cmd);
        let allow = |id: &str| find(&o, id).unwrap()["allow"].as_str().unwrap().parse::<u64>().unwrap();
        let deny = |id: &str| find(&o, id).unwrap()["deny"].as_str().unwrap().parse::<u64>().unwrap();
        assert_eq!(allow("R_VIEW") & SEND_MESSAGES, 0);
        assert_eq!(deny("R_VIEW") & SEND_MESSAGES, SEND_MESSAGES, "lecture seule = écriture explicitement interdite");
        assert_ne!(allow("R_OP") & SEND_MESSAGES, 0);
        assert_ne!(allow("R_CMD") & SEND_MESSAGES, 0);
        assert_ne!(allow("R_BOTH") & SEND_MESSAGES, 0);
        assert!(find(&o, "R_CHAT").is_none(), "le rôle chat ne voit pas la console");
        // Chat : R_CHAT écrit, R_VIEW ne le voit pas.
        let o = overwrites(Kind::Chat, "G1", "BOT", &cfg, &cmd);
        assert!(find(&o, "R_CHAT").is_some() && find(&o, "R_VIEW").is_none());
        // Statut : personne n'écrit.
        let o = overwrites(Kind::Status, "G1", "BOT", &cfg, &cmd);
        assert_eq!(find(&o, "R_VIEW").unwrap()["deny"], SEND_MESSAGES.to_string());
    }

    #[test]
    fn invite_link_requests_needed_permissions() {
        let u = invite_url("APP1");
        assert!(u.contains("client_id=APP1") && u.contains("applications.commands"));
        assert!(u.contains(&format!("permissions={INVITE_PERMISSIONS}")));
        assert_ne!(INVITE_PERMISSIONS & MANAGE_CHANNELS, 0);
        assert_eq!(INVITE_PERMISSIONS & (1 << 3), 0, "jamais Administrateur");
    }

    use crate::discord::testutil::*;

    fn bridge_cfg() -> DiscordConfig {
        let mut cfg = base_cfg();
        cfg.operator_role_ids = vec!["R_OP".into()];
        cfg.console_view_role_ids = vec!["R_VIEW".into()];
        cfg.chat_role_ids = vec!["R_CHAT".into()];
        cfg
    }

    #[tokio::test]
    async fn setup_creates_private_channels_and_survives_manual_deletion() {
        let (rest, rec) = mock_rest().await;
        let http = reqwest::Client::new();
        let mut cfg = bridge_cfg();
        let id = Uuid::new_v4();
        let opts = SetupOptions { stream_console: true, bridge_chat: true, command_role_ids: vec!["R_CMD".into()] };

        let sc = setup_server_channels(&http, &rest, &mut cfg, id, "Survie", &opts).await.unwrap();
        assert_eq!(rec.count("POST", "/guilds/G1/channels"), 4, "catégorie + statut + console + chat");
        let ids = [&sc.category_id, &sc.status_channel_id, &sc.console_channel_id, &sc.chat_channel_id];
        assert!(ids.iter().all(|i| !i.is_empty()) && ids.iter().collect::<HashSet<_>>().len() == 4);
        assert!(sc.stream_console && sc.bridge_chat);
        assert_eq!(sc.console_command_role_ids, vec!["R_CMD".to_string()]);
        let bodies = rec.find("/guilds/G1/channels");
        let creates: Vec<&Value> = bodies.iter().filter(|b| b.get("name").is_some()).collect();
        assert_eq!(creates[0]["type"], 4);
        assert_eq!(creates[0]["name"], "🎮 Survie");
        for child in &creates[1..] {
            assert_eq!(child["type"], 0);
            assert_eq!(child["parent_id"], sc.category_id.as_str(), "rangés dans la catégorie du serveur");
            let o = child["permission_overwrites"].as_array().unwrap();
            assert_eq!(find(o, "G1").unwrap()["deny"], VIEW_CHANNEL.to_string(), "invisibles pour @everyone");
        }
        assert_eq!(cfg.servers[&id].console_channel_id, sc.console_channel_id);

        // Relancer = mise à jour des droits, pas de doublons.
        let again = setup_server_channels(&http, &rest, &mut cfg, id, "Survie", &opts).await.unwrap();
        assert_eq!(rec.count("POST", "/guilds/G1/channels"), 4, "aucun nouveau salon");
        assert_eq!(rec.count("PATCH", "/channels/"), 4);
        assert_eq!(again.chat_channel_id, sc.chat_channel_id);

        // Un salon supprimé à la main sur Discord est recréé.
        rec.missing.lock().unwrap().insert(sc.console_channel_id.clone());
        let healed = setup_server_channels(&http, &rest, &mut cfg, id, "Survie", &opts).await.unwrap();
        assert_eq!(rec.count("POST", "/guilds/G1/channels"), 5);
        assert_ne!(healed.console_channel_id, sc.console_channel_id);
        assert_eq!(healed.chat_channel_id, sc.chat_channel_id, "les autres salons sont conservés");

        // Retrait + suppression sur Discord.
        teardown_server_channels(&http, &rest, &mut cfg, id, true).await.unwrap();
        assert!(cfg.servers.is_empty());
        assert_eq!(rec.count("DELETE", "/channels/"), 4);
        // Sans token configuré : message clair.
        let mut empty = DiscordConfig::default();
        assert!(setup_server_channels(&http, &rest, &mut empty, id, "x", &opts).await.is_err());
    }

    #[tokio::test]
    async fn detection_gives_menus_instead_of_ids() {
        let (rest, _rec) = mock_rest().await;
        let http = reqwest::Client::new();
        let guilds = detect_guilds(&http, &rest, "tok").await.unwrap();
        assert_eq!(guilds, vec![json!({ "id": "G1", "name": "Test Guild" })]);
        let g = detect_guild(&http, &rest, "tok", "G1").await.unwrap();
        let roles = g["roles"].as_array().unwrap();
        let names: Vec<&str> = roles.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Admin", "Membre"], "pas @everyone, pas les rôles de bots, du plus haut au plus bas");
        assert_eq!(roles[0]["color"], "#e74c3c");
        let channels = g["channels"].as_array().unwrap();
        assert_eq!(channels.len(), 1, "salons textuels seulement");
        assert_eq!(channels[0]["name"], "general");
        assert_eq!(application_id(&http, &rest, "tok").await.unwrap(), "APP1");
    }

    fn message(channel: &str, author: &str, roles: &[&str], content: &str) -> Value {
        json!({
            "id": "M1", "channel_id": channel, "guild_id": "G1", "content": content,
            "author": { "id": author, "username": format!("u-{author}"), "global_name": "Alexandre" },
            "member": { "roles": roles, "nick": null },
        })
    }

    #[tokio::test]
    async fn console_channel_runs_commands_only_for_allowed_roles() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("console").await;
        let sid = Uuid::new_v4();
        state.servers.write().await.insert(sid, sample_server(sid, "Survie", &dir));
        let out = dir.join("stdin.txt");
        fake_running_server(&state, sid, &out).await;
        let mut cfg = bridge_cfg();
        cfg.servers.insert(sid, ServerChannels {
            console_channel_id: "CON1".into(), chat_channel_id: "CHAT1".into(), stream_console: true, bridge_chat: true,
            console_command_role_ids: vec!["R_CMD".into()], ..Default::default()
        });
        discord::save_config(&dir, &cfg).await.unwrap();
        let reactions = |rec: &Recorder| rec.all().into_iter().filter(|(m, p, _)| m == "PUT" && p.contains("/reactions/")).map(|(_, p, _)| p).collect::<Vec<_>>();
        let sent = || std::fs::read_to_string(&out).unwrap_or_default();
        let settle = || async { tokio::time::sleep(Duration::from_millis(250)).await };

        // Rôle autorisé (par serveur) : la commande part, ✅.
        handle_message(state.clone(), rest.clone(), message("CON1", "U5", &["R_CMD"], "/say bonjour")).await;
        settle().await;
        assert_eq!(sent(), "say bonjour\n");
        assert!(reactions(&rec).iter().any(|p| p.ends_with("/%E2%9C%85/@me")));
        // Opérateur global (rôle) : autorisé aussi.
        handle_message(state.clone(), rest.clone(), message("CON1", "U6", &["R_OP"], "list")).await;
        settle().await;
        assert!(sent().ends_with("list\n"));
        // Simple lecteur de la console : refusé (⛔), rien n'est envoyé.
        let before = sent();
        handle_message(state.clone(), rest.clone(), message("CON1", "U7", &["R_VIEW"], "op U7")).await;
        settle().await;
        assert_eq!(sent(), before, "un lecteur ne peut pas envoyer de commande");
        assert!(reactions(&rec).iter().any(|p| p.ends_with("/%E2%9B%94/@me")));
        // Plusieurs lignes : refusé (une seule commande par message), ⚠️.
        handle_message(state.clone(), rest.clone(), message("CON1", "U6", &["R_OP"], "say a\nstop")).await;
        settle().await;
        assert_eq!(sent(), before.clone() , "{}", sent());
        assert!(reactions(&rec).iter().any(|p| p.contains("%E2%9A%A0")));
        // Bot, webhook, mauvaise guilde, salon inconnu, message vide : ignorés.
        let mut bot = message("CON1", "U6", &["R_OP"], "stop");
        bot["author"]["bot"] = json!(true);
        let mut hook = message("CON1", "U6", &["R_OP"], "stop");
        hook["webhook_id"] = json!("W1");
        let mut other_guild = message("CON1", "U6", &["R_OP"], "stop");
        other_guild["guild_id"] = json!("G2");
        for m in [bot, hook, other_guild, message("AUTRE", "U6", &["R_OP"], "stop"), message("CON1", "U6", &["R_OP"], "   ")] {
            handle_message(state.clone(), rest.clone(), m).await;
        }
        settle().await;
        assert_eq!(sent(), before);

        // Chat : relayé en jeu via tellraw, avec le pseudo Discord ; tout le monde (le salon filtre déjà par rôle).
        handle_message(state.clone(), rest.clone(), message("CHAT1", "U8", &[], "salut \"les\" gens @everyone")).await;
        settle().await;
        let last = sent().lines().last().unwrap().to_string();
        assert!(last.starts_with("tellraw @a ["), "{last}");
        let v: Value = serde_json::from_str(last.trim_start_matches("tellraw @a ")).unwrap();
        assert_eq!(v[1]["text"], "<Alexandre> ");
        assert_eq!(v[2]["text"], "salut \"les\" gens @everyone", "le texte est affiché tel quel dans le jeu, sans pouvoir sortir du JSON");
        // Pont de chat coupé : plus rien ne passe.
        cfg.servers.get_mut(&sid).unwrap().bridge_chat = false;
        discord::save_config(&dir, &cfg).await.unwrap();
        let n = sent().lines().count();
        handle_message(state.clone(), rest.clone(), message("CHAT1", "U8", &[], "invisible")).await;
        settle().await;
        assert_eq!(sent().lines().count(), n);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn console_output_and_game_chat_reach_discord() {
        let (rest, rec) = mock_rest().await;
        let (state, dir) = temp_state("pump").await;
        let sid = Uuid::new_v4();
        let mut cfg = bridge_cfg();
        cfg.servers.insert(sid, ServerChannels { console_channel_id: "CON1".into(), chat_channel_id: "CHAT1".into(), stream_console: true, bridge_chat: true, ..Default::default() });
        discord::save_config(&dir, &cfg).await.unwrap();
        let lines: Vec<String> = vec![
            "[12:00:00 INFO]: \u{1b}[32mDone (3.2s)!\u{1b}[0m For help, type \"help\"".into(),
            "[12:00:05 INFO]: <Alex> salut @everyone".into(),
            "[12:00:06 INFO]: Alex joined the game".into(),
        ];
        process_lines(&state, &rest, sid, &lines).await;
        let chat = rec.find("/channels/CHAT1/messages");
        assert_eq!(chat.len(), 1, "seul le vrai message de chat passe dans le salon chat");
        let text = chat[0]["content"].as_str().unwrap();
        assert!(text.starts_with("**Alex** » salut"), "{text}");
        assert!(!text.contains("@everyone") && text.contains("@\u{200B}everyone"), "mention désamorcée");
        assert_eq!(chat[0]["allowed_mentions"]["parse"], json!([]));
        let console = rec.find("/channels/CON1/messages");
        assert_eq!(console.len(), 1);
        let block = console[0]["content"].as_str().unwrap();
        assert!(block.starts_with("```") && block.contains("Done (3.2s)!") && block.contains("joined the game"));
        assert!(!block.contains('\u{1b}'), "codes couleur retirés");

        // Console coupée : seul le chat part.
        cfg.servers.get_mut(&sid).unwrap().stream_console = false;
        discord::save_config(&dir, &cfg).await.unwrap();
        process_lines(&state, &rest, sid, &lines).await;
        assert_eq!(rec.find("/channels/CON1/messages").len(), 1);
        assert_eq!(rec.find("/channels/CHAT1/messages").len(), 2);
        // Serveur sans salons configurés : rien.
        process_lines(&state, &rest, Uuid::new_v4(), &lines).await;
        assert_eq!(rec.find("/channels/CHAT1/messages").len(), 2);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
