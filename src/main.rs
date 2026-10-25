use std::net::SocketAddr;

use mcmanager::{cli, state, updater, web};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // CLI subcommands (management of an already-running instance)
    // are handled before anything else so `mcmanager cli ...` never binds a
    // port or touches the data directory itself.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::try_dispatch(&args).await {
        std::process::exit(code);
    }

    let app_state = state::build_state().await?;

    let port: u16 = std::env::var("MCMANAGER_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(7777);
    let host = std::env::var("MCMANAGER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

    // Desktop app: no login by default (bound to loopback). `MCMANAGER_AUTH=on`
    // turns the multi-user login on; for a networked deployment prefer
    // `mcmanager-server`, which has it on by default.
    let auth = web::auth_from_env(&app_state, false, &host).await?;
    let web_dir = web::resolve_web_dir();
    let app = web::build_app(app_state.clone(), web::WebOptions { web_dir, auth, remote: None });

    tracing::info!("MCManager v{} demarre sur http://{host}:{port}", updater::CURRENT_VERSION);

    web::spawn_background_tasks(app_state.clone());
    maybe_open_browser(&host, port);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    web::serve(app, addr, app_state.data_dir.clone()).await
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
