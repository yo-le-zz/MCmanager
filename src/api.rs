use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Multipart, Path as AxPath, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::state::{save_config, save_servers, AppState, BackupProgress};
use crate::{ai, backup, debug, downloader, files, history, modrinth, ntfy, playit, presets, process, remote, stats, updater, ws};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/version", get(get_version))
        .route("/api/update/check", get(update_check))
        .route("/api/update/apply", post(update_apply))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/loaders/:loader/versions", get(loader_versions))
        .route("/api/loaders/:loader/builds", get(loader_builds))
        .route("/api/presets", get(list_presets))
        .route("/api/servers", get(list_servers).post(create_server))
        .route("/api/servers/import", post(import_server))
        .route("/api/servers/:id", get(get_server).put(update_server).delete(delete_server))
        .route("/api/servers/:id/java/test", post(test_java))
        .route("/api/servers/:id/start", post(start_server))
        .route("/api/servers/:id/stop", post(stop_server))
        .route("/api/servers/:id/kill", post(kill_server))
        .route("/api/servers/:id/command", post(send_command))
        .route("/api/servers/:id/status", get(server_status))
        .route("/api/servers/:id/ws", get(ws::console_ws))
        .route("/api/servers/:id/console/clear", post(clear_console))
        .route("/api/servers/:id/files", get(list_files).delete(delete_file))
        .route("/api/servers/:id/files/content", get(read_file).put(write_file))
        .route("/api/servers/:id/files/upload", post(upload_file))
        .route("/api/servers/:id/files/export", get(export_files))
        .route("/api/servers/:id/files/import", post(import_files))
        .route("/api/servers/:id/open-folder", post(open_in_explorer))
        .route("/api/servers/:id/addons", get(list_addons))
        .route("/api/servers/:id/addons/:file/toggle", post(toggle_addon))
        .route("/api/servers/:id/addons/:file", delete(delete_addon))
        .route("/api/servers/:id/schematics", get(list_schematics).post(upload_schematic))
        .route("/api/servers/:id/schematics/:file", delete(delete_schematic))
        .route("/api/servers/:id/backups", get(list_backups).post(create_backup))
        .route("/api/servers/:id/backups/progress", get(backup_progress))
        .route("/api/servers/:id/backups/:name/restore", post(restore_backup))
        .route("/api/servers/:id/backups/:name", delete(delete_backup))
        .route("/api/servers/:id/presets/:key/install", post(install_preset))
        .route("/api/servers/:id/presets/category/:category/install", post(install_preset_category))
        .route("/api/servers/:id/managed-addons", post(add_managed_addon))
        .route("/api/servers/:id/addons/:file/track", post(track_existing_addon))
        .route("/api/servers/:id/managed-addons/:project_id", delete(remove_managed_addon))
        .route("/api/servers/:id/managed-addons/sync", post(sync_managed_addons))
        .route("/api/servers/:id/debug/crash-diagnostic", post(run_crash_diagnostic))
        .route("/api/servers/:id/history", get(get_server_history))
        .route("/api/ai/config", get(get_ai_config).post(save_ai_config))
        .route("/api/ai/models", get(list_ai_models))
        .route("/api/ai/chat", post(ai_chat))
        .route("/api/ntfy/config", get(get_ntfy_config).post(save_ntfy_config))
        .route("/api/ntfy/test", post(test_ntfy))
        .route("/api/remote/targets", get(remote_targets).post(remote_pair))
        .route("/api/remote/targets/:label", delete(remote_forget_target))
        .route("/api/remote/:label/call", post(remote_call))
        .route("/api/remote/:label/deploy/:server_id", post(remote_deploy))
        .route("/api/servers/:id/marketplace/updates", get(check_addon_updates))
        .route("/api/marketplace/search", get(marketplace_search))
        .route("/api/marketplace/project/:id/versions", get(marketplace_versions))
        .route("/api/servers/:id/marketplace/install", post(marketplace_install))
        .route("/api/playit/download", post(playit_download))
        .route("/api/playit/start", post(playit_start))
        .route("/api/playit/stop", post(playit_stop))
        .route("/api/playit/status", get(playit_status))
        .route("/api/playit/detect-local", post(playit_use_local))
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

async fn loader_versions(State(state): State<AppState>, AxPath(loader): AxPath<String>) -> AppResult<Json<Vec<String>>> {
    let loader = parse_loader(&loader)?;
    let versions = downloader::list_versions(&state.http, loader).await?;
    Ok(Json(versions))
}

#[derive(Deserialize)]
struct BuildsQuery {
    version: String,
}

async fn loader_builds(State(state): State<AppState>, AxPath(loader): AxPath<String>, Query(q): Query<BuildsQuery>) -> AppResult<Json<Vec<downloader::BuildOption>>> {
    let loader = parse_loader(&loader)?;
    let builds = downloader::list_builds(&state.http, loader, &q.version).await?;
    Ok(Json(builds))
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
        "quilt" => Loader::Quilt,
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
        extra_args: req.extra_args,
        eula_accepted: req.eula_accepted,
        auto_backup_minutes: req.auto_backup_minutes,
        backup_retention: req.backup_retention,
        auto_restart: req.auto_restart,
        auto_restart_delay_secs: req.auto_restart_delay_secs,
        scheduled_restart_minutes: req.scheduled_restart_minutes,
        stop_when_empty_minutes: req.stop_when_empty_minutes,
        aikar_flags: req.aikar_flags,
        managed_addons: vec![],
        dynamic_server: false,
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

/// Best-effort scan for a launchable server jar in an existing folder, used
/// by the "import an existing server" flow when the user doesn't specify one.
async fn detect_jar(folder: &Path) -> Option<String> {
    let mut rd = tokio::fs::read_dir(folder).await.ok()?;
    let mut best: Option<String> = None;
    while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(".jar") && !name.to_lowercase().contains("installer") {
            if name == "server.jar" {
                return Some(name);
            }
            best = Some(name);
        }
    }
    best
}

async fn read_port_from_properties(folder: &Path) -> Option<u16> {
    let content = tokio::fs::read_to_string(folder.join("server.properties")).await.ok()?;
    content.lines()
        .find_map(|l| l.strip_prefix("server-port="))
        .and_then(|v| v.trim().parse().ok())
}

async fn read_eula_accepted(folder: &Path) -> bool {
    match tokio::fs::read_to_string(folder.join("eula.txt")).await {
        Ok(content) => content.lines().any(|l| l.trim() == "eula=true"),
        Err(_) => false,
    }
}

async fn import_server(State(state): State<AppState>, Json(req): Json<ImportServerRequest>) -> AppResult<Json<ServerEntry>> {
    let folder = PathBuf::from(&req.folder_path);
    if !folder.is_dir() {
        return Err(AppError(anyhow::anyhow!("dossier introuvable : {}", req.folder_path)));
    }
    let jar_name = match req.jar_name {
        Some(j) => j,
        None => detect_jar(&folder).await.ok_or_else(|| {
            anyhow::anyhow!("aucun .jar de serveur trouve dans ce dossier - precisez le nom du fichier manuellement")
        })?,
    };
    if !folder.join(&jar_name).exists() {
        return Err(AppError(anyhow::anyhow!("le fichier {jar_name} n'existe pas dans ce dossier")));
    }

    let java_path = state.config.read().await.java_path.clone();
    let port = read_port_from_properties(&folder).await.unwrap_or(25565);
    let eula_accepted = read_eula_accepted(&folder).await;
    let id = Uuid::new_v4();

    let entry = ServerEntry {
        id,
        name: req.name,
        loader: req.loader,
        mc_version: req.mc_version,
        loader_version: None,
        folder: folder.to_string_lossy().to_string(),
        jar_name,
        java_path,
        xms_mb: 1024,
        xmx_mb: 2048,
        port,
        extra_args: vec![],
        eula_accepted,
        auto_backup_minutes: None,
        backup_retention: None,
        auto_restart: req.auto_restart,
        auto_restart_delay_secs: 5,
        scheduled_restart_minutes: None,
        stop_when_empty_minutes: None,
        aikar_flags: false,
        managed_addons: vec![],
        dynamic_server: false,
        created_at: chrono::Utc::now(),
    };

    {
        let mut servers = state.servers.write().await;
        servers.insert(id, entry.clone());
    }
    save_servers(&state).await?;
    Ok(Json(entry))
}

async fn update_server(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Json(req): Json<UpdateServerRequest>) -> AppResult<Json<ServerEntry>> {
    let mut servers = state.servers.write().await;
    let entry = servers.get_mut(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    if let Some(v) = req.name { entry.name = v; }
    if let Some(v) = req.xms_mb { entry.xms_mb = v; }
    if let Some(v) = req.xmx_mb { entry.xmx_mb = v; }
    if let Some(v) = req.port { entry.port = v; }
    if let Some(v) = req.extra_args { entry.extra_args = v; }
    if let Some(v) = req.aikar_flags { entry.aikar_flags = v; }
    if let Some(v) = req.auto_restart { entry.auto_restart = v; }
    if let Some(v) = req.auto_restart_delay_secs { entry.auto_restart_delay_secs = v; }
    if let Some(v) = req.scheduled_restart_minutes { entry.scheduled_restart_minutes = v; }
    if let Some(v) = req.stop_when_empty_minutes { entry.stop_when_empty_minutes = v; }
    if let Some(v) = req.dynamic_server { entry.dynamic_server = v; }
    if req.auto_backup_minutes.is_some() { entry.auto_backup_minutes = req.auto_backup_minutes; }
    if let Some(v) = req.backup_retention { entry.backup_retention = v; }
    if let Some(v) = req.java_path { entry.java_path = v; }
    let updated = entry.clone();
    drop(servers);
    save_servers(&state).await?;
    Ok(Json(updated))
}

#[derive(Deserialize)]
struct TestJavaBody {
    java_path: Option<String>,
    xmx_mb: Option<u32>,
}

/// Validates a Java executable (and, if the configured heap size is given,
/// that it can actually be allocated) *before* the user tries to start the
/// real server and gets a cryptic JVM crash. Reuses the server's saved
/// java_path/xmx_mb when the request doesn't override them, so "Tester ce
/// Java" in Settings checks exactly what a real launch would use.
async fn test_java(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, body: Option<Json<TestJavaBody>>) -> AppResult<Json<serde_json::Value>> {
    let entry = state.servers.read().await.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    let body = body.map(|b| b.0).unwrap_or(TestJavaBody { java_path: None, xmx_mb: None });
    let java_path = body.java_path.filter(|s| !s.trim().is_empty()).unwrap_or(entry.java_path);
    let xmx_mb = body.xmx_mb.unwrap_or(entry.xmx_mb);

    let output = tokio::process::Command::new(&java_path)
        .arg(format!("-Xmx{xmx_mb}M"))
        .arg("-version")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("impossible de lancer \"{java_path}\" : {e} (chemin introuvable ou non executable ?)"))?;

    // `java -version` prints to stderr by convention, not stdout.
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    Ok(Json(json!({
        "ok": output.status.success(),
        "java_path": java_path,
        "xmx_mb": xmx_mb,
        "output": text.trim(),
    })))
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
    state.runtime.write().await.remove(&id);
    state.backup_progress.write().await.remove(&id);

    let mut warnings = Vec::new();
    let folder = PathBuf::from(&entry.folder);
    if folder.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&folder).await {
            warnings.push(format!("dossier du serveur non supprime ({e})"));
        }
    }
    let backups_dir = state.data_dir.join("backups").join(id.to_string());
    if backups_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&backups_dir).await {
            warnings.push(format!("sauvegardes non supprimees ({e})"));
        }
    }
    Ok(Json(json!({ "ok": true, "warnings": warnings })))
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

async fn clear_console(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<serde_json::Value>> {
    process::clear_console(&state, id).await?;
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

/// Opens a folder inside the server's own directory in the host OS's native
/// file explorer (Explorer/Finder/the desktop's file manager). Only useful
/// when MCManager is being driven from a browser on the same machine as the
/// server (its normal desktop use case) - on a headless remote Ubuntu box
/// there is no desktop to open, and the underlying command will simply fail,
/// which we report back rather than silently ignore.
async fn open_in_explorer(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<PathQuery>) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    let target = match q.path.as_deref() {
        Some(p) if !p.is_empty() => files::safe_join(&root, p)?,
        _ => root,
    };
    if !target.exists() {
        return Err(AppError(anyhow::anyhow!("dossier introuvable")));
    }

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&target).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&target).spawn();

    result.map_err(|e| anyhow::anyhow!("impossible d'ouvrir l'explorateur de fichiers (pas d'environnement de bureau ?) : {e}"))?;
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

/// Downloads a zip of `path` (a file or folder inside the server's own
/// directory; empty/missing `path` = the whole server folder). Streams
/// from a temp file rather than buffering the whole archive in memory -
/// world folders can be large - and cleans the temp file up once the
/// response body has been fully sent.
async fn export_files(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<PathQuery>) -> AppResult<impl IntoResponse> {
    let root = server_folder(&state, id).await?;
    let rel = q.path.unwrap_or_default();
    let root2 = root.clone();
    let rel2 = rel.clone();
    let zip_path = tokio::task::spawn_blocking(move || files::export_zip(&root2, &rel2)).await
        .map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;

    let download_name = if rel.trim_matches('/').is_empty() {
        "server".to_string()
    } else {
        rel.trim_matches('/').replace('/', "-")
    };

    let file = tokio::fs::File::open(&zip_path).await?;
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    // Best-effort cleanup: the temp file is removed once the response has
    // been read. If the client disconnects mid-download this can leave a
    // stray file in the OS temp dir - acceptable (the OS cleans its temp
    // dir periodically, and this is bounded by how often exports happen).
    let cleanup_path = zip_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let _ = tokio::fs::remove_file(&cleanup_path).await;
    });

    let headers = [
        (header::CONTENT_TYPE, "application/zip".to_string()),
        (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{download_name}.zip\"")),
    ];
    Ok((headers, body))
}

/// Uploads a `.zip` (multipart field `file`) and extracts it into `path`
/// (multipart field `path`, optional - defaults to the server root).
/// Extraction is zip-slip safe: see `files::import_zip`.
async fn import_files(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, mut multipart: Multipart) -> AppResult<Json<serde_json::Value>> {
    let root = server_folder(&state, id).await?;
    let mut dest_dir = String::new();
    let mut zip_bytes: Option<bytes::Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| anyhow::anyhow!(e))? {
        let name = field.name().unwrap_or("").to_string();
        if name == "path" {
            dest_dir = String::from_utf8_lossy(&field.bytes().await.map_err(|e| anyhow::anyhow!(e))?).to_string();
            continue;
        }
        if name == "file" {
            zip_bytes = Some(field.bytes().await.map_err(|e| anyhow::anyhow!(e))?);
        }
    }
    let zip_bytes = zip_bytes.ok_or_else(|| anyhow::anyhow!("aucun fichier .zip recu"))?;
    let count = tokio::task::spawn_blocking(move || files::import_zip(&root, &dest_dir, &zip_bytes)).await
        .map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;
    Ok(Json(json!({ "ok": true, "extracted": count })))
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
    {
        let bp = state.backup_progress.read().await;
        if bp.contains_key(&id) {
            return Err(AppError(anyhow::anyhow!("une sauvegarde est deja en cours pour ce serveur")));
        }
    }
    let folder = PathBuf::from(&entry.folder);
    let folder_for_count = folder.clone();
    let total = tokio::task::spawn_blocking(move || backup::count_entries(&folder_for_count)).await
        .map_err(|e| anyhow::anyhow!("erreur interne: {e}"))?;
    let done = Arc::new(AtomicU64::new(0));
    state.backup_progress.write().await.insert(id, BackupProgress { done: done.clone(), total });

    let state2 = state.clone();
    let data_dir = state.data_dir.clone();
    tokio::spawn(async move {
        let folder2 = folder.clone();
        let done2 = done.clone();
        let result = tokio::task::spawn_blocking(move || backup::create_backup_tracked(&data_dir, &id, &folder2, done2)).await;
        state2.backup_progress.write().await.remove(&id);
        match result {
            Ok(Ok(name)) => {
                tracing::info!("Sauvegarde {name} terminee pour {id}");
                backup::after_backup_created(&state2, id, &name).await;
            }
            Ok(Err(e)) => tracing::error!("Echec de sauvegarde pour {id}: {e}"),
            Err(e) => tracing::error!("Tache de sauvegarde annulee pour {id}: {e}"),
        }
    });

    Ok(Json(json!({ "ok": true, "started": true, "total": total })))
}

async fn backup_progress(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> Json<serde_json::Value> {
    let bp = state.backup_progress.read().await;
    match bp.get(&id) {
        Some(p) => Json(json!({
            "running": true,
            "done": p.done.load(Ordering::Relaxed),
            "total": p.total,
        })),
        None => Json(json!({ "running": false })),
    }
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
    let data_dir = state.data_dir.clone();
    let folder = PathBuf::from(&entry.folder);
    tokio::task::spawn_blocking(move || backup::restore_backup(&data_dir, &id, &name, &folder))
        .await.map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;
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

/// Powers the "add all performance mods/plugins" button: installs every
/// curated preset in the given category (currently just "Performance",
/// e.g. Chunky/Lithium/spark) that's compatible with this server's loader,
/// in one click. Installs are independent - one failing (e.g. no build for
/// this MC version yet) doesn't stop the rest, and each result is reported
/// back individually so the UI can show exactly what happened.
async fn install_preset_category(State(state): State<AppState>, AxPath((id, category)): AxPath<(Uuid, String)>) -> AppResult<Json<Vec<serde_json::Value>>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let matching: Vec<_> = presets::all().into_iter()
        .filter(|p| p.category.eq_ignore_ascii_case(&category) && p.loaders.contains(&entry.loader.as_str().to_string()))
        .collect();
    if matching.is_empty() {
        return Err(AppError(anyhow::anyhow!("aucun preset '{category}' compatible avec ce type de serveur")));
    }
    let dest = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    let mut results = Vec::new();
    for preset in matching {
        let outcome = async {
            let version = modrinth::latest_matching_version(&state.http, &preset.modrinth_slug, entry.loader.modrinth_loader(), &entry.mc_version).await?
                .ok_or_else(|| anyhow::anyhow!("aucune version compatible"))?;
            modrinth::download_version_file(&state.http, &version, &dest).await
        }.await;
        match outcome {
            Ok(filename) => results.push(json!({ "key": preset.key, "label": preset.label, "ok": true, "file": filename })),
            Err(e) => results.push(json!({ "key": preset.key, "label": preset.label, "ok": false, "error": e.to_string() })),
        }
    }
    Ok(Json(results))
}

#[derive(Deserialize)]
struct AddManagedAddonBody {
    project_id: String,
    label: String,
}

async fn add_managed_addon(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Json(body): Json<AddManagedAddonBody>) -> AppResult<Json<ServerEntry>> {
    let mut servers = state.servers.write().await;
    let entry = servers.get_mut(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    if !entry.managed_addons.iter().any(|m| m.project_id == body.project_id) {
        entry.managed_addons.push(crate::models::ManagedAddon { project_id: body.project_id, label: body.label });
    }
    let updated = entry.clone();
    drop(servers);
    save_servers(&state).await?;
    Ok(Json(updated))
}

/// "Start tracking" for a mod/plugin that's already sitting in the
/// mods/plugins folder (installed by hand, or before managed-addons
/// existed) - identifies it by file hash via Modrinth (same lookup
/// `check_addon_updates` already uses) rather than requiring the user to
/// know its project slug/ID.
async fn track_existing_addon(State(state): State<AppState>, AxPath((id, file_name)): AxPath<(Uuid, String)>) -> AppResult<Json<ServerEntry>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    let path = PathBuf::from(&entry.folder).join(entry.loader.addon_dir()).join(&file_name);
    let bytes = tokio::fs::read(&path).await.map_err(|_| anyhow::anyhow!("fichier introuvable"))?;
    let hash = sha512_hex(&bytes);
    let info = modrinth::identify_by_hash(&state.http, &hash).await?
        .ok_or_else(|| anyhow::anyhow!("ce fichier n'a pas ete reconnu par Modrinth (installe manuellement depuis une autre source ?)"))?;
    let project_id = info["project_id"].as_str().unwrap_or_default().to_string();
    if project_id.is_empty() {
        return Err(anyhow::anyhow!("projet Modrinth introuvable pour ce fichier").into());
    }
    let label = file_name.trim_end_matches(".jar").to_string();

    let mut servers = state.servers.write().await;
    let entry = servers.get_mut(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    if !entry.managed_addons.iter().any(|m| m.project_id == project_id) {
        entry.managed_addons.push(crate::models::ManagedAddon { project_id, label });
    }
    let updated = entry.clone();
    drop(servers);
    save_servers(&state).await?;
    Ok(Json(updated))
}

async fn remove_managed_addon(State(state): State<AppState>, AxPath((id, project_id)): AxPath<(Uuid, String)>) -> AppResult<Json<ServerEntry>> {
    let mut servers = state.servers.write().await;
    let entry = servers.get_mut(&id).ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    entry.managed_addons.retain(|m| m.project_id != project_id);
    let updated = entry.clone();
    drop(servers);
    save_servers(&state).await?;
    Ok(Json(updated))
}

/// Downloads the latest loader+MC-version-compatible build of every
/// user-defined "managed" mod/plugin, replacing any older build of the same
/// project already installed (identified via Modrinth's file-hash lookup,
/// same mechanism as `check_addon_updates`) so re-syncing never leaves
/// duplicate jars of the same plugin behind.
async fn sync_managed_addons(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<Vec<serde_json::Value>>> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?
    };
    if entry.managed_addons.is_empty() {
        return Ok(Json(vec![]));
    }
    let dir = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    tokio::fs::create_dir_all(&dir).await.ok();

    let mut results = Vec::new();
    for managed in &entry.managed_addons {
        let outcome = async {
            let version = modrinth::latest_matching_version(&state.http, &managed.project_id, entry.loader.modrinth_loader(), &entry.mc_version).await?
                .ok_or_else(|| anyhow::anyhow!("aucune version compatible pour {}/{}", entry.loader.as_str(), entry.mc_version))?;

            // Remove any existing file(s) belonging to the same project
            // before writing the fresh one.
            if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(f)) = rd.next_entry().await {
                    let Ok(bytes) = tokio::fs::read(f.path()).await else { continue };
                    let hash = sha512_hex(&bytes);
                    if let Ok(Some(info)) = modrinth::identify_by_hash(&state.http, &hash).await {
                        if info["project_id"].as_str() == Some(managed.project_id.as_str()) {
                            tokio::fs::remove_file(f.path()).await.ok();
                        }
                    }
                }
            }

            modrinth::download_version_file(&state.http, &version, &dir).await
        }.await;
        match outcome {
            Ok(filename) => results.push(json!({ "project_id": managed.project_id, "label": managed.label, "ok": true, "file": filename })),
            Err(e) => results.push(json!({ "project_id": managed.project_id, "label": managed.label, "ok": false, "error": e.to_string() })),
        }
    }
    Ok(Json(results))
}

#[derive(Deserialize)]
struct DiagnosticQuery {
    timeout_secs: Option<u64>,
}

/// Runs the automated crash-cause isolation pass (see `debug.rs`). Blocks
/// until finished - for a server with many addons at the default 45s/addon
/// timeout this can take several minutes, so the frontend also watches the
/// normal console WebSocket for live "[Debug] ..." progress lines rather
/// than only waiting on this response.
async fn run_crash_diagnostic(State(state): State<AppState>, AxPath(id): AxPath<Uuid>, Query(q): Query<DiagnosticQuery>) -> AppResult<Json<debug::DiagnosticReport>> {
    let timeout_secs = q.timeout_secs.unwrap_or(45).clamp(5, 120);
    let report = debug::run_crash_diagnostic(state, id, timeout_secs).await?;
    Ok(Json(report))
}

async fn get_server_history(State(state): State<AppState>, AxPath(id): AxPath<Uuid>) -> AppResult<Json<history::ServerHistory>> {
    Ok(Json(history::get(&state.data_dir, id).await))
}

#[derive(Deserialize)]
struct SaveAiConfigBody {
    provider: String,
    /// Empty string = "keep the currently saved key" (so changing just the
    /// model or base URL doesn't force re-pasting the API key every time).
    api_key: String,
    model: String,
    #[serde(default)]
    ollama_base_url: String,
    #[serde(default)]
    omniroute_base_url: String,
}

#[derive(Serialize)]
struct AiConfigView {
    provider: String,
    model: String,
    ollama_base_url: String,
    omniroute_base_url: String,
    has_key: bool,
    masked_key: String,
}

async fn get_ai_config(State(state): State<AppState>) -> AppResult<Json<AiConfigView>> {
    let cfg = ai::load_config(&state.data_dir).await;
    Ok(Json(AiConfigView {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        ollama_base_url: cfg.ollama_base_url.clone(),
        omniroute_base_url: cfg.omniroute_base_url.clone(),
        has_key: !cfg.api_key.is_empty(),
        masked_key: cfg.masked_key(),
    }))
}

async fn save_ai_config(State(state): State<AppState>, Json(body): Json<SaveAiConfigBody>) -> AppResult<Json<AiConfigView>> {
    let mut cfg = ai::load_config(&state.data_dir).await;
    cfg.provider = body.provider;
    if !body.api_key.trim().is_empty() {
        cfg.api_key = body.api_key.trim().to_string();
    }
    cfg.model = body.model;
    cfg.ollama_base_url = body.ollama_base_url;
    cfg.omniroute_base_url = body.omniroute_base_url;
    ai::save_config(&state.data_dir, &cfg).await?;
    Ok(Json(AiConfigView {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        ollama_base_url: cfg.ollama_base_url.clone(),
        omniroute_base_url: cfg.omniroute_base_url.clone(),
        has_key: !cfg.api_key.is_empty(),
        masked_key: cfg.masked_key(),
    }))
}

async fn list_ai_models(State(state): State<AppState>) -> AppResult<Json<Vec<String>>> {
    let cfg = ai::load_config(&state.data_dir).await;
    let models = ai::list_models(&state.http, &cfg).await?;
    Ok(Json(models))
}

#[derive(Deserialize)]
struct AiChatBody {
    message: String,
    #[serde(default)]
    history: Vec<ai::ChatMessage>,
    /// Optional: which server this conversation is about, so the assistant
    /// gets real context (loader/version/addons/status) instead of guessing.
    server_id: Option<Uuid>,
}

async fn ai_chat(State(state): State<AppState>, Json(body): Json<AiChatBody>) -> AppResult<Json<serde_json::Value>> {
    let cfg = ai::load_config(&state.data_dir).await;
    let context = match body.server_id {
        Some(id) => {
            let servers = state.servers.read().await;
            match servers.get(&id) {
                Some(entry) => {
                    let running = state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false);
                    let addons = files::list_addons(&PathBuf::from(&entry.folder), entry.loader).unwrap_or_default();
                    let addon_list = if addons.is_empty() {
                        "aucun".to_string()
                    } else {
                        addons.iter().map(|a| format!("{} ({})", a.file_name, if a.enabled { "actif" } else { "desactive" })).collect::<Vec<_>>().join(", ")
                    };
                    format!(
                        "Serveur \"{}\" - {} {} - {} - mods/plugins installes : {}",
                        entry.name, entry.loader.as_str(), entry.mc_version,
                        if running { "en cours d'execution" } else { "arrete" },
                        addon_list
                    )
                }
                None => "aucun serveur selectionne".to_string(),
            }
        }
        None => "aucun serveur selectionne".to_string(),
    };
    let reply = ai::chat(&state.http, &cfg, &state, body.server_id, &context, &body.history, &body.message).await?;
    Ok(Json(json!({ "reply": reply })))
}

#[derive(Serialize)]
struct NtfyConfigView {
    enabled: bool,
    server_url: String,
    topic: String,
    has_token: bool,
    notify_crash: bool,
    notify_backup: bool,
    notify_scheduled_restart: bool,
    notify_auto_stop: bool,
    notify_player_join_leave: bool,
}

impl From<&ntfy::NtfyConfig> for NtfyConfigView {
    fn from(cfg: &ntfy::NtfyConfig) -> Self {
        NtfyConfigView {
            enabled: cfg.enabled,
            server_url: cfg.server_url.clone(),
            topic: cfg.topic.clone(),
            has_token: !cfg.auth_token.is_empty(),
            notify_crash: cfg.notify_crash,
            notify_backup: cfg.notify_backup,
            notify_scheduled_restart: cfg.notify_scheduled_restart,
            notify_auto_stop: cfg.notify_auto_stop,
            notify_player_join_leave: cfg.notify_player_join_leave,
        }
    }
}

async fn get_ntfy_config(State(state): State<AppState>) -> AppResult<Json<NtfyConfigView>> {
    let cfg = ntfy::load_config(&state.data_dir).await;
    Ok(Json((&cfg).into()))
}

#[derive(Deserialize)]
struct SaveNtfyConfigBody {
    enabled: bool,
    server_url: String,
    topic: String,
    /// Empty string = keep the currently saved token.
    auth_token: String,
    notify_crash: bool,
    notify_backup: bool,
    notify_scheduled_restart: bool,
    notify_auto_stop: bool,
    notify_player_join_leave: bool,
}

async fn save_ntfy_config(State(state): State<AppState>, Json(body): Json<SaveNtfyConfigBody>) -> AppResult<Json<NtfyConfigView>> {
    let mut cfg = ntfy::load_config(&state.data_dir).await;
    cfg.enabled = body.enabled;
    cfg.server_url = body.server_url;
    cfg.topic = body.topic;
    if !body.auth_token.trim().is_empty() {
        cfg.auth_token = body.auth_token.trim().to_string();
    }
    cfg.notify_crash = body.notify_crash;
    cfg.notify_backup = body.notify_backup;
    cfg.notify_scheduled_restart = body.notify_scheduled_restart;
    cfg.notify_auto_stop = body.notify_auto_stop;
    cfg.notify_player_join_leave = body.notify_player_join_leave;
    ntfy::save_config(&state.data_dir, &cfg).await?;
    Ok(Json((&cfg).into()))
}

async fn test_ntfy(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let cfg = ntfy::load_config(&state.data_dir).await;
    ntfy::send_test(&state.http, &cfg).await?;
    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── controle a distance (piloter une instance mcmanager-headless) ─────────────────────────
//
// The desktop app acts as just another client of the RSA/AES-encrypted
// remote protocol implemented in `remote.rs` (and already exercised by
// `mcmanager-headless`'s own `remote pair`/`remote list`/etc REPL
// commands) - the browser never touches any crypto, it talks plain HTTP
// to this backend, which does the encrypted round-trip to the actual
// remote instance on the browser's behalf.

async fn remote_targets(State(state): State<AppState>) -> AppResult<Json<Vec<remote::RemoteTarget>>> {
    Ok(Json(remote::load_targets(&state.data_dir).await))
}

#[derive(Deserialize)]
struct RemotePairBody {
    host: String,
    label: String,
    code: String,
}

async fn remote_pair(State(state): State<AppState>, Json(body): Json<RemotePairBody>) -> AppResult<Json<serde_json::Value>> {
    let identity = remote::load_or_create_identity(&state.data_dir).await?;
    remote::client_pair(&state.http, &state.data_dir, &identity, &body.host, &body.label, &body.code).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn remote_forget_target(State(state): State<AppState>, AxPath(label): AxPath<String>) -> AppResult<Json<serde_json::Value>> {
    let removed = remote::forget_target(&state.data_dir, &label).await?;
    Ok(Json(json!({ "ok": removed })))
}

/// Generic passthrough: the request body is the exact action MCManager
/// would otherwise send from the CLI (`{"action":"list"}`,
/// `{"action":"start","server_id":"..."}`, etc - see
/// `remote::dispatch_action`), so the frontend can drive every remote
/// action through one endpoint instead of one route per action.
async fn remote_call(State(state): State<AppState>, AxPath(label): AxPath<String>, Json(action): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let identity = remote::load_or_create_identity(&state.data_dir).await?;
    let targets = remote::load_targets(&state.data_dir).await;
    let target = targets.iter().find(|t| t.label == label).ok_or_else(|| anyhow::anyhow!("instance distante \"{label}\" inconnue"))?;
    let result = remote::client_call(&state.http, &identity, target, action).await?;
    Ok(Json(result))
}

/// "Envoyer ce serveur vers une instance distante" - zips the local
/// server and ships it to the remote instance's `import_server` action.
/// See that action's own doc comment in `remote.rs` for the size caveat
/// (fine for typical setups, not chunked/resumable for huge worlds).
async fn remote_deploy(State(state): State<AppState>, AxPath((label, server_id)): AxPath<(String, Uuid)>) -> AppResult<Json<serde_json::Value>> {
    let entry = state.servers.read().await.get(&server_id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    let root = PathBuf::from(&entry.folder);
    let zip_path = tokio::task::spawn_blocking(move || files::export_zip(&root, "")).await
        .map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;
    let zip_bytes = tokio::fs::read(&zip_path).await?;
    tokio::fs::remove_file(&zip_path).await.ok();
    let zip_b64 = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);

    let identity = remote::load_or_create_identity(&state.data_dir).await?;
    let targets = remote::load_targets(&state.data_dir).await;
    let target = targets.iter().find(|t| t.label == label).ok_or_else(|| anyhow::anyhow!("instance distante \"{label}\" inconnue"))?;

    let action = json!({
        "action": "import_server",
        "name": entry.name,
        "loader": entry.loader,
        "mc_version": entry.mc_version,
        "port": entry.port,
        "zip_base64": zip_b64,
    });
    let result = remote::client_call(&state.http, &identity, target, action).await?;
    Ok(Json(result))
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

async fn playit_use_local(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    match playit::use_local(&state).await? {
        Some(path) => Ok(Json(json!({ "ok": true, "found": true, "path": path }))),
        None => Ok(Json(json!({ "ok": true, "found": false }))),
    }
}
