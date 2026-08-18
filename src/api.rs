use std::path::PathBuf;

use axum::extract::{Multipart, Path as AxPath, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::state::{save_config, save_servers, AppState};
use crate::{backup, downloader, files, modrinth, playit, presets, process, stats, updater, ws};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/version", get(get_version))
        .route("/api/update/check", get(update_check))
        .route("/api/update/apply", post(update_apply))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/loaders/:loader/versions", get(loader_versions))
        .route("/api/presets", get(list_presets))
        .route("/api/servers", get(list_servers).post(create_server))
        .route("/api/servers/:id", get(get_server).delete(delete_server))
        .route("/api/servers/:id/start", post(start_server))
        .route("/api/servers/:id/stop", post(stop_server))
        .route("/api/servers/:id/kill", post(kill_server))
        .route("/api/servers/:id/command", post(send_command))
        .route("/api/servers/:id/status", get(server_status))
        .route("/api/servers/:id/ws", get(ws::console_ws))
        .route("/api/servers/:id/files", get(list_files).delete(delete_file))
        .route("/api/servers/:id/files/content", get(read_file).put(write_file))
        .route("/api/servers/:id/files/upload", post(upload_file))
        .route("/api/servers/:id/addons", get(list_addons))
        .route("/api/servers/:id/addons/:file/toggle", post(toggle_addon))
        .route("/api/servers/:id/addons/:file", delete(delete_addon))
        .route("/api/servers/:id/schematics", get(list_schematics).post(upload_schematic))
        .route("/api/servers/:id/schematics/:file", delete(delete_schematic))
        .route("/api/servers/:id/backups", get(list_backups).post(create_backup))
        .route("/api/servers/:id/backups/:name/restore", post(restore_backup))
        .route("/api/servers/:id/backups/:name", delete(delete_backup))
        .route("/api/servers/:id/presets/:key/install", post(install_preset))
        .route("/api/servers/:id/marketplace/updates", get(check_addon_updates))
        .route("/api/marketplace/search", get(marketplace_search))
        .route("/api/marketplace/project/:id/versions", get(marketplace_versions))
        .route("/api/servers/:id/marketplace/install", post(marketplace_install))
        .route("/api/playit/download", post(playit_download))
        .route("/api/playit/start", post(playit_start))
        .route("/api/playit/stop", post(playit_stop))
        .route("/api/playit/status", get(playit_status))
        .route("/api/playit/ws", get(ws::playit_ws))
        .with_state(state)
}

// ───────────────────────── misc ─────────────────────────

async fn get_version() -> Json<serde_json::Value> {
    Json(json!({ "version": updater::CURRENT_VERSION }))
}

async fn update_check(State(state): State<AppState>) -> AppResult<Json<updater::UpdateInfo>> {
    let repo = state.config.read().await.update_repo.clone();
    let info = updater::check_for_update(&state.http, &repo).await?;
    Ok(Json(info))
}

async fn update_apply(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let repo = state.config.read().await.update_repo.clone();
    let info = updater::check_for_update(&state.http, &repo).await?;
    let url = info.download_url.ok_or_else(|| anyhow::anyhow!("aucun binaire disponible pour cette plateforme"))?;
    updater::apply_update(&state.http, &url).await?;
    Ok(Json(json!({ "ok": true, "message": "Mise a jour appliquee. Redemarrez MCManager." })))
}

async fn get_settings(State(state): State<AppState>) -> Json<AppConfig> {
    Json(state.config.read().await.clone())
}

async fn put_settings(State(state): State<AppState>, Json(cfg): Json<AppConfig>) -> AppResult<Json<AppConfig>> {
    {
        let mut c = state.config.write().await;
        *c = cfg;
    }
    save_config(&state).await?;
    Ok(Json(state.config.read().await.clone()))
}

#[derive(Deserialize)]
struct LoaderVersionsQuery {}

async fn loader_versions(State(state): State<AppState>, AxPath(loader): AxPath<String>) -> AppResult<Json<Vec<String>>> {
    let loader = parse_loader(&loader)?;
    let versions = downloader::list_versions(&state.http, loader).await?;
    Ok(Json(versions))
}

async fn list_presets() -> Json<Vec<PresetItem>> {
    Json(presets::all())
}

fn parse_loader(s: &str) -> AppResult<Loader> {
    Ok(match s.to_lowercase().as_str() {
        "vanilla" => Loader::Vanilla,
        "paper" => Loader::Paper,
        "purpur" => Loader::Purpur,
        "spigot" => Loader::Spigot,
        "fabric" => Loader::Fabric,
        "forge" => Loader::Forge,
        "neoforge" => Loader::Neoforge,
        other => return Err(AppError(anyhow::anyhow!("loader inconnu: {other}"))),
    })
}

// ───────────────────────── servers ─────────────────────────

async fn list_servers(State(state): State<AppState>) -> Json<Vec<ServerEntry>> {
    let servers = state.servers.read().await;
    Json(servers.values().cloned().collect())
}

async fn get_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<ServerEntry>> {
    let servers = state.servers.read().await;
    let entry = servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    Ok(Json(entry))
}

async fn create_server(State(state): State<AppState>, Json(req): Json<CreateServerRequest>) -> AppResult<Json<ServerEntry>> {
    let id = Uuid::new_v4();
    let folder = state.data_dir.join("servers").join(id.to_string());
    let java_path = state.config.read().await.java_path.clone();

    let result = downloader::setup_server(
        &state.http,
        req.loader,
        &req.mc_version,
        req.loader_version.as_deref(),
        &folder,
        &java_path,
    ).await?;

    let entry = ServerEntry {
        id,
        name: req.name,
        loader: req.loader,
        mc_version: req.mc_version,
        loader_version: result.loader_version,
        folder: folder.to_string_lossy().to_string(),
        jar_name: result.jar_name,
        java_path,
        xms_mb: req.xms_mb,
        xmx_mb: req.xmx_mb,
        port: req.port,
        extra_args: vec![],
        eula_accepted: req.eula_accepted,
        auto_backup_minutes: None,
        created_at: chrono::Utc::now(),
    };

    if entry.eula_accepted {
        tokio::fs::write(folder.join("eula.txt"), "eula=true\n").await.ok();
    }
    // Reasonable default server.properties so a beginner gets a working server immediately.
    let props_path = folder.join("server.properties");
    if !props_path.exists() {
        tokio::fs::write(&props_path, format!("server-port={}\nmotd=Serveur cree avec MCManager\nonline-mode=true\n", entry.port)).await.ok();
    }

    {
        let mut servers = state.servers.write().await;
        servers.insert(id, entry.clone());
    }
    save_servers(&state).await?;

    Ok(Json(entry))
}

async fn delete_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    {
        let runtime = state.runtime.read().await;
        if let Some(rt) = runtime.get(&id) {
            if rt.running {
                return Err(AppError(anyhow::anyhow!("arretez le serveur avant de le supprimer")));
            }
        }
    }
    let entry = {
        let mut servers = state.servers.write().await;
        servers.remove(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    save_servers(&state).await?;
    tokio::fs::remove_dir_all(&entry.folder).await.ok();
    Ok(Json(json!({ "ok": true })))
}

async fn start_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    process::start_server(&state, id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn stop_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    process::stop_server(&state, id, false).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn kill_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    process::stop_server(&state, id, true).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CommandBody {
    cmd: String,
}

async fn send_command(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Json(body): Json<CommandBody>) -> AppResult<Json<serde_json::Value>> {
    process::send_command(&state, id, &body.cmd).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn server_status(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<ServerStatus>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let (running, started_at, pid) = {
        let runtime = state.runtime.read().await;
        match runtime.get(&id) {
            Some(rt) => (rt.running, rt.started_at, rt.pid),
            None => (false, None, None),
        }
    };
    let (cpu, mem) = pid.map(stats::process_stats).unwrap_or((0.0, 0.0));
    let (players_online, players_max, motd) = if running {
        stats::ping_server(entry.port).await.unwrap_or((None, None, None))
    } else {
        (None, None, None)
    };
    Ok(Json(ServerStatus {
        id,
        running,
        started_at,
        cpu_percent: cpu,
        mem_mb: mem,
        players_online,
        players_max,
        motd,
    }))
}

// ───────────────────────── files ─────────────────────────

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

async fn server_folder(state: &AppState, id: Uuid) -> AppResult<PathBuf> {
    let servers = state.servers.read().await;
    let entry = servers.get(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    Ok(PathBuf::from(&entry.folder))
}

async fn list_files(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<PathQuery>) -> AppResult<Json<Vec<files::FileEntry>>> {
    let root = server_folder(&state, id).await?;
    let entries = files::list_dir(&root, q.path.as_deref().unwrap_or(""))?;
    Ok(Json(entries))
}

async fn delete_file(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<PathQuery>) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    let path = q.path.ok_or_else(|| anyhow::anyhow!("parametre path manquant"))?;
    files::delete_path(&root, &path)?;
    Ok(Json(json!({ "ok": true })))
}

async fn read_file(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<PathQuery>) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    let path = q.path.ok_or_else(|| anyhow::anyhow!("parametre path manquant"))?;
    let content = files::read_text_file(&root, &path)?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Deserialize)]
struct WriteFileBody {
    path: String,
    content: String,
}

async fn write_file(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Json(body): Json<WriteFileBody>) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    files::write_text_file(&root, &body.path, &body.content)?;
    Ok(Json(json!({ "ok": true })))
}

async fn upload_file(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, mut multipart: Multipart) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    let mut dest_dir = String::new();
    let mut saved = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|e| anyhow::anyhow!(e))? {
        let name = field.name().unwrap_or("").to_string();
        if name == "path" {
            dest_dir = String::from_utf8_lossy(&field.bytes().await.map_err(|e| anyhow::anyhow!(e))?).to_string();
            continue;
        }
        let filename = field.file_name().unwrap_or("upload.bin").to_string();
        let data = field.bytes().await.map_err(|e| anyhow::anyhow!(e))?;
        files::save_upload(&root, &dest_dir, &filename, &data).await?;
        saved.push(filename);
    }
    Ok(Json(json!({ "ok": true, "saved": saved })))
}

// ───────────────────────── addons (mods/plugins) ─────────────────────────

async fn list_addons(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<Vec<AddonInfo>>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let list = files::list_addons(&PathBuf::from(&entry.folder), entry.loader)?;
    Ok(Json(list))
}

async fn toggle_addon(State(state): State<AppState>, AxPath((id, file)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    files::toggle_addon(&PathBuf::from(&entry.folder), entry.loader, &file)?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_addon(State(state): State<AppState>, AxPath((id, file)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    files::delete_addon(&PathBuf::from(&entry.folder), entry.loader, &file)?;
    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── schematics ─────────────────────────

async fn list_schematics(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<Vec<files::FileEntry>>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let dir = files::schematics_dir(&PathBuf::from(&entry.folder));
    tokio::fs::create_dir_all(&dir).await.ok();
    let mut out = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if let Ok(meta) = e.metadata().await {
                out.push(files::FileEntry { name: e.file_name().to_string_lossy().to_string(), is_dir: meta.is_dir(), size_bytes: meta.len() });
            }
        }
    }
    Ok(Json(out))
}

async fn upload_schematic(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, mut multipart: Multipart) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let dir = files::schematics_dir(&PathBuf::from(&entry.folder));
    tokio::fs::create_dir_all(&dir).await?;
    let mut saved = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|e| anyhow::anyhow!(e))? {
        let filename = field.file_name().unwrap_or("upload.schem").to_string();
        if !(filename.ends_with(".schem") || filename.ends_with(".schematic")) {
            continue;
        }
        let data = field.bytes().await.map_err(|e| anyhow::anyhow!(e))?;
        tokio::fs::write(dir.join(&filename), &data).await?;
        saved.push(filename);
    }
    Ok(Json(json!({ "ok": true, "saved": saved })))
}

async fn delete_schematic(State(state): State<AppState>, AxPath((id, file)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let dir = files::schematics_dir(&PathBuf::from(&entry.folder));
    tokio::fs::remove_file(dir.join(file)).await.map_err(|e| anyhow::anyhow!(e))?;
    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── backups ─────────────────────────

async fn list_backups(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<Vec<BackupInfo>>> {
    let list = backup::list_backups(&state.data_dir, &id)?;
    Ok(Json(list))
}

async fn create_backup(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let name = backup::create_backup(&state.data_dir, &id, &PathBuf::from(&entry.folder))?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

async fn restore_backup(State(state): State<AppState>, AxPath((id, name)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    {
        let runtime = state.runtime.read().await;
        if let Some(rt) = runtime.get(&id) {
            if rt.running {
                return Err(AppError(anyhow::anyhow!("arretez le serveur avant de restaurer une sauvegarde")));
            }
        }
    }
    backup::restore_backup(&state.data_dir, &id, &name, &PathBuf::from(&entry.folder))?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_backup(State(state): State<AppState>, AxPath((id, name)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    backup::delete_backup(&state.data_dir, &id, &name)?;
    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── marketplace ─────────────────────────

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    #[serde(rename = "type")]
    project_type: Option<String>,
    loader: Option<String>,
    version: Option<String>,
}

async fn marketplace_search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> AppResult<Json<Vec<modrinth::SearchHit>>> {
    let hits = modrinth::search(
        &state.http,
        q.q.as_deref().unwrap_or(""),
        q.project_type.as_deref().unwrap_or("mod"),
        q.loader.as_deref(),
        q.version.as_deref(),
        30,
    ).await?;
    Ok(Json(hits))
}

async fn marketplace_versions(State(state): State<AppState>, AxPath(id): AxPath<String>, Query(q): Query<SearchQuery>) -> AppResult<Json<Vec<modrinth::ProjectVersion>>> {
    let versions = modrinth::project_versions(&state.http, &id, q.loader.as_deref(), q.version.as_deref()).await?;
    Ok(Json(versions))
}

#[derive(Deserialize)]
struct InstallBody {
    project_id: String,
    version_id: Option<String>,
}

async fn marketplace_install(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Json(body): Json<InstallBody>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let version = match body.version_id {
        Some(vid) => modrinth::project_versions(&state.http, &body.project_id, None, None).await?
            .into_iter().find(|v| v.id == vid).ok_or_else(|| anyhow::anyhow!("version introuvable"))?,
        None => modrinth::latest_matching_version(&state.http, &body.project_id, entry.loader.modrinth_loader(), &entry.mc_version).await?
            .ok_or_else(|| anyhow::anyhow!("aucune version compatible trouvee pour ce serveur"))?,
    };
    let dest = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    let filename = modrinth::download_version_file(&state.http, &version, &dest).await?;
    Ok(Json(json!({ "ok": true, "file": filename, "version": version.version_number })))
}

async fn install_preset(State(state): State<AppState>, AxPath((id, key)): AxPath<(Uuid, String)>) -> AppResult<Json<serde_json::Value>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let preset = presets::all().into_iter().find(|p| p.key == key).ok_or_else(|| anyhow::anyhow!("preset inconnu"))?;
    let version = modrinth::latest_matching_version(&state.http, &preset.modrinth_slug, entry.loader.modrinth_loader(), &entry.mc_version).await?
        .ok_or_else(|| anyhow::anyhow!("{} n'est pas disponible pour ce loader/cette version", preset.label))?;
    let dest = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    let filename = modrinth::download_version_file(&state.http, &version, &dest).await?;
    Ok(Json(json!({ "ok": true, "file": filename })))
}

async fn check_addon_updates(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<Vec<serde_json::Value>>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let addons = files::list_addons(&PathBuf::from(&entry.folder), entry.loader)?;
    let dir = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    let mut results = Vec::new();
    for addon in addons {
        if !addon.enabled {
            continue;
        }
        let path = dir.join(&addon.file_name);
        let Ok(bytes) = tokio::fs::read(&path).await else { continue };
        let hash = sha512_hex(&bytes);
        let Ok(Some(info)) = modrinth::identify_by_hash(&state.http, &hash).await else { continue };
        let project_id = info["project_id"].as_str().unwrap_or_default();
        let current_version_id = info["id"].as_str().unwrap_or_default();
        if let Ok(Some(latest)) = modrinth::latest_matching_version(&state.http, project_id, entry.loader.modrinth_loader(), &entry.mc_version).await {
            if latest.id != current_version_id {
                results.push(json!({
                    "file_name": addon.file_name,
                    "project_id": project_id,
                    "current_version_id": current_version_id,
                    "latest_version_id": latest.id,
                    "latest_version_number": latest.version_number,
                }));
            }
        }
    }
    Ok(Json(results))
}

fn sha512_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha512};
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ───────────────────────── playit.gg ─────────────────────────

async fn playit_download(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let path = playit::download_agent(&state).await?;
    Ok(Json(json!({ "ok": true, "path": path })))
}

async fn playit_start(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    playit::start_agent(&state).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn playit_stop(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    playit::stop_agent(&state).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn playit_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (running, path) = playit::status(&state).await;
    Json(json!({ "running": running, "path": path }))
}
