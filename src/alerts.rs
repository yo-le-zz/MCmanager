//! Alertes du tableau de bord : ce qui mérite l'attention de l'administrateur, calculé à partir d'un instantané.
//!
//! `evaluate` est pur (instantané en entrée, alertes en sortie) : toutes les règles sont testées sans disque ni réseau.
//! `gather` construit l'instantané depuis l'état réel de l'application.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

pub const BACKUP_MAX_AGE_DAYS: i64 = 7;
pub const DISK_WARN_PCT: f64 = 20.0;
pub const DISK_CRITICAL_PCT: f64 = 10.0;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Alert {
    /// Identifiant stable (type + serveur) : permet à l'interface de mémoriser une alerte masquée.
    pub id: String,
    pub level: Level,
    pub server_id: Option<Uuid>,
    pub fr: String,
    pub en: String,
}

#[derive(Debug, Clone)]
pub struct ServerFacts {
    pub id: Uuid,
    pub name: String,
    pub eula_accepted: bool,
    pub auto_backup: bool,
    pub last_backup: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub now: DateTime<Utc>,
    pub servers: Vec<ServerFacts>,
    /// Part d'espace libre (en %) du disque qui porte les données.
    pub disk_free_pct: Option<f64>,
    pub discord_enabled: bool,
    pub discord_state: String,
}

pub fn evaluate(s: &Snapshot) -> Vec<Alert> {
    let mut out = Vec::new();
    if let Some(p) = s.disk_free_pct {
        if p < DISK_CRITICAL_PCT {
            out.push(Alert { id: "disk".into(), level: Level::Critical, server_id: None, fr: format!("Espace disque presque épuisé : {p:.0} % libre. Les sauvegardes et le monde Minecraft risquent d'échouer."), en: format!("Disk almost full: {p:.0}% free. Backups and the Minecraft world may fail.") });
        } else if p < DISK_WARN_PCT {
            out.push(Alert { id: "disk".into(), level: Level::Warning, server_id: None, fr: format!("Espace disque bas : {p:.0} % libre."), en: format!("Low disk space: {p:.0}% free.") });
        }
    }
    for sv in &s.servers {
        if !sv.eula_accepted {
            out.push(Alert { id: format!("eula:{}", sv.id), level: Level::Warning, server_id: Some(sv.id), fr: format!("« {} » : l'EULA de Minecraft n'est pas acceptée, le serveur ne démarrera pas.", sv.name), en: format!("“{}”: the Minecraft EULA is not accepted, the server will not start.", sv.name) });
        }
        match sv.last_backup {
            None if !sv.auto_backup => out.push(Alert { id: format!("nobackup:{}", sv.id), level: Level::Info, server_id: Some(sv.id), fr: format!("« {} » n'a jamais été sauvegardé et aucune sauvegarde automatique n'est planifiée.", sv.name), en: format!("“{}” has never been backed up and no automatic backup is scheduled.", sv.name) }),
            Some(t) if !sv.auto_backup && s.now - t > Duration::days(BACKUP_MAX_AGE_DAYS) => {
                let days = (s.now - t).num_days();
                out.push(Alert { id: format!("oldbackup:{}", sv.id), level: Level::Warning, server_id: Some(sv.id), fr: format!("« {} » : dernière sauvegarde il y a {days} jours et aucune sauvegarde automatique.", sv.name), en: format!("“{}”: last backup {days} days ago and no automatic backup.", sv.name) });
            }
            _ => {}
        }
    }
    if s.discord_enabled && s.discord_state == "error" {
        out.push(Alert { id: "discord".into(), level: Level::Warning, server_id: None, fr: "Le bot Discord est activé mais ne fonctionne pas (voir Réglages → Discord).".into(), en: "The Discord bot is enabled but not working (see Settings → Discord).".into() });
    }
    // Les plus graves d'abord, puis par identifiant : l'ordre ne bouge pas d'une actualisation à l'autre.
    out.sort_by(|a, b| (a.level as u8).cmp(&(b.level as u8)).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Espace libre du disque qui contient `path`, en pourcentage.
pub fn disk_free_pct(path: &Path) -> Option<f64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // Le disque dont le point de montage est le plus long préfixe du chemin.
    let disk = disks.iter().filter(|d| canon.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len())?;
    (disk.total_space() > 0).then(|| disk.available_space() as f64 * 100.0 / disk.total_space() as f64)
}

pub async fn gather(state: &AppState) -> Snapshot {
    let entries: Vec<_> = state.servers.read().await.values().cloned().collect();
    let mut servers = Vec::new();
    for e in entries {
        let last_backup = crate::backup::list_backups(&state.data_dir, &e.id).ok().and_then(|l| l.iter().map(|b| b.created_at).max());
        servers.push(ServerFacts { id: e.id, name: e.name.clone(), eula_accepted: e.eula_accepted, auto_backup: e.auto_backup_minutes.is_some(), last_backup });
    }
    servers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let dir = state.data_dir.clone();
    let disk = tokio::task::spawn_blocking(move || disk_free_pct(&dir)).await.ok().flatten();
    let cfg = crate::discord::load_config(&state.data_dir).await;
    Snapshot { now: Utc::now(), servers, disk_free_pct: disk, discord_enabled: cfg.enabled, discord_state: crate::discord::status().state }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        Snapshot { now: Utc::now(), servers: vec![], disk_free_pct: Some(60.0), discord_enabled: false, discord_state: "disabled".into() }
    }
    fn server(name: &str, eula: bool, auto: bool, last: Option<Duration>) -> ServerFacts {
        ServerFacts { id: Uuid::new_v4(), name: name.into(), eula_accepted: eula, auto_backup: auto, last_backup: last.map(|d| Utc::now() - d) }
    }

    #[test]
    fn healthy_installation_has_no_alerts() {
        let mut s = snap();
        s.servers = vec![server("Survie", true, true, Some(Duration::hours(3))), server("Créatif", true, false, Some(Duration::days(2)))];
        assert!(evaluate(&s).is_empty());
    }

    #[test]
    fn disk_thresholds() {
        let mut s = snap();
        s.disk_free_pct = Some(15.0);
        let a = evaluate(&s);
        assert_eq!((a.len(), a[0].level, a[0].id.as_str()), (1, Level::Warning, "disk"));
        s.disk_free_pct = Some(4.2);
        let a = evaluate(&s);
        assert_eq!(a[0].level, Level::Critical);
        assert!(a[0].fr.contains("4 %") && a[0].en.contains("4%"));
        s.disk_free_pct = None;
        assert!(evaluate(&s).is_empty(), "disque inconnu : pas de fausse alerte");
    }

    #[test]
    fn backup_rules_depend_on_automation_and_age() {
        let mut s = snap();
        s.servers = vec![
            server("Vieux", true, false, Some(Duration::days(10) + Duration::hours(1))),
            server("Jamais", true, false, None),
            server("AutoJamais", true, true, None),
            server("AutoVieux", true, true, Some(Duration::days(30))),
            server("Recent", true, false, Some(Duration::days(6))),
        ];
        let a = evaluate(&s);
        let ids: Vec<&str> = a.iter().map(|x| x.id.split(':').next().unwrap()).collect();
        assert_eq!(ids, ["oldbackup", "nobackup"], "{a:?}");
        assert!(a[0].fr.contains("10 jours") && a[0].fr.contains("Vieux"));
        assert_eq!(a[1].level, Level::Info);
    }

    #[test]
    fn eula_discord_and_ordering() {
        let mut s = snap();
        s.disk_free_pct = Some(5.0);
        s.discord_enabled = true;
        s.discord_state = "error".into();
        s.servers = vec![server("Nouveau", false, true, Some(Duration::hours(1)))];
        let a = evaluate(&s);
        assert_eq!(a.iter().map(|x| x.level).collect::<Vec<_>>(), [Level::Critical, Level::Warning, Level::Warning]);
        assert_eq!(a[0].id, "disk", "le plus grave d'abord");
        assert!(a.iter().any(|x| x.id == "discord") && a.iter().any(|x| x.id.starts_with("eula:") && x.server_id.is_some()));
        s.discord_state = "connected".into();
        assert!(!evaluate(&s).iter().any(|x| x.id == "discord"));
        s.discord_enabled = false;
        s.discord_state = "error".into();
        assert!(!evaluate(&s).iter().any(|x| x.id == "discord"), "bot désactivé : aucune alerte");
    }

    #[test]
    fn disk_probe_returns_a_sane_percentage_for_an_existing_path() {
        if let Some(p) = disk_free_pct(&std::env::temp_dir()) {
            assert!((0.0..=100.0).contains(&p), "{p}");
        }
        assert!(disk_free_pct(Path::new("/chemin/qui/n/existe/pas")).map(|p| (0.0..=100.0).contains(&p)).unwrap_or(true));
    }
}
