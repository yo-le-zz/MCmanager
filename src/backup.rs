use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use walkdir::WalkDir;
use zip::write::FileOptions;

use crate::models::BackupInfo;

pub fn backups_dir(data_dir: &Path, server_id: &uuid::Uuid) -> PathBuf {
    data_dir.join("backups").join(server_id.to_string())
}

/// Counts folder entries the same way `create_backup_tracked` walks them, so
/// the progress percentage shown in the UI lines up with reality.
pub fn count_entries(server_folder: &Path) -> u64 {
    WalkDir::new(server_folder).into_iter().filter_map(|e| e.ok()).count() as u64
}

pub fn create_backup(data_dir: &Path, server_id: &uuid::Uuid, server_folder: &Path) -> Result<String> {
    create_backup_tracked(data_dir, server_id, server_folder, Arc::new(AtomicU64::new(0)))
}

pub fn create_backup_tracked(
    data_dir: &Path,
    server_id: &uuid::Uuid,
    server_folder: &Path,
    progress: Arc<AtomicU64>,
) -> Result<String> {
    create_backup_limited(data_dir, server_id, server_folder, progress, None)
}

/// Comme `create_backup_tracked`, avec des longueurs exactes à copier pour certains fichiers de `worlds/`.
/// Sert à la sauvegarde « à chaud » d'un serveur Bedrock : après `save hold`, les fichiers de la base continuent de
/// grossir et il faut s'arrêter à la longueur annoncée par `save query` pour obtenir une copie cohérente.
pub fn create_backup_limited(
    data_dir: &Path,
    server_id: &uuid::Uuid,
    server_folder: &Path,
    progress: Arc<AtomicU64>,
    limits: Option<&std::collections::HashMap<String, u64>>,
) -> Result<String> {
    let dir = backups_dir(data_dir, server_id);
    std::fs::create_dir_all(&dir)?;
    let name = format!("backup-{}.zip", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let dest = dir.join(&name);
    // L'archive est écrite sous « .part » puis renommée une fois complète : une sauvegarde en cours ou interrompue
    // n'apparaît jamais dans la liste (elle ne peut être ni restaurée, ni envoyée à une autre machine à moitié écrite).
    let part = dir.join(format!("{name}.part"));
    struct PartGuard(PathBuf, bool);
    impl Drop for PartGuard {
        fn drop(&mut self) {
            if !self.1 {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
    let mut guard = PartGuard(part.clone(), false);
    let file = File::create(&part)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: FileOptions = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(server_folder).into_iter().filter_map(|e| e.ok()) {
        progress.fetch_add(1, Ordering::Relaxed);
        let path = entry.path();
        let rel = path.strip_prefix(server_folder).unwrap_or(path);
        // zip entries must use forward slashes regardless of the host OS
        // (the zip spec requires it; Windows would otherwise emit backslashes
        // here, breaking extraction on other platforms).
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        // Skip huge/regenerable junk to keep backups fast and small.
        if rel_str.contains("logs/") || rel_str.ends_with(".log") || rel_str.contains("crash-reports") {
            continue;
        }
        if path.is_dir() {
            zip.add_directory(format!("{rel_str}/"), options)?;
        } else {
            // Copie en flux : un monde Minecraft peut peser plusieurs Go, on ne le charge jamais en mémoire.
            let mut f = File::open(path)?;
            let limit = limits.and_then(|m| rel_str.strip_prefix("worlds/").and_then(|r| m.get(r))).copied();
            let len = limit.unwrap_or_else(|| f.metadata().map(|m| m.len()).unwrap_or(0));
            zip.start_file(rel_str, options.large_file(len >= u32::MAX as u64))?;
            match limit {
                Some(n) => std::io::copy(&mut std::io::Read::take(&mut f, n), &mut zip)?,
                None => std::io::copy(&mut f, &mut zip)?,
            };
        }
    }
    zip.finish()?;
    std::fs::rename(&part, &dest)?;
    guard.1 = true;
    Ok(name)
}

pub fn list_backups(data_dir: &Path, server_id: &uuid::Uuid) -> Result<Vec<BackupInfo>> {
    let dir = backups_dir(data_dir, server_id);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() || !entry.file_name().to_string_lossy().ends_with(".zip") {
            continue; // « .part » = sauvegarde en cours d'écriture
        }
        let created: chrono::DateTime<chrono::Utc> = meta.modified().ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
            .unwrap_or_else(chrono::Utc::now);
        out.push(BackupInfo {
            name: entry.file_name().to_string_lossy().to_string(),
            size_bytes: meta.len(),
            created_at: created,
        });
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

pub fn restore_backup(data_dir: &Path, server_id: &uuid::Uuid, name: &str, server_folder: &Path) -> Result<()> {
    let dir = backups_dir(data_dir, server_id);
    let zip_path = dir.join(name);
    if !zip_path.exists() {
        anyhow::bail!("sauvegarde introuvable");
    }
    // Clear current server folder contents (best-effort) before restoring.
    if server_folder.exists() {
        for entry in std::fs::read_dir(server_folder)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                std::fs::remove_dir_all(&p).ok();
            } else {
                std::fs::remove_file(&p).ok();
            }
        }
    } else {
        std::fs::create_dir_all(server_folder)?;
    }

    let file = File::open(&zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out_path = server_folder.join(entry.mangled_name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }
    Ok(())
}

pub fn delete_backup(data_dir: &Path, server_id: &uuid::Uuid, name: &str) -> Result<()> {
    let dir = backups_dir(data_dir, server_id);
    let path = dir.join(name);
    std::fs::remove_file(path).context("suppression impossible")?;
    Ok(())
}

/// Deletes the oldest backups beyond `keep` (by creation time, newest
/// first - same ordering as `list_backups`). Called right after a new
/// backup finishes, from both the manual (`api.rs`) and scheduled
/// (`main.rs`) backup paths via `after_backup_created` below, so the two
/// don't duplicate this logic.
pub fn apply_retention(data_dir: &Path, server_id: &uuid::Uuid, keep: u32) -> Result<Vec<String>> {
    let backups = list_backups(data_dir, server_id)?;
    let mut deleted = Vec::new();
    for old in backups.into_iter().skip(keep as usize) {
        if delete_backup(data_dir, server_id, &old.name).is_ok() {
            deleted.push(old.name);
        }
    }
    Ok(deleted)
}

/// Shared tail end of "a backup just finished successfully": prune old
/// backups per the server's retention setting (if any), then send a
/// notification. Kept in one place so the manual-backup and
/// scheduled-auto-backup code paths can't quietly drift apart.
pub async fn after_backup_created(state: &crate::state::AppState, id: uuid::Uuid, backup_name: &str) {
    let entry = state.servers.read().await.get(&id).cloned();
    let Some(entry) = entry else { return };

    if let Some(keep) = entry.backup_retention {
        let data_dir = state.data_dir.clone();
        let deleted = tokio::task::spawn_blocking(move || apply_retention(&data_dir, &id, keep)).await
            .unwrap_or(Ok(Vec::new()))
            .unwrap_or_default();
        if !deleted.is_empty() {
            tracing::info!("Retention: {} ancienne(s) sauvegarde(s) supprimee(s) pour {id}", deleted.len());
        }
    }

    // Copie automatique vers une autre machine si un miroir est configuré pour ce serveur (en arrière-plan).
    crate::transfer::mirror_backup(state, id, backup_name).await;

    crate::discord::notify(state, crate::discord::Event::Backup, Some(id), &entry.name, &format!("Nouvelle sauvegarde créée : `{backup_name}`."), &format!("New backup created: `{backup_name}`.")).await;
    crate::ntfy::notify(
        &state.http, &state.data_dir, crate::ntfy::Event::Backup,
        &format!("💾 {} : sauvegarde terminee", entry.name),
        &format!("Nouvelle sauvegarde creee : {backup_name}."),
    ).await;
}

#[cfg(test)]
mod partial_tests {
    use super::*;

    #[test]
    fn backup_is_listed_only_when_complete_and_failures_leave_nothing() {
        let d = std::env::temp_dir().join(format!("mcm-bk-{}", uuid::Uuid::new_v4()));
        let folder = d.join("srv");
        std::fs::create_dir_all(folder.join("world")).unwrap();
        std::fs::write(folder.join("world/level.dat"), "L").unwrap();
        let id = uuid::Uuid::new_v4();
        let name = create_backup_tracked(&d, &id, &folder, Arc::new(AtomicU64::new(0))).unwrap();
        let dir = backups_dir(&d, &id);
        assert!(dir.join(&name).is_file() && !dir.join(format!("{name}.part")).exists(), "pas de fichier .part après une sauvegarde réussie");
        // Un « .part » abandonné (arrêt brutal) n'apparaît pas dans la liste.
        std::fs::write(dir.join("backup-20200101-000000.zip.part"), "incomplet").unwrap();
        let list = list_backups(&d, &id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, name);
        // Dossier source illisible : l'erreur ne laisse aucun fichier partiel.
        let err = create_backup_tracked(&d, &id, &d.join("absent"), Arc::new(AtomicU64::new(0)));
        let after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".part") && !n.starts_with("backup-2020")).collect();
        assert!(after.is_empty(), "{after:?} ({err:?})");
        let _ = std::fs::remove_dir_all(&d);
    }
}
