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
    // Serveur officiel Bedrock : pas de fichiers de plugin, mais des packs (voir bedrock_packs.rs).
    let Some(ext) = loader.addon_extension() else { return Ok(out) };
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(&format!(".{ext}")) || name.ends_with(&format!(".{ext}.disabled"))) {
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

/// Nom de fichier d'extension sans chemin : empêche `../` et les séparateurs.
fn plain_file_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name == "." || name == ".." || name.contains("..") {
        anyhow::bail!("nom de fichier invalide");
    }
    Ok(())
}

pub fn toggle_addon(server_folder: &Path, loader: Loader, file_name: &str) -> Result<()> {
    plain_file_name(file_name)?;
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
    plain_file_name(file_name)?;
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
        let mut f = std::fs::File::open(&source)?;
        let big = f.metadata().map(|m| m.len() >= u32::MAX as u64).unwrap_or(false);
        zip.start_file(name, options.large_file(big))?;
        std::io::copy(&mut f, &mut zip)?;
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
                let mut f = std::fs::File::open(path)?;
                let big = f.metadata().map(|m| m.len() >= u32::MAX as u64).unwrap_or(false);
                zip.start_file(rel_str, options.large_file(big))?;
                std::io::copy(&mut f, &mut zip)?;
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
    import_zip_limited(root, rel_dir, zip_bytes, MAX_ZIP_ENTRIES, MAX_ZIP_BYTES)
}

/// Garde-fous contre les « zip bombs » : nombre de fichiers et octets réellement écrits (les tailles déclarées dans
/// l'archive peuvent mentir, on compte donc ce qui sort du décompresseur).
pub const MAX_ZIP_ENTRIES: usize = 200_000;
pub const MAX_ZIP_BYTES: u64 = 64 * 1024 * 1024 * 1024;

pub fn import_zip_limited(root: &Path, rel_dir: &str, zip_bytes: &[u8], max_entries: usize, max_bytes: u64) -> Result<usize> {
    import_zip_reader(root, rel_dir, std::io::Cursor::new(zip_bytes), max_entries, max_bytes)
}

/// Extrait une archive qui se trouve déjà sur disque, sans la charger en mémoire (transferts de gros mondes).
pub fn import_zip_file(root: &Path, rel_dir: &str, zip_path: &Path) -> Result<usize> {
    let f = std::fs::File::open(zip_path).context("archive introuvable")?;
    import_zip_reader(root, rel_dir, std::io::BufReader::new(f), MAX_ZIP_ENTRIES, MAX_ZIP_BYTES)
}

pub fn import_zip_reader<R: std::io::Read + std::io::Seek>(root: &Path, rel_dir: &str, reader: R, max_entries: usize, max_bytes: u64) -> Result<usize> {
    use std::io::Read;
    // Le dossier racine est choisi par l'appelant (pas par l'archive) : on le crée avant de résoudre les chemins.
    std::fs::create_dir_all(root)?;
    let dest_root = safe_join(root, rel_dir)?;
    std::fs::create_dir_all(&dest_root)?;
    let mut archive = zip::ZipArchive::new(reader).context("archive .zip invalide")?;
    let mut extracted = 0usize;
    if archive.len() > max_entries {
        anyhow::bail!("archive refusee : {} fichiers (maximum {max_entries})", archive.len());
    }
    let mut written_total = 0u64;
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
        // Extraction en flux (pas de copie complète en mémoire).
        let mut out = std::fs::File::create(&target)?;
        // On ne copie jamais plus que le budget restant (+1 octet pour détecter le dépassement).
        let budget = max_bytes.saturating_sub(written_total);
        let copied = std::io::copy(&mut (&mut entry).take(budget + 1), &mut out)?;
        written_total += copied;
        if written_total > max_bytes {
            drop(out);
            let _ = std::fs::remove_file(&target);
            anyhow::bail!("archive refusee : plus de {} Mo decompresses (protection contre les archives piegees)", max_bytes / 1_048_576);
        }
        extracted += 1;
    }
    Ok(extracted)
}

#[cfg(test)]
mod zip_limit_tests {
    use super::*;
    use std::io::Write;

    fn zip_with(files: &[(&str, usize)]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, size) in files {
            z.start_file(*n, zip::write::FileOptions::default()).unwrap();
            z.write_all(&vec![b'a'; *size]).unwrap();
        }
        z.finish().unwrap().into_inner()
    }
    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-zip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn normal_archives_extract_and_oversized_ones_are_refused() {
        let d = tmp();
        assert_eq!(import_zip_limited(&d, "ok", &zip_with(&[("a.txt", 10), ("dir/b.txt", 20)]), 10, 1000).unwrap(), 2);
        assert_eq!(std::fs::read(d.join("ok/dir/b.txt")).unwrap().len(), 20);
        // Trop de fichiers.
        let e = import_zip_limited(&d, "n", &zip_with(&[("1", 1), ("2", 1), ("3", 1)]), 2, 1000).unwrap_err().to_string();
        assert!(e.contains("3 fichiers"), "{e}");
        // Trop d'octets : refusé, et le fichier fautif n'est pas laissé sur le disque.
        let e = import_zip_limited(&d, "big", &zip_with(&[("gros.bin", 5000)]), 10, 1000).unwrap_err().to_string();
        assert!(e.contains("piegees"), "{e}");
        assert!(!d.join("big/gros.bin").exists());
        // Le budget est cumulé sur l'ensemble des fichiers.
        assert!(import_zip_limited(&d, "cumul", &zip_with(&[("a", 600), ("b", 600)]), 10, 1000).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
