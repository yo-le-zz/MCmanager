use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::models::{AddonInfo, Loader};

/// Resolves `rel` inside `root`, refusing any path that escapes the root
/// (blocks `..` traversal, absolute-path overrides, symlink escapes).
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    // Reject absolute paths outright (Unix `/etc/passwd`, Windows `C:\...`
    // or `\Windows\...`) instead of relying only on the canonicalize-based
    // containment check below: `PathBuf::join` treats an absolute `rel` as
    // a full replacement of `root`, which is surprising and worth failing
    // fast on rather than trusting the walk-up-to-an-existing-ancestor
    // logic to always run before any caller acts on the path.
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() || rel.contains(':') || rel.starts_with('\\') {
        anyhow::bail!("chemin invalide (chemin absolu refuse)");
    }
    let rel = rel.trim_start_matches(['/', '\\']);
    let candidate = root.join(rel);
    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // The target may not exist yet (e.g. new file); canonicalize the closest existing ancestor.
    let mut check = candidate.clone();
    while !check.exists() {
        match check.parent() {
            Some(p) => check = p.to_path_buf(),
            None => break,
        }
    }
    let check_canon = std::fs::canonicalize(&check).unwrap_or(check);
    if !check_canon.starts_with(&root_canon) {
        anyhow::bail!("chemin invalide (hors du dossier du serveur)");
    }
    Ok(candidate)
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
}

pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<FileEntry>> {
    let dir = safe_join(root, rel)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).context("dossier introuvable")? {
        let entry = entry?;
        let meta = entry.metadata()?;
        out.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: meta.is_dir(),
            size_bytes: meta.len(),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

const MAX_EDITABLE_BYTES: u64 = 2 * 1024 * 1024;

pub fn read_text_file(root: &Path, rel: &str) -> Result<String> {
    let path = safe_join(root, rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_EDITABLE_BYTES {
        anyhow::bail!("fichier trop volumineux pour l'editeur (> 2 Mo)");
    }
    Ok(std::fs::read_to_string(path)?)
}

pub fn write_text_file(root: &Path, rel: &str, content: &str) -> Result<()> {
    let path = safe_join(root, rel)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

pub fn delete_path(root: &Path, rel: &str) -> Result<()> {
    let path = safe_join(root, rel)?;
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub async fn save_upload(root: &Path, rel_dir: &str, filename: &str, bytes: &[u8]) -> Result<()> {
    let dir = safe_join(root, rel_dir)?;
    tokio::fs::create_dir_all(&dir).await?;
    let dest = safe_join(&dir, filename)?;
    tokio::fs::write(dest, bytes).await?;
    Ok(())
}

/// Lists installed mods/plugins for a server (`.jar` = enabled, `.jar.disabled` = disabled).
pub fn list_addons(server_folder: &Path, loader: Loader) -> Result<Vec<AddonInfo>> {
    let dir = server_folder.join(loader.addon_dir());
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".jar") || name.ends_with(".jar.disabled")) {
            continue;
        }
        let meta = entry.metadata()?;
        out.push(AddonInfo {
            enabled: !name.ends_with(".disabled"),
            file_name: name,
            size_bytes: meta.len(),
            modrinth_project_id: None,
            modrinth_version_id: None,
        });
    }
    Ok(out)
}

pub fn toggle_addon(server_folder: &Path, loader: Loader, file_name: &str) -> Result<()> {
    let dir = server_folder.join(loader.addon_dir());
    let path = dir.join(file_name);
    if !path.exists() {
        anyhow::bail!("fichier introuvable");
    }
    if file_name.ends_with(".disabled") {
        let new_name = file_name.trim_end_matches(".disabled");
        std::fs::rename(&path, dir.join(new_name))?;
    } else {
        std::fs::rename(&path, dir.join(format!("{file_name}.disabled")))?;
    }
    Ok(())
}

pub fn delete_addon(server_folder: &Path, loader: Loader, file_name: &str) -> Result<()> {
    let dir = server_folder.join(loader.addon_dir());
    std::fs::remove_file(dir.join(file_name)).context("suppression impossible")?;
    Ok(())
}

/// Destination folder for WorldEdit/FastAsyncWorldEdit schematics.
pub fn schematics_dir(server_folder: &Path) -> PathBuf {
    let fawe = server_folder.join("plugins").join("FastAsyncWorldEdit").join("schematics");
    if fawe.exists() {
        return fawe;
    }
    server_folder.join("plugins").join("WorldEdit").join("schematics")
}

/// Zips `rel` (a file or a folder; empty string = the whole server folder)
/// into a fresh temp file and returns its path, for the caller to stream
/// back as a download. Written to disk rather than built in memory since
/// a world folder can be gigabytes - same reasoning as `backup.rs`.
pub fn export_zip(root: &Path, rel: &str) -> Result<PathBuf> {
    use std::io::{Read, Write};
    let source = safe_join(root, rel)?;
    if !source.exists() {
        anyhow::bail!("chemin introuvable");
    }
    let tmp = std::env::temp_dir().join(format!("mcmanager-export-{}.zip", uuid::Uuid::new_v4()));
    let file = std::fs::File::create(&tmp)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    if source.is_file() {
        let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".to_string());
        zip.start_file(name, options)?;
        let mut f = std::fs::File::open(&source)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        zip.write_all(&buf)?;
    } else {
        for entry in walkdir::WalkDir::new(&source).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let rel_entry = path.strip_prefix(&source).unwrap_or(path);
            let rel_str = rel_entry.to_string_lossy().replace('\\', "/");
            if rel_str.is_empty() {
                continue;
            }
            if path.is_dir() {
                zip.add_directory(format!("{rel_str}/"), options)?;
            } else {
                zip.start_file(rel_str, options)?;
                let mut f = std::fs::File::open(path)?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                zip.write_all(&buf)?;
            }
        }
    }
    zip.finish()?;
    Ok(tmp)
}

/// Extracts an uploaded zip archive into `rel_dir` inside `root`. Every
/// entry path is resolved via `safe_join` before being written, the same
/// path-traversal guard used everywhere else in this module - a malicious
/// zip with a `../../etc/cron.d/x` entry ("zip-slip") is rejected entry by
/// entry rather than trusted just because it arrived inside a zip.
pub fn import_zip(root: &Path, rel_dir: &str, zip_bytes: &[u8]) -> Result<usize> {
    use std::io::Read;
    let dest_root = safe_join(root, rel_dir)?;
    std::fs::create_dir_all(&dest_root)?;
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("archive .zip invalide")?;
    let mut extracted = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let entry_name = entry.name().to_string();
        // Reject anything that isn't a plain relative path up front - belt
        // and suspenders alongside safe_join's own checks below.
        if entry_name.contains("..") {
            continue;
        }
        let target = match safe_join(&dest_root, &entry_name) {
            Ok(p) => p,
            Err(_) => continue, // skip any entry that resolves outside the target dir
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&target, buf)?;
        extracted += 1;
    }
    Ok(extracted)
}
