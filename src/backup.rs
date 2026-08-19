use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use walkdir::WalkDir;
use zip::write::FileOptions;

use crate::models::BackupInfo;

fn backups_dir(data_dir: &Path, server_id: &uuid::Uuid) -> PathBuf {
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
    let dir = backups_dir(data_dir, server_id);
    std::fs::create_dir_all(&dir)?;
    let name = format!("backup-{}.zip", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let dest = dir.join(&name);
    let file = File::create(&dest)?;
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
            zip.start_file(rel_str, options)?;
            let mut f = File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            zip.write_all(&buf)?;
        }
    }
    zip.finish()?;
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
        if !meta.is_file() {
            continue;
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
