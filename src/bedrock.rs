//! Minecraft **Bedrock Edition** : tout ce qui diffère de Java pour piloter un serveur.
//!
//! - lecture du journal (joueurs connectés, serveur prêt),
//! - état du serveur par le protocole RakNet (UDP) à la place du ping Java (TCP),
//! - réglages `server.properties` propres à Bedrock (catalogue, validation),
//! - liste d'autorisation (`allowlist.json`) et permissions (`permissions.json`),
//! - sauvegarde « à chaud » du serveur officiel (`save hold` / `save query` / `save resume`).
//!
//! Les téléchargements sont dans `bedrock_dl.rs`, les packs et mondes dans `bedrock_packs.rs`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::Loader;

// ───────────────────────── journal du serveur ─────────────────────────

/// Joueur qui se connecte ou se déconnecte d'après une ligne de journal Bedrock.
/// `Player connected: Steve, xuid: 2535428000000000` / `Player disconnected: Steve, xuid: 2535...`
/// Renvoie `(nom, connecté, xuid)`.
pub fn parse_player_event(line: &str) -> Option<(String, bool, Option<String>)> {
    for (marker, joined) in [("Player connected: ", true), ("Player disconnected: ", false)] {
        if let Some(pos) = line.find(marker) {
            let rest = &line[pos + marker.len()..];
            let (name, xuid) = match rest.split_once(", xuid:") {
                Some((n, x)) => (n.trim(), Some(x.trim().to_string()).filter(|x| !x.is_empty())),
                None => (rest.trim(), None),
            };
            if !name.is_empty() && name.chars().count() <= 64 {
                return Some((name.to_string(), joined, xuid));
            }
        }
    }
    None
}

/// Le serveur accepte les joueurs : `Server started.` (officiel) ou `Done (1.2s)! For help…` (PocketMine, Nukkit).
pub fn is_ready_line(line: &str) -> bool {
    line.contains("Server started.") || (line.contains("Done (") && (line.contains("For help") || line.contains("help")))
}

// ───────────────────────── état par RakNet (UDP) ─────────────────────────

const RAKNET_MAGIC: [u8; 16] = [0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BedrockStatus {
    pub motd: String,
    pub protocol: Option<u32>,
    pub version: String,
    pub online: Option<u32>,
    pub max: Option<u32>,
    pub world: String,
    pub gamemode: String,
}

/// Paquet « Unconnected Ping » (0x01) : horodatage, nombre magique RakNet, identifiant client.
pub fn build_ping(timestamp: u64, client_guid: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(33);
    p.push(0x01);
    p.extend_from_slice(&timestamp.to_be_bytes());
    p.extend_from_slice(&RAKNET_MAGIC);
    p.extend_from_slice(&client_guid.to_be_bytes());
    p
}

/// Décode un « Unconnected Pong » (0x1c). La chaîne finale est
/// `MCPE;MOTD;protocole;version;joueurs;max;idServeur;nomDuMonde;modeDeJeu;…`.
pub fn parse_pong(buf: &[u8]) -> Result<BedrockStatus> {
    if buf.len() < 35 || buf[0] != 0x1c {
        anyhow::bail!("réponse RakNet inattendue");
    }
    if buf[17..33] != RAKNET_MAGIC {
        anyhow::bail!("nombre magique RakNet incorrect");
    }
    let len = u16::from_be_bytes([buf[33], buf[34]]) as usize;
    let payload = buf.get(35..35 + len).context("chaîne d'état tronquée")?;
    let text = String::from_utf8_lossy(payload);
    let f: Vec<&str> = text.split(';').collect();
    if f.first().map(|s| *s != "MCPE" && *s != "MCEE").unwrap_or(true) || f.len() < 6 {
        anyhow::bail!("chaîne d'état Bedrock invalide");
    }
    Ok(BedrockStatus {
        motd: f[1].to_string(),
        protocol: f[2].parse().ok(),
        version: f[3].to_string(),
        online: f[4].parse().ok(),
        max: f[5].parse().ok(),
        world: f.get(7).map(|s| s.to_string()).unwrap_or_default(),
        gamemode: f.get(8).map(|s| s.to_string()).unwrap_or_default(),
    })
}

/// Interroge un serveur Bedrock par UDP (`host:port`). Échoue au bout de `timeout` (serveur arrêté, en démarrage...).
pub async fn query(host: &str, port: u16, timeout: Duration) -> Result<BedrockStatus> {
    let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    sock.connect((host, port)).await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    sock.send(&build_ping(now, 0x4d43_4d61_6e61_6765)).await?;
    let mut buf = vec![0u8; 2048];
    let n = tokio::time::timeout(timeout, sock.recv(&mut buf)).await.map_err(|_| anyhow::anyhow!("pas de réponse du serveur Bedrock (UDP {port})"))??;
    parse_pong(&buf[..n])
}

// ───────────────────────── réglages (server.properties) ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PropDef {
    pub key: &'static str,
    pub label_fr: &'static str,
    pub label_en: &'static str,
    /// `text` | `number` | `bool` | `select`
    pub kind: &'static str,
    pub options: &'static [&'static str],
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub help_fr: &'static str,
    pub help_en: &'static str,
}

const fn d(key: &'static str, fr: &'static str, en: &'static str, kind: &'static str, options: &'static [&'static str], min: Option<i64>, max: Option<i64>, hfr: &'static str, hen: &'static str) -> PropDef {
    PropDef { key, label_fr: fr, label_en: en, kind, options, min, max, help_fr: hfr, help_en: hen }
}

/// Réglages du serveur dédié officiel (https://learn.microsoft.com/minecraft/creator/documents/dedicatedserver).
pub const OFFICIAL_PROPS: &[PropDef] = &[
    d("server-name", "Nom du serveur (liste d'amis LAN)", "Server name (LAN list)", "text", &[], None, None, "Affiché dans la liste des serveurs LAN.", "Shown in the LAN server list."),
    d("gamemode", "Mode de jeu", "Game mode", "select", &["survival", "creative", "adventure"], None, None, "Mode des nouveaux joueurs.", "Mode for new players."),
    d("force-gamemode", "Forcer le mode de jeu", "Force game mode", "bool", &[], None, None, "Impose le mode à chaque connexion.", "Apply the mode on every join."),
    d("difficulty", "Difficulté", "Difficulty", "select", &["peaceful", "easy", "normal", "hard"], None, None, "", ""),
    d("allow-cheats", "Autoriser les commandes (triche)", "Allow cheats", "bool", &[], None, None, "Active les commandes comme /gamemode ou /give.", "Enables commands such as /gamemode or /give."),
    d("max-players", "Joueurs maximum", "Max players", "number", &[], Some(1), Some(1000), "", ""),
    d("online-mode", "Authentification Xbox Live", "Xbox Live authentication", "bool", &[], None, None, "Désactivez seulement pour un réseau local de test.", "Disable only for a local test network."),
    d("allow-list", "Liste d'autorisation (whitelist)", "Allow list", "bool", &[], None, None, "Seuls les joueurs de allowlist.json peuvent rejoindre.", "Only players in allowlist.json can join."),
    d("server-port", "Port IPv4 (UDP)", "IPv4 port (UDP)", "number", &[], Some(1), Some(65535), "19132 par défaut. Protocole UDP, pas TCP.", "19132 by default. UDP, not TCP."),
    d("server-portv6", "Port IPv6 (UDP)", "IPv6 port (UDP)", "number", &[], Some(1), Some(65535), "19133 par défaut.", "19133 by default."),
    d("enable-lan-visibility", "Visible sur le réseau local", "LAN visibility", "bool", &[], None, None, "", ""),
    d("view-distance", "Distance de vue (chunks)", "View distance (chunks)", "number", &[], Some(5), Some(96), "Plus bas = serveur plus léger.", "Lower = lighter server."),
    d("tick-distance", "Distance de simulation (chunks)", "Tick distance (chunks)", "number", &[], Some(4), Some(12), "Rayon où le monde est simulé autour des joueurs.", "Radius simulated around players."),
    d("player-idle-timeout", "Expulsion des joueurs inactifs (minutes, 0 = jamais)", "Idle kick (minutes, 0 = never)", "number", &[], Some(0), Some(100000), "", ""),
    d("max-threads", "Threads maximum (0 = automatique)", "Max threads (0 = auto)", "number", &[], Some(0), Some(256), "Limitez-le sur une petite machine.", "Limit it on a small machine."),
    d("level-name", "Nom du monde (dossier dans worlds/)", "World name (folder in worlds/)", "text", &[], None, None, "Changer le nom crée un nouveau monde.", "Changing it creates a new world."),
    d("level-seed", "Seed du monde (à la création)", "World seed (on creation)", "text", &[], None, None, "", ""),
    d("level-type", "Type de monde", "World type", "select", &["DEFAULT", "FLAT", "LEGACY"], None, None, "", ""),
    d("default-player-permission-level", "Permission par défaut des joueurs", "Default player permission", "select", &["visitor", "member", "operator"], None, None, "", ""),
    d("texturepack-required", "Packs de ressources obligatoires", "Require resource packs", "bool", &[], None, None, "", ""),
    d("content-log-file-enabled", "Journal de contenu (erreurs de packs)", "Content log file", "bool", &[], None, None, "Utile pour déboguer les add-ons.", "Useful to debug add-ons."),
    d("compression-threshold", "Seuil de compression (octets)", "Compression threshold (bytes)", "number", &[], Some(0), Some(65535), "", ""),
    d("compression-algorithm", "Algorithme de compression", "Compression algorithm", "select", &["zlib", "snappy"], None, None, "", ""),
    d("server-authoritative-movement", "Déplacements validés par le serveur", "Server-authoritative movement", "select", &["client-auth", "server-auth", "server-auth-with-rewind"], None, None, "Contre la triche (vol, vitesse).", "Against cheating (fly, speed)."),
    d("player-position-acceptance-threshold", "Tolérance de position", "Position acceptance threshold", "number", &[], Some(0), Some(1000), "", ""),
    d("player-movement-action-direction-threshold", "Tolérance de direction", "Direction threshold", "text", &[], None, None, "", ""),
    d("server-authoritative-block-breaking", "Casse des blocs validée par le serveur", "Server-authoritative block breaking", "bool", &[], None, None, "", ""),
    d("chat-restriction", "Restriction du chat", "Chat restriction", "select", &["None", "Dropped", "Disabled"], None, None, "", ""),
    d("disable-player-interaction", "Désactiver les interactions entre joueurs", "Disable player interaction", "bool", &[], None, None, "", ""),
    d("client-side-chunk-generation-enabled", "Génération de chunks côté client", "Client-side chunk generation", "bool", &[], None, None, "", ""),
    d("block-network-ids-are-hashes", "Identifiants de blocs en empreintes", "Block network IDs are hashes", "bool", &[], None, None, "", ""),
    d("disable-persona", "Désactiver les personas", "Disable personas", "bool", &[], None, None, "", ""),
    d("disable-custom-skins", "Désactiver les skins personnalisés", "Disable custom skins", "bool", &[], None, None, "", ""),
    d("server-build-radius-ratio", "Ratio du rayon de construction", "Build radius ratio", "text", &[], None, None, "« Disabled » ou un nombre entre 0 et 1.", "“Disabled” or a number between 0 and 1."),
    d("emit-server-telemetry", "Télémétrie du serveur", "Server telemetry", "bool", &[], None, None, "", ""),
];

/// Réglages des serveurs communautaires (PocketMine-MP, Nukkit) : proches de Java.
pub const COMMUNITY_PROPS: &[PropDef] = &[
    d("motd", "Message d'accueil (MOTD)", "Message of the day (MOTD)", "text", &[], None, None, "", ""),
    d("server-name", "Nom du serveur", "Server name", "text", &[], None, None, "", ""),
    d("server-port", "Port (UDP)", "Port (UDP)", "number", &[], Some(1), Some(65535), "19132 par défaut.", "19132 by default."),
    d("gamemode", "Mode de jeu", "Game mode", "select", &["survival", "creative", "adventure", "spectator"], None, None, "", ""),
    d("force-gamemode", "Forcer le mode de jeu", "Force game mode", "bool", &[], None, None, "", ""),
    d("difficulty", "Difficulté", "Difficulty", "select", &["peaceful", "easy", "normal", "hard"], None, None, "", ""),
    d("max-players", "Joueurs maximum", "Max players", "number", &[], Some(1), Some(1000), "", ""),
    d("white-list", "Liste blanche", "Whitelist", "bool", &[], None, None, "", ""),
    d("xbox-auth", "Authentification Xbox Live", "Xbox Live authentication", "bool", &[], None, None, "Désactivée = n'importe qui peut usurper un pseudo.", "Disabled = anyone can spoof a name."),
    d("view-distance", "Distance de vue (chunks)", "View distance (chunks)", "number", &[], Some(2), Some(64), "", ""),
    d("level-name", "Nom du monde", "World name", "text", &[], None, None, "", ""),
    d("level-seed", "Seed du monde", "World seed", "text", &[], None, None, "", ""),
    d("level-type", "Type de monde", "World type", "text", &[], None, None, "DEFAULT, FLAT...", "DEFAULT, FLAT..."),
    d("hardcore", "Mode hardcore", "Hardcore mode", "bool", &[], None, None, "", ""),
    d("pvp", "PvP activé", "PvP enabled", "bool", &[], None, None, "", ""),
    d("auto-save", "Sauvegarde automatique du monde", "World auto-save", "bool", &[], None, None, "", ""),
    d("enable-query", "Activer GameSpy Query (UDP)", "Enable GameSpy query (UDP)", "bool", &[], None, None, "", ""),
];

/// Catalogue de réglages à proposer pour ce serveur (vide pour un serveur Java).
pub fn props_for(loader: Loader) -> &'static [PropDef] {
    match loader {
        Loader::Bedrock | Loader::Bedrockpreview => OFFICIAL_PROPS,
        Loader::Pocketmine | Loader::Nukkit => COMMUNITY_PROPS,
        _ => &[],
    }
}

/// Valide et normalise une valeur ; les clés inconnues sont acceptées telles quelles (une seule ligne, sans `=` dans la clé).
pub fn validate_property(loader: Loader, key: &str, value: &str) -> Result<String> {
    if key.is_empty() || key.len() > 80 || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        anyhow::bail!("nom de réglage invalide : {key}");
    }
    if value.contains(['\n', '\r', '\0']) {
        anyhow::bail!("{key} : la valeur doit tenir sur une seule ligne");
    }
    let value = value.trim();
    let Some(def) = props_for(loader).iter().find(|p| p.key == key) else { return Ok(value.to_string()) };
    match def.kind {
        "bool" => match value.to_ascii_lowercase().as_str() {
            "true" | "false" => Ok(value.to_ascii_lowercase()),
            _ => anyhow::bail!("{key} : true ou false attendu"),
        },
        "number" => {
            let n: i64 = value.parse().map_err(|_| anyhow::anyhow!("{key} : nombre entier attendu"))?;
            if def.min.map(|m| n < m).unwrap_or(false) || def.max.map(|m| n > m).unwrap_or(false) {
                anyhow::bail!("{key} : valeur entre {} et {} attendue", def.min.unwrap_or(i64::MIN), def.max.unwrap_or(i64::MAX));
            }
            Ok(n.to_string())
        }
        "select" => def.options.iter().find(|o| o.eq_ignore_ascii_case(value)).map(|o| o.to_string()).ok_or_else(|| anyhow::anyhow!("{key} : l'une de {} attendue", def.options.join(", "))),
        _ => Ok(value.to_string()),
    }
}

/// Nom du monde actif (`level-name`, « Bedrock level » par défaut pour le serveur officiel).
pub fn level_name(props: &str) -> String {
    props.lines().find_map(|l| l.trim().strip_prefix("level-name=").map(|v| v.trim().to_string())).filter(|v| !v.is_empty() && !v.contains(['/', '\\']) && v != "." && v != "..").unwrap_or_else(|| "Bedrock level".to_string())
}

/// Applique nom et port à un `server.properties` de départ (et le port IPv6 à `port + 1`).
pub fn apply_defaults(props: &str, loader: Loader, name: &str, port: u16) -> String {
    let mut set: Vec<(&str, String)> = Vec::new();
    let clean = name.chars().filter(|c| !c.is_control() && *c != ';').take(60).collect::<String>();
    match loader {
        Loader::Bedrock | Loader::Bedrockpreview => {
            set.push(("server-name", clean));
            set.push(("server-port", port.to_string()));
            set.push(("server-portv6", port.saturating_add(1).to_string()));
        }
        _ => {
            set.push(("motd", clean.clone()));
            set.push(("server-name", clean));
            set.push(("server-port", port.to_string()));
        }
    }
    let mut lines: Vec<String> = props.lines().map(String::from).collect();
    for (k, v) in set {
        let mut done = false;
        for l in lines.iter_mut() {
            if !l.trim_start().starts_with('#') && l.split_once('=').map(|(a, _)| a.trim() == k).unwrap_or(false) {
                *l = format!("{k}={v}");
                done = true;
            }
        }
        if !done {
            lines.push(format!("{k}={v}"));
        }
    }
    lines.join("\n") + "\n"
}

// ───────────────────────── allowlist.json et permissions.json ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xuid: Option<String>,
    #[serde(default, rename = "ignoresPlayerLimit")]
    pub ignores_player_limit: bool,
}

pub fn valid_gamertag(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 32 && n.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '#'))
}

pub fn read_allowlist(folder: &Path) -> Vec<AllowEntry> {
    std::fs::read_to_string(folder.join("allowlist.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_allowlist(folder: &Path, list: &[AllowEntry]) -> Result<()> {
    std::fs::write(folder.join("allowlist.json"), serde_json::to_string_pretty(list)?)?;
    Ok(())
}

/// Ajoute (ou met à jour) un joueur ; renvoie `false` s'il y était déjà.
pub fn allowlist_add(folder: &Path, name: &str, xuid: Option<&str>) -> Result<bool> {
    if !valid_gamertag(name) {
        anyhow::bail!("pseudo Xbox invalide");
    }
    let mut list = read_allowlist(folder);
    if list.iter().any(|e| e.name.eq_ignore_ascii_case(name.trim())) {
        return Ok(false);
    }
    list.push(AllowEntry { name: name.trim().to_string(), xuid: xuid.filter(|x| x.chars().all(|c| c.is_ascii_digit()) && !x.is_empty()).map(String::from), ignores_player_limit: false });
    write_allowlist(folder, &list)?;
    Ok(true)
}

pub fn allowlist_remove(folder: &Path, name: &str) -> Result<bool> {
    let mut list = read_allowlist(folder);
    let before = list.len();
    list.retain(|e| !e.name.eq_ignore_ascii_case(name.trim()));
    if list.len() == before {
        return Ok(false);
    }
    write_allowlist(folder, &list)?;
    Ok(true)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermEntry {
    pub permission: String,
    pub xuid: String,
}

pub fn read_permissions(folder: &Path) -> Vec<PermEntry> {
    std::fs::read_to_string(folder.join("permissions.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Définit le niveau (`visitor` | `member` | `operator`) d'un joueur identifié par son XUID (chiffres).
pub fn set_permission(folder: &Path, xuid: &str, level: &str) -> Result<()> {
    if xuid.is_empty() || xuid.len() > 20 || !xuid.chars().all(|c| c.is_ascii_digit()) {
        anyhow::bail!("XUID invalide (chiffres uniquement)");
    }
    if !["visitor", "member", "operator"].contains(&level) {
        anyhow::bail!("niveau inconnu (visitor, member ou operator)");
    }
    let mut list = read_permissions(folder);
    list.retain(|e| e.xuid != xuid);
    list.push(PermEntry { permission: level.to_string(), xuid: xuid.to_string() });
    std::fs::write(folder.join("permissions.json"), serde_json::to_string_pretty(&list)?)?;
    Ok(())
}

pub fn remove_permission(folder: &Path, xuid: &str) -> Result<bool> {
    let mut list = read_permissions(folder);
    let before = list.len();
    list.retain(|e| e.xuid != xuid);
    if list.len() == before {
        return Ok(false);
    }
    std::fs::write(folder.join("permissions.json"), serde_json::to_string_pretty(&list)?)?;
    Ok(true)
}

// ───────────────────────── sauvegarde à chaud ─────────────────────────

/// Lit la réponse de `save query` : une ligne « Data saved. Files are now ready to be copied. » puis la liste
/// `chemin:longueur, chemin:longueur…`. Renvoie les longueurs à respecter en copiant (les fichiers continuent de grossir).
pub fn parse_save_query(lines: &[String]) -> Option<HashMap<String, u64>> {
    let idx = lines.iter().rposition(|l| l.contains("Data saved") || l.contains("ready to be copied"))?;
    let list = lines.get(idx + 1..)?.iter().find(|l| l.contains(':') && l.contains('/'))?;
    let mut out = HashMap::new();
    for part in list.split(", ") {
        let part = part.trim();
        if let Some((path, len)) = part.rsplit_once(':') {
            if let Ok(n) = len.trim().parse::<u64>() {
                out.insert(path.trim().replace('\\', "/"), n);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Fige l'écriture du monde du serveur officiel et attend que les fichiers soient prêts à être copiés.
/// Renvoie les longueurs de fichiers à respecter. À terminer par [`save_resume`], même en cas d'erreur.
pub async fn save_hold(state: &crate::state::AppState, id: uuid::Uuid) -> Result<HashMap<String, u64>> {
    let seq = state.runtime.read().await.get(&id).map(|r| r.seq()).unwrap_or(0);
    crate::process::send_command(state, id, "save hold").await?;
    // `save hold` ne répond rien d'utile : on interroge `save query` jusqu'à « Data saved ».
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        crate::process::send_command(state, id, "save query").await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let lines = state.runtime.read().await.get(&id).map(|r| r.lines_after(seq)).unwrap_or_default();
        if let Some(limits) = parse_save_query(&lines) {
            return Ok(limits);
        }
    }
    anyhow::bail!("le serveur Bedrock n'a pas confirmé la sauvegarde (save hold) en 60 s")
}

pub async fn save_resume(state: &crate::state::AppState, id: uuid::Uuid) {
    let _ = crate::process::send_command(state, id, "save resume").await;
}

/// Résumé JSON d'un serveur Bedrock pour l'API (version, ports, packs...).
pub fn summary(entry: &crate::models::ServerEntry) -> Value {
    json!({ "edition": entry.loader.edition(), "official": entry.loader.is_official_bedrock(), "port_udp": entry.port, "runtime": format!("{:?}", entry.loader.runtime()).to_lowercase() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-bedrock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn player_events_from_the_console() {
        assert_eq!(parse_player_event("[2026-11-14 20:10:01:123 INFO] Player connected: Steve, xuid: 2535428000000001"), Some(("Steve".into(), true, Some("2535428000000001".into()))));
        assert_eq!(parse_player_event("[2026-11-14 20:12:00:001 INFO] Player disconnected: Alex Smith, xuid: 99"), Some(("Alex Smith".into(), false, Some("99".into()))));
        assert_eq!(parse_player_event("Player connected: Sans xuid").map(|e| (e.0, e.2)), Some(("Sans xuid".into(), None)));
        assert_eq!(parse_player_event("Player Spawned: Steve xuid: 1, pfid: 2"), None);
        assert_eq!(parse_player_event("[INFO] Server started."), None);
        assert!(is_ready_line("[2026-11-14 20:00:00:000 INFO] Server started."));
        assert!(is_ready_line("[17:02:01] [Server thread/INFO]: Done (1.2s)! For help, type \"help\" or \"?\""));
        assert!(!is_ready_line("[INFO] Starting Server"));
    }

    fn pong(text: &str) -> Vec<u8> {
        let mut p = vec![0x1c];
        p.extend_from_slice(&1u64.to_be_bytes());
        p.extend_from_slice(&2u64.to_be_bytes());
        p.extend_from_slice(&RAKNET_MAGIC);
        p.extend_from_slice(&(text.len() as u16).to_be_bytes());
        p.extend_from_slice(text.as_bytes());
        p
    }

    #[test]
    fn raknet_ping_and_pong() {
        let p = build_ping(42, 7);
        assert_eq!((p.len(), p[0]), (33, 0x01));
        assert_eq!(&p[9..25], &RAKNET_MAGIC);
        let st = parse_pong(&pong("MCPE;Mon serveur;712;1.21.50;3;20;1234567;Bedrock level;Survival;1;19132;19133;")).unwrap();
        assert_eq!((st.motd.as_str(), st.protocol, st.version.as_str(), st.online, st.max, st.world.as_str(), st.gamemode.as_str()), ("Mon serveur", Some(712), "1.21.50", Some(3), Some(20), "Bedrock level", "Survival"));
        assert!(parse_pong(&[0x1c; 10]).is_err());
        let mut bad = pong("MCPE;a;1;1;0;1");
        bad[0] = 0x1d;
        assert!(parse_pong(&bad).is_err());
        let mut bad_magic = pong("MCPE;a;1;1;0;1");
        bad_magic[20] ^= 1;
        assert!(parse_pong(&bad_magic).is_err());
        assert!(parse_pong(&pong("HTTP;x;1;1;0;1")).is_err());
        let mut short = pong("MCPE;a;1;1;0;1");
        short.truncate(40);
        assert!(parse_pong(&short).is_err(), "chaîne tronquée");
    }

    #[tokio::test]
    async fn query_talks_to_a_udp_server() {
        let srv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = srv.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, from) = srv.recv_from(&mut buf).await.unwrap();
            assert_eq!((n, buf[0]), (33, 0x01));
            srv.send_to(&pong("MCPE;Test;712;1.21.50;1;10;1;Monde;Creative;1;"), from).await.unwrap();
        });
        let st = query("127.0.0.1", port, Duration::from_secs(2)).await.unwrap();
        assert_eq!((st.online, st.max, st.gamemode.as_str()), (Some(1), Some(10), "Creative"));
        // Serveur muet : délai dépassé, pas de blocage.
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let p2 = silent.local_addr().unwrap().port();
        assert!(query("127.0.0.1", p2, Duration::from_millis(150)).await.is_err());
    }

    #[test]
    fn property_validation_by_kind() {
        let o = Loader::Bedrock;
        assert_eq!(validate_property(o, "gamemode", " Creative ").unwrap(), "creative");
        assert!(validate_property(o, "gamemode", "spectator").is_err());
        assert_eq!(validate_property(o, "allow-cheats", "TRUE").unwrap(), "true");
        assert!(validate_property(o, "allow-cheats", "oui").is_err());
        assert_eq!(validate_property(o, "max-players", "20").unwrap(), "20");
        assert!(validate_property(o, "max-players", "0").is_err() && validate_property(o, "max-players", "abc").is_err());
        assert!(validate_property(o, "tick-distance", "3").is_err() && validate_property(o, "tick-distance", "13").is_err());
        assert_eq!(validate_property(o, "tick-distance", "8").unwrap(), "8");
        assert_eq!(validate_property(o, "server-authoritative-movement", "server-auth").unwrap(), "server-auth");
        assert_eq!(validate_property(o, "chat-restriction", "dropped").unwrap(), "Dropped", "casse canonique");
        assert_eq!(validate_property(o, "cle-inconnue", "x").unwrap(), "x");
        assert!(validate_property(o, "motd", "a\nb").is_err() && validate_property(o, "a=b", "x").is_err() && validate_property(o, "", "x").is_err());
        assert!(validate_property(Loader::Paper, "max-players", "abc").is_ok(), "Java : pas de validation ici");
        assert!(props_for(Loader::Paper).is_empty() && !props_for(Loader::Nukkit).is_empty());
        for p in OFFICIAL_PROPS.iter().chain(COMMUNITY_PROPS) {
            assert!(!p.label_fr.is_empty() && !p.label_en.is_empty(), "{}", p.key);
            if p.kind == "select" {
                assert!(!p.options.is_empty(), "{}", p.key);
            }
        }
    }

    #[test]
    fn defaults_and_level_name() {
        let base = "server-name=Dedicated Server\nserver-port=19132\nserver-portv6=19133\ngamemode=survival\n";
        let out = apply_defaults(base, Loader::Bedrock, "Mon;Serveur\n", 19140);
        assert!(out.contains("server-name=MonServeur") && out.contains("server-port=19140") && out.contains("server-portv6=19141") && out.contains("gamemode=survival"));
        let c = apply_defaults("", Loader::Pocketmine, "Salut", 19132);
        assert!(c.contains("motd=Salut") && c.contains("server-port=19132"));
        assert_eq!(level_name("level-name=Mon monde\n"), "Mon monde");
        assert_eq!(level_name("level-name=../x"), "Bedrock level");
        assert_eq!(level_name(""), "Bedrock level");
    }

    #[test]
    fn allowlist_and_permissions_files() {
        let d = tmp();
        assert!(read_allowlist(&d).is_empty());
        assert!(allowlist_add(&d, " Steve ", Some("2535428000000001")).unwrap());
        assert!(!allowlist_add(&d, "steve", None).unwrap(), "pas de doublon, casse ignorée");
        assert!(allowlist_add(&d, "Alex", Some("pas-un-xuid")).unwrap());
        assert!(allowlist_add(&d, "bad;name", None).is_err() && allowlist_add(&d, "", None).is_err());
        let l = read_allowlist(&d);
        assert_eq!((l.len(), l[0].xuid.as_deref(), l[1].xuid.as_deref()), (2, Some("2535428000000001"), None));
        let raw = std::fs::read_to_string(d.join("allowlist.json")).unwrap();
        assert!(raw.contains("ignoresPlayerLimit") && !raw.contains("\"xuid\": null"));
        assert!(allowlist_remove(&d, "STEVE").unwrap() && !allowlist_remove(&d, "STEVE").unwrap());
        set_permission(&d, "2535428000000001", "operator").unwrap();
        set_permission(&d, "2535428000000001", "member").unwrap();
        assert_eq!(read_permissions(&d), vec![PermEntry { permission: "member".into(), xuid: "2535428000000001".into() }]);
        assert!(set_permission(&d, "abc", "operator").is_err() && set_permission(&d, "1", "dieu").is_err());
        assert!(remove_permission(&d, "2535428000000001").unwrap() && read_permissions(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_query_output_gives_exact_file_lengths() {
        let lines: Vec<String> = ["[INFO] Saving...", "[INFO] Data saved. Files are now ready to be copied.", "Mon monde/db/MANIFEST-000012:258, Mon monde/db/000015.ldb:1234, Mon monde/level.dat:2024"].iter().map(|s| s.to_string()).collect();
        let m = parse_save_query(&lines).unwrap();
        assert_eq!((m.len(), m["Mon monde/db/000015.ldb"], m["Mon monde/level.dat"]), (3, 1234, 2024));
        // Un ancien « Data saved » ne doit pas masquer le plus récent.
        let mut two = lines.clone();
        two.push("[INFO] Data saved. Files are now ready to be copied.".into());
        two.push("Mon monde/db/000020.ldb:99".into());
        assert_eq!(parse_save_query(&two).unwrap().len(), 1);
        assert!(parse_save_query(&["[INFO] A previous save has not been completed.".to_string()]).is_none());
        assert!(parse_save_query(&["Data saved. Files are now ready to be copied.".to_string()]).is_none(), "liste absente");
        assert!(parse_save_query(&[]).is_none());
    }

    /// Serveur « Bedrock » factice (script) : prêt, répond à `save hold/query/resume`, notifie un joueur, s'arrête sur `stop`.
    #[cfg(unix)]
    const FAKE_BDS: &str = r#"#!/bin/sh
echo "[INFO] Starting Server"
echo "[INFO] Server started."
while read line; do
  case "$line" in
    stop) echo "[INFO] Quit correctly"; exit 0 ;;
    "save hold") echo "[INFO] Saving..." ;;
    "save query") echo "[INFO] Data saved. Files are now ready to be copied."; echo "Mon monde/db/000001.ldb:5, Mon monde/level.dat:3" ;;
    "save resume") echo "[INFO] Changes to the world are resumed." ;;
    join) echo "[INFO] Player connected: Steve, xuid: 2535428000000001" ;;
  esac
done
"#;

    #[cfg(unix)]
    #[tokio::test]
    async fn bedrock_server_runs_and_is_backed_up_hot_with_exact_lengths() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp();
        let state = crate::state::build_state_for_test(&d).await;
        let id = uuid::Uuid::new_v4();
        let folder = d.join("servers").join(id.to_string());
        std::fs::create_dir_all(folder.join("worlds/Mon monde/db")).unwrap();
        std::fs::write(folder.join("bedrock_server"), FAKE_BDS).unwrap();
        std::fs::set_permissions(folder.join("bedrock_server"), std::fs::Permissions::from_mode(0o644)).unwrap(); // sans bit exécutable : MCManager doit le poser
        std::fs::write(folder.join("server.properties"), "server-name=Test\nlevel-name=Mon monde\nserver-port=19132\n").unwrap();
        std::fs::write(folder.join("worlds/Mon monde/db/000001.ldb"), "HELLOWORLD").unwrap();
        std::fs::write(folder.join("worlds/Mon monde/level.dat"), "ABCDEF").unwrap();
        let mut entry = crate::models::ServerEntry::test_entry(id, "Bedrock test", &folder);
        entry.loader = Loader::Bedrock;
        entry.jar_name = "bedrock_server".into();
        entry.java_path = String::new();
        entry.port = 19132;
        state.servers.write().await.insert(id, entry);

        crate::process::start_server(&state, id).await.unwrap();
        // Prêt : « Server started. » reconnu comme le « Done » de Java.
        let mut ready = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if state.runtime.read().await.get(&id).map(|r| r.backlog.iter().any(|l| l.contains("Server started."))).unwrap_or(false) {
                ready = true;
                break;
            }
        }
        assert!(ready, "le serveur factice n'a rien écrit");
        assert!(std::fs::metadata(folder.join("bedrock_server")).unwrap().permissions().mode() & 0o111 != 0, "bit exécutable posé");
        assert!(crate::resume::load(&d).await.contains(&id), "serveur en marche = au manifeste de reprise");

        // Sauvegarde à chaud : save hold → save query → copie tronquée aux longueurs annoncées → save resume.
        let total = crate::api::start_backup(&state, id).await.unwrap();
        assert!(total > 0);
        let bdir = crate::backup::backups_dir(&d, &id);
        let mut zip_path = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let done = state.backup_progress.read().await.get(&id).is_none();
            if done {
                zip_path = std::fs::read_dir(&bdir).ok().and_then(|mut r| r.next()).map(|e| e.unwrap().path());
                if zip_path.is_some() { break; }
            }
        }
        let zip_path = zip_path.expect("sauvegarde produite");
        let mut z = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let read = |z: &mut zip::ZipArchive<std::fs::File>, n: &str| { let mut s = String::new(); std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut s).unwrap(); s };
        assert_eq!(read(&mut z, "worlds/Mon monde/db/000001.ldb"), "HELLO", "tronqué à 5 octets (longueur de save query)");
        assert_eq!(read(&mut z, "worlds/Mon monde/level.dat"), "ABC");
        assert_eq!(read(&mut z, "server.properties").lines().next(), Some("server-name=Test"), "les autres fichiers sont copiés entiers");
        let resumed = state.runtime.read().await.get(&id).map(|r| r.backlog.iter().any(|l| l.contains("resumed"))).unwrap_or(false);
        assert!(resumed, "save resume envoyé : le serveur reprend l'écriture");

        // Un joueur se connecte : le journal Bedrock est compris (pour Discord, le suivi des joueurs, l'arrêt si vide).
        crate::process::send_command(&state, id, "join").await.unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        let line = state.runtime.read().await.get(&id).unwrap().backlog.iter().find(|l| l.contains("Player connected")).cloned().unwrap();
        assert_eq!(parse_player_event(&line).map(|e| e.0), Some("Steve".to_string()));

        // Arrêt propre ; l'arrêt voulu retire le serveur du manifeste de reprise.
        crate::process::stop_server(&state, id, false).await.unwrap();
        let mut stopped = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !state.runtime.read().await.get(&id).map(|r| r.running).unwrap_or(false) { stopped = true; break; }
        }
        assert!(stopped, "arrêt sur la commande « stop »");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!crate::resume::load(&d).await.contains(&id));
        let _ = std::fs::remove_dir_all(&d);
    }
}
