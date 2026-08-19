mod api;
mod backup;
mod cli;
mod downloader;
mod error;
mod files;
mod models;
mod modrinth;
mod playit;
mod presets;
mod process;
mod state;
mod stats;
mod updater;
mod ws;

use std::net::SocketAddr;

use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // CLI subcommands (headless management of an already-running instance)
    // are handled before anything else so `mcmanager cli ...` never binds a
    // port or touches the data directory itself.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::try_dispatch(&args).await {
        std::process::exit(code);
    }

    let app_state = state::build_state().await?;

    let port: u16 = std::env::var("MCMANAGER_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(7777);
    let host = std::env::var("MCMANAGER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

    let web_dir = std::env::var("MCMANAGER_WEB_DIR").unwrap_or_else(|_| resolve_web_dir());
    let index = format!("{web_dir}/index.html");
    let static_service = ServeDir::new(&web_dir).not_found_service(ServeFile::new(&index));

    let app = api::router(app_state.clone())
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    tracing::info!("MCManager v{} demarre sur http://{host}:{port}", updater::CURRENT_VERSION);

    background_update_checker(app_state.clone());
    background_auto_backup(app_state.clone());
    maybe_open_browser(&host, port);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Looks for the bundled `web/` assets next to the executable, in the current
/// directory, or in the standard Linux install location, in that order.
fn resolve_web_dir() -> String {
    let candidates = [
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("web"))),
        Some(std::path::PathBuf::from("web")),
        Some(std::path::PathBuf::from("/usr/share/mcmanager/web")),
        Some(std::path::PathBuf::from("/usr/lib/mcmanager/web")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.join("index.html").exists() {
            return c.to_string_lossy().to_string();
        }
    }
    "web".to_string()
}

/// Opens the default browser to the web UI once the server is about to
/// start listening, so a beginner double-clicking the binary (or the
/// Windows/desktop shortcut) lands straight on the dashboard instead of a
/// blank terminal. Opt out with MCMANAGER_NO_BROWSER=1 (used by the
/// systemd/service builds, which have no desktop session to open a browser
/// in anyway).
fn maybe_open_browser(host: &str, port: u16) {
    if std::env::var("MCMANAGER_NO_BROWSER").ok().as_deref() == Some("1") {
        return;
    }
    // A "0.0.0.0" bind means "listen on every interface" - not something a
    // browser can open directly, so target loopback instead in that case.
    let browse_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let url = format!("http://{browse_host}:{port}");
    tokio::spawn(async move {
        // Give the listener a moment to actually be up before opening the tab.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let result = if cfg!(target_os = "windows") {
            tokio::process::Command::new("cmd").args(["/C", "start", "", &url]).status().await
        } else if cfg!(target_os = "macos") {
            tokio::process::Command::new("open").arg(&url).status().await
        } else {
            tokio::process::Command::new("xdg-open").arg(&url).status().await
        };
        if let Err(e) = result {
            tracing::debug!("ouverture automatique du navigateur impossible ({e}) - ouvrez {url} manuellement");
        }
    });
}

fn background_update_checker(state: state::AppState) {
    tokio::spawn(async move {
        let check_on_start = state.config.read().await.check_updates_on_start;
        if !check_on_start {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let repo = state.config.read().await.update_repo.clone();
        match updater::check_for_update(&state.http, &repo).await {
            Ok(info) if info.update_available => {
                tracing::info!(
                    "Mise a jour disponible : {} -> {}",
                    info.current_version,
                    info.latest_version.unwrap_or_default()
                );
            }
            Ok(_) => tracing::info!("MCManager est a jour."),
            Err(e) => tracing::debug!("Verification de mise a jour impossible : {e}"),
        }
    });
}

fn background_auto_backup(state: state::AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let servers = state.servers.read().await.clone();
            for (id, entry) in servers {
                let Some(minutes) = entry.auto_backup_minutes else { continue };
                if minutes == 0 {
                    continue;
                }
                let running = {
                    let runtime = state.runtime.read().await;
                    runtime.get(&id).map(|r| r.running).unwrap_or(false)
                };
                if !running {
                    continue;
                }
                if state.backup_progress.read().await.contains_key(&id) {
                    continue;
                }
                if let Ok(list) = backup::list_backups(&state.data_dir, &id) {
                    let due = match list.first() {
                        None => true,
                        Some(b) => (chrono::Utc::now() - b.created_at).num_minutes() >= minutes as i64,
                    };
                    if due {
                        let folder = std::path::PathBuf::from(&entry.folder);
                        let data_dir = state.data_dir.clone();
                        let _ = tokio::task::spawn_blocking(move || backup::create_backup(&data_dir, &id, &folder)).await;
                    }
                }
            }
        }
    });
}
