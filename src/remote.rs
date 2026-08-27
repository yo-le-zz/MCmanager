//! RSA-secured remote control for `mcmanager-headless`: lets another
//! machine manage this instance's servers over the network (list, status,
//! start/stop/restart, send a console command, read recent logs) without a
//! full desktop web UI. Strictly opt-in and off by default - binding
//! `0.0.0.0` is real attack surface, so nothing here runs unless the user
//! explicitly enables it via the headless REPL (`remote enable`).
//!
//! ## Design
//!
//! Hybrid encryption, same shape as TLS/PGP/SSH: RSA is never used to
//! encrypt bulk data directly (it can't - a 2048-bit key can only wrap
//! ~190 bytes of plaintext), only to exchange a symmetric key.
//!
//! 1. **Identity**: on first use, the daemon generates its own RSA-2048
//!    keypair and keeps the private key on disk (owner-only permissions).
//! 2. **Pairing**: the daemon prints a random one-time pairing code (valid
//!    10 minutes, single use) to its console - so pairing requires
//!    whoever runs the daemon to actively read and hand out that code; a
//!    stranger on the network can't self-pair just by finding the port. A
//!    client wanting to pair sends its own RSA public key plus that code
//!    to `/remote/pair`. The daemon checks the code, stores the client's
//!    public key as trusted, and returns its own public key - both sides
//!    now know each other's identity.
//! 3. **Session**: the client generates a random AES-256 key locally,
//!    encrypts it with the daemon's RSA public key (RSA-OAEP/SHA-256), and
//!    sends it to `/remote/session`. The daemon decrypts it with its
//!    private key and keeps it as that client's session key. From then on,
//!    every request/response body for that client is encrypted with that
//!    AES-256-GCM key (reusing `crate::secrets`'s AEAD helpers).
//! 4. **Authentication**: every request to `/remote/api` is signed
//!    (RSASSA-PKCS1v15/SHA-256) by the client's private key over
//!    `{timestamp}.{encrypted_body}`, checked against the client's trusted
//!    public key, with a 60-second replay window. This is what actually
//!    authorizes an action - payload confidentiality (step 3) and sender
//!    authentication (step 4) are separate properties, both needed.
//!
//! Only previously-paired clients can do anything at all; there is no
//! "first request creates an account" style bootstrap.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use rsa::pkcs1v15::{SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::signature::{RandomizedSigner, SignatureEncoding, Verifier};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::secrets::EncryptedBlob;
use crate::state::AppState;

const KEY_BITS: usize = 2048;
const PAIRING_CODE_TTL_SECS: i64 = 600;
const REQUEST_MAX_SKEW_SECS: i64 = 60;

fn identity_path(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_identity.pem")
}
fn trusted_path(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_trusted_clients.json")
}

pub struct RemoteIdentity {
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
}

impl RemoteIdentity {
    pub fn public_pem(&self) -> String {
        self.public_key.to_public_key_pem(LineEnding::LF).expect("encodage cle publique")
    }

    /// SHA-256 of the DER-encoded public key, hex-grouped like an SSH host
    /// key fingerprint - what the user reads aloud/compares out of band to
    /// confirm they're pairing with the right machine.
    pub fn fingerprint(&self) -> String {
        use sha2::Digest;
        let der = self.public_key.to_public_key_der().expect("encodage DER");
        let hash = sha2::Sha256::digest(der.as_bytes());
        hash.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
    }
}

impl RemoteIdentity {
    /// Fallback used only if the persistent identity file couldn't be
    /// loaded/created (e.g. read-only data dir) - a fresh, never-persisted
    /// keypair so the rest of the app keeps working, at the cost of
    /// `remote` commands not being usable (or losing pairing on restart if
    /// this fallback is what's in use).
    pub fn ephemeral() -> Self {
        let private_key = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, KEY_BITS).expect("generation de secours de la cle RSA");
        let public_key = RsaPublicKey::from(&private_key);
        RemoteIdentity { private_key, public_key }
    }
}

pub async fn load_or_create_identity(data_dir: &Path) -> Result<RemoteIdentity> {
    let path = identity_path(data_dir);
    if let Ok(pem) = tokio::fs::read_to_string(&path).await {
        let private_key = RsaPrivateKey::from_pkcs8_pem(&pem).context("cle d'identite locale corrompue")?;
        let public_key = RsaPublicKey::from(&private_key);
        return Ok(RemoteIdentity { private_key, public_key });
    }
    let private_key = RsaPrivateKey::new(&mut OsRng, KEY_BITS).context("generation de la cle RSA")?;
    let public_key = RsaPublicKey::from(&private_key);
    let pem = private_key.to_pkcs8_pem(LineEnding::LF).context("encodage de la cle")?;
    tokio::fs::write(&path, pem.as_bytes()).await.context("ecriture de l'identite locale")?;
    crate::secrets::restrict_to_owner(&path).await;
    Ok(RemoteIdentity { private_key, public_key })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedClient {
    pub id: String,
    pub label: String,
    pub public_key_pem: String,
    pub paired_at: DateTime<Utc>,
}

pub async fn load_trusted(data_dir: &Path) -> Vec<TrustedClient> {
    match tokio::fs::read_to_string(trusted_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save_trusted(data_dir: &Path, list: &[TrustedClient]) -> Result<()> {
    let path = trusted_path(data_dir);
    tokio::fs::write(&path, serde_json::to_string_pretty(list)?).await?;
    crate::secrets::restrict_to_owner(&path).await;
    Ok(())
}

pub async fn revoke_client(data_dir: &Path, client_id: &str) -> Result<bool> {
    let mut list = load_trusted(data_dir).await;
    let before = list.len();
    list.retain(|c| c.id != client_id);
    let removed = list.len() != before;
    if removed {
        save_trusted(data_dir, &list).await?;
    }
    Ok(removed)
}

struct PairingState {
    code: String,
    expires_at: DateTime<Utc>,
}

/// Everything the remote API router needs, owned by whoever enables remote
/// control (`mcmanager-headless`) and handed to the router as its `State`.
#[derive(Clone)]
pub struct RemoteRuntime {
    pub app_state: AppState,
    pub data_dir: PathBuf,
    identity: Arc<RemoteIdentity>,
    pairing: Arc<tokio::sync::RwLock<Option<PairingState>>>,
    /// client_id -> AES-256 session key established via /remote/session.
    sessions: Arc<tokio::sync::RwLock<HashMap<String, [u8; 32]>>>,
}

impl RemoteRuntime {
    pub async fn new(app_state: AppState, data_dir: PathBuf) -> Result<Self> {
        let identity = load_or_create_identity(&data_dir).await?;
        Ok(Self {
            app_state,
            data_dir,
            identity: Arc::new(identity),
            pairing: Arc::new(tokio::sync::RwLock::new(None)),
            sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    pub fn fingerprint(&self) -> String {
        self.identity.fingerprint()
    }

    /// Generates a fresh one-time pairing code, replacing any still-active
    /// one (only one pairing can be in flight at a time - simpler to
    /// reason about, and pairing is a rare, deliberate, attended action).
    pub async fn generate_pairing_code(&self) -> String {
        let mut bytes = [0u8; 4];
        OsRng.fill_bytes(&mut bytes);
        let code = format!("{:08}", u32::from_be_bytes(bytes) % 100_000_000);
        *self.pairing.write().await = Some(PairingState {
            code: code.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(PAIRING_CODE_TTL_SECS),
        });
        code
    }
}

// ───────────────────────── crypto helpers ─────────────────────────

fn rsa_encrypt(public_key: &RsaPublicKey, plaintext: &[u8]) -> Result<Vec<u8>> {
    public_key.encrypt(&mut OsRng, Oaep::new::<Sha256>(), plaintext).map_err(|e| anyhow::anyhow!("chiffrement RSA echoue: {e}"))
}

fn rsa_decrypt(private_key: &RsaPrivateKey, ciphertext: &[u8]) -> Result<Vec<u8>> {
    private_key.decrypt(Oaep::new::<Sha256>(), ciphertext).map_err(|e| anyhow::anyhow!("dechiffrement RSA echoue: {e}"))
}

fn rsa_sign(private_key: &RsaPrivateKey, message: &[u8]) -> Result<Vec<u8>> {
    let signing_key = SigningKey::<Sha256>::new(private_key.clone());
    let signature = signing_key.sign_with_rng(&mut OsRng, message);
    Ok(signature.to_bytes().to_vec())
}

fn rsa_verify(public_key: &RsaPublicKey, message: &[u8], signature: &[u8]) -> Result<()> {
    let verifying_key = VerifyingKey::<Sha256>::new(public_key.clone());
    let sig = rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| anyhow::anyhow!("signature malformee"))?;
    verifying_key.verify(message, &sig).map_err(|_| anyhow::anyhow!("signature invalide"))
}

fn parse_public_key(pem: &str) -> Result<RsaPublicKey> {
    RsaPublicKey::from_public_key_pem(pem).context("cle publique invalide")
}

// ───────────────────────── protocole HTTP ─────────────────────────

#[derive(Deserialize)]
pub struct PairRequest {
    pub client_id: String,
    pub label: String,
    pub public_key_pem: String,
    pub code: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairResponse {
    pub server_public_key_pem: String,
}

pub async fn pair(rt: &RemoteRuntime, req: PairRequest) -> Result<PairResponse> {
    {
        let pairing = rt.pairing.read().await;
        match pairing.as_ref() {
            Some(p) if p.code == req.code && p.expires_at > Utc::now() => {}
            Some(_) => anyhow::bail!("code de jumelage incorrect ou expire"),
            None => anyhow::bail!("aucun jumelage en cours - lancez 'remote pairing-code' sur la machine hebergeant MCManager"),
        }
    }
    // Validate the submitted key actually parses as RSA before trusting it.
    parse_public_key(&req.public_key_pem)?;

    let mut list = load_trusted(&rt.data_dir).await;
    list.retain(|c| c.id != req.client_id);
    list.push(TrustedClient {
        id: req.client_id,
        label: req.label,
        public_key_pem: req.public_key_pem,
        paired_at: Utc::now(),
    });
    save_trusted(&rt.data_dir, &list).await?;
    *rt.pairing.write().await = None; // single-use

    Ok(PairResponse { server_public_key_pem: rt.identity.public_pem() })
}

#[derive(Deserialize)]
pub struct SessionRequest {
    pub client_id: String,
    /// Base64 RSA-OAEP(server_public_key, random 32-byte AES key).
    pub encrypted_key: String,
}

pub async fn start_session(rt: &RemoteRuntime, req: SessionRequest) -> Result<()> {
    let trusted = load_trusted(&rt.data_dir).await;
    if !trusted.iter().any(|c| c.id == req.client_id) {
        anyhow::bail!("client non jumele");
    }
    let wrapped = B64.decode(&req.encrypted_key).context("cle de session invalide (base64)")?;
    let key_bytes = rsa_decrypt(&rt.identity.private_key, &wrapped)?;
    if key_bytes.len() != 32 {
        anyhow::bail!("cle de session de taille inattendue");
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);
    rt.sessions.write().await.insert(req.client_id, key);
    Ok(())
}

#[derive(Deserialize)]
pub struct ApiRequest {
    pub client_id: String,
    pub timestamp: i64,
    /// Base64 RSA signature over `"{timestamp}.{encrypted_body}"`.
    pub signature: String,
    /// Base64-encoded JSON `EncryptedBlob` (AES-GCM under the session key).
    pub encrypted_body: String,
}

#[derive(Serialize, Deserialize)]
pub struct ApiResponse {
    pub encrypted_body: String,
}

/// Verifies signature + freshness + decrypts the body, returning the
/// plaintext JSON action payload. Deliberately a single choke point so
/// every remote action goes through the same checks - a handler can't
/// forget to call this.
async fn authenticate_and_decrypt(rt: &RemoteRuntime, req: &ApiRequest) -> Result<serde_json::Value> {
    let now = Utc::now().timestamp();
    if (now - req.timestamp).abs() > REQUEST_MAX_SKEW_SECS {
        anyhow::bail!("requete trop ancienne ou horloge desynchronisee (fenetre de {REQUEST_MAX_SKEW_SECS}s)");
    }

    let trusted = load_trusted(&rt.data_dir).await;
    let client = trusted.iter().find(|c| c.id == req.client_id).ok_or_else(|| anyhow::anyhow!("client non jumele"))?;
    let client_pubkey = parse_public_key(&client.public_key_pem)?;

    let signature = B64.decode(&req.signature).context("signature invalide (base64)")?;
    let signed_message = format!("{}.{}", req.timestamp, req.encrypted_body);
    rsa_verify(&client_pubkey, signed_message.as_bytes(), &signature)?;

    let sessions = rt.sessions.read().await;
    let aes_key = sessions.get(&req.client_id).ok_or_else(|| anyhow::anyhow!("aucune session active - appelez /remote/session d'abord"))?;

    let blob_json = B64.decode(&req.encrypted_body).context("corps chiffre invalide (base64)")?;
    let blob: EncryptedBlob = serde_json::from_slice(&blob_json).context("enveloppe chiffree invalide")?;
    let plaintext = crate::secrets::decrypt(aes_key, &blob)?;
    serde_json::from_str(&plaintext).context("action JSON invalide")
}

async fn encrypt_response(rt: &RemoteRuntime, client_id: &str, value: &serde_json::Value) -> Result<String> {
    let sessions = rt.sessions.read().await;
    let aes_key = sessions.get(client_id).ok_or_else(|| anyhow::anyhow!("session perdue"))?;
    let blob = crate::secrets::encrypt(aes_key, &value.to_string())?;
    Ok(B64.encode(serde_json::to_vec(&blob)?))
}

/// Dispatches one decrypted action to the actual MCManager operation and
/// returns its (still-plaintext, to be encrypted by the caller) JSON
/// result. Mirrors the headless REPL's command set (`list`, `status`,
/// `start`, `stop`, `restart`, `send`, `logs`) - same actions, remote
/// transport.
pub async fn dispatch_action(rt: &RemoteRuntime, action: serde_json::Value) -> serde_json::Value {
    let state = &rt.app_state;
    let name = action["action"].as_str().unwrap_or("");
    let server_id = action["server_id"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok());

    let result: Result<serde_json::Value> = async {
        match name {
            "list" => {
                let servers = state.servers.read().await;
                let runtime = state.runtime.read().await;
                let list: Vec<_> = servers.iter().map(|(id, s)| {
                    let running = runtime.get(id).map(|r| r.running).unwrap_or(false);
                    serde_json::json!({ "id": id, "name": s.name, "loader": s.loader.as_str(), "mc_version": s.mc_version, "running": running })
                }).collect();
                Ok(serde_json::json!({ "servers": list }))
            }
            "status" => {
                let id = server_id.context("server_id manquant")?;
                let entry = state.servers.read().await.get(&id).cloned().context("serveur introuvable")?;
                let running = state.runtime.read().await.get(&id).map(|r| r.running).unwrap_or(false);
                let players = if running { crate::stats::ping_server(entry.port).await.ok() } else { None };
                Ok(serde_json::json!({ "name": entry.name, "running": running, "players": players.map(|(o, m, _)| serde_json::json!({"online": o, "max": m})) }))
            }
            "start" => {
                let id = server_id.context("server_id manquant")?;
                crate::process::start_server(state, id).await?;
                Ok(serde_json::json!({ "ok": true }))
            }
            "stop" => {
                let id = server_id.context("server_id manquant")?;
                let force = action["force"].as_bool().unwrap_or(false);
                crate::process::stop_server(state, id, force).await?;
                Ok(serde_json::json!({ "ok": true }))
            }
            "restart" => {
                let id = server_id.context("server_id manquant")?;
                let _ = crate::process::stop_server(state, id, false).await;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                crate::process::start_server(state, id).await?;
                Ok(serde_json::json!({ "ok": true }))
            }
            "send" => {
                let id = server_id.context("server_id manquant")?;
                let cmd = action["command"].as_str().context("command manquant")?;
                crate::process::send_command(state, id, cmd).await?;
                Ok(serde_json::json!({ "ok": true }))
            }
            "logs" => {
                let id = server_id.context("server_id manquant")?;
                let n = action["n"].as_u64().unwrap_or(40) as usize;
                let runtime = state.runtime.read().await;
                let lines = runtime.get(&id).map(|rt| {
                    let start = rt.backlog.len().saturating_sub(n);
                    rt.backlog[start..].to_vec()
                }).unwrap_or_default();
                Ok(serde_json::json!({ "lines": lines }))
            }
            "import_server" => {
                // Receives a whole server (zipped, base64) from a remote
                // caller - what powers "send a local server to a remote
                // instance" from the desktop app's new Contrôle à distance
                // tab. Simple by design (one request, one blob): fine for
                // typical setups, but means very large worlds will be slow
                // and memory-heavy to transfer this way - a deliberate
                // trade-off to avoid a chunked-upload protocol.
                let name = action["name"].as_str().context("name manquant")?.to_string();
                let loader: crate::models::Loader = serde_json::from_value(action["loader"].clone()).context("loader invalide")?;
                let mc_version = action["mc_version"].as_str().context("mc_version manquant")?.to_string();
                let port = action["port"].as_u64().unwrap_or(25565) as u16;
                let zip_b64 = action["zip_base64"].as_str().context("zip_base64 manquant")?;
                let zip_bytes = B64.decode(zip_b64).context("archive invalide (base64)")?;

                let id = uuid::Uuid::new_v4();
                let folder = state.data_dir.join("servers").join(id.to_string());
                tokio::fs::create_dir_all(&folder).await?;
                let folder2 = folder.clone();
                tokio::task::spawn_blocking(move || crate::files::import_zip(&folder2, "", &zip_bytes)).await
                    .map_err(|e| anyhow::anyhow!("erreur interne: {e}"))??;

                let java_path = state.config.read().await.java_path.clone();
                let entry = crate::models::ServerEntry {
                    id, name, loader, mc_version,
                    loader_version: None,
                    folder: folder.to_string_lossy().to_string(),
                    jar_name: String::new(),
                    java_path,
                    xms_mb: 1024, xmx_mb: 2048, port,
                    extra_args: vec![],
                    eula_accepted: true,
                    auto_backup_minutes: None,
                    backup_retention: None,
                    auto_restart: false,
                    auto_restart_delay_secs: 5,
                    scheduled_restart_minutes: None,
                    stop_when_empty_minutes: None,
                    dynamic_server: false,
                    aikar_flags: false,
                    managed_addons: vec![],
                    created_at: chrono::Utc::now(),
                };
                state.servers.write().await.insert(id, entry);
                crate::state::save_servers(state).await?;
                Ok(serde_json::json!({ "ok": true, "server_id": id }))
            }
            other => anyhow::bail!("action inconnue: {other}"),
        }
    }.await;

    match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

pub async fn handle_api_request(rt: &RemoteRuntime, req: ApiRequest) -> Result<ApiResponse> {
    let client_id = req.client_id.clone();
    let action = authenticate_and_decrypt(rt, &req).await?;
    let result = dispatch_action(rt, action).await;
    let encrypted_body = encrypt_response(rt, &client_id, &result).await?;
    Ok(ApiResponse { encrypted_body })
}

// ───────────────────────── routeur HTTP (cote expose) ─────────────────────────

#[derive(Serialize, Deserialize)]
struct InfoResponse {
    fingerprint: String,
    public_key_pem: String,
}

async fn http_info(axum::extract::State(rt): axum::extract::State<RemoteRuntime>) -> axum::Json<InfoResponse> {
    axum::Json(InfoResponse { fingerprint: rt.fingerprint(), public_key_pem: rt.identity.public_pem() })
}

async fn http_pair(axum::extract::State(rt): axum::extract::State<RemoteRuntime>, axum::Json(req): axum::Json<PairRequest>) -> Result<axum::Json<PairResponse>, axum::response::Response> {
    pair(&rt, req).await.map(axum::Json).map_err(|e| bad_request(e))
}

async fn http_session(axum::extract::State(rt): axum::extract::State<RemoteRuntime>, axum::Json(req): axum::Json<SessionRequest>) -> Result<axum::Json<serde_json::Value>, axum::response::Response> {
    start_session(&rt, req).await.map(|_| axum::Json(serde_json::json!({ "ok": true }))).map_err(|e| bad_request(e))
}

async fn http_api(axum::extract::State(rt): axum::extract::State<RemoteRuntime>, axum::Json(req): axum::Json<ApiRequest>) -> Result<axum::Json<ApiResponse>, axum::response::Response> {
    handle_api_request(&rt, req).await.map(axum::Json).map_err(|e| bad_request(e))
}

fn bad_request(e: anyhow::Error) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}

pub fn router(rt: RemoteRuntime) -> axum::Router {
    axum::Router::new()
        .route("/remote/info", axum::routing::get(http_info))
        .route("/remote/pair", axum::routing::post(http_pair))
        .route("/remote/session", axum::routing::post(http_session))
        .route("/remote/api", axum::routing::post(http_api))
        .with_state(rt)
}

/// Binds `0.0.0.0:port` and serves the remote-control API until the
/// returned future is dropped/aborted. The caller (the headless REPL)
/// keeps the `JoinHandle` around so `remote disable` can cancel it.
pub async fn serve(rt: RemoteRuntime, port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await
        .with_context(|| format!("impossible d'ecouter sur le port {port}"))?;
    tracing::info!("[remote] API de controle a distance active sur le port {port} (toutes interfaces)");
    axum::serve(listener, router(rt)).await?;
    Ok(())
}

// ───────────────────────── cote client (piloter une autre instance) ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteTarget {
    pub label: String,
    pub host: String, // "host:port"
    pub server_public_key_pem: String,
}

fn targets_path(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_targets.json")
}

pub async fn load_targets(data_dir: &Path) -> Vec<RemoteTarget> {
    match tokio::fs::read_to_string(targets_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save_targets(data_dir: &Path, list: &[RemoteTarget]) -> Result<()> {
    tokio::fs::write(targets_path(data_dir), serde_json::to_string_pretty(list)?).await?;
    Ok(())
}

pub async fn forget_target(data_dir: &Path, label: &str) -> Result<bool> {
    let mut list = load_targets(data_dir).await;
    let before = list.len();
    list.retain(|t| t.label != label);
    let removed = list.len() != before;
    if removed {
        save_targets(data_dir, &list).await?;
    }
    Ok(removed)
}

/// Fetches a not-yet-trusted instance's identity (fingerprint + public
/// key) - the fingerprint is what the user should read back to whoever is
/// physically at that machine to confirm before pairing, the same way one
/// checks an SSH host key fingerprint on first connect.
pub async fn client_fetch_info(http: &reqwest::Client, host: &str) -> Result<(String, String)> {
    let resp = http.get(format!("http://{host}/remote/info")).send().await.context("instance distante injoignable")?;
    let info: InfoResponse = resp.json().await.context("reponse /remote/info invalide")?;
    Ok((info.fingerprint, info.public_key_pem))
}

/// Completes pairing with a remote instance using a one-time code obtained
/// out of band (read from that instance's own console/`remote
/// pairing-code`), and remembers it locally under `label` for future
/// `client_call`s.
pub async fn client_pair(http: &reqwest::Client, data_dir: &Path, identity: &RemoteIdentity, host: &str, label: &str, code: &str) -> Result<()> {
    let client_id = identity.fingerprint();
    let body = serde_json::json!({
        "client_id": client_id,
        "label": label,
        "public_key_pem": identity.public_pem(),
        "code": code,
    });
    let resp = http.post(format!("http://{host}/remote/pair")).json(&body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("echec du jumelage : {}", resp.text().await.unwrap_or_default());
    }
    let pair_resp: PairResponse = resp.json().await?;

    let mut targets = load_targets(data_dir).await;
    targets.retain(|t| t.label != label);
    targets.push(RemoteTarget { label: label.to_string(), host: host.to_string(), server_public_key_pem: pair_resp.server_public_key_pem });
    save_targets(data_dir, &targets).await?;
    Ok(())
}

/// One remote action end to end: fresh AES session, signed+encrypted
/// request, decrypted response. A new session is established for every
/// call rather than cached - one extra RSA-OAEP wrap is cheap, and it
/// avoids having to reason about session expiry/invalidation across
/// restarts of either side.
pub async fn client_call(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, action: serde_json::Value) -> Result<serde_json::Value> {
    let mut aes_key = [0u8; 32];
    OsRng.fill_bytes(&mut aes_key);
    let server_pubkey = parse_public_key(&target.server_public_key_pem)?;
    let wrapped = rsa_encrypt(&server_pubkey, &aes_key)?;
    let client_id = identity.fingerprint();

    let session_body = serde_json::json!({ "client_id": client_id, "encrypted_key": B64.encode(wrapped) });
    let resp = http.post(format!("http://{}/remote/session", target.host)).json(&session_body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("echec d'etablissement de session : {}", resp.text().await.unwrap_or_default());
    }

    let blob = crate::secrets::encrypt(&aes_key, &action.to_string())?;
    let encrypted_body = B64.encode(serde_json::to_vec(&blob)?);
    let timestamp = Utc::now().timestamp();
    let signature = rsa_sign(&identity.private_key, format!("{timestamp}.{encrypted_body}").as_bytes())?;

    let req_body = serde_json::json!({
        "client_id": client_id, "timestamp": timestamp,
        "signature": B64.encode(signature), "encrypted_body": encrypted_body,
    });
    let resp = http.post(format!("http://{}/remote/api", target.host)).json(&req_body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("erreur distante : {}", resp.text().await.unwrap_or_default());
    }
    let api_resp: ApiResponse = resp.json().await?;

    let resp_blob_json = B64.decode(&api_resp.encrypted_body)?;
    let resp_blob: EncryptedBlob = serde_json::from_slice(&resp_blob_json)?;
    let plain = crate::secrets::decrypt(&aes_key, &resp_blob)?;
    Ok(serde_json::from_str(&plain)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_keypair() -> (RsaPrivateKey, RsaPublicKey) {
        let sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap(); // small key: fast tests, crypto correctness doesn't depend on key size
        let pk = RsaPublicKey::from(&sk);
        (sk, pk)
    }

    #[test]
    fn oaep_roundtrip_wraps_an_aes_key() {
        let (sk, pk) = gen_keypair();
        let mut aes_key = [0u8; 32];
        OsRng.fill_bytes(&mut aes_key);
        let wrapped = rsa_encrypt(&pk, &aes_key).unwrap();
        assert_ne!(wrapped, aes_key.to_vec());
        let unwrapped = rsa_decrypt(&sk, &wrapped).unwrap();
        assert_eq!(unwrapped, aes_key.to_vec());
    }

    #[test]
    fn signature_roundtrip() {
        let (sk, pk) = gen_keypair();
        let msg = b"1234567890.some-encrypted-body-base64";
        let sig = rsa_sign(&sk, msg).unwrap();
        assert!(rsa_verify(&pk, msg, &sig).is_ok());
    }

    #[test]
    fn signature_rejects_tampered_message() {
        let (sk, pk) = gen_keypair();
        let sig = rsa_sign(&sk, b"original message").unwrap();
        assert!(rsa_verify(&pk, b"tampered message", &sig).is_err());
    }

    #[test]
    fn signature_rejects_wrong_key() {
        let (sk, _pk) = gen_keypair();
        let (_sk2, pk2) = gen_keypair();
        let sig = rsa_sign(&sk, b"message").unwrap();
        assert!(rsa_verify(&pk2, b"message", &sig).is_err());
    }

    #[test]
    fn fingerprint_is_stable_for_same_key() {
        let sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let pk = RsaPublicKey::from(&sk);
        let identity = RemoteIdentity { private_key: sk, public_key: pk.clone() };
        let fp1 = identity.fingerprint();
        let identity2 = RemoteIdentity { private_key: RsaPrivateKey::new(&mut OsRng, 1024).unwrap(), public_key: pk };
        let fp2 = identity2.fingerprint();
        assert_eq!(fp1, fp2, "fingerprint depends only on the public key, not the (regenerated) private key struct");
    }

    /// Full round-trip of the actual wire protocol: pairing -> session
    /// establishment -> a signed+encrypted request -> verified decrypt.
    /// This is the part that matters most to get right, so it's tested
    /// end-to-end rather than just its pieces in isolation.
    #[tokio::test]
    async fn full_pairing_and_request_flow() {
        let tmp = std::env::temp_dir().join(format!("mcmanager-remote-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&tmp).await.unwrap();

        let app_state = crate::state::build_state_for_test(&tmp).await;
        let rt = RemoteRuntime::new(app_state, tmp.clone()).await.unwrap();

        // Client generates its own identity.
        let client_sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let client_pk = RsaPublicKey::from(&client_sk);
        let client_pk_pem = client_pk.to_public_key_pem(LineEnding::LF).unwrap();

        let code = rt.generate_pairing_code().await;
        let pair_resp = pair(&rt, PairRequest {
            client_id: "test-client".into(),
            label: "Test PC".into(),
            public_key_pem: client_pk_pem,
            code,
        }).await.unwrap();
        assert_eq!(pair_resp.server_public_key_pem, rt.identity.public_pem());

        // Client establishes a session: generates an AES key, wraps it with the server's public key.
        let mut aes_key = [0u8; 32];
        OsRng.fill_bytes(&mut aes_key);
        let server_pubkey = parse_public_key(&pair_resp.server_public_key_pem).unwrap();
        let wrapped = rsa_encrypt(&server_pubkey, &aes_key).unwrap();
        start_session(&rt, SessionRequest { client_id: "test-client".into(), encrypted_key: B64.encode(wrapped) }).await.unwrap();

        // Client sends a signed, encrypted "list" action.
        let action = serde_json::json!({ "action": "list" });
        let blob = crate::secrets::encrypt(&aes_key, &action.to_string()).unwrap();
        let encrypted_body = B64.encode(serde_json::to_vec(&blob).unwrap());
        let timestamp = Utc::now().timestamp();
        let signed_message = format!("{timestamp}.{encrypted_body}");
        let signature = rsa_sign(&client_sk, signed_message.as_bytes()).unwrap();

        let resp = handle_api_request(&rt, ApiRequest {
            client_id: "test-client".into(),
            timestamp,
            signature: B64.encode(signature),
            encrypted_body,
        }).await.unwrap();

        // Decrypt the response the way a real client would.
        let resp_blob_json = B64.decode(&resp.encrypted_body).unwrap();
        let resp_blob: EncryptedBlob = serde_json::from_slice(&resp_blob_json).unwrap();
        let resp_plain = crate::secrets::decrypt(&aes_key, &resp_blob).unwrap();
        let resp_value: serde_json::Value = serde_json::from_str(&resp_plain).unwrap();
        assert!(resp_value["servers"].is_array());

        tokio::fs::remove_dir_all(&tmp).await.ok();
    }

    #[tokio::test]
    async fn unpaired_client_is_rejected() {
        let tmp = std::env::temp_dir().join(format!("mcmanager-remote-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        let app_state = crate::state::build_state_for_test(&tmp).await;
        let rt = RemoteRuntime::new(app_state, tmp.clone()).await.unwrap();

        let client_sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let action = serde_json::json!({ "action": "list" });
        let fake_key = [0u8; 32];
        let blob = crate::secrets::encrypt(&fake_key, &action.to_string()).unwrap();
        let encrypted_body = B64.encode(serde_json::to_vec(&blob).unwrap());
        let timestamp = Utc::now().timestamp();
        let signature = rsa_sign(&client_sk, format!("{timestamp}.{encrypted_body}").as_bytes()).unwrap();

        let result = handle_api_request(&rt, ApiRequest {
            client_id: "never-paired".into(),
            timestamp,
            signature: B64.encode(signature),
            encrypted_body,
        }).await;
        assert!(result.is_err(), "a client that never paired must be rejected");

        tokio::fs::remove_dir_all(&tmp).await.ok();
    }

    /// Exercises the *actual* network path end to end: binds a real TCP
    /// listener via `serve()`, runs the full axum router, and drives it
    /// entirely through `client_fetch_info`/`client_pair`/`client_call`
    /// over real HTTP - not by calling the handler functions directly like
    /// the tests above. This is what would catch a routing mistake, a
    /// serialization mismatch between client and server structs, or the
    /// listener failing to bind, none of which the in-process tests can see.
    #[tokio::test]
    async fn full_round_trip_over_real_http() {
        let host_dir = std::env::temp_dir().join(format!("mcmanager-remote-host-{}", uuid::Uuid::new_v4()));
        let client_dir = std::env::temp_dir().join(format!("mcmanager-remote-client-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&host_dir).await.unwrap();
        tokio::fs::create_dir_all(&client_dir).await.unwrap();

        // Bind on an OS-assigned free port for the test, then hand that
        // same port to `serve()`.
        let temp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = temp_listener.local_addr().unwrap().port();
        drop(temp_listener);

        let host_app_state = crate::state::build_state_for_test(&host_dir).await;
        let host_rt = RemoteRuntime::new(host_app_state, host_dir.clone()).await.unwrap();
        let host_rt_for_task = host_rt.clone();
        let server_task = tokio::spawn(async move {
            let _ = serve(host_rt_for_task, port).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let http = reqwest::Client::new();
        let host_addr = format!("127.0.0.1:{port}");

        // Client fetches the host's identity over real HTTP.
        let (fingerprint, _pubkey) = client_fetch_info(&http, &host_addr).await.unwrap();
        assert_eq!(fingerprint, host_rt.fingerprint());

        // Pair using a real one-time code generated on the "host" side.
        let code = host_rt.generate_pairing_code().await;
        let client_identity = load_or_create_identity(&client_dir).await.unwrap();
        client_pair(&http, &client_dir, &client_identity, &host_addr, "test-host", &code).await.unwrap();

        let targets = load_targets(&client_dir).await;
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].label, "test-host");

        // A real signed+encrypted call over the wire.
        let result = client_call(&http, &client_identity, &targets[0], serde_json::json!({ "action": "list" })).await.unwrap();
        assert!(result["servers"].is_array());

        // A second, unpaired "attacker" identity must be rejected even
        // with a syntactically valid request shape.
        let attacker_dir = std::env::temp_dir().join(format!("mcmanager-remote-attacker-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&attacker_dir).await.unwrap();
        let attacker_identity = load_or_create_identity(&attacker_dir).await.unwrap();
        let fake_target = RemoteTarget { label: "x".into(), host: host_addr.clone(), server_public_key_pem: host_rt.identity.public_pem() };
        let attacker_result = client_call(&http, &attacker_identity, &fake_target, serde_json::json!({ "action": "list" })).await;
        assert!(attacker_result.is_err(), "an unpaired identity must be rejected even over the real HTTP path");

        server_task.abort();
        tokio::fs::remove_dir_all(&host_dir).await.ok();
        tokio::fs::remove_dir_all(&client_dir).await.ok();
        tokio::fs::remove_dir_all(&attacker_dir).await.ok();
    }
}
