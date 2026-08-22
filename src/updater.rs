use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How MCManager ended up on disk. Determines whether replacing the running
/// binary in place is safe (portable build) or would fight the system that
/// manages it (package manager / Nix store) — see `install_kind()`.
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
    /// Installed via Nix (/nix/store/...). Store paths are read-only and
    /// content-addressed by design - they cannot be written to, by anyone,
    /// ever. Updates must go through Nix itself.
    NixStore,
}

impl InstallKind {
    pub fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else { return InstallKind::Portable };
        let path = exe.to_string_lossy();
        if path.starts_with("/nix/store/") {
            InstallKind::NixStore
        } else if path.starts_with("/usr/") {
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
            InstallKind::SystemPackage => Some(
                "Installe via le paquet systeme (.deb) : la mise a jour automatique est desactivee pour ne pas casser dpkg/apt. Telechargez le nouveau .deb depuis GitHub Releases et lancez 'sudo dpkg -i mcmanager_X.Y.Z_amd64.deb', ou configurez un depot apt."
            ),
            InstallKind::NixStore => Some(
                "Installe via Nix : le store Nix est en lecture seule par design, la mise a jour automatique est impossible depuis l'application. Mettez a jour avec 'nix flake update' (si vous utilisez un flake local) ou en relancant 'nix run github:yo-le-zz/MCmanager' qui recuperera la derniere version taguee."
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

fn asset_keyword_for_platform() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-aarch64"
    } else if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux-x86_64"
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

    let asset_kw = asset_keyword_for_platform();
    let download_url = data["assets"].as_array().and_then(|assets| {
        assets.iter()
            .find(|a| a["name"].as_str().unwrap_or_default().contains(asset_kw))
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

/// Downloads the new binary, moves the current executable aside and replaces it.
/// The caller is responsible for restarting the process afterwards.
///
/// Refuses outright for `.deb`/Nix installs (see `InstallKind`) instead of
/// silently failing on a permission error or, worse, succeeding and leaving
/// the package manager / Nix store bookkeeping out of sync.
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
    let tmp_path = current_exe.with_extension("new");
    tokio::fs::write(&tmp_path, &bytes).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&tmp_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&tmp_path, perms).await?;
    }

    let old_path = current_exe.with_extension("old");
    tokio::fs::rename(&current_exe, &old_path).await.ok();
    match tokio::fs::rename(&tmp_path, &current_exe).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort rollback so we don't leave the app half-updated.
            tokio::fs::rename(&old_path, &current_exe).await.ok();
            Err(anyhow::anyhow!(
                "impossible de remplacer l'executable (droits insuffisants ? sur Windows, essayez de lancer MCManager en administrateur) : {e}"
            ))
        }
    }
}
