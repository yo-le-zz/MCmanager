use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How MCManager ended up on disk. Determines whether replacing the running
/// binary in place is safe (portable build) or would fight the system that
/// manages it (package manager) — see `install_kind()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// Standalone binary the user downloaded/unzipped themselves - safe to
    /// self-replace.
    Portable,
    /// Installed via a system package manager (.deb -> /usr/bin/...). Its
    /// files are owned/tracked by dpkg; overwriting them outside of apt/dpkg
    /// desyncs the package database and gets silently reverted or flagged by
    /// `dpkg -V` on the next audit.
    SystemPackage,
    /// `MCManager.app` (macOS) : le binaire est dans un bundle signé, le remplacer casserait la signature.
    AppBundle,
}

impl InstallKind {
    pub fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else { return InstallKind::Portable };
        let path = exe.to_string_lossy();
        if path.contains(".app/Contents/MacOS") {
            InstallKind::AppBundle
        } else if path.starts_with("/usr/") || path.starts_with("/opt/homebrew/") {
            InstallKind::SystemPackage
        } else {
            InstallKind::Portable
        }
    }

    pub fn self_update_supported(&self) -> bool {
        matches!(self, InstallKind::Portable)
    }

    pub fn note(&self) -> Option<&'static str> {
        match self {
            InstallKind::Portable => None,
            InstallKind::AppBundle => Some("Installe via MCManager.app : la mise a jour automatique est desactivee pour ne pas casser la signature de l'application. Telechargez le nouveau .dmg depuis GitHub Releases et remplacez l'application."),
            InstallKind::SystemPackage => Some(
                "Installe via le paquet systeme (.deb) : la mise a jour automatique est desactivee pour ne pas casser dpkg/apt. Telechargez le nouveau .deb depuis GitHub Releases et lancez 'sudo dpkg -i mcmanager_X.Y.Z_amd64.deb', ou configurez un depot apt."
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub release_notes: Option<String>,
    pub download_url: Option<String>,
    pub install_kind: InstallKind,
    pub self_update_supported: bool,
    pub self_update_note: Option<String>,
}

/// Fin du nom de l'archive portable de la release pour cette plateforme (voir release.yml / build.sh).
fn asset_suffix_for_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") { "windows-arm64.zip" } else { "windows-x86_64.zip" }
    } else if cfg!(target_os = "macos") {
        // Seule l'archive Intel est publiée ; sur Apple Silicon elle tourne via Rosetta 2.
        "macos-x86_64.tar.gz"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64.tar.gz"
    } else {
        "linux-x86_64.tar.gz"
    }
}

pub async fn check_for_update(client: &reqwest::Client, repo: &str) -> Result<UpdateInfo> {
    let install_kind = InstallKind::detect();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client.get(url).send().await;
    let data: Value = match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        _ => {
            return Ok(UpdateInfo {
                current_version: CURRENT_VERSION.to_string(),
                latest_version: None,
                update_available: false,
                release_notes: None,
                download_url: None,
                install_kind,
                self_update_supported: install_kind.self_update_supported(),
                self_update_note: install_kind.note().map(String::from),
            })
        }
    };

    let tag = data["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    let current = semver::Version::parse(CURRENT_VERSION).unwrap_or_else(|_| semver::Version::new(0, 0, 0));
    let latest = semver::Version::parse(&tag).ok();
    let update_available = matches!((&latest, &current), (Some(l), c) if l > c);

    let asset_kw = asset_suffix_for_platform();
    let download_url = data["assets"].as_array().and_then(|assets| {
        assets.iter()
            .find(|a| a["name"].as_str().unwrap_or_default().ends_with(asset_kw))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(String::from)
    });

    Ok(UpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        latest_version: if tag.is_empty() { None } else { Some(tag) },
        update_available,
        release_notes: data["body"].as_str().map(String::from),
        download_url,
        install_kind,
        self_update_supported: install_kind.self_update_supported(),
        self_update_note: install_kind.note().map(String::from),
    })
}

/// Télécharge l'archive portable de la nouvelle version et remplace les exécutables **et** le dossier `web/`
/// (l'interface évolue avec le binaire : remplacer l'un sans l'autre donnerait une page qui ne correspond plus à l'API).
/// L'appelant redémarre le processus ensuite.
///
/// Refuse pour les installations gérées par un paquet système ou un bundle `.app` (voir `InstallKind`).
pub async fn apply_update(client: &reqwest::Client, download_url: &str) -> Result<()> {
    let kind = InstallKind::detect();
    if !kind.self_update_supported() {
        anyhow::bail!(
            "{}",
            kind.note().unwrap_or("La mise a jour automatique n'est pas disponible pour ce type d'installation.")
        );
    }
    let bytes = client.get(download_url).send().await?.error_for_status()?.bytes().await?;
    let current_exe = std::env::current_exe().context("impossible de localiser l'executable actuel")?;
    let exe_dir = current_exe.parent().context("dossier de l'executable introuvable")?.to_path_buf();
    // Le dossier web de l'installation portable est à côté de l'exécutable ; sinon on n'y touche pas.
    let web_dir = Some(exe_dir.join("web")).filter(|d| d.join("index.html").exists());
    let is_zip = download_url.ends_with(".zip");
    tokio::task::spawn_blocking(move || apply_archive(&bytes, is_zip, &exe_dir, web_dir.as_deref())).await.context("tache de mise a jour interrompue")??;
    Ok(())
}

fn exe_names() -> [&'static str; 2] {
    if cfg!(windows) { ["mcmanager.exe", "mcmanager-server.exe"] } else { ["mcmanager", "mcmanager-server"] }
}

/// Extrait l'archive dans un dossier temporaire à côté de l'installation (même volume : les `rename` restent atomiques),
/// puis remplace les exécutables et `web/`. En cas d'échec en cours de route, tout est remis comme avant.
pub fn apply_archive(bytes: &[u8], is_zip: bool, exe_dir: &std::path::Path, web_dir: Option<&std::path::Path>) -> Result<()> {
    use std::fs;
    let staging = exe_dir.join(format!(".mcmanager-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        extract_archive(bytes, is_zip, &staging)?;
        let root = find_release_root(&staging).context("archive inattendue : executable mcmanager introuvable")?;
        // Plan : (nouveau fichier ou dossier, emplacement actuel).
        let mut plan: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
        for name in exe_names() {
            let new = root.join(name);
            let cur = exe_dir.join(name);
            // On ne remplace mcmanager-server que s'il est déjà installé ; mcmanager, toujours.
            if new.is_file() && (name.starts_with("mcmanager.") || name == "mcmanager" || cur.exists()) {
                plan.push((new, cur));
            }
        }
        if let (Some(web), true) = (web_dir, root.join("web/index.html").is_file()) {
            plan.push((root.join("web"), web.to_path_buf()));
        }
        let mut done: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new(); // (emplacement, sauvegarde .old)
        for (new, cur) in &plan {
            let old = cur.with_file_name(format!("{}.old", cur.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
            let _ = if old.is_dir() { fs::remove_dir_all(&old) } else { fs::remove_file(&old) };
            let had_old = cur.exists();
            if had_old && fs::rename(cur, &old).is_err() {
                rollback(&done);
                anyhow::bail!("impossible de remplacer {} (droits insuffisants ? sur Windows, lancez MCManager en administrateur)", cur.display());
            }
            if let Err(e) = fs::rename(new, cur) {
                if had_old {
                    let _ = fs::rename(&old, cur);
                }
                rollback(&done);
                anyhow::bail!("impossible de remplacer {} : {e}", cur.display());
            }
            #[cfg(unix)]
            if cur.is_file() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(cur, fs::Permissions::from_mode(0o755))?;
            }
            done.push((cur.clone(), old));
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Remet les anciens fichiers en place (le nouveau, déjà installé, est écarté).
fn rollback(done: &[(std::path::PathBuf, std::path::PathBuf)]) {
    for (cur, old) in done.iter().rev() {
        if old.exists() {
            let _ = if cur.is_dir() { std::fs::remove_dir_all(cur) } else { std::fs::remove_file(cur) };
            let _ = std::fs::rename(old, cur);
        }
    }
}

fn extract_archive(bytes: &[u8], is_zip: bool, dest: &std::path::Path) -> Result<()> {
    if is_zip {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("archive .zip invalide")?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            // `enclosed_name` écarte les chemins absolus et les `..` (zip-slip).
            let Some(rel) = entry.enclosed_name().map(|p| p.to_path_buf()) else { continue };
            let target = dest.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::io::copy(&mut entry, &mut std::fs::File::create(&target)?)?;
            }
        }
    } else {
        let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
        // `unpack` refuse les entrées qui sortiraient du dossier de destination.
        tar::Archive::new(gz).unpack(dest).context("archive .tar.gz invalide")?;
    }
    Ok(())
}

/// Dossier de l'archive qui contient l'exécutable (`mcmanager-<version>-<plateforme>/`, ou la racine).
fn find_release_root(staging: &std::path::Path) -> Option<std::path::PathBuf> {
    let [main, _] = exe_names();
    if staging.join(main).is_file() {
        return Some(staging.to_path_buf());
    }
    std::fs::read_dir(staging).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.join(main).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-upd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Archive de release factice : `mcmanager-9.9.9-test/{mcmanager,mcmanager-server,web/index.html}`.
    fn release_targz(with_server: bool) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut add = |name: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, format!("mcmanager-9.9.9-test/{name}"), data).unwrap();
        };
        add(exe_names()[0], b"NOUVEAU-BIN");
        if with_server {
            add(exe_names()[1], b"NOUVEAU-SERVER");
        }
        add("web/index.html", b"<html>nouvelle ui</html>");
        add("web/app.js", b"//nouveau");
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn release_zip() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::FileOptions::default();
        for (n, d) in [(exe_names()[0], &b"NOUVEAU-BIN"[..]), ("web/index.html", b"<html>zip</html>")] {
            zip.start_file(format!("mcmanager-9.9.9-test/{n}"), o).unwrap();
            zip.write_all(d).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn install(dir: &std::path::Path, server: bool) {
        std::fs::write(dir.join(exe_names()[0]), b"ANCIEN-BIN").unwrap();
        if server {
            std::fs::write(dir.join(exe_names()[1]), b"ANCIEN-SERVER").unwrap();
        }
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::write(dir.join("web/index.html"), b"ancienne ui").unwrap();
        std::fs::write(dir.join("web/obsolete.js"), b"x").unwrap();
    }

    #[test]
    fn updates_binaries_and_web_from_targz() {
        let d = tmp();
        install(&d, true);
        apply_archive(&release_targz(true), false, &d, Some(&d.join("web"))).unwrap();
        assert_eq!(std::fs::read(d.join(exe_names()[0])).unwrap(), b"NOUVEAU-BIN");
        assert_eq!(std::fs::read(d.join(exe_names()[1])).unwrap(), b"NOUVEAU-SERVER");
        assert_eq!(std::fs::read(d.join("web/index.html")).unwrap(), b"<html>nouvelle ui</html>");
        assert!(!d.join("web/obsolete.js").exists(), "web/ est remplacé en entier, pas fusionné");
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".mcmanager-update")), "dossier temporaire nettoyé");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.join(exe_names()[0])).unwrap().permissions().mode() & 0o111, 0o111, "exécutable");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn updates_from_zip_and_skips_absent_server_binary() {
        let d = tmp();
        install(&d, false);
        apply_archive(&release_zip(), true, &d, Some(&d.join("web"))).unwrap();
        assert_eq!(std::fs::read(d.join(exe_names()[0])).unwrap(), b"NOUVEAU-BIN");
        assert_eq!(std::fs::read(d.join("web/index.html")).unwrap(), b"<html>zip</html>");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn does_not_install_server_binary_when_it_was_not_installed() {
        let d = tmp();
        install(&d, false);
        apply_archive(&release_targz(true), false, &d, Some(&d.join("web"))).unwrap();
        assert!(!d.join(exe_names()[1]).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_archives_change_nothing() {
        let d = tmp();
        install(&d, true);
        assert!(apply_archive(b"ce n'est pas une archive", false, &d, Some(&d.join("web"))).is_err());
        assert!(apply_archive(b"pas un zip", true, &d, None).is_err());
        // Archive valide mais sans exécutable.
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_cksum();
        t.append_data(&mut h, "dossier/readme.txt", &b"x"[..]).unwrap();
        let bytes = t.into_inner().unwrap().finish().unwrap();
        assert!(apply_archive(&bytes, false, &d, Some(&d.join("web"))).unwrap_err().to_string().contains("introuvable"));
        assert_eq!(std::fs::read(d.join(exe_names()[0])).unwrap(), b"ANCIEN-BIN");
        assert_eq!(std::fs::read(d.join("web/index.html")).unwrap(), b"ancienne ui");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn platform_suffix_matches_release_asset_names() {
        let s = asset_suffix_for_platform();
        assert!(["linux-x86_64.tar.gz", "linux-aarch64.tar.gz", "macos-arm64.tar.gz", "macos-x86_64.tar.gz", "windows-x86_64.zip", "windows-arm64.zip"].contains(&s));
    }

    #[test]
    fn install_kinds() {
        assert!(InstallKind::Portable.self_update_supported());
        assert!(!InstallKind::SystemPackage.self_update_supported());
        assert!(!InstallKind::AppBundle.self_update_supported() && InstallKind::AppBundle.note().unwrap().contains(".dmg"));
    }
}
