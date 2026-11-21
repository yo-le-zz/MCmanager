//! Téléchargement et installation des serveurs Bedrock : serveur officiel Mojang (stable et preview), PocketMine-MP, Nukkit.
//!
//! Les fonctions pures (analyse des réponses, choix des fichiers, URL) sont testées avec des réponses types ; les
//! téléchargements eux-mêmes dépendent des services de Mojang, de GitHub et d'OpenCollab.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::models::Loader;

/// API « liens de téléchargement » de Mojang (page minecraft.net/download/server/bedrock).
const MOJANG_LINKS: &str = "https://net-secondary.web.minecraft-services.net/api/v1.0/download/links";
/// Liste communautaire de toutes les versions publiées du serveur officiel (permet de choisir une version précise).
const BDS_VERSIONS: &str = "https://raw.githubusercontent.com/Bedrock-OSS/BDS-Versions/main";
const PMMP_RELEASES: &str = "https://api.github.com/repos/pmmp/PocketMine-MP/releases?per_page=40";
const PMMP_PHP_RELEASES: &str = "https://api.github.com/repos/pmmp/PHP-Binaries/releases?per_page=30";
const NUKKIT_CI: &str = "https://ci.opencollab.dev/job/NukkitX/job/Nukkit/job/master/lastSuccessfulBuild";
/// minecraft.net refuse les clients sans en-tête User-Agent de navigateur.
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36 MCManager";

pub struct Installed {
    pub jar_name: String,
    pub loader_version: Option<String>,
    /// Exécutable à enregistrer comme « runtime » (PHP pour PocketMine) ; vide sinon.
    pub runtime_path: Option<String>,
}

// ───────────────────────── fonctions pures ─────────────────────────

/// `"1.21.51.02"` : au moins trois nombres séparés par des points.
pub fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 3 && parts.len() <= 5 && parts.iter().all(|p| !p.is_empty() && p.len() <= 4 && p.chars().all(|c| c.is_ascii_digit()))
}

pub fn version_key(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Version contenue dans une URL `…/bedrock-server-1.21.51.02.zip`.
pub fn version_from_url(url: &str) -> Option<String> {
    let name = url.rsplit('/').next()?;
    let v = name.strip_prefix("bedrock-server-")?.strip_suffix(".zip")?;
    is_version(v).then(|| v.to_string())
}

/// Plateforme du serveur officiel pour cette machine, ou une explication claire.
pub fn bds_platform(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux"),
        ("windows", "x86_64") => Ok("win"),
        ("linux", "aarch64") | ("linux", "arm") => Err(anyhow!("le serveur officiel Bedrock n'existe que pour x86_64 (Linux, Windows) : sur ARM (Raspberry Pi), choisissez PocketMine-MP ou Nukkit, ou un serveur x86_64")),
        ("macos", _) => Err(anyhow!("le serveur officiel Bedrock n'existe pas pour macOS : choisissez PocketMine-MP ou Nukkit (ou Docker/Linux)")),
        _ => Err(anyhow!("plateforme non prise en charge par le serveur officiel Bedrock ({os}/{arch})")),
    }
}

pub fn current_platform() -> Result<&'static str> {
    bds_platform(std::env::consts::OS, std::env::consts::ARCH)
}

/// URL d'une version précise (le schéma des URL de Mojang est stable depuis des années).
pub fn download_url(version: &str, preview: bool, platform: &str) -> String {
    let dir = match (platform, preview) {
        ("win", false) => "bin-win",
        ("win", true) => "bin-win-preview",
        (_, false) => "bin-linux",
        (_, true) => "bin-linux-preview",
    };
    format!("https://www.minecraft.net/bedrockdedicatedserver/{dir}/bedrock-server-{version}.zip")
}

/// Extrait la version stable ou preview de la réponse de l'API de Mojang.
pub fn parse_links(json: &Value, preview: bool, platform: &str) -> Option<String> {
    let wanted = match (platform, preview) {
        ("win", false) => "serverBedrockWindows",
        ("win", true) => "serverBedrockPreviewWindows",
        (_, false) => "serverBedrockLinux",
        (_, true) => "serverBedrockPreviewLinux",
    };
    let links = json["result"]["links"].as_array().or_else(|| json["links"].as_array())?;
    links.iter().find(|l| l["downloadType"] == wanted).and_then(|l| l["downloadUrl"].as_str()).and_then(version_from_url)
}

/// Toutes les chaînes de version trouvées n'importe où dans un JSON (la forme exacte de la liste communautaire a varié).
pub fn collect_versions(json: &Value, out: &mut Vec<String>) {
    match json {
        Value::String(s) if is_version(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|v| collect_versions(v, out)),
        Value::Object(o) => o.iter().for_each(|(k, v)| {
            if is_version(k) {
                out.push(k.clone());
            }
            collect_versions(v, out);
        }),
        _ => {}
    }
}

pub fn sort_dedup_desc(v: &mut Vec<String>) {
    v.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    v.dedup();
}

#[derive(Debug, Clone, PartialEq)]
pub struct PmmpRelease {
    pub version: String,
    pub prerelease: bool,
    pub phar_url: String,
}

/// Versions de PocketMine-MP (avec leur `.phar`) d'après l'API des releases GitHub.
pub fn parse_pmmp_releases(json: &Value) -> Vec<PmmpRelease> {
    json.as_array().cloned().unwrap_or_default().iter().filter_map(|r| {
        if r["draft"] == true {
            return None;
        }
        let tag = r["tag_name"].as_str()?.trim_start_matches('v').to_string();
        let phar = r["assets"].as_array()?.iter().find(|a| a["name"] == "PocketMine-MP.phar")?["browser_download_url"].as_str()?.to_string();
        (!tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))).then(|| PmmpRelease { version: tag, prerelease: r["prerelease"] == true, phar_url: phar })
    }).collect()
}

/// Archive PHP prête à l'emploi de pmmp/PHP-Binaries pour cette machine (nommage `PHP-8.3-Linux-x86_64-PM5.tar.gz`).
pub fn pick_php_asset(assets: &[(String, String)], os: &str, arch: &str) -> Option<(String, String)> {
    let os_part = match os { "linux" => "Linux", "macos" => "MacOS", "windows" => "Windows", _ => return None };
    let arch_part: &[&str] = match (os, arch) {
        ("windows", _) => &["x64"],
        (_, "x86_64") => &["x86_64"],
        (_, "aarch64") => &["arm64", "aarch64"],
        _ => return None,
    };
    let mut best: Option<((u32, u32), String, String)> = None;
    for (name, url) in assets {
        let Some(rest) = name.strip_prefix("PHP-") else { continue };
        let Some((ver, tail)) = rest.split_once('-') else { continue };
        if !tail.starts_with(os_part) || !tail.contains("-PM5") || !(tail.ends_with(".tar.gz") || tail.ends_with(".zip")) || !arch_part.iter().any(|a| tail.contains(&format!("-{a}-"))) {
            continue;
        }
        let mut it = ver.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        let key = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        if best.as_ref().map(|b| key > b.0).unwrap_or(true) {
            best = Some((key, name.clone(), url.clone()));
        }
    }
    best.map(|(_, n, u)| (n, u))
}

/// Cherche l'exécutable PHP dans un dossier extrait (`bin/php7/bin/php`, `bin/php/php.exe`...).
pub fn find_php_binary(root: &Path) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["php.exe"] } else { &["php", "php8"] };
    walkdir::WalkDir::new(root).max_depth(6).into_iter().flatten().find(|e| e.file_type().is_file() && names.iter().any(|n| e.file_name() == *n)).map(|e| e.into_path())
}

/// Fichier de lancement d'un serveur déjà présent dans un dossier (import) : binaire officiel, `.phar` PocketMine ou jar Nukkit.
pub fn detect_launch_file(folder: &Path, loader: Loader) -> Option<String> {
    let exists = |n: &str| folder.join(n).is_file();
    match loader {
        Loader::Bedrock | Loader::Bedrockpreview => [executable_name(), "bedrock_server", "bedrock_server.exe"].into_iter().find(|n| exists(n)).map(String::from),
        Loader::Pocketmine => {
            if exists("PocketMine-MP.phar") { return Some("PocketMine-MP.phar".into()) }
            std::fs::read_dir(folder).ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).find(|n| n.to_lowercase().ends_with(".phar") && n.to_lowercase().contains("pocketmine"))
        }
        Loader::Nukkit => {
            if exists("nukkit.jar") { return Some("nukkit.jar".into()) }
            std::fs::read_dir(folder).ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).find(|n| n.to_lowercase().ends_with(".jar") && n.to_lowercase().contains("nukkit"))
        }
        _ => None,
    }
}

pub fn executable_name() -> &'static str {
    if cfg!(windows) { "bedrock_server.exe" } else { "bedrock_server" }
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
pub fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Fichiers du serveur officiel à conserver quand on installe une autre version par-dessus (la procédure officielle de mise à jour).
pub const PRESERVED: [&str; 3] = ["server.properties", "allowlist.json", "permissions.json"];

// ───────────────────────── réseau ─────────────────────────

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value> {
    Ok(client.get(url).header(reqwest::header::USER_AGENT, BROWSER_UA).send().await?.error_for_status()?.json().await?)
}

async fn download_to(client: &reqwest::Client, url: &str, dest: &Path) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let resp = client.get(url).header(reqwest::header::USER_AGENT, BROWSER_UA).send().await?.error_for_status().with_context(|| format!("téléchargement de {url}"))?;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

/// Versions proposées, la plus récente d'abord.
pub async fn list_versions(client: &reqwest::Client, loader: Loader) -> Result<Vec<String>> {
    match loader {
        Loader::Bedrock | Loader::Bedrockpreview => {
            let preview = loader == Loader::Bedrockpreview;
            let platform = current_platform().unwrap_or("linux"); // la liste reste consultable même sur une plateforme non supportée
            let mut out = Vec::new();
            if let Ok(j) = get_json(client, MOJANG_LINKS).await {
                out.extend(parse_links(&j, preview, platform));
            }
            // Versions plus anciennes (liste communautaire) : facultatif, on ignore l'échec.
            let path = if platform == "win" { "windows" } else { "linux" };
            if let Ok(j) = get_json(client, &format!("{BDS_VERSIONS}/{path}/versions.json")).await {
                let key = if preview { "preview" } else { "stable" };
                let scoped = j.get("versions").and_then(|v| v.get(key)).cloned().or_else(|| j.get(key).cloned()).unwrap_or(j);
                collect_versions(&scoped, &mut out);
            }
            sort_dedup_desc(&mut out);
            if out.is_empty() {
                anyhow::bail!("impossible d'obtenir la liste des versions Bedrock (minecraft.net et GitHub injoignables ?)");
            }
            Ok(out)
        }
        Loader::Pocketmine => {
            let j = get_json(client, PMMP_RELEASES).await?;
            let v: Vec<String> = parse_pmmp_releases(&j).into_iter().map(|r| if r.prerelease { format!("{} (bêta)", r.version) } else { r.version }).collect();
            if v.is_empty() { anyhow::bail!("aucune version de PocketMine-MP trouvée"); }
            Ok(v)
        }
        Loader::Nukkit => Ok(vec![nukkit_build(client).await.unwrap_or_else(|_| "latest".into())]),
        _ => Ok(vec![]),
    }
}

async fn nukkit_build(client: &reqwest::Client) -> Result<String> {
    let n = client.get(format!("{NUKKIT_CI}/buildNumber")).send().await?.error_for_status()?.text().await?;
    let n = n.trim();
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) { anyhow::bail!("numéro de build invalide"); }
    Ok(format!("build-{n}"))
}

/// Installe le serveur demandé dans `folder`.
pub async fn setup(client: &reqwest::Client, loader: Loader, version: &str, folder: &Path, runtime_hint: &str) -> Result<Installed> {
    tokio::fs::create_dir_all(folder).await?;
    match loader {
        Loader::Bedrock | Loader::Bedrockpreview => {
            install_official(client, loader == Loader::Bedrockpreview, version, folder).await?;
            Ok(Installed { jar_name: executable_name().into(), loader_version: None, runtime_path: None })
        }
        Loader::Pocketmine => {
            let wanted = version.trim_end_matches(" (bêta)");
            let j = get_json(client, PMMP_RELEASES).await?;
            let rel = parse_pmmp_releases(&j).into_iter().find(|r| r.version == wanted).context("version de PocketMine-MP introuvable")?;
            download_to(client, &rel.phar_url, &folder.join("PocketMine-MP.phar")).await?;
            let php = ensure_php(client, folder, runtime_hint).await?;
            Ok(Installed { jar_name: "PocketMine-MP.phar".into(), loader_version: Some(rel.version), runtime_path: Some(php) })
        }
        Loader::Nukkit => {
            download_to(client, &format!("{NUKKIT_CI}/artifact/target/nukkit-1.0-SNAPSHOT.jar"), &folder.join("nukkit.jar")).await?;
            Ok(Installed { jar_name: "nukkit.jar".into(), loader_version: Some(version.to_string()), runtime_path: None })
        }
        _ => Err(anyhow!("ce n'est pas un serveur Bedrock")),
    }
}

/// Télécharge l'archive officielle et l'extrait dans `folder` ; les fichiers de [`PRESERVED`] déjà présents sont conservés.
pub async fn install_official(client: &reqwest::Client, preview: bool, version: &str, folder: &Path) -> Result<()> {
    let platform = current_platform()?;
    if !is_version(version) {
        anyhow::bail!("version Bedrock invalide : {version}");
    }
    let tmp = std::env::temp_dir().join(format!("mcmanager-bds-{}.zip", uuid::Uuid::new_v4()));
    download_to(client, &download_url(version, preview, platform), &tmp).await.with_context(|| format!("version {version} introuvable chez Mojang"))?;
    let f = folder.to_path_buf();
    let t = tmp.clone();
    let res = tokio::task::spawn_blocking(move || extract_official(&t, &f)).await.map_err(|e| anyhow!("erreur interne: {e}"))?;
    let _ = tokio::fs::remove_file(&tmp).await;
    res
}

/// Extraction (bloquante) en conservant la configuration existante.
pub fn extract_official(zip_path: &Path, folder: &Path) -> Result<()> {
    let kept: Vec<(String, Vec<u8>)> = PRESERVED.iter().filter_map(|n| std::fs::read(folder.join(n)).ok().map(|b| (n.to_string(), b))).collect();
    crate::files::import_zip_file(folder, "", zip_path)?;
    for (name, bytes) in kept {
        std::fs::write(folder.join(name), bytes)?;
    }
    let exe = folder.join(executable_name());
    if !exe.is_file() {
        anyhow::bail!("l'archive ne contient pas {} : version incompatible avec cette plateforme ?", executable_name());
    }
    make_executable(&exe)?;
    Ok(())
}

/// Trouve (ou télécharge) un PHP capable de lancer PocketMine-MP (extension `pmmpthread`).
pub async fn ensure_php(client: &reqwest::Client, folder: &Path, hint: &str) -> Result<String> {
    let usable = |path: String| async move {
        let out = tokio::process::Command::new(&path).arg("-m").output().await.ok()?;
        (out.status.success() && String::from_utf8_lossy(&out.stdout).to_lowercase().contains("pmmpthread")).then_some(path)
    };
    for cand in [hint.to_string(), "php".to_string()] {
        if !cand.trim().is_empty() && cand != "java" {
            if let Some(p) = usable(cand).await {
                return Ok(p);
            }
        }
    }
    // PHP du système absent ou sans pmmpthread : binaires officiels de PocketMine.
    let j = get_json(client, PMMP_PHP_RELEASES).await?;
    let assets: Vec<(String, String)> = j.as_array().cloned().unwrap_or_default().iter().flat_map(|r| r["assets"].as_array().cloned().unwrap_or_default()).filter_map(|a| Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string()))).collect();
    let (name, url) = pick_php_asset(&assets, std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| anyhow!("aucun PHP prêt à l'emploi pour {}/{} : installez PHP 8 avec l'extension pmmpthread (voir pmmp.readthedocs.io)", std::env::consts::OS, std::env::consts::ARCH))?;
    let archive = folder.join(&name);
    download_to(client, &url, &archive).await?;
    let (f, a) = (folder.to_path_buf(), archive.clone());
    let php = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        if a.to_string_lossy().ends_with(".zip") {
            crate::files::import_zip_file(&f, "", &a)?;
        } else {
            tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(&a)?)).unpack(&f).context("extraction de PHP")?;
        }
        let p = find_php_binary(&f).context("PHP introuvable dans l'archive")?;
        make_executable(&p)?;
        Ok(p)
    }).await.map_err(|e| anyhow!("erreur interne: {e}"))??;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(php.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn version_syntax() {
        for ok in ["1.21.51.02", "1.20.80", "26.0.1.2"] {
            assert!(is_version(ok), "{ok}");
        }
        for bad in ["", "1.21", "latest", "1..2.3", "1.2.3.4.5.6", "../../1.2.3", "1.2.3a", "12345.1.1"] {
            assert!(!is_version(bad), "{bad}");
        }
        assert_eq!(version_from_url("https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-1.21.51.02.zip"), Some("1.21.51.02".into()));
        assert_eq!(version_from_url("https://x/other.zip"), None);
        let mut v = vec!["1.9.0".to_string(), "1.21.51.02".into(), "1.21.9.1".into(), "1.9.0".into()];
        sort_dedup_desc(&mut v);
        assert_eq!(v, ["1.21.51.02", "1.21.9.1", "1.9.0"], "tri numérique, pas alphabétique");
    }

    #[test]
    fn platform_choice_explains_unsupported_machines() {
        assert_eq!(bds_platform("linux", "x86_64").unwrap(), "linux");
        assert_eq!(bds_platform("windows", "x86_64").unwrap(), "win");
        let arm = bds_platform("linux", "aarch64").unwrap_err().to_string();
        assert!(arm.contains("PocketMine-MP") && arm.contains("ARM"), "{arm}");
        assert!(bds_platform("macos", "aarch64").unwrap_err().to_string().contains("macOS"));
        assert!(bds_platform("freebsd", "x86_64").is_err());
        assert_eq!(download_url("1.21.51.02", false, "linux"), "https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-1.21.51.02.zip");
        assert!(download_url("1.21.60.20", true, "win").contains("bin-win-preview/"));
    }

    #[test]
    fn mojang_links_response() {
        let j = json!({ "result": { "links": [
            { "downloadType": "serverBedrockWindows", "downloadUrl": "https://www.minecraft.net/bedrockdedicatedserver/bin-win/bedrock-server-1.21.51.02.zip" },
            { "downloadType": "serverBedrockLinux", "downloadUrl": "https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-1.21.51.02.zip" },
            { "downloadType": "serverBedrockPreviewLinux", "downloadUrl": "https://www.minecraft.net/bedrockdedicatedserver/bin-linux-preview/bedrock-server-1.21.60.21.zip" } ] } });
        assert_eq!(parse_links(&j, false, "linux"), Some("1.21.51.02".into()));
        assert_eq!(parse_links(&j, true, "linux"), Some("1.21.60.21".into()));
        assert_eq!(parse_links(&j, false, "win"), Some("1.21.51.02".into()));
        assert_eq!(parse_links(&j, true, "win"), None);
        assert_eq!(parse_links(&json!({ "links": [{ "downloadType": "serverBedrockLinux", "downloadUrl": "https://x/bedrock-server-1.20.1.1.zip" }] }), false, "linux"), Some("1.20.1.1".into()), "variante sans « result »");
        assert_eq!(parse_links(&json!({}), false, "linux"), None);
    }

    #[test]
    fn community_version_list_in_any_shape() {
        let mut out = Vec::new();
        collect_versions(&json!({ "stable": ["1.21.51.02", "1.21.44.01"], "versions": { "1.20.80.05": { "date": "x" } }, "note": "1.2" }), &mut out);
        sort_dedup_desc(&mut out);
        assert_eq!(out, ["1.21.51.02", "1.21.44.01", "1.20.80.05"]);
    }

    #[test]
    fn pocketmine_releases_and_php_assets() {
        let j = json!([
            { "tag_name": "5.22.0", "prerelease": false, "draft": false, "assets": [{ "name": "PocketMine-MP.phar", "browser_download_url": "https://github.com/pmmp/PocketMine-MP/releases/download/5.22.0/PocketMine-MP.phar" }] },
            { "tag_name": "5.23.0-beta1", "prerelease": true, "draft": false, "assets": [{ "name": "PocketMine-MP.phar", "browser_download_url": "https://x/beta.phar" }] },
            { "tag_name": "5.21.1", "prerelease": false, "draft": true, "assets": [{ "name": "PocketMine-MP.phar", "browser_download_url": "https://x/draft.phar" }] },
            { "tag_name": "4.0.0", "prerelease": false, "draft": false, "assets": [{ "name": "start.sh", "browser_download_url": "https://x/start.sh" }] },
            { "tag_name": "../evil", "prerelease": false, "draft": false, "assets": [{ "name": "PocketMine-MP.phar", "browser_download_url": "https://x/e.phar" }] } ]);
        let r = parse_pmmp_releases(&j);
        assert_eq!(r.iter().map(|x| x.version.as_str()).collect::<Vec<_>>(), ["5.22.0", "5.23.0-beta1"]);
        assert!(!r[0].prerelease && r[1].prerelease);

        let assets: Vec<(String, String)> = ["PHP-8.2-Linux-x86_64-PM5.tar.gz", "PHP-8.3-Linux-x86_64-PM5.tar.gz", "PHP-8.3-Linux-arm64-PM5.tar.gz", "PHP-8.3-MacOS-arm64-PM5.tar.gz", "PHP-8.3-MacOS-x86_64-PM5.tar.gz", "PHP-8.3-Windows-x64-PM5.zip", "PHP-8.3-Linux-x86_64-PM4.tar.gz", "README.md"]
            .iter().map(|n| (n.to_string(), format!("https://x/{n}"))).collect();
        assert_eq!(pick_php_asset(&assets, "linux", "x86_64").unwrap().0, "PHP-8.3-Linux-x86_64-PM5.tar.gz", "la version de PHP la plus récente");
        assert_eq!(pick_php_asset(&assets, "linux", "aarch64").unwrap().0, "PHP-8.3-Linux-arm64-PM5.tar.gz");
        assert_eq!(pick_php_asset(&assets, "macos", "aarch64").unwrap().0, "PHP-8.3-MacOS-arm64-PM5.tar.gz");
        assert_eq!(pick_php_asset(&assets, "windows", "x86_64").unwrap().0, "PHP-8.3-Windows-x64-PM5.zip");
        assert!(pick_php_asset(&assets, "freebsd", "x86_64").is_none() && pick_php_asset(&[], "linux", "x86_64").is_none());
    }

    #[test]
    fn official_archive_extraction_keeps_the_existing_configuration() {
        let d = std::env::temp_dir().join(format!("mcm-bds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let zip_path = d.join("bds.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::FileOptions::default();
            for (n, data) in [(executable_name(), "BIN-NEW"), ("server.properties", "server-name=Par defaut\n"), ("allowlist.json", "[]"), ("behavior_packs/vanilla/manifest.json", "{}")] {
                z.start_file(n, o).unwrap();
                z.write_all(data.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let srv = d.join("srv");
        std::fs::create_dir_all(srv.join("worlds/Mon monde")).unwrap();
        std::fs::write(srv.join("server.properties"), "server-name=Mon serveur\ngamemode=creative\n").unwrap();
        std::fs::write(srv.join("worlds/Mon monde/level.dat"), "MONDE").unwrap();
        std::fs::write(srv.join(executable_name()), "BIN-OLD").unwrap();
        extract_official(&zip_path, &srv).unwrap();
        assert_eq!(std::fs::read_to_string(srv.join(executable_name())).unwrap(), "BIN-NEW", "binaire mis à jour");
        assert_eq!(std::fs::read_to_string(srv.join("server.properties")).unwrap(), "server-name=Mon serveur\ngamemode=creative\n", "réglages conservés");
        assert_eq!(std::fs::read_to_string(srv.join("worlds/Mon monde/level.dat")).unwrap(), "MONDE", "le monde n'est pas touché");
        assert!(srv.join("behavior_packs/vanilla/manifest.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(srv.join(executable_name())).unwrap().permissions().mode() & 0o111, 0o111);
        }
        // Archive sans binaire : refusée avec une explication.
        let bad = d.join("bad.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
            z.start_file("readme.txt", zip::write::FileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let fresh = d.join("fresh");
        assert!(extract_official(&bad, &fresh).unwrap_err().to_string().contains("ne contient pas"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn launch_files_are_detected_for_imports() {
        let d = std::env::temp_dir().join(format!("mcm-detect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(detect_launch_file(&d, Loader::Bedrock), None);
        std::fs::write(d.join("bedrock_server"), "x").unwrap();
        assert_eq!(detect_launch_file(&d, Loader::Bedrock).as_deref(), Some("bedrock_server"));
        assert_eq!(detect_launch_file(&d, Loader::Pocketmine), None);
        std::fs::write(d.join("PocketMine-MP_5.22.phar"), "x").unwrap();
        assert_eq!(detect_launch_file(&d, Loader::Pocketmine).as_deref(), Some("PocketMine-MP_5.22.phar"));
        std::fs::write(d.join("Nukkit-1.0.jar"), "x").unwrap();
        assert_eq!(detect_launch_file(&d, Loader::Nukkit).as_deref(), Some("Nukkit-1.0.jar"));
        assert_eq!(detect_launch_file(&d, Loader::Paper), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn php_binary_is_found_in_an_extracted_tree() {
        let d = std::env::temp_dir().join(format!("mcm-php-{}", uuid::Uuid::new_v4()));
        let name = if cfg!(windows) { "php.exe" } else { "php" };
        std::fs::create_dir_all(d.join("bin/php7/bin")).unwrap();
        std::fs::write(d.join("bin/php7/bin").join(name), "x").unwrap();
        std::fs::write(d.join("bin/php7/bin/php-config"), "x").unwrap();
        assert_eq!(find_php_binary(&d).unwrap().file_name().unwrap(), name);
        assert!(find_php_binary(&d.join("bin/php7/bin/nope")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
