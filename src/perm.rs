//! Droits d'accès par serveur (mode serveur, avec connexion).
//!
//! Chaque compte non administrateur a, pour chaque serveur Minecraft, un niveau
//! d'accès (ou aucun) :
//!
//! | Niveau | Peut |
//! |---|---|
//! | `view` | voir le serveur, son état, l'historique, la console (lecture seule), la liste des mods/sauvegardes |
//! | `console` | + envoyer des commandes à la console, vider la console, diagnostic de crash |
//! | `control` | + démarrer, arrêter, tuer le processus |
//! | `manage` | + fichiers, mods/plugins, sauvegardes, propriétés, schematics, réglages du serveur |
//!
//! Les administrateurs ont tout, y compris ce qui est global : créer ou supprimer un
//! serveur, réglages, Discord, domaines, jumelage de machines, comptes, mises à jour.
//!
//! Un compte a un niveau **par défaut** (valable pour tous les serveurs, y compris ceux
//! créés plus tard) et des niveaux **par serveur** qui le remplacent (pour monter *ou*
//! descendre : « manage partout sauf view sur le serveur Survie »).
//!
//! La classification ci-dessous est **à refus par défaut** : une route non listée est
//! réservée aux administrateurs (ou exige `manage` si elle porte sur un serveur), donc une
//! route ajoutée plus tard n'est jamais ouverte par oubli.

use axum::http::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    View = 1,
    Console = 2,
    Control = 3,
    Manage = 4,
}

impl Level {
    pub fn label(&self) -> &'static str {
        match self {
            Level::View => "view",
            Level::Console => "console",
            Level::Control => "control",
            Level::Manage => "manage",
        }
    }
}

/// Ce qu'exige une requête.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Tout compte connecté (lecture de catalogues publics : versions, marketplace...).
    Anyone,
    /// Réservé aux administrateurs.
    Admin,
    /// La liste des serveurs : accessible, mais filtrée selon les droits du compte.
    List,
    /// Un niveau minimum sur un serveur précis.
    Server(Uuid, Level),
}

pub fn classify(method: &Method, path: &str) -> Need {
    let p = path.trim_end_matches('/');
    if p == "/api/servers" {
        return if *method == Method::GET { Need::List } else { Need::Admin };
    }
    if p == "/api/servers/import" || p == "/api/servers/import-zip" {
        return Need::Admin;
    }
    if let Some(rest) = p.strip_prefix("/api/servers/") {
        let mut parts = rest.splitn(2, '/');
        let Ok(id) = Uuid::parse_str(parts.next().unwrap_or("")) else { return Need::Admin };
        let sub = parts.next().unwrap_or("");
        let read = *method == Method::GET;
        let level = match sub {
            "" if read => Level::View,
            "" if *method == Method::DELETE => return Need::Admin,
            "" => Level::Manage,
            "status" | "history" | "ws" if read => Level::View,
            "addons" | "managed-addons" | "backups" | "backups/progress" | "schematics" if read => Level::View,
            "command" | "console/clear" | "debug/crash-diagnostic" => Level::Console,
            "start" | "stop" | "kill" => Level::Control,
            _ => Level::Manage,
        };
        return Need::Server(id, level);
    }
    let public_read = *method == Method::GET && (p == "/api/version" || p == "/api/presets" || p.starts_with("/api/loaders/") || p.starts_with("/api/marketplace/"));
    if public_read {
        return Need::Anyone;
    }
    Need::Admin
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Uuid {
        Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap()
    }
    fn c(m: Method, p: &str) -> Need {
        classify(&m, p)
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::View < Level::Console && Level::Console < Level::Control && Level::Control < Level::Manage);
    }

    #[test]
    fn server_routes_map_to_the_right_level() {
        let base = format!("/api/servers/{}", id());
        let s = |l| Need::Server(id(), l);
        assert_eq!(c(Method::GET, &base), s(Level::View));
        assert_eq!(c(Method::PUT, &base), s(Level::Manage));
        assert_eq!(c(Method::DELETE, &base), Need::Admin, "supprimer un serveur : administrateurs seulement");
        assert_eq!(c(Method::GET, &format!("{base}/status")), s(Level::View));
        assert_eq!(c(Method::GET, &format!("{base}/ws")), s(Level::View));
        assert_eq!(c(Method::GET, &format!("{base}/history")), s(Level::View));
        assert_eq!(c(Method::GET, &format!("{base}/addons")), s(Level::View));
        assert_eq!(c(Method::GET, &format!("{base}/backups")), s(Level::View));
        assert_eq!(c(Method::POST, &format!("{base}/command")), s(Level::Console));
        assert_eq!(c(Method::POST, &format!("{base}/console/clear")), s(Level::Console));
        assert_eq!(c(Method::POST, &format!("{base}/start")), s(Level::Control));
        assert_eq!(c(Method::POST, &format!("{base}/stop")), s(Level::Control));
        assert_eq!(c(Method::POST, &format!("{base}/kill")), s(Level::Control));
        // Tout ce qui touche aux fichiers ou à la configuration : manage (même en lecture : server.properties contient des secrets).
        for p in ["files", "files/content", "files/export", "files/upload", "files/import", "backups/x", "backups/x/restore", "java/test", "open-folder", "schematics/a.schem", "addons/a.jar", "addons/a.jar/toggle", "managed-addons/sync", "marketplace/install", "presets/k/install"] {
            assert_eq!(c(Method::GET, &format!("{base}/{p}")), s(Level::Manage), "GET {p}");
            assert_eq!(c(Method::POST, &format!("{base}/{p}")), s(Level::Manage), "POST {p}");
        }
        // Écriture sur les listes en lecture : manage.
        assert_eq!(c(Method::POST, &format!("{base}/addons")), s(Level::Manage));
        assert_eq!(c(Method::POST, &format!("{base}/backups")), s(Level::Manage));
        // Route inconnue sur un serveur : manage (refus par défaut).
        assert_eq!(c(Method::GET, &format!("{base}/nouvelle-route")), s(Level::Manage));
    }

    #[test]
    fn global_routes_are_admin_only_by_default() {
        assert_eq!(c(Method::GET, "/api/servers"), Need::List);
        assert_eq!(c(Method::POST, "/api/servers"), Need::Admin);
        assert_eq!(c(Method::POST, "/api/servers/import"), Need::Admin);
        assert_eq!(c(Method::POST, "/api/servers/import-zip"), Need::Admin);
        for p in ["/api/settings", "/api/ai/chat", "/api/ai/config", "/api/discord/config", "/api/network", "/api/ntfy/config", "/api/playit/start", "/api/playit/ws", "/api/remote/hub", "/api/remote/targets", "/api/update/apply", "/api/update/check", "/api/server/info", "/api/audit", "/api/inconnue", "/api/servers/pas-un-uuid/status"] {
            assert_eq!(c(Method::GET, p), Need::Admin, "{p}");
            assert_eq!(c(Method::POST, p), Need::Admin, "{p}");
        }
        for p in ["/api/version", "/api/presets", "/api/loaders/paper/versions", "/api/marketplace/search"] {
            assert_eq!(c(Method::GET, p), Need::Anyone, "{p}");
        }
        // Les catalogues publics ne sont publics qu'en lecture.
        assert_eq!(c(Method::POST, "/api/marketplace/search"), Need::Admin);
        // Slash final et casse des méthodes sans importance.
        assert_eq!(c(Method::GET, &format!("/api/servers/{}/", id())), Need::Server(id(), Level::View));
    }
}
