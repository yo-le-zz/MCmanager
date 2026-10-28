//! Domaines personnalisés et redirections (nginx / Caddy / DNS).
//!
//! MCManager **génère** la configuration ; il ne touche jamais lui-même à
//! nginx ni au DNS (écrire dans `/etc/nginx` demande root, et une config
//! cassée peut couper tout un serveur web). Vous relisez, copiez, appliquez.
//!
//! ## Ce que fait chaque mode (et pourquoi)
//!
//! - **SRV (recommandé, aucun proxy)** : Minecraft Java sait lire un
//!   enregistrement DNS `_minecraft._tcp.<domaine> SRV … <port> <domaine>`.
//!   Les joueurs tapent `play.exemple.fr` sans port, même si le serveur écoute
//!   sur 25570 ; plusieurs serveurs sur une même IP = un sous-domaine chacun,
//!   chacun avec son SRV. C'est la solution standard, sans machine
//!   intermédiaire et sans latence ajoutée.
//! - **Proxy TCP nginx (`stream`)** : utile quand le port public doit
//!   différer du port réel, pour cacher la machine qui héberge vraiment le
//!   serveur (VPS en façade d'une machine maison / d'une instance distante),
//!   ou pour ajouter le PROXY protocol. Précision : nginx ne peut pas router
//!   plusieurs domaines vers plusieurs serveurs sur *le même port* (il
//!   ne lit pas l'adresse du handshake Minecraft) : un port public par serveur,
//!   ou des SRV.
//! - **Interface web** : reverse proxy HTTP(S) vers MCManager, avec
//!   WebSocket (console en direct), envois volumineux et HTTPS (Let's Encrypt).
//!
//! Toute valeur insérée dans une configuration (domaine, hôte, port) est
//! validée strictement : un nom de domaine ne peut pas être détourné pour
//! injecter des directives nginx.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn default_web_port() -> u16 {
    7777
}
fn default_public_port() -> u16 {
    25565
}
fn default_mode() -> String {
    "srv".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerDomain {
    pub domain: String,
    /// Port que les joueurs utilisent côté public (25565 = celui par défaut, rien à saisir).
    #[serde(default = "default_public_port")]
    pub public_port: u16,
    /// `srv` (DNS seulement) ou `proxy` (nginx stream).
    #[serde(default = "default_mode")]
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Domaine de l'interface web (ex. mc.exemple.fr), vide = non utilisé.
    #[serde(default)]
    pub web_domain: String,
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    /// IP publique, pour pré-remplir les enregistrements A (facultatif).
    #[serde(default)]
    pub public_ip: String,
    #[serde(default)]
    pub servers: HashMap<Uuid, ServerDomain>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        NetworkConfig { web_domain: String::new(), web_port: 7777, public_ip: String::new(), servers: HashMap::new() }
    }
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("network_config.json")
}

pub async fn load(data_dir: &Path) -> NetworkConfig {
    match tokio::fs::read_to_string(config_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => NetworkConfig::default(),
    }
}

pub async fn save(data_dir: &Path, cfg: &NetworkConfig) -> Result<()> {
    validate(cfg)?;
    tokio::fs::write(config_path(data_dir), serde_json::to_string_pretty(cfg)?).await?;
    Ok(())
}

// ───────────────────────── validation ─────────────────────────

/// Nom d'hôte DNS : étiquettes de 1 à 63 caractères `[a-z0-9-]` (ni début ni fin
/// par un tiret), au moins un point, 253 caractères au plus. Rien d'autre :
/// ni espace, ni `;`, ni `{`, ni retour à la ligne.
pub fn valid_domain(d: &str) -> bool {
    if d.is_empty() || d.len() > 253 || !d.contains('.') {
        return false;
    }
    d.split('.').all(|l| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

fn valid_ip(ip: &str) -> bool {
    ip.parse::<std::net::IpAddr>().is_ok()
}

pub fn validate(cfg: &NetworkConfig) -> Result<()> {
    if !cfg.web_domain.is_empty() && !valid_domain(&cfg.web_domain) {
        bail!("domaine de l'interface invalide : « {} » (exemple : mc.exemple.fr, en minuscules)", cfg.web_domain);
    }
    if cfg.web_port == 0 {
        bail!("port de l'interface invalide");
    }
    if !cfg.public_ip.is_empty() && !valid_ip(&cfg.public_ip) {
        bail!("IP publique invalide : « {} »", cfg.public_ip);
    }
    let mut seen: HashMap<&str, Uuid> = HashMap::new();
    for (id, sd) in &cfg.servers {
        if !valid_domain(&sd.domain) {
            bail!("domaine invalide : « {} » (exemple : play.exemple.fr, en minuscules, sans http://)", sd.domain);
        }
        if sd.public_port == 0 {
            bail!("port public invalide pour {}", sd.domain);
        }
        if sd.mode != "srv" && sd.mode != "proxy" {
            bail!("mode inconnu pour {} : « {} »", sd.domain, sd.mode);
        }
        if let Some(other) = seen.insert(sd.domain.as_str(), *id) {
            if other != *id {
                bail!("le domaine {} est utilisé par deux serveurs : un domaine = un serveur (utilisez un sous-domaine chacun)", sd.domain);
            }
        }
    }
    Ok(())
}

// ───────────────────────── génération ─────────────────────────

/// Un serveur connu de MCManager, local ou distant.
#[derive(Debug, Clone)]
pub struct KnownServer {
    pub id: Uuid,
    pub name: String,
    /// Port réel du serveur Minecraft sur sa machine.
    pub port: u16,
    /// Machine qui l'héberge, vue depuis ici (127.0.0.1 pour un serveur local).
    pub backend_host: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Generated {
    pub dns: String,
    pub nginx_stream: String,
    pub nginx_web: String,
    pub caddy: String,
    pub notes: Vec<String>,
}

pub fn generate(cfg: &NetworkConfig, servers: &[KnownServer]) -> Generated {
    let ip = if cfg.public_ip.is_empty() { "<IP_PUBLIQUE>".to_string() } else { cfg.public_ip.clone() };
    let record_type = if cfg.public_ip.contains(':') { "AAAA" } else { "A" };
    let mut dns = String::new();
    let mut stream = String::new();
    let mut notes = Vec::new();
    let mut any_proxy = false;

    if !cfg.web_domain.is_empty() {
        dns.push_str(&format!("; Interface web\n{}.  300  IN  {record_type}  {ip}\n\n", cfg.web_domain));
    }

    let mut entries: Vec<(&Uuid, &ServerDomain)> = cfg.servers.iter().collect();
    entries.sort_by(|a, b| a.1.domain.cmp(&b.1.domain));
    for (id, sd) in entries {
        let Some(server) = servers.iter().find(|s| &s.id == id) else { continue };
        dns.push_str(&format!("; {} (serveur Minecraft)\n{}.  300  IN  {record_type}  {ip}\n", server.name.replace(['\n', '\r'], " "), sd.domain));
        if sd.mode == "proxy" {
            any_proxy = true;
            // Le port que les joueurs utilisent : si ce n'est pas 25565, un SRV le leur évite de le saisir.
            if sd.public_port != 25565 {
                dns.push_str(&format!("_minecraft._tcp.{}.  300  IN  SRV  0 5 {} {}.\n", sd.domain, sd.public_port, sd.domain));
            }
            stream.push_str(&format!(
                "# {name} : {domain} -> {host}:{port}\nserver {{\n    listen {public};\n    proxy_pass {host}:{port};\n    proxy_connect_timeout 5s;\n    # proxy_protocol on;   # seulement si le serveur est configuré pour le recevoir (Velocity/Bungee)\n}}\n\n",
                name = server.name.replace(['\n', '\r'], " "), domain = sd.domain, host = server.backend_host, port = server.port, public = sd.public_port
            ));
        } else if server.port != 25565 {
            dns.push_str(&format!("_minecraft._tcp.{}.  300  IN  SRV  0 5 {} {}.\n", sd.domain, server.port, sd.domain));
            notes.push(format!("{} : le serveur écoute sur le port {}, le SRV permet aux joueurs de taper « {} » sans port. Ouvrez/redirigez le port {} (TCP) vers la machine.", sd.domain, server.port, sd.domain, server.port));
        } else {
            notes.push(format!("{} : port 25565 par défaut, un simple enregistrement {record_type} suffit. Ouvrez/redirigez le port 25565 (TCP).", sd.domain));
        }
        dns.push('\n');
    }

    let nginx_stream = if any_proxy {
        let header = "# /etc/nginx/streams-enabled/mcmanager.conf\n# Nécessite le module stream (Debian/Ubuntu : sudo apt install libnginx-mod-stream)\n# et, une seule fois, à la racine de /etc/nginx/nginx.conf (hors du bloc http) :\n#     stream { include /etc/nginx/streams-enabled/*.conf; }\n# Puis : sudo nginx -t && sudo systemctl reload nginx\n\n";
        format!("{header}{stream}")
    } else {
        String::new()
    };

    let (mut nginx_web, mut caddy) = (String::new(), String::new());
    if !cfg.web_domain.is_empty() {
        nginx_web = format!(
            "# /etc/nginx/sites-available/mcmanager  (puis : sudo ln -s ../sites-available/mcmanager /etc/nginx/sites-enabled/)\n\
server {{\n\
    listen 80;\n\
    listen [::]:80;\n\
    server_name {domain};\n\
\n\
    # Envois de mods / mondes / schematics (MCManager accepte jusqu'à 512 Mo)\n\
    client_max_body_size 512m;\n\
    proxy_request_buffering off;\n\
\n\
    location / {{\n\
        proxy_pass http://127.0.0.1:{port};\n\
        proxy_http_version 1.1;\n\
        # WebSocket : indispensable pour la console en direct\n\
        proxy_set_header Upgrade $http_upgrade;\n\
        proxy_set_header Connection \"upgrade\";\n\
        proxy_set_header Host $host;\n\
        proxy_set_header X-Real-IP $remote_addr;\n\
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
        proxy_set_header X-Forwarded-Proto $scheme;\n\
        proxy_read_timeout 3600s;\n\
        proxy_send_timeout 3600s;\n\
    }}\n\
}}\n\
\n\
# HTTPS gratuit (Let's Encrypt) : modifie ce fichier pour ajouter le certificat et la redirection 80 -> 443\n\
#   sudo apt install certbot python3-certbot-nginx\n\
#   sudo certbot --nginx -d {domain}\n",
            domain = cfg.web_domain, port = cfg.web_port
        );
        caddy = format!(
            "# /etc/caddy/Caddyfile : HTTPS automatique (certificat obtenu et renouvelé tout seul), WebSocket inclus\n{domain} {{\n    request_body {{\n        max_size 512MB\n    }}\n    reverse_proxy 127.0.0.1:{port}\n}}\n# Puis : sudo systemctl reload caddy\n",
            domain = cfg.web_domain, port = cfg.web_port
        );
        notes.push("Derrière un reverse proxy HTTPS, démarrez MCManager avec MCMANAGER_TRUST_PROXY=1 et MCMANAGER_COOKIE_SECURE=1 (fichier /etc/mcmanager/server.env) : l'adresse des clients et le cookie de session sécurisé en dépendent.".to_string());
        if cfg.web_port != 7777 {
            notes.push(format!("Port de l'interface : {} (MCMANAGER_PORT).", cfg.web_port));
        }
    }
    if dns.is_empty() && nginx_web.is_empty() {
        notes.push("Associez un domaine à un serveur (ou à l'interface web) pour obtenir la configuration.".to_string());
    }
    notes.push("Le DNS peut mettre quelques minutes à plusieurs heures à se propager. Vérifiez avec : dig +short SRV _minecraft._tcp.votre-domaine".to_string());

    Generated { dns, nginx_stream, nginx_web, caddy, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srv(id: Uuid, name: &str, port: u16) -> KnownServer {
        KnownServer { id, name: name.into(), port, backend_host: "127.0.0.1".into() }
    }

    #[test]
    fn domain_validation_blocks_injection() {
        for ok in ["play.exemple.fr", "mc-1.a.b.example.com", "x.io"] {
            assert!(valid_domain(ok), "{ok}");
        }
        for bad in ["", "localhost", "Play.exemple.fr", "a b.fr", "a.fr; } evil {", "a.fr\nlisten 1;", "http://a.fr", "a..fr", "-a.fr", "a-.fr", "a.fr/", "é.fr", "a_b.fr", "$(id).fr", &"a".repeat(64).to_string()] {
            assert!(!valid_domain(bad), "{bad:?} doit être refusé");
        }
        assert!(!valid_domain(&format!("{}.fr", "a".repeat(64))), "étiquette de plus de 63 caractères");
    }

    #[test]
    fn config_validation() {
        let mut cfg = NetworkConfig::default();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        cfg.servers.insert(a, ServerDomain { domain: "play.exemple.fr".into(), public_port: 25565, mode: "srv".into() });
        assert!(validate(&cfg).is_ok());
        cfg.servers.insert(b, ServerDomain { domain: "play.exemple.fr".into(), public_port: 25566, mode: "srv".into() });
        assert!(validate(&cfg).is_err(), "deux serveurs ne partagent pas un domaine");
        cfg.servers.get_mut(&b).unwrap().domain = "creatif.exemple.fr".into();
        assert!(validate(&cfg).is_ok());
        cfg.servers.get_mut(&b).unwrap().mode = "evil".into();
        assert!(validate(&cfg).is_err());
        cfg.servers.get_mut(&b).unwrap().mode = "srv".into();
        cfg.public_ip = "pas une ip".into();
        assert!(validate(&cfg).is_err());
        cfg.public_ip = "203.0.113.7".into();
        cfg.web_domain = "mc.exemple.fr; return 301 http://evil".into();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn srv_mode_needs_no_proxy() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut cfg = NetworkConfig { public_ip: "203.0.113.7".into(), ..Default::default() };
        cfg.servers.insert(a, ServerDomain { domain: "play.exemple.fr".into(), public_port: 25565, mode: "srv".into() });
        cfg.servers.insert(b, ServerDomain { domain: "creatif.exemple.fr".into(), public_port: 25565, mode: "srv".into() });
        let g = generate(&cfg, &[srv(a, "Survie", 25565), srv(b, "Créatif", 25570)]);
        assert!(g.dns.contains("play.exemple.fr.  300  IN  A  203.0.113.7"));
        assert!(!g.dns.contains("_minecraft._tcp.play.exemple.fr"), "25565 : pas de SRV");
        assert!(g.dns.contains("_minecraft._tcp.creatif.exemple.fr.  300  IN  SRV  0 5 25570 creatif.exemple.fr."));
        assert!(g.nginx_stream.is_empty(), "aucun proxy en mode SRV");
        assert!(g.nginx_web.is_empty() && g.caddy.is_empty());
    }

    #[test]
    fn proxy_mode_generates_a_stream_block_for_remote_backends() {
        let a = Uuid::new_v4();
        let mut cfg = NetworkConfig::default();
        cfg.servers.insert(a, ServerDomain { domain: "play.exemple.fr".into(), public_port: 25570, mode: "proxy".into() });
        let server = KnownServer { id: a, name: "Survie distant".into(), port: 25565, backend_host: "192.168.1.50".into() };
        let g = generate(&cfg, &[server]);
        assert!(g.nginx_stream.contains("listen 25570;"));
        assert!(g.nginx_stream.contains("proxy_pass 192.168.1.50:25565;"));
        assert!(g.nginx_stream.contains("libnginx-mod-stream"));
        assert!(g.dns.contains("SRV  0 5 25570 play.exemple.fr."), "le port public non standard est annoncé par SRV");
        assert!(g.dns.contains("<IP_PUBLIQUE>"), "sans IP saisie, un repère clair");
    }

    #[test]
    fn web_proxy_has_websocket_and_https_hints() {
        let cfg = NetworkConfig { web_domain: "mc.exemple.fr".into(), web_port: 7777, ..Default::default() };
        let g = generate(&cfg, &[]);
        assert!(g.nginx_web.contains("server_name mc.exemple.fr;"));
        assert!(g.nginx_web.contains("proxy_set_header Upgrade $http_upgrade;"), "console en direct = WebSocket");
        assert!(g.nginx_web.contains("proxy_pass http://127.0.0.1:7777;"));
        assert!(g.nginx_web.contains("client_max_body_size 512m;"));
        assert!(g.nginx_web.contains("certbot --nginx -d mc.exemple.fr"));
        assert!(g.caddy.contains("mc.exemple.fr {") && g.caddy.contains("reverse_proxy 127.0.0.1:7777"));
        assert!(g.notes.iter().any(|n| n.contains("MCMANAGER_TRUST_PROXY")));
    }

    #[test]
    fn server_names_cannot_break_comments() {
        let a = Uuid::new_v4();
        let mut cfg = NetworkConfig::default();
        cfg.servers.insert(a, ServerDomain { domain: "play.exemple.fr".into(), public_port: 25570, mode: "proxy".into() });
        let g = generate(&cfg, &[srv(a, "Survie\nserver { listen 1; }", 25565)]);
        assert!(!g.nginx_stream.contains("\nserver { listen 1; }"), "un nom de serveur ne doit pas injecter de directive");
    }
}
