//! Migration des données d'une version antérieure de MCManager.
//!
//! - Un fichier `schema.json` mémorise la version du format des données. Absent = données d'avant la 1.1.3 (schéma 1).
//! - Avant toute modification, les fichiers de configuration sont **copiés** dans `migration-backups/v<schéma>-<date>/`.
//! - `servers.json` est lu **serveur par serveur** : un serveur illisible (fichier édité à la main, type inconnu d'une version
//!   plus récente...) est mis de côté dans `servers.rejected.json` sans emporter les autres. Avant, une seule entrée invalide
//!   faisait disparaître *tous* les serveurs de la liste, et la sauvegarde suivante écrasait le fichier.
//! - Les champs apparus depuis les anciennes versions sont complétés avec leurs valeurs par défaut.
//! - La migration est **idempotente** : la relancer ne change rien.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::models::ServerEntry;

/// Version du format des données. 1 = jusqu'à MCManager 1.1.2 ; 2 = 1.1.3 (Bedrock, reprise, modèles, transferts).
pub const CURRENT_SCHEMA: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct SchemaFile {
    schema: u32,
    app_version: String,
    #[serde(default)]
    migrated_from: Option<u32>,
    #[serde(default)]
    migrated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub from: u32,
    pub to: u32,
    pub backup: Option<PathBuf>,
    pub servers: usize,
    pub rejected: usize,
    pub notes: Vec<String>,
}

fn schema_path(dir: &Path) -> PathBuf {
    dir.join("schema.json")
}

pub fn read_schema(dir: &Path) -> Option<u32> {
    std::fs::read_to_string(schema_path(dir)).ok().and_then(|s| serde_json::from_str::<SchemaFile>(&s).ok()).map(|s| s.schema)
}

/// Complète une entrée de serveur issue d'une ancienne version avec les valeurs par défaut des champs manquants.
pub fn repair_entry(v: &mut Value, id_hint: Option<&str>, data_dir: &Path) {
    let Some(o) = v.as_object_mut() else { return };
    let loader = o.get("loader").and_then(|l| l.as_str()).unwrap_or("paper").to_string();
    let bedrock = matches!(loader.as_str(), "bedrock" | "bedrockpreview" | "pocketmine" | "nukkit");
    if !o.contains_key("id") {
        if let Some(id) = id_hint {
            o.insert("id".into(), json!(id));
        }
    }
    let id = o.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
    let defaults: Vec<(&str, Value)> = vec![
        ("name", json!("Serveur")), ("mc_version", json!("")), ("loader_version", Value::Null),
        ("folder", json!(data_dir.join("servers").join(&id).to_string_lossy())), ("jar_name", json!("server.jar")),
        ("java_path", json!("java")), ("xms_mb", json!(1024)), ("xmx_mb", json!(2048)), ("port", json!(if bedrock { 19132 } else { 25565 })),
        ("extra_args", json!([])), ("eula_accepted", json!(false)), ("auto_backup_minutes", Value::Null), ("backup_retention", Value::Null),
        ("auto_restart", json!(false)), ("auto_restart_delay_secs", json!(5)), ("scheduled_restart_minutes", Value::Null),
        ("stop_when_empty_minutes", Value::Null), ("dynamic_server", json!(false)), ("aikar_flags", json!(false)),
        ("managed_addons", json!([])), ("created_at", json!(chrono::Utc::now())),
    ];
    for (k, d) in defaults {
        o.entry(k.to_string()).or_insert(d);
    }
}

/// Lit `servers.json` tolérant : les entrées valides d'un côté, les entrées illisibles (conservées telles quelles) de l'autre.
/// Accepte le format actuel (objet `{id: serveur}`) et un tableau de serveurs (très anciennes versions).
pub fn parse_servers(text: &str, data_dir: &Path) -> (HashMap<Uuid, ServerEntry>, Vec<Value>) {
    let mut ok = HashMap::new();
    let mut rejected = Vec::new();
    let Ok(root) = serde_json::from_str::<Value>(text) else { return (ok, rejected) };
    let entries: Vec<(Option<String>, Value)> = match root {
        Value::Object(m) => m.into_iter().map(|(k, v)| (Some(k), v)).collect(),
        Value::Array(a) => a.into_iter().map(|v| (None, v)).collect(),
        _ => vec![],
    };
    for (key, original) in entries {
        let mut v = original.clone();
        repair_entry(&mut v, key.as_deref(), data_dir);
        match serde_json::from_value::<ServerEntry>(v) {
            Ok(e) => {
                ok.insert(e.id, e);
            }
            Err(_) => rejected.push(json!({ "key": key, "entry": original })),
        }
    }
    (ok, rejected)
}

fn backup_dir(data_dir: &Path, from: u32) -> PathBuf {
    data_dir.join("migration-backups").join(format!("v{from}-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")))
}

/// Copie les fichiers de configuration du dossier de données (pas les serveurs, mondes ni sauvegardes).
fn backup_config(data_dir: &Path, dest: &Path) -> Result<usize> {
    std::fs::create_dir_all(dest)?;
    let mut n = 0;
    for e in std::fs::read_dir(data_dir)?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if p.is_file() && matches!(ext, "json" | "key" | "yml" | "yaml" | "toml") && e.metadata().map(|m| m.len() < 50_000_000).unwrap_or(false) && name != "schema.json" {
            std::fs::copy(&p, dest.join(&name))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Écrit `servers.json` de façon atomique.
fn write_servers(data_dir: &Path, servers: &HashMap<Uuid, ServerEntry>) -> Result<()> {
    let tmp = data_dir.join("servers.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(servers)?)?;
    std::fs::rename(&tmp, data_dir.join("servers.json"))?;
    Ok(())
}

/// À appeler au démarrage, avant de lire les serveurs. Idempotent.
pub async fn run(data_dir: &Path) -> Result<Report> {
    let dir = data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || run_blocking(&dir)).await.map_err(|e| anyhow::anyhow!("erreur interne: {e}"))?
}

pub fn run_blocking(data_dir: &Path) -> Result<Report> {
    let servers_file = data_dir.join("servers.json");
    let has_legacy_data = servers_file.exists() || data_dir.join("config.json").exists();
    let from = read_schema(data_dir).unwrap_or(1);
    let mut report = Report { from, to: CURRENT_SCHEMA, ..Default::default() };
    let text = std::fs::read_to_string(&servers_file).unwrap_or_default();
    let (servers, rejected) = parse_servers(&text, data_dir);
    report.servers = servers.len();
    report.rejected = rejected.len();

    // Quelque chose à faire : passage d'un ancien schéma sur des données existantes, ou entrées illisibles à écarter.
    let needs_migration = from < CURRENT_SCHEMA && has_legacy_data;
    if !needs_migration && rejected.is_empty() {
        if read_schema(data_dir).is_none() || from > CURRENT_SCHEMA {
            // Installation neuve (ou données d'une version plus récente : on n'y touche pas).
            if from <= CURRENT_SCHEMA {
                write_schema(data_dir, None)?;
            } else {
                report.notes.push(format!("Les données viennent d'une version plus récente de MCManager (schéma {from}) : elles sont lues telles quelles, sans modification."));
            }
        }
        return Ok(report);
    }

    if has_legacy_data {
        let dest = backup_dir(data_dir, from);
        let n = backup_config(data_dir, &dest).context("sauvegarde des anciens réglages")?;
        report.notes.push(format!("{n} fichier(s) de réglages copiés dans {}", dest.display()));
        report.backup = Some(dest);
    }
    if !rejected.is_empty() {
        // On ajoute aux entrées déjà écartées : rien n'est jamais perdu.
        let path = data_dir.join("servers.rejected.json");
        let mut all: Vec<Value> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        all.extend(rejected);
        std::fs::write(&path, serde_json::to_string_pretty(&all)?)?;
        report.notes.push(format!("{} serveur(s) illisible(s) mis de côté dans servers.rejected.json (ils ne sont pas supprimés)", report.rejected));
    }
    if !servers.is_empty() || servers_file.exists() {
        write_servers(data_dir, &servers)?;
    }
    if needs_migration {
        write_schema(data_dir, Some(from))?;
        report.notes.push(format!("Données migrées du schéma {from} au schéma {CURRENT_SCHEMA} ({} serveur(s))", report.servers));
    }
    Ok(report)
}

fn write_schema(data_dir: &Path, from: Option<u32>) -> Result<()> {
    let f = SchemaFile { schema: CURRENT_SCHEMA, app_version: env!("CARGO_PKG_VERSION").to_string(), migrated_from: from, migrated_at: from.map(|_| chrono::Utc::now()) };
    std::fs::write(schema_path(data_dir), serde_json::to_string_pretty(&f)?)?;
    Ok(())
}

/// Diagnostic lisible pour l'API (version du schéma, sauvegardes de migration, serveurs écartés).
pub fn status(data_dir: &Path) -> Value {
    let backups: Vec<String> = std::fs::read_dir(data_dir.join("migration-backups")).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    let rejected = std::fs::read_to_string(data_dir.join("servers.rejected.json")).ok().and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok()).map(|v| v.len()).unwrap_or(0);
    json!({ "schema": read_schema(data_dir), "current": CURRENT_SCHEMA, "backups": backups, "rejected_servers": rejected })
}

#[allow(dead_code)]
fn _map(_: Map<String, Value>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-migrate-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // Serveur au format de la 1.0.x : ni loader_version, ni aikar_flags, ni dynamic_server, ni managed_addons...
    fn legacy_servers(a: &str, b: &str) -> String {
        format!(r#"{{
            "{a}": {{ "id": "{a}", "name": "Survie", "loader": "paper", "mc_version": "1.20.4", "folder": "/srv/mc/survie", "jar_name": "paper.jar",
                      "java_path": "java", "xms_mb": 1024, "xmx_mb": 4096, "port": 25565, "eula_accepted": true }},
            "{b}": {{ "name": "Mods", "loader": "fabric", "mc_version": "1.21", "port": 25570 }}
        }}"#)
    }

    #[test]
    fn legacy_entries_are_completed_and_nothing_is_lost() {
        let d = tmp();
        let (a, b) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        let (ok, rej) = parse_servers(&legacy_servers(&a, &b), &d);
        assert!(rej.is_empty(), "{rej:?}");
        assert_eq!(ok.len(), 2);
        let s = ok.get(&a.parse().unwrap()).unwrap();
        assert_eq!((s.name.as_str(), s.xmx_mb, s.eula_accepted, s.jar_name.as_str(), s.aikar_flags, s.dynamic_server), ("Survie", 4096, true, "paper.jar", false, false));
        let m = ok.get(&b.parse().unwrap()).unwrap();
        assert_eq!((m.port, m.xmx_mb, m.java_path.as_str()), (25570, 2048, "java"), "défauts pour les champs absents, id repris de la clé");
        assert!(m.folder.contains("servers"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn one_bad_entry_does_not_take_the_others_down() {
        let d = tmp();
        let (a, b) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        let text = format!(r#"{{ "{a}": {{ "name": "Bon", "loader": "paper", "mc_version": "1.21" }},
                              "{b}": {{ "name": "Futur", "loader": "un-loader-de-la-version-9", "mc_version": "9.0" }},
                              "pas-un-uuid": {{ "name": "X", "loader": "paper" }} }}"#);
        let (ok, rej) = parse_servers(&text, &d);
        assert_eq!(ok.len(), 1);
        assert_eq!(rej.len(), 2);
        assert!(rej.iter().any(|r| r["entry"]["name"] == "Futur"), "l'entrée écartée est conservée intacte");
        // Format « tableau » des toutes premières versions.
        let arr = format!(r#"[ {{ "id": "{a}", "name": "Ancien", "loader": "vanilla", "mc_version": "1.12.2" }} ]"#);
        let (ok, _) = parse_servers(&arr, &d);
        assert_eq!(ok.len(), 1);
        assert!(parse_servers("pas du json", &d).0.is_empty() && parse_servers("42", &d).0.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn migration_backs_up_rewrites_stamps_and_is_idempotent() {
        let d = tmp();
        let (a, b) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        std::fs::write(d.join("servers.json"), legacy_servers(&a, &b)).unwrap();
        std::fs::write(d.join("config.json"), r#"{"java_path":"/usr/bin/java"}"#).unwrap();
        std::fs::write(d.join("secret.key"), "K").unwrap();
        std::fs::create_dir_all(d.join("servers/xyz")).unwrap();
        std::fs::write(d.join("servers/xyz/world.dat"), "monde").unwrap();

        let r = run_blocking(&d).unwrap();
        assert_eq!((r.from, r.to, r.servers, r.rejected), (1, CURRENT_SCHEMA, 2, 0));
        let backup = r.backup.clone().expect("sauvegarde");
        assert!(backup.join("servers.json").is_file() && backup.join("config.json").is_file() && backup.join("secret.key").is_file());
        assert_eq!(std::fs::read_to_string(backup.join("servers.json")).unwrap(), legacy_servers(&a, &b), "l'original est copié tel quel");
        assert!(!backup.join("world.dat").exists() && !backup.join("servers").exists(), "pas de copie des mondes");
        assert_eq!(read_schema(&d), Some(CURRENT_SCHEMA));
        // servers.json est maintenant complet et relisible sans rien écarter.
        let (ok, rej) = parse_servers(&std::fs::read_to_string(d.join("servers.json")).unwrap(), &d);
        assert_eq!((ok.len(), rej.len()), (2, 0));
        assert!(std::fs::read_to_string(d.join("servers.json")).unwrap().contains("aikar_flags"));
        assert_eq!(std::fs::read_to_string(d.join("servers/xyz/world.dat")).unwrap(), "monde");

        // Deuxième exécution : rien ne change, pas de nouvelle sauvegarde.
        let before = std::fs::read_to_string(d.join("servers.json")).unwrap();
        let r2 = run_blocking(&d).unwrap();
        assert!(r2.backup.is_none() && r2.notes.is_empty(), "{r2:?}");
        assert_eq!(std::fs::read_to_string(d.join("servers.json")).unwrap(), before);
        assert_eq!(std::fs::read_dir(d.join("migration-backups")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejected_servers_are_set_aside_not_deleted() {
        let d = tmp();
        let (a, b) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        std::fs::write(d.join("servers.json"), format!(r#"{{ "{a}": {{ "name": "Bon", "loader": "paper", "mc_version": "1.21" }}, "{b}": {{ "name": "Inconnu", "loader": "zzz" }} }}"#)).unwrap();
        let r = run_blocking(&d).unwrap();
        assert_eq!((r.servers, r.rejected), (1, 1));
        let rej: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(d.join("servers.rejected.json")).unwrap()).unwrap();
        assert_eq!(rej.len(), 1);
        assert_eq!(rej[0]["entry"]["name"], "Inconnu");
        assert_eq!(parse_servers(&std::fs::read_to_string(d.join("servers.json")).unwrap(), &d).0.len(), 1);
        assert_eq!(status(&d)["rejected_servers"], 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fresh_install_and_newer_data() {
        let d = tmp();
        let r = run_blocking(&d).unwrap();
        assert!(r.backup.is_none() && r.notes.is_empty());
        assert_eq!(read_schema(&d), Some(CURRENT_SCHEMA), "installation neuve : schéma courant, aucune sauvegarde inutile");
        assert!(!d.join("migration-backups").exists());
        // Données d'une version plus récente : lues, jamais modifiées.
        let d2 = tmp();
        std::fs::write(d2.join("schema.json"), r#"{"schema": 99, "app_version": "9.9.9"}"#).unwrap();
        std::fs::write(d2.join("servers.json"), "{}").unwrap();
        let r2 = run_blocking(&d2).unwrap();
        assert!(r2.notes[0].contains("plus récente"));
        assert_eq!(read_schema(&d2), Some(99));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

    #[test]
    fn bedrock_entries_default_to_the_bedrock_port() {
        let d = tmp();
        let id = Uuid::new_v4().to_string();
        let (ok, _) = parse_servers(&format!(r#"{{ "{id}": {{ "name": "B", "loader": "bedrock", "mc_version": "1.21.50.07" }} }}"#), &d);
        assert_eq!(ok.values().next().unwrap().port, 19132);
        let _ = std::fs::remove_dir_all(&d);
    }
}
