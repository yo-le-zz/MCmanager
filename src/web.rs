//! Assembly of the HTTP application shared by both binaries: the REST API,
//! the static web UI, optional login (`auth.rs`), optional machine-to-machine
//! remote control (`remote.rs`), plus the background jobs every long-running
//! instance needs (update check, scheduled backups, remote-server refresh).
//!
//! `mcmanager` (desktop) builds it without login, bound to `127.0.0.1`.
//! `mcmanager-server --daemon` / Docker build it with login, bound to
//! `0.0.0.0`, and with the `/remote/*` endpoints on the same port.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::{self, AuthState, User};
use crate::remote::{self, RemoteRuntime};
use crate::state::AppState;
use crate::{api, backup, hub, updater};

pub struct WebOptions {
    pub web_dir: String,
    pub auth: Option<Arc<AuthState>>,
    /// When set, the `/remote/*` machine-to-machine endpoints and the
    /// `/api/server/*` management routes are served on the same port.
    pub remote: Option<RemoteRuntime>,
}

/// Looks for the bundled `web/` assets next to the executable, in the current
/// directory, or in the standard Linux install locations, in that order.
pub fn resolve_web_dir() -> String {
    if let Ok(dir) = std::env::var("MCMANAGER_WEB_DIR") {
        return dir;
    }
    let candidates = [
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("web"))),
        Some(PathBuf::from("web")),
        Some(PathBuf::from("/usr/share/mcmanager/web")),
        Some(PathBuf::from("/usr/lib/mcmanager/web")),
        Some(PathBuf::from("/app/web")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.join("index.html").exists() {
            return c.to_string_lossy().to_string();
        }
    }
    "web".to_string()
}

pub fn build_app(state: AppState, opts: WebOptions) -> Router {
    let index = format!("{}/index.html", opts.web_dir);
    let static_service = ServeDir::new(&opts.web_dir).not_found_service(ServeFile::new(&index));

    let mut app = api::router(state).route("/healthz", get(healthz));
    if let Some(rt) = opts.remote.clone() {
        app = app.merge(remote::router(rt.clone())).merge(server_routes(rt, opts.auth.is_some()));
    }
    if let Some(auth) = opts.auth.clone() {
        app = app.merge(auth::router(auth.clone())).layer(axum::middleware::from_fn_with_state(auth, auth::require_auth));
    }
    // No CORS layer on purpose: the UI is same-origin, and a permissive CORS
    // policy would let any web page the user happens to visit drive the API
    // of a locally running instance.
    app.fallback_service(static_service).layer(TraceLayer::new_for_http())
}

/// Resolves `MCMANAGER_AUTH` (`on` | `off` | unset = `default_on`) into an
/// `AuthState`, creating the first administrator when there is none and
/// printing its credentials once if they were generated.
pub async fn auth_from_env(state: &AppState, default_on: bool, host: &str) -> anyhow::Result<Option<Arc<AuthState>>> {
    let setting = std::env::var("MCMANAGER_AUTH").unwrap_or_default().to_lowercase();
    let enabled = match setting.as_str() {
        "on" | "1" | "true" | "yes" => true,
        "off" | "0" | "false" | "no" => false,
        _ => default_on,
    };
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    if !enabled {
        if !loopback {
            eprintln!("⚠ ATTENTION : l'interface ecoute sur {host} SANS authentification (MCMANAGER_AUTH=off). Toute personne qui atteint ce port controle vos serveurs.");
        }
        return Ok(None);
    }
    let auth = AuthState::load(&state.data_dir).await?;
    if let Some((user, password, generated)) = auth.bootstrap_admin().await? {
        if generated {
            println!("┌──────────────────────────────────────────────────────────────┐");
            println!("│ Premier demarrage : compte administrateur cree.              │");
            println!("│   utilisateur   : {user:<42} │");
            println!("│   mot de passe  : {password:<42} │");
            println!("│ Notez-le (affiche une seule fois) puis changez-le dans      │");
            println!("│ l'interface. Perdu ? mcmanager-server -> user passwd admin   │");
            println!("└──────────────────────────────────────────────────────────────┘");
        } else {
            println!("Compte administrateur '{user}' cree depuis MCMANAGER_ADMIN_USER / MCMANAGER_ADMIN_PASSWORD.");
        }
    }
    Ok(Some(auth))
}

/// Binds and serves until Ctrl+C / SIGTERM, then clears the instance lock.
pub async fn serve(app: Router, addr: SocketAddr, data_dir: PathBuf) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| anyhow::anyhow!("impossible d'ecouter sur {addr} : {e}"))?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal(data_dir))
        .await?;
    Ok(())
}

/// Waits for Ctrl+C (or SIGTERM under systemd/Docker) and clears the instance
/// lock on the way out, so a normal restart doesn't require manually deleting
/// `mcmanager.lock` every time - only an unclean crash (`kill -9`, power
/// loss) still leaves it behind.
pub async fn shutdown_signal(data_dir: PathBuf) {
    wait_for_stop_signal().await;
    tracing::info!("Arret demande, fermeture propre...");
    let _ = tokio::fs::remove_file(data_dir.join("mcmanager.lock")).await;
}

pub async fn wait_for_stop_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok(); };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "version": updater::CURRENT_VERSION }))
}

// ───────────────────────── gestion du mode serveur (/api/server/*) ─────────────────────────

#[derive(Clone)]
struct ServerCtx {
    rt: RemoteRuntime,
    auth_enabled: bool,
}

fn server_routes(rt: RemoteRuntime, auth_enabled: bool) -> Router {
    Router::new()
        .route("/api/server/info", get(server_info))
        .route("/api/server/pairing-code", post(server_pairing_code))
        .route("/api/server/clients/:id", delete(server_revoke_client))
        .with_state(ServerCtx { rt, auth_enabled })
}

/// Pairing machines = handing out full control of this host, so with login
/// enabled only administrators may do it. (Requests authenticated with the
/// static API token carry no user and are treated as administrative.)
fn admin_only(ctx: &ServerCtx, user: &Option<Extension<User>>) -> Result<(), Response> {
    if ctx.auth_enabled {
        if let Some(Extension(u)) = user {
            if !u.admin {
                return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Reserve aux administrateurs." }))).into_response());
            }
        }
    }
    Ok(())
}

async fn server_info(State(ctx): State<ServerCtx>) -> Json<serde_json::Value> {
    let clients: Vec<_> = ctx
        .rt
        .clients_with_sessions()
        .await
        .into_iter()
        .map(|(c, sessions)| json!({ "id": c.id, "label": c.label, "paired_at": c.paired_at, "last_used_at": c.last_used_at, "live_sessions": sessions }))
        .collect();
    Json(json!({
        "mode": "server",
        "version": updater::CURRENT_VERSION,
        "fingerprint": ctx.rt.fingerprint(),
        "auth_enabled": ctx.auth_enabled,
        "clients": clients,
    }))
}

#[derive(Deserialize, Default)]
struct PairingCodeBody {
    minutes: Option<i64>,
}

async fn server_pairing_code(State(ctx): State<ServerCtx>, user: Option<Extension<User>>, body: Option<Json<PairingCodeBody>>) -> Response {
    if let Err(r) = admin_only(&ctx, &user) {
        return r;
    }
    let minutes = body.and_then(|b| b.0.minutes).unwrap_or(remote::PAIRING_CODE_DEFAULT_TTL_MIN);
    let (code, applied) = ctx.rt.generate_pairing_code(minutes).await;
    Json(json!({ "code": code, "minutes": applied, "fingerprint": ctx.rt.fingerprint() })).into_response()
}

async fn server_revoke_client(State(ctx): State<ServerCtx>, user: Option<Extension<User>>, AxPath(id): AxPath<String>) -> Response {
    if let Err(r) = admin_only(&ctx, &user) {
        return r;
    }
    match ctx.rt.revoke(&id).await {
        Ok(removed) => Json(json!({ "ok": removed })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// ───────────────────────── taches de fond ─────────────────────────

/// Everything a long-running instance needs besides serving requests.
pub fn spawn_background_tasks(state: AppState) {
    background_update_checker(state.clone());
    background_auto_backup(state.clone());
    hub::spawn_refresher(state);
}

fn background_update_checker(state: AppState) {
    tokio::spawn(async move {
        let check_on_start = state.config.read().await.check_updates_on_start;
        if !check_on_start {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let repo = state.config.read().await.update_repo.clone();
        match updater::check_for_update(&state.http, &repo).await {
            Ok(info) if info.update_available => {
                tracing::info!("Mise a jour disponible : {} -> {}", info.current_version, info.latest_version.unwrap_or_default());
            }
            Ok(_) => tracing::info!("MCManager est a jour."),
            Err(e) => tracing::debug!("Verification de mise a jour impossible : {e}"),
        }
    });
}

fn background_auto_backup(state: AppState) {
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
                        let folder = PathBuf::from(&entry.folder);
                        let data_dir = state.data_dir.clone();
                        let result = tokio::task::spawn_blocking(move || backup::create_backup(&data_dir, &id, &folder)).await;
                        if let Ok(Ok(name)) = result {
                            backup::after_backup_created(&state, id, &name).await;
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    async fn call(app: &Router, req: Request<Body>) -> axum::response::Response {
        app.clone().oneshot(req).await.unwrap()
    }

    fn json_req(method: &str, uri: &str, body: serde_json::Value, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    fn get_req(uri: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().uri(uri);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        b.body(Body::empty()).unwrap()
    }

    fn cookie_of(resp: &axum::response::Response) -> String {
        resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string()
    }

    /// The whole network-facing assembly: login gate on /api, public login
    /// page + health check, machine-to-machine /remote left to its own
    /// protection, several users connected at once.
    #[tokio::test]
    async fn login_gates_the_api_and_supports_many_users() {
        let dir = std::env::temp_dir().join(format!("mcmanager-web-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let state = crate::state::build_state_for_test(&dir).await;
        let auth = AuthState::load(&dir).await.unwrap();
        auth.add_user("alice", "alice-password", true, false).await.unwrap();
        auth.add_user("bob", "bob-password-1", false, false).await.unwrap();
        let rt = RemoteRuntime::new(state.clone(), dir.clone()).await.unwrap();
        let web_dir = dir.join("web");
        tokio::fs::create_dir_all(&web_dir).await.unwrap();
        tokio::fs::write(web_dir.join("index.html"), "<html>login</html>").await.unwrap();
        let app = build_app(state, WebOptions { web_dir: web_dir.to_string_lossy().to_string(), auth: Some(auth), remote: Some(rt) });

        // Public: UI, health, /auth/me answers 401 (not a redirect, not HTML)
        assert_eq!(call(&app, get_req("/", None)).await.status(), StatusCode::OK);
        assert_eq!(call(&app, get_req("/healthz", None)).await.status(), StatusCode::OK);
        assert_eq!(call(&app, get_req("/auth/me", None)).await.status(), StatusCode::UNAUTHORIZED);
        // Machine-to-machine endpoint is NOT behind the browser login (it has its own RSA protection)
        assert_eq!(call(&app, get_req("/remote/info", None)).await.status(), StatusCode::OK);

        // Closed without login - API and WebSocket-style paths alike
        assert_eq!(call(&app, get_req("/api/servers", None)).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, get_req("/api/settings", None)).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, get_req("/api/server/info", None)).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, get_req("/api/servers", Some("mcm_session=forged"))).await.status(), StatusCode::UNAUTHORIZED);

        // Wrong password refused
        let bad = call(&app, json_req("POST", "/auth/login", json!({ "username": "alice", "password": "nope" }), None)).await;
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
        assert!(bad.headers().get(header::SET_COOKIE).is_none());

        // Two browsers for alice + one for bob, all at the same time
        let a1 = call(&app, json_req("POST", "/auth/login", json!({ "username": "alice", "password": "alice-password" }), None)).await;
        assert_eq!(a1.status(), StatusCode::OK);
        let cookie_header = a1.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
        assert!(cookie_header.contains("HttpOnly") && cookie_header.contains("SameSite=Strict"));
        let (a1, ) = (cookie_of(&a1), );
        let a2 = cookie_of(&call(&app, json_req("POST", "/auth/login", json!({ "username": "alice", "password": "alice-password" }), None)).await);
        let b1 = cookie_of(&call(&app, json_req("POST", "/auth/login", json!({ "username": "bob", "password": "bob-password-1" }), None)).await);
        assert_ne!(a1, a2);

        for c in [&a1, &a2, &b1] {
            assert_eq!(call(&app, get_req("/api/servers", Some(c))).await.status(), StatusCode::OK);
        }

        // Admin-only things: pairing code for alice, refused for bob
        assert_eq!(call(&app, json_req("POST", "/api/server/pairing-code", json!({}), Some(&a1))).await.status(), StatusCode::OK);
        assert_eq!(call(&app, json_req("POST", "/api/server/pairing-code", json!({}), Some(&b1))).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(call(&app, get_req("/auth/users", Some(&b1))).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(call(&app, get_req("/auth/users", Some(&a1))).await.status(), StatusCode::OK);

        // Session list: alice sees everyone, bob only himself; kicking alice's 2nd browser leaves the rest alone
        let all = axum::body::to_bytes(call(&app, get_req("/auth/sessions", Some(&a1))).await.into_body(), 1 << 20).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&all).unwrap();
        assert_eq!(all.as_array().unwrap().len(), 3);
        let mine = axum::body::to_bytes(call(&app, get_req("/auth/sessions", Some(&b1))).await.into_body(), 1 << 20).await.unwrap();
        let mine: serde_json::Value = serde_json::from_slice(&mine).unwrap();
        assert_eq!(mine.as_array().unwrap().len(), 1);
        let a2_id = all.as_array().unwrap().iter().find(|s| s["current"] == false && s["username"] == "alice").unwrap()["id"].as_str().unwrap().to_string();
        let kick = call(&app, Request::builder().method("DELETE").uri(format!("/auth/sessions/{a2_id}")).header(header::COOKIE, a1.as_str()).body(Body::empty()).unwrap()).await;
        assert_eq!(kick.status(), StatusCode::OK);
        assert_eq!(call(&app, get_req("/api/servers", Some(&a2))).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, get_req("/api/servers", Some(&a1))).await.status(), StatusCode::OK);
        assert_eq!(call(&app, get_req("/api/servers", Some(&b1))).await.status(), StatusCode::OK);

        // Cross-site request with a valid cookie but a foreign Origin is refused (CSRF defence in depth)
        let evil = Request::builder().method("POST").uri("/api/servers/import").header(header::COOKIE, a1.as_str())
            .header(header::HOST, "mc.example.org:7777").header(header::ORIGIN, "http://evil.example")
            .header(header::CONTENT_TYPE, "application/json").body(Body::from("{}")).unwrap();
        assert_eq!(call(&app, evil).await.status(), StatusCode::FORBIDDEN);

        // Logout ends only that session
        let out = call(&app, json_req("POST", "/auth/logout", json!({}), Some(&b1))).await;
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(call(&app, get_req("/api/servers", Some(&b1))).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, get_req("/api/servers", Some(&a1))).await.status(), StatusCode::OK);

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn api_token_works_for_automation() {
        let dir = std::env::temp_dir().join(format!("mcmanager-web-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let state = crate::state::build_state_for_test(&dir).await;
        let auth = AuthState::load(&dir).await.unwrap();
        let token = tokio::fs::read_to_string(dir.join("api_token")).await.unwrap();
        let app = build_app(state, WebOptions { web_dir: dir.to_string_lossy().to_string(), auth: Some(auth), remote: None });
        let ok = Request::builder().uri("/api/servers").header(header::AUTHORIZATION, format!("Bearer {}", token.trim())).body(Body::empty()).unwrap();
        assert_eq!(call(&app, ok).await.status(), StatusCode::OK);
        let bad = Request::builder().uri("/api/servers").header(header::AUTHORIZATION, "Bearer wrong").body(Body::empty()).unwrap();
        assert_eq!(call(&app, bad).await.status(), StatusCode::UNAUTHORIZED);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
