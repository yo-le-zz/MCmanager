use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const API: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub project_type: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectVersion {
    pub id: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub date_published: String,
    pub files: Vec<VersionFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
}

pub async fn search(
    client: &reqwest::Client,
    query: &str,
    project_type: &str, // "mod" or "plugin"
    loader: Option<&str>,
    game_version: Option<&str>,
    limit: u32,
) -> Result<Vec<SearchHit>> {
    let mut facets: Vec<Vec<String>> = vec![vec![format!("project_type:{project_type}")]];
    if let Some(l) = loader {
        facets.push(vec![format!("categories:{l}")]);
    }
    if let Some(v) = game_version {
        facets.push(vec![format!("versions:{v}")]);
    }
    let facets_json = serde_json::to_string(&facets)?;

    let resp: Value = client
        .get(format!("{API}/search"))
        .query(&[
            ("query", query),
            ("facets", facets_json.as_str()),
            ("limit", &limit.to_string()),
        ])
        .send()
        .await?
        .json()
        .await?;

    let hits = resp["hits"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for h in hits {
        out.push(SearchHit {
            project_id: h["project_id"].as_str().unwrap_or_default().to_string(),
            slug: h["slug"].as_str().unwrap_or_default().to_string(),
            title: h["title"].as_str().unwrap_or_default().to_string(),
            description: h["description"].as_str().unwrap_or_default().to_string(),
            icon_url: h["icon_url"].as_str().map(String::from),
            downloads: h["downloads"].as_u64().unwrap_or(0),
            project_type: h["project_type"].as_str().unwrap_or_default().to_string(),
            categories: h["categories"].as_array().cloned().unwrap_or_default()
                .into_iter().filter_map(|c| c.as_str().map(String::from)).collect(),
        });
    }
    Ok(out)
}

pub async fn project_versions(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Vec<ProjectVersion>> {
    let mut req = client.get(format!("{API}/project/{project_id_or_slug}/version"));
    if let Some(l) = loader {
        req = req.query(&[("loaders", format!("[\"{l}\"]"))]);
    }
    if let Some(v) = game_version {
        req = req.query(&[("game_versions", format!("[\"{v}\"]"))]);
    }
    let resp: Vec<Value> = req.send().await?.json().await?;
    let mut out = Vec::new();
    for v in resp {
        let files = v["files"].as_array().cloned().unwrap_or_default().into_iter().map(|f| VersionFile {
            url: f["url"].as_str().unwrap_or_default().to_string(),
            filename: f["filename"].as_str().unwrap_or_default().to_string(),
            primary: f["primary"].as_bool().unwrap_or(false),
        }).collect();
        out.push(ProjectVersion {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            version_number: v["version_number"].as_str().unwrap_or_default().to_string(),
            game_versions: v["game_versions"].as_array().cloned().unwrap_or_default()
                .into_iter().filter_map(|g| g.as_str().map(String::from)).collect(),
            loaders: v["loaders"].as_array().cloned().unwrap_or_default()
                .into_iter().filter_map(|g| g.as_str().map(String::from)).collect(),
            date_published: v["date_published"].as_str().unwrap_or_default().to_string(),
            files,
        });
    }
    Ok(out)
}

pub async fn latest_matching_version(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    loader: &str,
    game_version: &str,
) -> Result<Option<ProjectVersion>> {
    let versions = project_versions(client, project_id_or_slug, Some(loader), Some(game_version)).await?;
    Ok(versions.into_iter().next())
}

/// Downloads the primary file of a version into `dest_dir`, returning the filename.
pub async fn download_version_file(client: &reqwest::Client, version: &ProjectVersion, dest_dir: &std::path::Path) -> Result<String> {
    let file = version.files.iter().find(|f| f.primary).or_else(|| version.files.first())
        .context("cette version Modrinth n'a aucun fichier")?;
    let bytes = client.get(&file.url).send().await?.error_for_status()?.bytes().await?;
    tokio::fs::create_dir_all(dest_dir).await?;
    let dest = dest_dir.join(&file.filename);
    tokio::fs::write(&dest, &bytes).await?;
    Ok(file.filename.clone())
}

/// Given a sha1/sha512-less local jar, ask Modrinth's version-file lookup API
/// to identify which project/version it corresponds to (used to power update checks).
pub async fn identify_by_hash(client: &reqwest::Client, sha512_hex: &str) -> Result<Option<Value>> {
    let url = format!("{API}/version_file/{sha512_hex}?algorithm=sha512");
    let resp = client.get(url).send().await?;
    if resp.status().is_success() {
        Ok(Some(resp.json().await?))
    } else {
        Ok(None)
    }
}
