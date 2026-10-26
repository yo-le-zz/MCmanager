//! The "hub": makes servers that live on *other* MCManager instances (paired
//! through the remote-control feature) appear and behave exactly like local
//! ones in this instance's dashboard.
//!
//! ## How
//!
//! - A background refresher polls every paired instance (`GET /api/servers`
//!   through the encrypted tunnel, see `remote.rs`) and keeps a cache of
//!   their servers plus an `id -> instance` index.
//! - `GET /api/servers` (api.rs) merges the cached remote servers into the
//!   local list, each tagged with `"remote": {label, host, online}` - the UI
//!   shows that as a small "distant" badge.
//! - A middleware ([`proxy_middleware`]) sits in front of the whole REST API.
//!   Any request for `/api/servers/<id>/...` whose id belongs to a remote
//!   server is forwarded, unchanged, through the tunnel to that instance's
//!   own API, and the answer is relayed back. Console commands, file
//!   editing, schematics, mods/plugins, backups, settings, start/stop...
//!   all work with no per-feature remote code, because the remote machine
//!   simply serves its normal API.
//! - The live console is the one thing a plain request/response can't
//!   carry; [`console_subscribe`] runs one shared long-polling reader per
//!   remote server and fans its lines out to every connected browser.
//!
//! ## Limits (deliberate, documented in the UI/docs)
//!
//! - Everything is buffered in memory on both ends: transfers are capped at
//!   [`remote::TUNNEL_BODY_MAX`] (512 MB). A multi-GB world should be
//!   moved some other way (rsync/scp), not through the dashboard.
//! - "Open in file explorer" is refused for remote servers (it would open a
//!   window on the *remote* machine, where nobody is looking).
//! - No chaining: a request that arrived through a tunnel is never
//!   forwarded again, so A -> B -> C or A -> B -> A loops cannot happen.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use tokio::sync::{broadcast, Mutex, RwLock};
use uuid::Uuid;

use crate::remote::{self, RemoteIdentity, RemoteTarget, TUNNEL_BODY_MAX, TUNNEL_HEADER};
use crate::state::AppState;

/// Header the UI sets on `POST /api/servers` (create) and
/// `POST /api/servers/import` to say "do this on that remote instance".
pub const TARGET_HEADER: &str = "x-mcm-target";

const REFRESH_EVERY: Duration = Duration::from_secs(5);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
const CONSOLE_IDLE_EXIT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
pub struct TargetStatus {
    pub label: String,
    pub host: String,
    pub online: bool,
    pub last_error: Option<String>,
    pub last_ok: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub server_count: usize,
    #[serde(skip)]
    pub servers: Vec<serde_json::Value>,
}

struct RemoteConsole {
    inner: Mutex<ConsoleInner>,
    tx: broadcast::Sender<String>,
}

struct ConsoleInner {
    backlog: Vec<String>,
    running: bool,
}

#[derive(Default)]
pub struct RemoteHub {
    targets: RwLock<HashMap<String, TargetStatus>>,
    /// server id -> instance label.
    index: RwLock<HashMap<Uuid, String>>,
    consoles: Mutex<HashMap<Uuid, Arc<RemoteConsole>>>,
    identity: tokio::sync::OnceCell<Arc<RemoteIdentity>>,
}

impl RemoteHub {
    pub async fn label_for_server(&self, id: Uuid) -> Option<String> {
        self.index.read().await.get(&id).cloned()
    }

    /// Drops everything known about an instance (it was un-paired).
    pub async fn forget(&self, label: &str) {
        self.targets.write().await.remove(label);
        self.index.write().await.retain(|_, l| l != label);
    }
}

pub async fn identity(state: &AppState) -> anyhow::Result<Arc<RemoteIdentity>> {
    state
        .hub
        .identity
        .get_or_try_init(|| async { remote::load_or_create_identity(&state.data_dir).await.map(Arc::new) })
        .await
        .cloned()
}

pub async fn target_by_label(state: &AppState, label: &str) -> Option<RemoteTarget> {
    remote::load_targets(&state.data_dir).await.into_iter().find(|t| t.label == label)
}

// ───────────────────────── rafraichissement ─────────────────────────

/// Re-reads one instance's server list and updates the cache + index.
/// Failure keeps the previous list (flagged offline) so the servers don't
/// vanish from the dashboard the moment a VPS blips.
pub async fn refresh_target(state: &AppState, target: &RemoteTarget) {
    let started = Instant::now();
    let outcome: anyhow::Result<Vec<serde_json::Value>> = async {
        let identity = identity(state).await?;
        let resp = tokio::time::timeout(REFRESH_TIMEOUT, remote::client_http(&state.http, &identity, target, "GET", "/api/servers", None, Vec::new()))
            .await
            .map_err(|_| anyhow::anyhow!("delai depasse"))??;
        if resp.status != 200 {
            anyhow::bail!("l'instance distante a repondu HTTP {}", resp.status);
        }
        Ok(serde_json::from_slice(&resp.body)?)
    }
    .await;

    let mut targets = state.hub.targets.write().await;
    let entry = targets.entry(target.label.clone()).or_insert_with(|| TargetStatus {
        label: target.label.clone(),
        host: target.host.clone(),
        online: false,
        last_error: None,
        last_ok: None,
        latency_ms: None,
        server_count: 0,
        servers: Vec::new(),
    });
    entry.host = target.host.clone();
    match outcome {
        Ok(servers) => {
            entry.online = true;
            entry.last_error = None;
            entry.last_ok = Some(Utc::now());
            entry.latency_ms = Some(started.elapsed().as_millis() as u64);
            entry.server_count = servers.len();
            let mut index = state.hub.index.write().await;
            index.retain(|_, l| l != &target.label);
            for s in &servers {
                if let Some(id) = s["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                    // A local server always wins over a remote one with the same id (should never happen: v4 uuids).
                    if !state.servers.try_read().map(|l| l.contains_key(&id)).unwrap_or(false) {
                        index.insert(id, target.label.clone());
                    }
                }
            }
            entry.servers = servers;
        }
        Err(e) => {
            entry.online = false;
            entry.last_error = Some(e.to_string());
            entry.latency_ms = None;
        }
    }
}

pub async fn refresh_all(state: &AppState) {
    let targets = remote::load_targets(&state.data_dir).await;
    let labels: Vec<String> = targets.iter().map(|t| t.label.clone()).collect();
    // Instances un-paired since the last pass.
    let stale: Vec<String> = state.hub.targets.read().await.keys().filter(|l| !labels.contains(l)).cloned().collect();
    for l in stale {
        state.hub.forget(&l).await;
    }
    futures_util::future::join_all(targets.iter().map(|t| refresh_target(state, t))).await;
}

/// Starts the background refresher. Call once per process (the web binary
/// and the server daemon both do).
pub fn spawn_refresher(state: AppState) {
    tokio::spawn(async move {
        loop {
            refresh_all(&state).await;
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

pub async fn status_snapshot(state: &AppState) -> Vec<TargetStatus> {
    let targets = remote::load_targets(&state.data_dir).await;
    let known = state.hub.targets.read().await;
    targets
        .iter()
        .map(|t| {
            known.get(&t.label).cloned().unwrap_or_else(|| TargetStatus {
                label: t.label.clone(),
                host: t.host.clone(),
                online: false,
                last_error: Some("pas encore contacte".into()),
                last_ok: None,
                latency_ms: None,
                server_count: 0,
                servers: Vec::new(),
            })
        })
        .collect()
}

/// Cached remote servers, each tagged so the UI can badge them.
pub async fn remote_servers_json(state: &AppState) -> Vec<serde_json::Value> {
    let targets = state.hub.targets.read().await;
    let mut out = Vec::new();
    for status in targets.values() {
        for s in &status.servers {
            let mut s = s.clone();
            if let Some(obj) = s.as_object_mut() {
                obj.insert("remote".into(), json!({ "label": status.label, "host": status.host, "online": status.online }));
            }
            out.push(s);
        }
    }
    out
}

// ───────────────────────── relais ─────────────────────────

pub async fn forward(state: &AppState, label: &str, method: &str, path_and_query: &str, content_type: Option<&str>, body: Vec<u8>) -> anyhow::Result<remote::TunnelResponse> {
    let target = target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("instance distante « {label} » inconnue"))?;
    let identity = identity(state).await?;
    remote::client_http(&state.http, &identity, &target, method, path_and_query, content_type, body)
        .await
        .map_err(|e| anyhow::anyhow!("instance distante « {label} » : {e}"))
}

fn json_error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, axum::Json(json!({ "error": msg.into() }))).into_response()
}

/// Sits in front of the whole REST API (see module docs).
pub async fn proxy_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Arrived through a tunnel already: serve it locally, never forward again.
    if req.headers().contains_key(TUNNEL_HEADER) {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    let label: Option<String> = if let Some(rest) = path.strip_prefix("/api/servers/") {
        match Uuid::parse_str(rest.split('/').next().unwrap_or("")) {
            Ok(id) => state.hub.label_for_server(id).await,
            Err(_) => None,
        }
    } else if method == axum::http::Method::POST && (path == "/api/servers" || path == "/api/servers/import" || path == "/api/servers/import-zip") {
        match req.headers().get(TARGET_HEADER).and_then(|v| v.to_str().ok()) {
            Some(l) if !l.is_empty() && l != "local" => Some(l.to_string()),
            _ => None,
        }
    } else {
        None
    };
    let Some(label) = label else {
        return next.run(req).await;
    };

    // The live console WebSocket is bridged by `ws::console_ws`, not forwarded as plain HTTP.
    if path.ends_with("/ws") {
        return next.run(req).await;
    }
    if path.ends_with("/open-folder") {
        return json_error(StatusCode::BAD_REQUEST, "Indisponible pour un serveur distant : l'explorateur s'ouvrirait sur la machine distante. Utilisez l'onglet Fichiers.");
    }

    let path_and_query = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| path.clone());
    let content_type = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let body = match axum::body::to_bytes(req.into_body(), TUNNEL_BODY_MAX).await {
        Ok(b) => b.to_vec(),
        Err(_) => return json_error(StatusCode::PAYLOAD_TOO_LARGE, format!("Corps trop volumineux pour le relais distant (maximum {} Mo).", TUNNEL_BODY_MAX / (1024 * 1024))),
    };

    match forward(&state, &label, method.as_str(), &path_and_query, content_type.as_deref(), body).await {
        Ok(resp) => {
            // Create / delete / update change what the list should show:
            // refresh now rather than waiting for the next background pass.
            if method != axum::http::Method::GET {
                if let Some(target) = target_by_label(&state, &label).await {
                    refresh_target(&state, &target).await;
                }
            }
            let mut response = Response::new(Body::from(resp.body));
            *response.status_mut() = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::BAD_GATEWAY);
            if let Some(ct) = resp.content_type.and_then(|v| HeaderValue::from_str(&v).ok()) {
                response.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            if let Some(cd) = resp.content_disposition.and_then(|v| HeaderValue::from_str(&v).ok()) {
                response.headers_mut().insert(header::CONTENT_DISPOSITION, cd);
            }
            response
        }
        Err(e) => json_error(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

// ───────────────────────── console distante ─────────────────────────

/// Subscribes to a remote server's console: returns the lines gathered so
/// far plus a receiver for new ones. One shared long-poll reader per remote
/// server, however many browsers are watching.
pub async fn console_subscribe(state: &AppState, id: Uuid, label: &str) -> (Vec<String>, broadcast::Receiver<String>) {
    let console = {
        let mut consoles = state.hub.consoles.lock().await;
        consoles
            .entry(id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(1024);
                Arc::new(RemoteConsole { inner: Mutex::new(ConsoleInner { backlog: Vec::new(), running: false }), tx })
            })
            .clone()
    };
    let mut inner = console.inner.lock().await;
    let backlog = inner.backlog.clone();
    let rx = console.tx.subscribe();
    if !inner.running {
        inner.running = true;
        tokio::spawn(console_poll_loop(state.clone(), id, label.to_string(), console.clone()));
    }
    (backlog, rx)
}

async fn console_push(console: &RemoteConsole, line: String) {
    let mut inner = console.inner.lock().await;
    inner.backlog.push(line.clone());
    if inner.backlog.len() > crate::state::CONSOLE_BACKLOG {
        let excess = inner.backlog.len() - crate::state::CONSOLE_BACKLOG;
        inner.backlog.drain(0..excess);
    }
    let _ = console.tx.send(line);
}

async fn console_poll_loop(state: AppState, id: Uuid, label: String, console: Arc<RemoteConsole>) {
    let mut since: Option<u64> = None;
    let mut idle_since: Option<Instant> = None;
    let mut warned = false;
    loop {
        if console.tx.receiver_count() == 0 {
            let t = *idle_since.get_or_insert_with(Instant::now);
            if t.elapsed() > CONSOLE_IDLE_EXIT {
                // Re-check under the lock: a browser may have subscribed in the meantime.
                let mut inner = console.inner.lock().await;
                if console.tx.receiver_count() == 0 {
                    inner.running = false;
                    drop(inner);
                    state.hub.consoles.lock().await.remove(&id);
                    return;
                }
                idle_since = None;
            }
        } else {
            idle_since = None;
        }

        let Some(target) = target_by_label(&state, &label).await else {
            console.inner.lock().await.running = false;
            state.hub.consoles.lock().await.remove(&id);
            return;
        };
        let result = async {
            let identity = identity(&state).await?;
            remote::client_call_checked(
                &state.http,
                &identity,
                &target,
                json!({ "action": "console", "server_id": id, "since": since, "wait_ms": 8000 }),
                Duration::from_secs(25),
            )
            .await
        }
        .await;

        match result {
            Ok(v) => {
                warned = false;
                if let Some(lines) = v["lines"].as_array() {
                    for l in lines {
                        if let Some(l) = l.as_str() {
                            console_push(&console, l.to_string()).await;
                        }
                    }
                }
                since = v["next"].as_u64().or(since);
                // The remote answers immediately when there is nothing to wait for (no entry yet):
                // avoid a hot loop.
                if v["lines"].as_array().map(|a| a.is_empty()).unwrap_or(true) && v["running"].as_bool() == Some(false) {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
            }
            Err(e) => {
                if !warned {
                    warned = true;
                    console_push(&console, format!("[MCManager] Connexion a l'instance « {label} » perdue ({e}) - nouvelle tentative...")).await;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

/// Sends a console command to a remote server.
pub async fn console_send(state: &AppState, label: &str, id: Uuid, command: &str) -> anyhow::Result<()> {
    let target = target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("instance distante inconnue"))?;
    let identity = identity(state).await?;
    remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "send", "server_id": id, "command": command }), Duration::from_secs(20)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn entry(id: Uuid, folder: &std::path::Path) -> crate::models::ServerEntry {
        crate::models::ServerEntry {
            id, name: "vps-survival".into(), loader: crate::models::Loader::Paper, mc_version: "1.21.4".into(),
            loader_version: None, folder: folder.to_string_lossy().to_string(), jar_name: String::new(), java_path: "java".into(),
            xms_mb: 512, xmx_mb: 1024, port: 25588, extra_args: vec![], eula_accepted: true, auto_backup_minutes: None,
            backup_retention: None, auto_restart: false, auto_restart_delay_secs: 5, scheduled_restart_minutes: None,
            stop_when_empty_minutes: None, dynamic_server: false, aikar_flags: false, managed_addons: vec![], created_at: chrono::Utc::now(),
        }
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    async fn send(app: &axum::Router, method: &str, uri: &str, tunnel: bool) -> Response {
        let mut b = axum::http::Request::builder().method(method).uri(uri);
        if tunnel {
            b = b.header(TUNNEL_HEADER, "1");
        }
        app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap()
    }

    /// A "VPS" (real listener, one server with a file on disk) and a "PC"
    /// that pairs with it: the PC's own API must then show and operate the
    /// VPS's server as if it were local.
    #[tokio::test]
    async fn remote_servers_behave_like_local_ones() {
        let unique = |t: &str| std::env::temp_dir().join(format!("mcmanager-hub-{t}-{}", Uuid::new_v4()));
        let (vps_dir, pc_dir, srv_dir) = (unique("vps"), unique("pc"), unique("srv"));
        for d in [&vps_dir, &pc_dir, &srv_dir] {
            tokio::fs::create_dir_all(d).await.unwrap();
        }
        tokio::fs::write(srv_dir.join("hello.txt"), "bonjour").await.unwrap();

        // VPS side
        let vps_state = crate::state::build_state_for_test(&vps_dir).await;
        let sid = Uuid::new_v4();
        vps_state.servers.write().await.insert(sid, entry(sid, &srv_dir));
        let vps_rt = remote::RemoteRuntime::new(vps_state.clone(), vps_dir.clone()).await.unwrap();
        let port = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
        let rt2 = vps_rt.clone();
        let host_task = tokio::spawn(async move { let _ = remote::serve(rt2, port).await; });
        tokio::time::sleep(Duration::from_millis(200)).await;

        // PC side: pair, refresh
        let pc = crate::state::build_state_for_test(&pc_dir).await;
        let identity = identity(&pc).await.unwrap();
        let (code, _) = vps_rt.generate_pairing_code(10).await;
        remote::client_pair(&pc.http, &pc_dir, &identity, &format!("127.0.0.1:{port}"), "mon-vps", &code).await.unwrap();
        let target = target_by_label(&pc, "mon-vps").await.unwrap();
        refresh_target(&pc, &target).await;
        let app = crate::api::router(pc.clone());

        // 1. merged list, tagged
        let list = body_json(send(&app, "GET", "/api/servers", false).await).await;
        let found = list.as_array().unwrap().iter().find(|s| s["id"] == sid.to_string()).expect("remote server must be listed");
        assert_eq!(found["name"], "vps-survival");
        assert_eq!(found["remote"]["label"], "mon-vps");
        assert_eq!(found["remote"]["online"], true);

        // 2. a request for it is relayed to the VPS (its files, not ours)
        let files = body_json(send(&app, "GET", &format!("/api/servers/{sid}/files?path="), false).await).await;
        assert!(files.as_array().unwrap().iter().any(|f| f["name"] == "hello.txt"), "{files}");

        // 3. status goes through too
        let status = send(&app, "GET", &format!("/api/servers/{sid}/status"), false).await;
        assert_eq!(status.status(), StatusCode::OK);

        // 4. "open in explorer" is refused for remote servers
        assert_eq!(send(&app, "POST", &format!("/api/servers/{sid}/open-folder"), false).await.status(), StatusCode::BAD_REQUEST);

        // 5. a request that already came through a tunnel is never forwarded again (no chains / loops)
        let looped = send(&app, "GET", &format!("/api/servers/{sid}/files?path="), true).await;
        assert_ne!(looped.status(), StatusCode::OK, "tunnelled requests must be served locally, where this server does not exist");

        // 6. the same instance never re-exports remote servers to a tunnelled caller
        let tunnelled_list = body_json(send(&app, "GET", "/api/servers", true).await).await;
        assert_eq!(tunnelled_list.as_array().unwrap().len(), 0);

        // 7. live console bridged through the long-poll reader, shared by two viewers
        {
            let mut rt = vps_state.runtime.write().await;
            let r = rt.entry(sid).or_insert_with(crate::state::ServerRuntime::default);
            r.push_line("[Server] Done (3.2s)!".into());
        }
        let (backlog1, mut rx1) = console_subscribe(&pc, sid, "mon-vps").await;
        let (_backlog2, mut rx2) = console_subscribe(&pc, sid, "mon-vps").await;
        let mut seen = backlog1.clone();
        if seen.is_empty() {
            seen.push(tokio::time::timeout(Duration::from_secs(8), rx1.recv()).await.expect("first line in time").unwrap());
        }
        assert_eq!(seen[0], "[Server] Done (3.2s)!");
        vps_state.runtime.write().await.get_mut(&sid).unwrap().push_line("[Server] <Alex> salut".into());
        let live1 = tokio::time::timeout(Duration::from_secs(8), rx1.recv()).await.expect("live line in time").unwrap();
        let live2 = tokio::time::timeout(Duration::from_secs(8), async { loop { let l = rx2.recv().await.unwrap(); if l.contains("Alex") { return l; } } }).await.expect("second viewer gets it too");
        assert!(live1.contains("Alex") || seen.len() > 1);
        assert!(live2.contains("Alex"));

        // 8. VPS goes away: server stays listed but flagged offline, requests fail cleanly (502), nothing hangs
        host_task.abort();
        tokio::time::sleep(Duration::from_millis(200)).await;
        refresh_target(&pc, &target).await;
        let list = body_json(send(&app, "GET", "/api/servers", false).await).await;
        let found = list.as_array().unwrap().iter().find(|s| s["id"] == sid.to_string()).expect("still listed while offline");
        assert_eq!(found["remote"]["online"], false);
        let gone = send(&app, "GET", &format!("/api/servers/{sid}/files?path="), false).await;
        assert_eq!(gone.status(), StatusCode::BAD_GATEWAY);
        let snapshot = status_snapshot(&pc).await;
        assert!(!snapshot[0].online && snapshot[0].last_error.is_some());

        for d in [&vps_dir, &pc_dir, &srv_dir] {
            tokio::fs::remove_dir_all(d).await.ok();
        }
    }
}
