use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::models::Loader;

const MOJANG_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const PAPER_API: &str = "https://api.papermc.io/v2";
const PURPUR_API: &str = "https://api.purpurmc.org/v2";
const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_METADATA: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

/// Returns the list of Minecraft versions available for a given loader, newest first.
pub async fn list_versions(client: &reqwest::Client, loader: Loader) -> Result<Vec<String>> {
    match loader {
        Loader::Vanilla | Loader::Forge | Loader::Neoforge => {
            let manifest: Value = client.get(MOJANG_MANIFEST).send().await?.json().await?;
            let versions = manifest["versions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|v| v["type"] == "release")
                .filter_map(|v| v["id"].as_str().map(String::from))
                .collect();
            Ok(versions)
        }
        Loader::Paper => {
            let data: Value = client.get(format!("{PAPER_API}/projects/paper")).send().await?.json().await?;
            let mut versions: Vec<String> = data["versions"].as_array().cloned().unwrap_or_default()
                .into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
            versions.reverse();
            Ok(versions)
        }
        Loader::Purpur => {
            let data: Value = client.get(format!("{PURPUR_API}/purpur")).send().await?.json().await?;
            let mut versions: Vec<String> = data["versions"].as_array().cloned().unwrap_or_default()
                .into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
            versions.reverse();
            Ok(versions)
        }
        Loader::Spigot => {
            // Spigot has no public version API (BuildTools compiles locally); reuse Mojang's list.
            let manifest: Value = client.get(MOJANG_MANIFEST).send().await?.json().await?;
            let versions = manifest["versions"].as_array().cloned().unwrap_or_default()
                .into_iter().filter(|v| v["type"] == "release")
                .filter_map(|v| v["id"].as_str().map(String::from)).collect();
            Ok(versions)
        }
        Loader::Fabric => {
            let data: Value = client.get(format!("{FABRIC_META}/versions/game")).send().await?.json().await?;
            let versions = data.as_array().cloned().unwrap_or_default()
                .into_iter().filter(|v| v["stable"].as_bool().unwrap_or(false))
                .filter_map(|v| v["version"].as_str().map(String::from)).collect();
            Ok(versions)
        }
    }
}

pub struct DownloadResult {
    pub jar_name: String,
    pub loader_version: Option<String>,
}

/// Downloads (and for Forge/NeoForge, runs the installer for) the requested server
/// into `folder`. Returns the resulting jar filename to launch.
pub async fn setup_server(
    client: &reqwest::Client,
    loader: Loader,
    mc_version: &str,
    loader_version: Option<&str>,
    folder: &Path,
    java_path: &str,
) -> Result<DownloadResult> {
    tokio::fs::create_dir_all(folder).await?;
    match loader {
        Loader::Vanilla => download_vanilla(client, mc_version, folder).await,
        Loader::Paper => download_paper(client, mc_version, folder).await,
        Loader::Purpur => download_purpur(client, mc_version, folder).await,
        Loader::Spigot => Err(anyhow!(
            "Spigot ne fournit pas de jar precompile officiel (BuildTools requis). Utilisez Paper ou Purpur, compatibles avec les plugins Spigot."
        )),
        Loader::Fabric => download_fabric(client, mc_version, loader_version, folder).await,
        Loader::Forge => download_forge(client, mc_version, loader_version, folder, java_path).await,
        Loader::Neoforge => download_neoforge(client, mc_version, loader_version, folder, java_path).await,
    }
}

async fn save_stream(client: &reqwest::Client, url: &str, dest: &Path) -> Result<()> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    let mut file = tokio::fs::File::create(dest).await?;
    file.write_all(&bytes).await?;
    Ok(())
}

async fn download_vanilla(client: &reqwest::Client, mc_version: &str, folder: &Path) -> Result<DownloadResult> {
    let manifest: Value = client.get(MOJANG_MANIFEST).send().await?.json().await?;
    let entry = manifest["versions"].as_array().unwrap_or(&vec![]).iter()
        .find(|v| v["id"] == mc_version)
        .cloned()
        .context("version Minecraft introuvable")?;
    let version_url = entry["url"].as_str().context("url de version manquante")?;
    let version_data: Value = client.get(version_url).send().await?.json().await?;
    let jar_url = version_data["downloads"]["server"]["url"].as_str()
        .context("cette version n'a pas de jar serveur (trop ancienne ?)")?;
    let dest = folder.join("server.jar");
    save_stream(client, jar_url, &dest).await?;
    Ok(DownloadResult { jar_name: "server.jar".into(), loader_version: None })
}

async fn download_paper(client: &reqwest::Client, mc_version: &str, folder: &Path) -> Result<DownloadResult> {
    let builds: Value = client.get(format!("{PAPER_API}/projects/paper/versions/{mc_version}/builds")).send().await?.json().await?;
    let build = builds["builds"].as_array().and_then(|a| a.iter().filter(|b| b["channel"] == "default" || b["channel"].is_null()).last().or(a.last()))
        .context("aucun build Paper pour cette version")?;
    let build_num = build["build"].as_i64().context("build number manquant")?;
    let jar_name = build["downloads"]["application"]["name"].as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("paper-{mc_version}-{build_num}.jar"));
    let url = format!("{PAPER_API}/projects/paper/versions/{mc_version}/builds/{build_num}/downloads/{jar_name}");
    let dest = folder.join("server.jar");
    save_stream(client, &url, &dest).await?;
    Ok(DownloadResult { jar_name: "server.jar".into(), loader_version: Some(build_num.to_string()) })
}

async fn download_purpur(client: &reqwest::Client, mc_version: &str, folder: &Path) -> Result<DownloadResult> {
    let url = format!("{PURPUR_API}/purpur/{mc_version}/latest/download");
    let dest = folder.join("server.jar");
    save_stream(client, &url, &dest).await?;
    Ok(DownloadResult { jar_name: "server.jar".into(), loader_version: Some("latest".into()) })
}

async fn download_fabric(client: &reqwest::Client, mc_version: &str, loader_version: Option<&str>, folder: &Path) -> Result<DownloadResult> {
    let loader_ver = match loader_version {
        Some(v) => v.to_string(),
        None => {
            let loaders: Value = client.get(format!("{FABRIC_META}/versions/loader/{mc_version}")).send().await?.json().await?;
            loaders.as_array().and_then(|a| a.first()).and_then(|v| v["loader"]["version"].as_str())
                .context("aucun loader Fabric pour cette version")?.to_string()
        }
    };
    let installers: Value = client.get(format!("{FABRIC_META}/versions/installer")).send().await?.json().await?;
    let installer_ver = installers.as_array().and_then(|a| a.first()).and_then(|v| v["version"].as_str())
        .context("aucune version d'installeur Fabric disponible")?.to_string();
    let url = format!("{FABRIC_META}/versions/loader/{mc_version}/{loader_ver}/{installer_ver}/server/jar");
    let dest = folder.join("server.jar");
    save_stream(client, &url, &dest).await?;
    Ok(DownloadResult { jar_name: "server.jar".into(), loader_version: Some(loader_ver) })
}

async fn latest_forge_version(client: &reqwest::Client, mc_version: &str) -> Result<String> {
    let data: Value = client.get(FORGE_METADATA).send().await?.json().await?;
    let versions = data[mc_version].as_array().context("aucune version Forge pour cette version de Minecraft")?;
    let last = versions.last().and_then(|v| v.as_str()).context("liste de versions Forge vide")?;
    // maven-metadata.json values look like "1.20.1-47.2.0"; strip the mc prefix.
    Ok(last.trim_start_matches(&format!("{mc_version}-")).to_string())
}

async fn download_forge(client: &reqwest::Client, mc_version: &str, loader_version: Option<&str>, folder: &Path, java_path: &str) -> Result<DownloadResult> {
    let forge_ver = match loader_version {
        Some(v) => v.to_string(),
        None => latest_forge_version(client, mc_version).await?,
    };
    let full = format!("{mc_version}-{forge_ver}");
    let url = format!("{FORGE_MAVEN}/{full}/forge-{full}-installer.jar");
    let installer_path = folder.join("forge-installer.jar");
    save_stream(client, &url, &installer_path).await?;
    run_installer(java_path, &installer_path, folder).await?;
    let jar_name = find_server_jar(folder).await.unwrap_or_else(|| "server.jar".to_string());
    Ok(DownloadResult { jar_name, loader_version: Some(forge_ver) })
}

async fn download_neoforge(client: &reqwest::Client, mc_version: &str, loader_version: Option<&str>, folder: &Path, java_path: &str) -> Result<DownloadResult> {
    let neo_ver = match loader_version {
        Some(v) => v.to_string(),
        None => {
            let xml = client.get(format!("{NEOFORGE_MAVEN}/maven-metadata.xml")).send().await?.text().await?;
            // Minimal XML scrape: pick versions whose prefix matches the MC version's minor+patch scheme (NeoForge drops the "1." prefix).
            let short = mc_version.trim_start_matches("1.");
            let candidates: Vec<&str> = xml.split("<version>").skip(1)
                .filter_map(|s| s.split("</version>").next())
                .filter(|v| v.starts_with(short))
                .collect();
            candidates.last().map(|s| s.to_string()).context("aucune version NeoForge pour cette version de Minecraft")?
        }
    };
    let url = format!("{NEOFORGE_MAVEN}/{neo_ver}/neoforge-{neo_ver}-installer.jar");
    let installer_path = folder.join("neoforge-installer.jar");
    save_stream(client, &url, &installer_path).await?;
    run_installer(java_path, &installer_path, folder).await?;
    let jar_name = find_server_jar(folder).await.unwrap_or_else(|| "server.jar".to_string());
    Ok(DownloadResult { jar_name, loader_version: Some(neo_ver) })
}

async fn run_installer(java_path: &str, installer_path: &Path, folder: &Path) -> Result<()> {
    let output = tokio::process::Command::new(java_path)
        .arg("-jar")
        .arg(installer_path)
        .arg("--installServer")
        .current_dir(folder)
        .output()
        .await
        .context("impossible de lancer l'installeur (Java est-il installe et dans le PATH ?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "l'installeur a echoue: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// After a Forge/NeoForge install, the launchable jar name varies by version
/// (run.sh script, or a `*-server.jar` / `forge-*.jar` file). Best-effort scan.
async fn find_server_jar(folder: &Path) -> Option<String> {
    let mut entries = tokio::fs::read_dir(folder).await.ok()?;
    let mut best: Option<String> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".jar") && !name.contains("installer") {
            best = Some(name);
        }
    }
    best
}
