//! Journal d'audit : qui a fait quoi, quand, depuis où.
//!
//! Fichier `audit.log` (une entrée JSON par ligne, ajout seulement) dans le dossier de données,
//! avec rotation (5 Mo, 5 fichiers). Sont enregistrés :
//!
//! - toute requête **qui modifie** quelque chose (`POST`/`PUT`/`PATCH`/`DELETE` sur `/api/*`), avec
//!   son code de réponse - un refus (`403`) ou un échec de connexion (`401`) est donc visible ;
//! - les accès refusés, même en lecture ;
//! - les téléchargements d'export (un export de serveur sort des données de la machine) ;
//! - les évènements de compte : connexion réussie ou échouée, déconnexion, changement de mot de
//!   passe, création/suppression de compte, changement de droits, déconnexion forcée de session ;
//! - les actions des **machines jumelées** (`machine:<nom>`) et du **bot Discord** (`discord:<nom>`).
//!
//! Volontairement **jamais** enregistrés : corps de requête, paramètres d'URL (`?...`), mots de passe,
//! tokens, contenu de fichiers ou de commandes. Le journal dit *qu'une commande a été envoyée à un
//! serveur*, pas laquelle ; un journal d'audit ne doit pas devenir une fuite de secrets de plus.
//!
//! Lecture : `GET /api/audit` (administrateurs) et onglet *Utilisateurs* de l'interface.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::auth::Actor;

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const KEEP_FILES: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub ts: String,
    pub user: String,
    #[serde(default)]
    pub ip: String,
    /// `http` | `auth` | `remote` | `discord`
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Courte description (jamais de secret) : « connexion refusée », « droits modifiés »...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Entry {
    pub fn new(kind: &str, user: &str, ip: &str) -> Self {
        Entry { ts: Utc::now().to_rfc3339(), user: user.to_string(), ip: ip.to_string(), kind: kind.to_string(), method: None, path: None, status: None, server: None, detail: None }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    pub fn http(mut self, method: &str, path: &str, status: Option<u16>) -> Self {
        self.method = Some(method.to_string());
        self.server = server_in_path(path);
        self.path = Some(strip_query(path).to_string());
        self.status = status;
        self
    }
}

fn strip_query(p: &str) -> &str {
    p.split('?').next().unwrap_or(p)
}

/// L'identifiant de serveur contenu dans un chemin `/api/servers/<uuid>/...`.
pub fn server_in_path(path: &str) -> Option<String> {
    let rest = strip_query(path).strip_prefix("/api/servers/")?;
    Uuid::parse_str(rest.split('/').next()?).ok().map(|u| u.to_string())
}

fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("audit.log")
}

fn write_lock() -> &'static tokio::sync::Mutex<()> {
    static L: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Ajoute une entrée. Un échec d'écriture est journalisé (tracing) puis ignoré : l'audit ne doit
/// jamais empêcher une action légitime... mais il ne doit pas non plus échouer en silence.
pub async fn record(data_dir: &Path, entry: Entry) {
    let _guard = write_lock().lock().await;
    let path = log_path(data_dir);
    if let Ok(meta) = tokio::fs::metadata(&path).await {
        if meta.len() >= MAX_BYTES {
            rotate(&path).await;
        }
    }
    let Ok(mut line) = serde_json::to_string(&entry) else { return };
    line.push('\n');
    let result = async {
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        f.write_all(line.as_bytes()).await?;
        f.flush().await
    }
    .await;
    match result {
        Ok(()) => crate::secrets::restrict_to_owner(&path).await,
        Err(e) => tracing::error!("[audit] impossible d'écrire le journal d'audit : {e}"),
    }
}

async fn rotate(path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let dir = path.parent().unwrap_or(Path::new("."));
    let _ = tokio::fs::remove_file(dir.join(format!("{name}.{KEEP_FILES}"))).await;
    for i in (1..KEEP_FILES).rev() {
        let _ = tokio::fs::rename(dir.join(format!("{name}.{i}")), dir.join(format!("{name}.{}", i + 1))).await;
    }
    let _ = tokio::fs::rename(path, dir.join(format!("{name}.1"))).await;
}

#[derive(Debug, Default, Deserialize)]
pub struct Query {
    pub limit: Option<usize>,
    pub user: Option<String>,
    /// Texte recherché dans le chemin, le détail ou le type.
    pub q: Option<String>,
    /// Uniquement les refus (401/403) et les échecs.
    #[serde(default)]
    pub only_denied: bool,
}

/// Entrées les plus récentes d'abord (jusqu'à 1000), fichier courant puis archives.
pub async fn read(data_dir: &Path, q: &Query) -> Vec<Entry> {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let base = log_path(data_dir);
    let name = base.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut files = vec![base.clone()];
    files.extend((1..=KEEP_FILES).map(|i| data_dir.join(format!("{name}.{i}"))));
    let needle = q.q.as_ref().map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for f in files {
        let Ok(text) = tokio::fs::read_to_string(&f).await else { continue };
        for line in text.lines().rev() {
            let Ok(e) = serde_json::from_str::<Entry>(line) else { continue };
            if q.user.as_deref().map(|u| !u.is_empty() && u != e.user).unwrap_or(false) {
                continue;
            }
            if q.only_denied && !matches!(e.status, Some(401) | Some(403)) && !e.detail.as_deref().map(|d| d.contains("refus") || d.contains("échec")).unwrap_or(false) {
                continue;
            }
            if let Some(n) = &needle {
                let hay = format!("{} {} {} {} {}", e.kind, e.path.as_deref().unwrap_or(""), e.detail.as_deref().unwrap_or(""), e.user, e.method.as_deref().unwrap_or("")).to_lowercase();
                if !hay.contains(n) {
                    continue;
                }
            }
            out.push(e);
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

// ───────────────────────── couche HTTP ─────────────────────────

#[derive(Clone)]
pub struct AuditCtx {
    pub data_dir: Arc<PathBuf>,
    pub trust_proxy: bool,
}

pub fn client_ip(headers: &axum::http::HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(first) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()) {
            let first = first.trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_default()
}

/// À poser *au-dessus* de la couche d'authentification : elle voit ainsi aussi les refus.
pub async fn layer(State(ctx): State<AuditCtx>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    // Les évènements de compte (/auth/*) sont inscrits par leurs propres gestionnaires.
    if !path.starts_with("/api/") {
        return next.run(req).await;
    }
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let ip = client_ip(req.headers(), peer, ctx.trust_proxy);
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    let mutating = !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
    let denied = matches!(status, 401 | 403);
    let exports = method == Method::GET && (path.contains("/export") || path.contains("/download") || path.ends_with("/restore"));
    if mutating || denied || exports {
        let user = resp.extensions().get::<Actor>().map(|a| a.0.clone()).unwrap_or_else(|| if status == 401 { "anonyme".to_string() } else { "local".to_string() });
        record(&ctx.data_dir, Entry::new("http", &user, &ip).http(method.as_str(), &path, Some(status))).await;
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcmanager-audit-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn paths_never_keep_query_strings_and_expose_server_id() {
        let id = Uuid::new_v4();
        let e = Entry::new("http", "alice", "1.2.3.4").http("POST", &format!("/api/servers/{id}/command?token=SECRET&x=1"), Some(200));
        assert_eq!(e.path.as_deref(), Some(format!("/api/servers/{id}/command").as_str()));
        assert_eq!(e.server.as_deref(), Some(id.to_string().as_str()));
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("SECRET") && !json.contains('?'));
        assert_eq!(server_in_path("/api/settings"), None);
        assert_eq!(server_in_path("/api/servers/pas-un-uuid/x"), None);
    }

    #[tokio::test]
    async fn append_read_filter_and_newest_first() {
        let dir = tmp();
        for i in 0..5 {
            record(&dir, Entry::new("http", if i % 2 == 0 { "alice" } else { "bob" }, "1.1.1.1").http("POST", &format!("/api/servers/{}/start", Uuid::new_v4()), Some(200)).detail(format!("n{i}"))).await;
        }
        record(&dir, Entry::new("auth", "mallory", "6.6.6.6").detail("connexion refusée")).await;
        record(&dir, Entry::new("http", "bob", "2.2.2.2").http("POST", "/api/settings", Some(403))).await;
        let all = read(&dir, &Query::default()).await;
        assert_eq!(all.len(), 7);
        assert_eq!(all[0].path.as_deref(), Some("/api/settings"), "le plus récent d'abord");
        assert_eq!(read(&dir, &Query { user: Some("alice".into()), ..Default::default() }).await.len(), 3);
        assert_eq!(read(&dir, &Query { limit: Some(2), ..Default::default() }).await.len(), 2);
        let denied = read(&dir, &Query { only_denied: true, ..Default::default() }).await;
        assert_eq!(denied.len(), 2, "403 + connexion refusée");
        assert_eq!(read(&dir, &Query { q: Some("START".into()), ..Default::default() }).await.len(), 5);
        // Lignes corrompues ignorées.
        tokio::fs::write(log_path(&dir), "pas du json\n{\"ts\":\"x\"}\n").await.unwrap();
        assert!(read(&dir, &Query::default()).await.is_empty());
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn log_rotates_and_old_files_stay_readable() {
        let dir = tmp();
        let path = log_path(&dir);
        record(&dir, Entry::new("auth", "alice", "").detail("ancienne entrée")).await;
        // Fichier courant « plein » : le prochain ajout fait tourner.
        let big = vec![b'x'; (MAX_BYTES as usize) + 1];
        let mut content = tokio::fs::read(&path).await.unwrap();
        content.extend(big);
        content.push(b'\n');
        tokio::fs::write(&path, content).await.unwrap();
        record(&dir, Entry::new("auth", "alice", "").detail("nouvelle entrée")).await;
        assert!(dir.join("audit.log.1").exists(), "ancien fichier archivé");
        let all = read(&dir, &Query::default()).await;
        let details: Vec<_> = all.iter().filter_map(|e| e.detail.as_deref()).collect();
        assert_eq!(details, ["nouvelle entrée", "ancienne entrée"], "nouvelles d'abord, puis l'archive");
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[test]
    fn client_ip_trusts_proxy_header_only_when_asked() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", "9.9.9.9, 10.0.0.1".parse().unwrap());
        let peer: SocketAddr = "192.168.1.5:5000".parse().unwrap();
        assert_eq!(client_ip(&h, Some(peer), false), "192.168.1.5", "en-tête ignoré sans confiance explicite (falsifiable sinon)");
        assert_eq!(client_ip(&h, Some(peer), true), "9.9.9.9");
        assert_eq!(client_ip(&axum::http::HeaderMap::new(), None, false), "");
    }
}
