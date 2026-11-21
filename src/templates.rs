//! Modèles de serveur : un serveur configuré (réglages, `server.properties`, plugins/mods, configurations, éventuellement le monde)
//! devient un modèle réutilisable pour en créer d'autres en un clic — sans téléchargement : le modèle contient déjà le jar.
//!
//! Stockage : `<données>/templates/<id>/meta.json` + `files.zip`.
//! Ne sont jamais inclus : `logs/`, `crash-reports/`, `cache/`, les verrous de session ; le monde seulement à la demande.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{Loader, ServerEntry};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateMeta {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub loader: Loader,
    pub mc_version: String,
    #[serde(default)]
    pub loader_version: Option<String>,
    pub jar_name: String,
    pub xms_mb: u32,
    pub xmx_mb: u32,
    pub port: u16,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub aikar_flags: bool,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "five")]
    pub auto_restart_delay_secs: u32,
    #[serde(default)]
    pub auto_backup_minutes: Option<u32>,
    #[serde(default)]
    pub backup_retention: Option<u32>,
    pub include_world: bool,
    pub size_bytes: u64,
    pub source_server: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

fn five() -> u32 {
    5
}

pub fn templates_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("templates")
}

fn template_dir(data_dir: &Path, id: Uuid) -> PathBuf {
    templates_dir(data_dir).join(id.to_string())
}

pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 80 && !n.chars().any(|c| c.is_control())
}

/// Dossiers du monde d'après `level-name` (défaut « world »), avec Nether et End.
pub fn world_dirs(properties: &str) -> Vec<String> {
    let level = properties
        .lines()
        .find_map(|l| l.trim().strip_prefix("level-name=").map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty() && !v.contains(['/', '\\']) && v != "." && v != "..")
        .unwrap_or_else(|| "world".to_string());
    vec![level.clone(), format!("{level}_nether"), format!("{level}_the_end")]
}

/// Faut-il exclure ce chemin relatif (séparateur `/`) du modèle ?
pub fn excluded(rel: &str, worlds: &[String], include_world: bool) -> bool {
    let top = rel.split('/').next().unwrap_or("");
    if matches!(top, "logs" | "crash-reports" | "cache") || rel == "session.lock" || rel.ends_with("/session.lock") || rel == "usercache.json" {
        return true;
    }
    !include_world && worlds.iter().any(|w| w == top)
}

/// Archive le dossier du serveur en filtrant ce qui n'a pas sa place dans un modèle (bloquant).
fn zip_server(root: &Path, dest: &Path, include_world: bool) -> Result<u64> {
    use std::io::Write;
    let props = std::fs::read_to_string(root.join("server.properties")).unwrap_or_default();
    let worlds = world_dirs(&props);
    let file = std::fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let _ = &mut zip.flush();
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        let rel = e.path().strip_prefix(root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        rel.is_empty() || !excluded(&rel, &worlds, include_world)
    }) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(root)?.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            zip.add_directory(format!("{rel}/"), options)?;
        } else if entry.file_type().is_file() {
            let mut f = std::fs::File::open(entry.path())?;
            let big = f.metadata().map(|m| m.len() >= u32::MAX as u64).unwrap_or(false);
            zip.start_file(rel, options.large_file(big))?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }
    zip.finish()?;
    Ok(std::fs::metadata(dest)?.len())
}

pub async fn list(data_dir: &Path) -> Vec<TemplateMeta> {
    let mut out = Vec::new();
    let Ok(mut rd) = tokio::fs::read_dir(templates_dir(data_dir)).await else { return out };
    while let Ok(Some(e)) = rd.next_entry().await {
        if let Ok(s) = tokio::fs::read_to_string(e.path().join("meta.json")).await {
            if let Ok(m) = serde_json::from_str::<TemplateMeta>(&s) {
                out.push(m);
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub async fn create_from_server(state: &AppState, server_id: Uuid, name: &str, description: &str, include_world: bool) -> Result<TemplateMeta> {
    if !valid_name(name) {
        anyhow::bail!("nom de modèle invalide (1 à 80 caractères)");
    }
    let entry = state.servers.read().await.get(&server_id).cloned().context("serveur introuvable")?;
    if include_world && state.runtime.read().await.get(&server_id).map(|r| r.running).unwrap_or(false) {
        anyhow::bail!("arrêtez le serveur pour inclure son monde dans le modèle");
    }
    let id = Uuid::new_v4();
    let dir = template_dir(&state.data_dir, id);
    tokio::fs::create_dir_all(&dir).await?;
    let (root, dest) = (PathBuf::from(&entry.folder), dir.join("files.zip"));
    let size = match tokio::task::spawn_blocking(move || zip_server(&root, &dest, include_world)).await {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            anyhow::bail!("erreur interne: {e}");
        }
    };
    let meta = TemplateMeta {
        id, name: name.trim().to_string(), description: description.trim().chars().take(300).collect(), loader: entry.loader, mc_version: entry.mc_version.clone(),
        loader_version: entry.loader_version.clone(), jar_name: entry.jar_name.clone(), xms_mb: entry.xms_mb, xmx_mb: entry.xmx_mb, port: entry.port,
        extra_args: entry.extra_args.clone(), aikar_flags: entry.aikar_flags, auto_restart: entry.auto_restart, auto_restart_delay_secs: entry.auto_restart_delay_secs,
        auto_backup_minutes: entry.auto_backup_minutes, backup_retention: entry.backup_retention, include_world, size_bytes: size, source_server: entry.name.clone(),
        created_at: chrono::Utc::now(),
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)?).await?;
    Ok(meta)
}

pub async fn delete(data_dir: &Path, id: Uuid) -> Result<bool> {
    let dir = template_dir(data_dir, id);
    if !dir.is_dir() {
        return Ok(false);
    }
    tokio::fs::remove_dir_all(dir).await?;
    Ok(true)
}

/// Remplace (ou ajoute) `server-port` dans un `server.properties`.
pub fn set_server_port(props: &str, port: u16) -> String {
    let mut found = false;
    let mut lines: Vec<String> = props
        .lines()
        .map(|l| {
            if !l.trim_start().starts_with('#') && l.split_once('=').map(|(k, _)| k.trim() == "server-port").unwrap_or(false) {
                found = true;
                format!("server-port={port}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("server-port={port}"));
    }
    lines.join("\n") + "\n"
}

/// Crée un serveur depuis un modèle : extraction du contenu, port propre, EULA (acceptée avec le modèle d'origine).
pub async fn instantiate(state: &AppState, template_id: Uuid, name: &str, port: Option<u16>) -> Result<ServerEntry> {
    if !valid_name(name) {
        anyhow::bail!("nom de serveur invalide (1 à 80 caractères)");
    }
    let meta: TemplateMeta = serde_json::from_str(&tokio::fs::read_to_string(template_dir(&state.data_dir, template_id).join("meta.json")).await.context("modèle introuvable")?)?;
    let zip_path = template_dir(&state.data_dir, template_id).join("files.zip");
    let id = Uuid::new_v4();
    let folder = state.data_dir.join("servers").join(id.to_string());
    tokio::fs::create_dir_all(&folder).await?;
    let (f2, z2) = (folder.clone(), zip_path);
    if let Err(e) = tokio::task::spawn_blocking(move || crate::files::import_zip_file(&f2, "", &z2)).await.map_err(|e| anyhow::anyhow!("erreur interne: {e}")).and_then(|r| r) {
        let _ = tokio::fs::remove_dir_all(&folder).await;
        return Err(e);
    }
    // Un port libre par défaut : celui du modèle + le nombre de serveurs locaux déjà sur ce port.
    let port = match port {
        Some(p) if p > 0 => p,
        _ => {
            let used: std::collections::HashSet<u16> = state.servers.read().await.values().map(|e| e.port).collect();
            (meta.port..=u16::MAX).find(|p| !used.contains(p)).unwrap_or(meta.port)
        }
    };
    let props_path = folder.join("server.properties");
    let props = tokio::fs::read_to_string(&props_path).await.unwrap_or_default();
    tokio::fs::write(&props_path, set_server_port(&props, port)).await?;
    let java_path = state.config.read().await.java_path.clone();
    let entry = ServerEntry {
        id, name: name.trim().to_string(), loader: meta.loader, mc_version: meta.mc_version.clone(), loader_version: meta.loader_version.clone(), folder: folder.to_string_lossy().to_string(),
        jar_name: meta.jar_name.clone(), java_path, xms_mb: meta.xms_mb, xmx_mb: meta.xmx_mb, port, extra_args: meta.extra_args.clone(), eula_accepted: true,
        auto_backup_minutes: meta.auto_backup_minutes, backup_retention: meta.backup_retention, auto_restart: meta.auto_restart, auto_restart_delay_secs: meta.auto_restart_delay_secs,
        scheduled_restart_minutes: None, stop_when_empty_minutes: None, dynamic_server: false, aikar_flags: meta.aikar_flags, managed_addons: vec![], created_at: chrono::Utc::now(),
    };
    tokio::fs::write(folder.join("eula.txt"), "eula=true\n").await.ok();
    state.servers.write().await.insert(id, entry.clone());
    crate::state::save_servers(state).await?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-tpl-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn world_directories_follow_level_name() {
        assert_eq!(world_dirs(""), ["world", "world_nether", "world_the_end"]);
        assert_eq!(world_dirs("motd=x\nlevel-name=Survie\n"), ["Survie", "Survie_nether", "Survie_the_end"]);
        assert_eq!(world_dirs("level-name=../evil"), ["world", "world_nether", "world_the_end"], "nom dangereux ignoré");
        assert_eq!(world_dirs("level-name=\n")[0], "world");
    }

    #[test]
    fn exclusion_rules() {
        let w = world_dirs("");
        for out in ["logs/latest.log", "crash-reports/c.txt", "cache/x", "session.lock", "usercache.json"] {
            assert!(excluded(out, &w, true), "{out}");
        }
        assert!(excluded("world/level.dat", &w, false) && excluded("world_nether/DIM-1/x", &w, false));
        assert!(!excluded("world/level.dat", &w, true), "monde inclus à la demande");
        for keep in ["server.properties", "plugins/Essentials/config.yml", "paper.jar", "libraries/a.jar", "worldedit/schem.schem", "worldguard/x"] {
            assert!(!excluded(keep, &w, false), "{keep}");
        }
    }

    #[test]
    fn port_is_replaced_or_added() {
        assert_eq!(set_server_port("motd=a\nserver-port=25565\n# server-port=1\n", 25570), "motd=a\nserver-port=25570\n# server-port=1\n");
        assert_eq!(set_server_port("motd=a", 25571), "motd=a\nserver-port=25571\n");
    }

    #[tokio::test]
    async fn template_round_trip_without_world_then_with_world() {
        let d = tmp();
        let state = crate::state::build_state_for_test(&d).await;
        let id = Uuid::new_v4();
        let folder = d.join("servers").join(id.to_string());
        for (rel, data) in [("server.properties", "motd=Mon serveur\nserver-port=25565\nlevel-name=Survie\n"), ("paper.jar", "JAR"), ("plugins/Essentials/config.yml", "cfg"), ("logs/latest.log", "log"),
                            ("Survie/level.dat", "WORLD"), ("Survie_nether/DIM-1/r.mca", "NETHER"), ("crash-reports/c.txt", "crash"), ("session.lock", "x")] {
            let p = folder.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        }
        let mut e = crate::models::ServerEntry::test_entry(id, "Survie", &folder);
        e.jar_name = "paper.jar".into();
        e.xmx_mb = 4096;
        e.aikar_flags = true;
        e.auto_backup_minutes = Some(60);
        state.servers.write().await.insert(id, e);

        let meta = create_from_server(&state, id, "  Base survie  ", "Paper + Essentials", false).await.unwrap();
        assert_eq!((meta.name.as_str(), meta.xmx_mb, meta.include_world, meta.jar_name.as_str()), ("Base survie", 4096, false, "paper.jar"));
        assert_eq!(list(&d).await.len(), 1);
        assert!(create_from_server(&state, id, "   ", "", false).await.is_err());

        // Nouveau serveur depuis le modèle : fichiers de configuration oui, monde et journaux non, port décalé.
        let s1 = instantiate(&state, meta.id, "Copie 1", None).await.unwrap();
        let f1 = PathBuf::from(&s1.folder);
        assert_eq!(std::fs::read_to_string(f1.join("plugins/Essentials/config.yml")).unwrap(), "cfg");
        assert_eq!(std::fs::read_to_string(f1.join("paper.jar")).unwrap(), "JAR");
        assert!(!f1.join("Survie").exists() && !f1.join("logs").exists() && !f1.join("crash-reports").exists() && !f1.join("Survie_nether").exists());
        assert_eq!((s1.xmx_mb, s1.aikar_flags, s1.auto_backup_minutes, s1.jar_name.as_str(), s1.port), (4096, true, Some(60), "paper.jar", 25566), "réglages conservés, port libre suivant");
        assert!(std::fs::read_to_string(f1.join("server.properties")).unwrap().contains("server-port=25566"));
        assert_eq!(std::fs::read_to_string(f1.join("eula.txt")).unwrap(), "eula=true\n");
        let s2 = instantiate(&state, meta.id, "Copie 2", Some(25600)).await.unwrap();
        assert_eq!(s2.port, 25600);
        let s3 = instantiate(&state, meta.id, "Copie 3", None).await.unwrap();
        assert_eq!(s3.port, 25567, "25565 et 25566 sont pris, 25600 n'est pas sur le chemin");
        assert_eq!(state.servers.read().await.len(), 4);

        // Avec le monde : refusé tant que le serveur tourne, accepté à l'arrêt.
        let mut rt = crate::state::ServerRuntime::default();
        rt.running = true;
        state.runtime.write().await.insert(id, rt);
        assert!(create_from_server(&state, id, "Avec monde", "", true).await.unwrap_err().to_string().contains("arrêtez"));
        state.runtime.write().await.get_mut(&id).unwrap().running = false;
        let with_world = create_from_server(&state, id, "Avec monde", "", true).await.unwrap();
        let s4 = instantiate(&state, with_world.id, "Copie monde", None).await.unwrap();
        assert_eq!(std::fs::read_to_string(PathBuf::from(&s4.folder).join("Survie/level.dat")).unwrap(), "WORLD");
        assert!(!PathBuf::from(&s4.folder).join("session.lock").exists());

        // Suppression, et refus d'un modèle inconnu.
        assert!(delete(&d, meta.id).await.unwrap());
        assert!(!delete(&d, meta.id).await.unwrap());
        assert!(instantiate(&state, meta.id, "X", None).await.is_err());
        assert!(instantiate(&state, with_world.id, "", None).await.is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
