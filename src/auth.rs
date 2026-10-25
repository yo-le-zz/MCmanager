//! Login for the web UI when MCManager is reachable over a network
//! (`mcmanager-server`, Docker, a VPS...) - the desktop app bound to
//! `127.0.0.1` has none, as before.
//!
//! Multi-user and multi-connection by design: any number of accounts, any
//! number of simultaneous logged-in browsers per account, each one an
//! independent session that can be listed and revoked individually.
//!
//! - Passwords: PBKDF2-HMAC-SHA256 (310 000 rounds, per-user random salt),
//!   compared in constant time. Never stored or logged in clear.
//! - Sessions: 256-bit random token in an `HttpOnly; SameSite=Strict`
//!   cookie. The server keeps only a SHA-256 of the token, so a memory dump
//!   or a debug print can't be replayed as a login. 12 h idle / 7 d absolute.
//! - Brute force: failed logins are counted per client address, with a
//!   temporary lockout.
//! - CSRF: besides `SameSite=Strict`, requests carrying an `Origin` whose
//!   host differs from the `Host` header are refused.
//! - Automation (`mcmanager cli`, scripts): a static API token in
//!   `<data_dir>/api_token` (owner-only), sent as `Authorization: Bearer`.
//!
//! The `/remote/*` endpoints (machine-to-machine, RSA-signed) and the static
//! UI files deliberately sit outside this: they have their own protection,
//! and the login screen itself has to be loadable.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::{ConnectInfo, Path as AxPath, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, RwLock};

const PBKDF2_ROUNDS: u32 = 310_000;
const COOKIE_NAME: &str = "mcm_session";
const SESSION_IDLE: chrono::Duration = chrono::Duration::hours(12);
const SESSION_ABSOLUTE: chrono::Duration = chrono::Duration::days(7);
const MAX_FAILURES: usize = 8;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    salt: String,
    hash: String,
    rounds: u32,
    pub admin: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct SessionInfo {
    /// Short public id used to list / revoke the session (never the token).
    id: String,
    username: String,
    created_at: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    ip: String,
    user_agent: String,
}

pub struct AuthState {
    data_dir: PathBuf,
    users: RwLock<Vec<User>>,
    /// mtime of users.json as last read/written by this process: lets a CLI
    /// (`mcmanager-server user add ...`, `docker exec`) change accounts under
    /// a running daemon without restarting it.
    users_mtime: Mutex<Option<std::time::SystemTime>>,
    /// sha256(token) hex -> session
    sessions: RwLock<HashMap<String, SessionInfo>>,
    failures: Mutex<HashMap<String, Vec<Instant>>>,
    api_token_hash: [u8; 32],
    trust_proxy: bool,
    secure_cookie: bool,
}

fn users_path(data_dir: &Path) -> PathBuf {
    data_dir.join("users.json")
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn derive(password: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, rounds, &mut out);
    out
}

fn make_user(username: &str, password: &str, admin: bool) -> Result<User> {
    validate_username(username)?;
    if password.chars().count() < MIN_PASSWORD_LEN {
        anyhow::bail!("le mot de passe doit faire au moins {MIN_PASSWORD_LEN} caracteres");
    }
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    Ok(User {
        username: username.to_string(),
        salt: B64.encode(salt),
        hash: B64.encode(derive(password, &salt, PBKDF2_ROUNDS)),
        rounds: PBKDF2_ROUNDS,
        admin,
        created_at: Utc::now(),
    })
}

fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        anyhow::bail!("nom d'utilisateur invalide (1 a 32 caracteres : lettres, chiffres, _ - .)");
    }
    Ok(())
}

fn verify_password(user: &User, password: &str) -> bool {
    let Ok(salt) = B64.decode(&user.salt) else { return false };
    let Ok(expected) = B64.decode(&user.hash) else { return false };
    let got = derive(password, &salt, user.rounds);
    got.ct_eq(expected.as_slice()).into()
}

impl AuthState {
    /// Loads users and the API token (creating the token on first run).
    pub async fn load(data_dir: &Path) -> Result<Arc<Self>> {
        let users: Vec<User> = match tokio::fs::read_to_string(users_path(data_dir)).await {
            Ok(s) => serde_json::from_str(&s).context("users.json corrompu")?,
            Err(_) => Vec::new(),
        };
        let token_path = data_dir.join("api_token");
        let token = match tokio::fs::read_to_string(&token_path).await {
            Ok(t) if t.trim().len() >= 32 => t.trim().to_string(),
            _ => {
                let t = random_hex(32);
                tokio::fs::write(&token_path, &t).await.context("ecriture de api_token")?;
                crate::secrets::restrict_to_owner(&token_path).await;
                t
            }
        };
        let flag = |name: &str| std::env::var(name).map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on")).unwrap_or(false);
        Ok(Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            users: RwLock::new(users),
            users_mtime: Mutex::new(tokio::fs::metadata(users_path(data_dir)).await.ok().and_then(|m| m.modified().ok())),
            sessions: RwLock::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            api_token_hash: Sha256::digest(token.as_bytes()).into(),
            trust_proxy: flag("MCMANAGER_TRUST_PROXY"),
            secure_cookie: flag("MCMANAGER_COOKIE_SECURE"),
        }))
    }

    async fn persist(&self, users: &[User]) -> Result<()> {
        let path = users_path(&self.data_dir);
        tokio::fs::write(&path, serde_json::to_string_pretty(users)?).await?;
        crate::secrets::restrict_to_owner(&path).await;
        *self.users_mtime.lock().await = tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok());
        Ok(())
    }

    /// Re-reads users.json if another process changed it; sessions of
    /// accounts that no longer exist are dropped.
    async fn reload_if_changed(&self) {
        let mtime = tokio::fs::metadata(users_path(&self.data_dir)).await.ok().and_then(|m| m.modified().ok());
        let mut known = self.users_mtime.lock().await;
        if *known == mtime {
            return;
        }
        if let Ok(s) = tokio::fs::read_to_string(users_path(&self.data_dir)).await {
            if let Ok(list) = serde_json::from_str::<Vec<User>>(&s) {
                let names: Vec<String> = list.iter().map(|u| u.username.clone()).collect();
                *self.users.write().await = list;
                self.sessions.write().await.retain(|_, sess| names.contains(&sess.username));
                *known = mtime;
            }
        }
    }

    pub async fn user_count(&self) -> usize {
        self.users.read().await.len()
    }

    pub async fn list_users(&self) -> Vec<User> {
        self.users.read().await.clone()
    }

    /// Creates a user, or - when `replace` - sets a new password for an existing one.
    pub async fn add_user(&self, username: &str, password: &str, admin: bool, replace: bool) -> Result<()> {
        let mut users = self.users.write().await;
        let exists = users.iter().any(|u| u.username == username);
        if exists && !replace {
            anyhow::bail!("l'utilisateur existe deja");
        }
        let mut new_user = make_user(username, password, admin)?;
        if let Some(old) = users.iter().find(|u| u.username == username) {
            new_user.admin = old.admin;
            new_user.created_at = old.created_at;
        }
        users.retain(|u| u.username != username);
        users.push(new_user);
        let snapshot = users.clone();
        drop(users);
        self.persist(&snapshot).await?;
        if replace {
            // A password change ends every session of that account.
            self.sessions.write().await.retain(|_, s| s.username != username);
        }
        Ok(())
    }

    pub async fn remove_user(&self, username: &str) -> Result<bool> {
        let mut users = self.users.write().await;
        let target_is_admin = users.iter().find(|u| u.username == username).map(|u| u.admin).unwrap_or(false);
        if target_is_admin && users.iter().filter(|u| u.admin).count() <= 1 {
            anyhow::bail!("impossible de supprimer le dernier administrateur");
        }
        let before = users.len();
        users.retain(|u| u.username != username);
        let removed = users.len() != before;
        let snapshot = users.clone();
        drop(users);
        if removed {
            self.persist(&snapshot).await?;
            self.sessions.write().await.retain(|_, s| s.username != username);
        }
        Ok(removed)
    }

    /// First start with no account: either the one given through
    /// `MCMANAGER_ADMIN_USER` / `MCMANAGER_ADMIN_PASSWORD` (Docker), or an
    /// `admin` with a random password, returned so the caller can print it
    /// once. Does nothing if users already exist.
    pub async fn bootstrap_admin(&self) -> Result<Option<(String, String, bool)>> {
        if self.user_count().await > 0 {
            return Ok(None);
        }
        let env_user = std::env::var("MCMANAGER_ADMIN_USER").ok().filter(|s| !s.is_empty());
        let env_pass = std::env::var("MCMANAGER_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty());
        match (env_user, env_pass) {
            (Some(u), Some(p)) => {
                self.add_user(&u, &p, true, false).await?;
                Ok(Some((u, p, false)))
            }
            _ => {
                let password = random_hex(9); // 18 hex chars
                self.add_user("admin", &password, true, false).await?;
                Ok(Some(("admin".to_string(), password, true)))
            }
        }
    }

    fn client_ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
        if self.trust_proxy {
            if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
                if let Some(first) = v.split(',').next() {
                    let first = first.trim();
                    if !first.is_empty() {
                        return first.to_string();
                    }
                }
            }
        }
        peer.map(|p| p.ip().to_string()).unwrap_or_else(|| "inconnu".into())
    }

    async fn locked_out(&self, ip: &str) -> Option<u64> {
        let mut failures = self.failures.lock().await;
        let list = failures.entry(ip.to_string()).or_default();
        list.retain(|t| t.elapsed() < FAILURE_WINDOW);
        if list.len() >= MAX_FAILURES {
            let oldest = list.iter().min().copied().unwrap_or_else(Instant::now);
            Some(FAILURE_WINDOW.saturating_sub(oldest.elapsed()).as_secs().max(1))
        } else {
            None
        }
    }

    async fn record_failure(&self, ip: &str) {
        self.failures.lock().await.entry(ip.to_string()).or_default().push(Instant::now());
    }

    async fn create_session(&self, username: &str, ip: &str, user_agent: &str) -> String {
        let token = random_hex(32);
        let now = Utc::now();
        let info = SessionInfo {
            id: random_hex(6),
            username: username.to_string(),
            created_at: now,
            last_seen: now,
            ip: ip.to_string(),
            user_agent: user_agent.chars().take(160).collect(),
        };
        let mut sessions = self.sessions.write().await;
        sessions.retain(|_, s| now - s.last_seen < SESSION_IDLE && now - s.created_at < SESSION_ABSOLUTE);
        sessions.insert(sha256_hex(&token), info);
        token
    }

    /// Resolves the session behind a request's cookie, refreshing its
    /// idle timer. Returns (session id, user).
    async fn session_from_headers(&self, headers: &HeaderMap) -> Option<(String, User)> {
        let token = cookie_value(headers, COOKIE_NAME)?;
        self.reload_if_changed().await;
        let key = sha256_hex(&token);
        let now = Utc::now();
        let username = {
            let mut sessions = self.sessions.write().await;
            let s = sessions.get_mut(&key)?;
            if now - s.last_seen >= SESSION_IDLE || now - s.created_at >= SESSION_ABSOLUTE {
                sessions.remove(&key);
                return None;
            }
            s.last_seen = now;
            (s.id.clone(), s.username.clone())
        };
        let user = self.users.read().await.iter().find(|u| u.username == username.1).cloned()?;
        Some((username.0, user))
    }

    fn bearer_ok(&self, headers: &HeaderMap) -> bool {
        let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else { return false };
        let Some(token) = value.strip_prefix("Bearer ") else { return false };
        let hash: [u8; 32] = Sha256::digest(token.trim().as_bytes()).into();
        hash.ct_eq(&self.api_token_hash).into()
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// Same-origin check for browser requests: refuses a mismatching `Origin`.
fn origin_ok(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else { return false };
    let origin_host = origin.split_once("://").map(|(_, rest)| rest).unwrap_or(origin);
    origin_host.trim_end_matches('/') == host
}

// ───────────────────────── middleware ─────────────────────────

/// Gate for `/api/*`. Everything else (static UI, `/auth/*`, `/remote/*`,
/// `/healthz`) passes through - see module docs.
pub async fn require_auth(State(auth): State<Arc<AuthState>>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/") {
        return next.run(req).await;
    }
    if auth.bearer_ok(req.headers()) {
        return next.run(req).await;
    }
    if !origin_ok(req.headers()) {
        return err(StatusCode::FORBIDDEN, "Origine refusee.");
    }
    match auth.session_from_headers(req.headers()).await {
        Some((_, user)) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None => err(StatusCode::UNAUTHORIZED, "Connexion requise."),
    }
}

// ───────────────────────── routes /auth ─────────────────────────

pub fn router(auth: Arc<AuthState>) -> Router {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/auth/password", post(change_password))
        .route("/auth/sessions", get(list_sessions))
        .route("/auth/sessions/:id", delete(revoke_session))
        .route("/auth/users", get(list_users).post(create_user))
        .route("/auth/users/:name", delete(delete_user))
        .with_state(auth)
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(State(auth): State<Arc<AuthState>>, peer: Option<ConnectInfo<SocketAddr>>, headers: HeaderMap, Json(body): Json<LoginBody>) -> Response {
    if !origin_ok(&headers) {
        return err(StatusCode::FORBIDDEN, "Origine refusee.");
    }
    let ip = auth.client_ip(&headers, peer.map(|p| p.0));
    if let Some(wait) = auth.locked_out(&ip).await {
        return err(StatusCode::TOO_MANY_REQUESTS, format!("Trop de tentatives echouees. Reessayez dans {} min.", wait / 60 + 1));
    }
    auth.reload_if_changed().await;
    let user = auth.users.read().await.iter().find(|u| u.username == body.username).cloned();
    // Always run the key derivation, even for an unknown user, so response time doesn't reveal which usernames exist.
    let ok = match &user {
        Some(u) => verify_password(u, &body.password),
        None => {
            let _ = derive(&body.password, b"mcmanager-dummy-salt", PBKDF2_ROUNDS);
            false
        }
    };
    let Some(user) = user.filter(|_| ok) else {
        auth.record_failure(&ip).await;
        return err(StatusCode::UNAUTHORIZED, "Identifiants incorrects.");
    };
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = auth.create_session(&user.username, &ip, ua).await;
    let cookie = format!(
        "{COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
        SESSION_ABSOLUTE.num_seconds(),
        if auth.secure_cookie { "; Secure" } else { "" }
    );
    let mut resp = Json(json!({ "ok": true, "username": user.username, "admin": user.admin })).into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

async fn logout(State(auth): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, COOKIE_NAME) {
        auth.sessions.write().await.remove(&sha256_hex(&token));
    }
    let mut resp = Json(json!({ "ok": true })).into_response();
    resp.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_static("mcm_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"));
    resp
}

async fn me(State(auth): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    match auth.session_from_headers(&headers).await {
        Some((_, u)) => Json(json!({ "auth_required": true, "username": u.username, "admin": u.admin })).into_response(),
        None => err(StatusCode::UNAUTHORIZED, "Connexion requise."),
    }
}

#[derive(Deserialize)]
struct ChangePasswordBody {
    current: String,
    new: String,
}

async fn change_password(State(auth): State<Arc<AuthState>>, headers: HeaderMap, Json(body): Json<ChangePasswordBody>) -> Response {
    if !origin_ok(&headers) {
        return err(StatusCode::FORBIDDEN, "Origine refusee.");
    }
    let Some((_, user)) = auth.session_from_headers(&headers).await else { return err(StatusCode::UNAUTHORIZED, "Connexion requise.") };
    if !verify_password(&user, &body.current) {
        return err(StatusCode::UNAUTHORIZED, "Mot de passe actuel incorrect.");
    }
    match auth.add_user(&user.username, &body.new, user.admin, true).await {
        // add_user(replace) ends all sessions of the account, this one included: log in again.
        Ok(()) => Json(json!({ "ok": true, "relogin": true })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn list_sessions(State(auth): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    let Some((current_id, user)) = auth.session_from_headers(&headers).await else { return err(StatusCode::UNAUTHORIZED, "Connexion requise.") };
    let now = Utc::now();
    let sessions = auth.sessions.read().await;
    let mut list: Vec<_> = sessions
        .values()
        .filter(|s| now - s.last_seen < SESSION_IDLE && (user.admin || s.username == user.username))
        .map(|s| json!({
            "id": s.id, "username": s.username, "ip": s.ip, "user_agent": s.user_agent,
            "created_at": s.created_at, "last_seen": s.last_seen, "current": s.id == current_id,
        }))
        .collect();
    list.sort_by(|a, b| b["last_seen"].as_str().cmp(&a["last_seen"].as_str()));
    Json(list).into_response()
}

async fn revoke_session(State(auth): State<Arc<AuthState>>, headers: HeaderMap, AxPath(id): AxPath<String>) -> Response {
    if !origin_ok(&headers) {
        return err(StatusCode::FORBIDDEN, "Origine refusee.");
    }
    let Some((_, user)) = auth.session_from_headers(&headers).await else { return err(StatusCode::UNAUTHORIZED, "Connexion requise.") };
    let mut sessions = auth.sessions.write().await;
    let key = sessions.iter().find(|(_, s)| s.id == id && (user.admin || s.username == user.username)).map(|(k, _)| k.clone());
    match key {
        Some(k) => {
            sessions.remove(&k);
            Json(json!({ "ok": true })).into_response()
        }
        None => err(StatusCode::NOT_FOUND, "Session introuvable."),
    }
}

async fn require_admin(auth: &AuthState, headers: &HeaderMap) -> Result<User, Response> {
    if !origin_ok(headers) {
        return Err(err(StatusCode::FORBIDDEN, "Origine refusee."));
    }
    match auth.session_from_headers(headers).await {
        Some((_, u)) if u.admin => Ok(u),
        Some(_) => Err(err(StatusCode::FORBIDDEN, "Reserve aux administrateurs.")),
        None => Err(err(StatusCode::UNAUTHORIZED, "Connexion requise.")),
    }
}

async fn list_users(State(auth): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if let Err(r) = require_admin(&auth, &headers).await {
        return r;
    }
    let users: Vec<_> = auth.list_users().await.into_iter().map(|u| json!({ "username": u.username, "admin": u.admin, "created_at": u.created_at })).collect();
    Json(users).into_response()
}

#[derive(Deserialize)]
struct CreateUserBody {
    username: String,
    password: String,
    #[serde(default)]
    admin: bool,
}

async fn create_user(State(auth): State<Arc<AuthState>>, headers: HeaderMap, Json(body): Json<CreateUserBody>) -> Response {
    if let Err(r) = require_admin(&auth, &headers).await {
        return r;
    }
    match auth.add_user(&body.username, &body.password, body.admin, false).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn delete_user(State(auth): State<Arc<AuthState>>, headers: HeaderMap, AxPath(name): AxPath<String>) -> Response {
    if let Err(r) = require_admin(&auth, &headers).await {
        return r;
    }
    match auth.remove_user(&name).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "Utilisateur introuvable."),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh() -> (Arc<AuthState>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mcmanager-auth-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        (AuthState::load(&dir).await.unwrap(), dir)
    }

    #[test]
    fn password_hash_roundtrip() {
        let u = make_user("alice", "correct horse", true).unwrap();
        assert!(verify_password(&u, "correct horse"));
        assert!(!verify_password(&u, "correct horse "));
        assert!(!verify_password(&u, ""));
        assert_ne!(u.hash, B64.encode("correct horse"));
    }

    #[test]
    fn rejects_weak_or_odd_input() {
        assert!(make_user("bob", "short", false).is_err());
        assert!(make_user("", "long enough pw", false).is_err());
        assert!(make_user("bad name!", "long enough pw", false).is_err());
        assert!(make_user("../etc", "long enough pw", false).is_err());
    }

    #[test]
    fn origin_check() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("mc.example.org:7777"));
        assert!(origin_ok(&h)); // no Origin header (curl, same-origin GET)
        h.insert(header::ORIGIN, HeaderValue::from_static("http://mc.example.org:7777"));
        assert!(origin_ok(&h));
        h.insert(header::ORIGIN, HeaderValue::from_static("http://evil.example"));
        assert!(!origin_ok(&h));
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; mcm_session=abc123; b=2"));
        assert_eq!(cookie_value(&h, "mcm_session").as_deref(), Some("abc123"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    /// Several simultaneous logins for the same and different accounts are
    /// independent: revoking one leaves the others working, changing a
    /// password ends that account's sessions only.
    #[tokio::test]
    async fn many_sessions_are_independent() {
        let (auth, dir) = fresh().await;
        auth.add_user("alice", "alice-password", true, false).await.unwrap();
        auth.add_user("bob", "bob-password-1", false, false).await.unwrap();

        let t_a1 = auth.create_session("alice", "10.0.0.1", "firefox").await;
        let t_a2 = auth.create_session("alice", "10.0.0.2", "phone").await;
        let t_b1 = auth.create_session("bob", "10.0.0.3", "chrome").await;

        let headers_for = |token: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, HeaderValue::from_str(&format!("{COOKIE_NAME}={token}")).unwrap());
            h
        };
        assert!(auth.session_from_headers(&headers_for(&t_a1)).await.is_some());
        assert!(auth.session_from_headers(&headers_for(&t_a2)).await.is_some());
        assert!(auth.session_from_headers(&headers_for(&t_b1)).await.is_some());
        assert!(auth.session_from_headers(&headers_for("deadbeef")).await.is_none());

        // The token itself is never stored.
        assert!(auth.sessions.read().await.keys().all(|k| *k != t_a1 && *k != t_a2 && *k != t_b1));

        auth.add_user("bob", "new-bob-password", false, true).await.unwrap();
        assert!(auth.session_from_headers(&headers_for(&t_b1)).await.is_none(), "password change must end that account's sessions");
        assert!(auth.session_from_headers(&headers_for(&t_a1)).await.is_some(), "other accounts are untouched");

        // The last admin can't be deleted; a regular user can.
        assert!(auth.remove_user("alice").await.is_err());
        assert!(auth.remove_user("bob").await.unwrap());
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn brute_force_lockout() {
        let (auth, dir) = fresh().await;
        assert!(auth.locked_out("1.2.3.4").await.is_none());
        for _ in 0..MAX_FAILURES {
            auth.record_failure("1.2.3.4").await;
        }
        assert!(auth.locked_out("1.2.3.4").await.is_some());
        assert!(auth.locked_out("5.6.7.8").await.is_none(), "other addresses are not affected");
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn users_survive_restart_and_bootstrap_runs_once() {
        let (auth, dir) = fresh().await;
        let first = auth.bootstrap_admin().await.unwrap();
        assert!(first.is_some());
        let (name, password, generated) = first.unwrap();
        assert_eq!(name, "admin");
        assert!(generated && password.len() >= 16);
        assert!(auth.bootstrap_admin().await.unwrap().is_none(), "no second bootstrap");

        let reloaded = AuthState::load(&dir).await.unwrap();
        let u = reloaded.list_users().await.into_iter().find(|u| u.username == "admin").unwrap();
        assert!(verify_password(&u, &password));
        // users.json holds no clear-text password.
        let raw = tokio::fs::read_to_string(users_path(&dir)).await.unwrap();
        assert!(!raw.contains(&password));
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
