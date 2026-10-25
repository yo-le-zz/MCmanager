//! RSA-secured remote control for `mcmanager-server` (and, through the
//! tunnel below, for the whole web API): lets another machine manage this
//! instance's servers over the network. Strictly opt-in and off by default -
//! binding `0.0.0.0` is real attack surface, so nothing here runs unless the
//! user explicitly enables it (`remote enable`, `remote install-permanent`,
//! or the `mcmanager-server --daemon` service).
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
//!    10 minutes by default, single use) to its console - so pairing
//!    requires whoever runs the daemon to actively read and hand out that
//!    code; a stranger on the network can't self-pair just by finding the
//!    port. A client wanting to pair sends its own RSA public key plus that
//!    code to `/remote/pair`. The daemon checks the code, stores the
//!    client's public key as trusted, and returns its own public key - both
//!    sides now know each other's identity.
//! 3. **Session** (v2): the client generates a random AES-256 key locally,
//!    encrypts it with the daemon's RSA public key (RSA-OAEP/SHA-256),
//!    **signs the request with its own key**, and sends it to
//!    `/remote/session`. The daemon answers with a fresh random
//!    `session_id`. Sessions are keyed by that id - *not* by client id - so
//!    several concurrent sessions of the same client (two browser tabs, a
//!    background refresher and a console stream...) never overwrite each
//!    other. This was the root of a real bug in v1: every call replaced the
//!    client's single session key, so two simultaneous calls broke each other.
//! 4. **Authentication**: every request to `/remote/api` is signed
//!    (RSASSA-PKCS1v15/SHA-256) by the client's private key over
//!    `{timestamp}.{session_id}.{nonce}.{encrypted_body}`, checked against
//!    the client's trusted public key, with a 60-second freshness window
//!    **and a single-use nonce** (a captured request can no longer be
//!    replayed inside that window). This is what actually authorizes an
//!    action - payload confidentiality (step 3) and sender authentication
//!    (step 4) are separate properties, both needed.
//!
//! Only previously-paired clients can do anything at all; there is no
//! "first request creates an account" style bootstrap.
//!
//! ## The tunnel
//!
//! On top of the fixed action set (`list`, `start`, `stop`, ...) there is a
//! generic `http` action: the paired client sends `{method, path, body}` and
//! this instance executes it *in-process* against its own REST API router
//! (restricted to `/api/servers`, `/api/loaders`, `/api/marketplace`,
//! `/api/presets`, `/api/version`). That is what lets a remote machine's
//! servers show up in the local dashboard with console, files, mods/plugins,
//! schematics, backups... all working exactly like local servers, without
//! re-implementing each feature as a dedicated remote action. Bodies larger
//! than a few hundred KB go through a chunked blob store (`blob_put` /
//! `blob_get`), in both directions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

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
use tower::ServiceExt;

use crate::secrets::EncryptedBlob;
use crate::state::AppState;

const KEY_BITS: usize = 2048;
pub const PAIRING_CODE_DEFAULT_TTL_MIN: i64 = 10;
/// Hard ceiling regardless of what is requested - a pairing code is a bearer
/// secret usable by anyone who has it during its lifetime, with no other
/// check. Configurability must never turn it into a long-lived shared password.
const PAIRING_CODE_MAX_TTL_MIN: i64 = 60;
const REQUEST_MAX_SKEW_SECS: i64 = 60;
/// A session unused for this long is forgotten (the client transparently
/// opens a new one - see `ensure_session`).
const SESSION_IDLE_TTL_SECS: i64 = 15 * 60;
/// Per paired client. Generous (a dashboard with several tabs + background
/// pollers uses a handful) but bounded, so a client can't grow server memory
/// without limit.
const MAX_SESSIONS_PER_CLIENT: usize = 64;
/// How long a used nonce is remembered; must exceed the freshness window.
const NONCE_TTL_SECS: i64 = REQUEST_MAX_SKEW_SECS * 2 + 5;
/// Bodies up to this many raw bytes travel inline in the encrypted JSON;
/// bigger ones go through the chunked blob store.
const INLINE_BODY_MAX: usize = 512 * 1024;
const BLOB_CHUNK: usize = 2 * 1024 * 1024;
/// Largest single tunnelled request/response body. Everything is buffered in
/// memory on both ends, so this is a deliberate safety valve, not a goal.
pub const TUNNEL_BODY_MAX: usize = 512 * 1024 * 1024;
const BLOB_MAX_AGE_SECS: u64 = 60 * 60;
/// What a paired client may reach through the `http` tunnel action. Anything
/// else (settings, AI keys, ntfy token, remote targets, self-update...) stays
/// strictly local to the machine it belongs to.
const TUNNEL_ALLOWED_PREFIXES: [&str; 5] = ["/api/servers", "/api/loaders", "/api/marketplace", "/api/presets", "/api/version"];
/// Marks a request as having arrived through the tunnel, so the receiving
/// instance never forwards it a second time (no A -> B -> A loops, no chains).
pub const TUNNEL_HEADER: &str = "x-mcm-tunnel";

fn identity_path(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_identity.pem")
}
fn trusted_path(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_trusted_clients.json")
}
fn blob_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("remote_tmp")
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
    /// Updated on every successful authenticated request from this client, so
    /// `remote clients` can show who is actually active when several are paired.
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
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

struct Session {
    client_id: String,
    key: [u8; 32],
    created_at: DateTime<Utc>,
    last_used: DateTime<Utc>,
}

#[derive(Default)]
struct TrustedCache {
    loaded: bool,
    mtime: Option<SystemTime>,
    list: Vec<TrustedClient>,
    last_touch: HashMap<String, Instant>,
}

/// Everything the remote API router needs, owned by whoever enables remote
/// control (`mcmanager-server`) and handed to the router as its `State`.
#[derive(Clone)]
pub struct RemoteRuntime {
    pub app_state: AppState,
    pub data_dir: PathBuf,
    identity: Arc<RemoteIdentity>,
    pairing: Arc<tokio::sync::RwLock<Option<PairingState>>>,
    /// session_id -> session. Keyed by a random id, *not* by client id: any
    /// number of simultaneous sessions per paired client (see module docs).
    sessions: Arc<tokio::sync::RwLock<HashMap<String, Session>>>,
    /// Single-use nonces seen recently -> unix expiry. Replay protection.
    nonces: Arc<tokio::sync::Mutex<HashMap<String, i64>>>,
    trusted: Arc<tokio::sync::Mutex<TrustedCache>>,
    /// The REST API this instance serves locally, driven in-process by the
    /// `http` tunnel action.
    api_router: axum::Router,
}

impl RemoteRuntime {
    pub async fn new(app_state: AppState, data_dir: PathBuf) -> Result<Self> {
        let identity = load_or_create_identity(&data_dir).await?;
        let api_router = crate::api::router(app_state.clone());
        Ok(Self {
            app_state,
            data_dir,
            identity: Arc::new(identity),
            pairing: Arc::new(tokio::sync::RwLock::new(None)),
            sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            nonces: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            trusted: Arc::new(tokio::sync::Mutex::new(TrustedCache::default())),
            api_router,
        })
    }

    pub fn fingerprint(&self) -> String {
        self.identity.fingerprint()
    }

    /// Generates a fresh one-time pairing code, replacing any still-active
    /// one (only one pairing can be in flight at a time - simpler to
    /// reason about, and pairing is a rare, deliberate, attended action).
    pub async fn generate_pairing_code(&self, minutes: i64) -> (String, i64) {
        let minutes = minutes.clamp(1, PAIRING_CODE_MAX_TTL_MIN);
        let mut bytes = [0u8; 4];
        OsRng.fill_bytes(&mut bytes);
        let code = format!("{:08}", u32::from_be_bytes(bytes) % 100_000_000);
        *self.pairing.write().await = Some(PairingState {
            code: code.clone(),
            expires_at: Utc::now() + chrono::Duration::minutes(minutes),
        });
        (code, minutes)
    }

    /// Paired clients with the number of live sessions each currently has -
    /// what `remote clients` shows so an admin sees who is connected *now*.
    pub async fn clients_with_sessions(&self) -> Vec<(TrustedClient, usize)> {
        let trusted = self.trusted_snapshot().await;
        let sessions = self.sessions.read().await;
        let now = Utc::now();
        trusted
            .into_iter()
            .map(|c| {
                let n = sessions
                    .values()
                    .filter(|s| s.client_id == c.id && (now - s.last_used).num_seconds() < SESSION_IDLE_TTL_SECS)
                    .count();
                (c, n)
            })
            .collect()
    }

    /// Revokes a client *and* drops its live sessions at once.
    pub async fn revoke(&self, client_id: &str) -> Result<bool> {
        let removed = revoke_client(&self.data_dir, client_id).await?;
        self.sessions.write().await.retain(|_, s| s.client_id != client_id);
        self.trusted.lock().await.loaded = false;
        Ok(removed)
    }

    /// The trusted-clients list, re-read from disk only when the file
    /// actually changed (a revoke from another process still takes effect on
    /// the very next request) instead of on every single request.
    async fn trusted_snapshot(&self) -> Vec<TrustedClient> {
        let mtime = tokio::fs::metadata(trusted_path(&self.data_dir)).await.ok().and_then(|m| m.modified().ok());
        let mut cache = self.trusted.lock().await;
        if !cache.loaded || cache.mtime != mtime {
            cache.list = load_trusted(&self.data_dir).await;
            cache.mtime = mtime;
            cache.loaded = true;
        }
        cache.list.clone()
    }

    /// Records activity for `remote clients`, but writes to disk at most
    /// every 30 s per client - with a busy dashboard this runs on every
    /// request, and rewriting the file each time would be wasteful and racy.
    async fn touch_last_used(&self, client_id: &str) {
        let mut cache = self.trusted.lock().await;
        let now = Instant::now();
        if cache.last_touch.get(client_id).map(|t| now.duration_since(*t) < Duration::from_secs(30)).unwrap_or(false) {
            return;
        }
        cache.last_touch.insert(client_id.to_string(), now);
        let mut list = load_trusted(&self.data_dir).await;
        if let Some(c) = list.iter_mut().find(|c| c.id == client_id) {
            c.last_used_at = Some(Utc::now());
            if save_trusted(&self.data_dir, &list).await.is_ok() {
                cache.mtime = tokio::fs::metadata(trusted_path(&self.data_dir)).await.ok().and_then(|m| m.modified().ok());
                cache.list = list;
                cache.loaded = true;
            }
        }
    }

    /// Rejects a nonce that was already used inside the validity window.
    async fn consume_nonce(&self, nonce: &str) -> Result<()> {
        if nonce.len() < 8 || nonce.len() > 256 {
            anyhow::bail!("nonce invalide");
        }
        let now = Utc::now().timestamp();
        let mut nonces = self.nonces.lock().await;
        nonces.retain(|_, exp| *exp > now);
        if nonces.contains_key(nonce) {
            anyhow::bail!("requete deja recue (rejeu refuse)");
        }
        nonces.insert(nonce.to_string(), now + NONCE_TTL_SECS);
        Ok(())
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
        last_used_at: None,
    });
    save_trusted(&rt.data_dir, &list).await?;
    rt.trusted.lock().await.loaded = false;
    *rt.pairing.write().await = None; // single-use

    Ok(PairResponse { server_public_key_pem: rt.identity.public_pem() })
}

fn check_skew(timestamp: i64) -> Result<()> {
    let now = Utc::now().timestamp();
    if (now - timestamp).abs() > REQUEST_MAX_SKEW_SECS {
        anyhow::bail!("requete trop ancienne ou horloge desynchronisee (fenetre de {REQUEST_MAX_SKEW_SECS}s)");
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct SessionRequest {
    pub client_id: String,
    /// Base64 RSA-OAEP(server_public_key, random 32-byte AES key).
    pub encrypted_key: String,
    pub timestamp: i64,
    /// Base64 RSA signature over `session.{timestamp}.{client_id}.{encrypted_key}` -
    /// without it anybody who merely knows a paired client's (non-secret)
    /// fingerprint could open sessions in its name and crowd out the real ones.
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
pub struct SessionResponse {
    pub ok: bool,
    pub session_id: String,
}

fn session_signed_message(client_id: &str, timestamp: i64, encrypted_key: &str) -> String {
    format!("session.{timestamp}.{client_id}.{encrypted_key}")
}

pub async fn start_session(rt: &RemoteRuntime, req: SessionRequest) -> Result<SessionResponse> {
    check_skew(req.timestamp)?;
    let trusted = rt.trusted_snapshot().await;
    let client = trusted.iter().find(|c| c.id == req.client_id).ok_or_else(|| anyhow::anyhow!("client non jumele"))?;
    let client_pubkey = parse_public_key(&client.public_key_pem)?;
    let signature = B64.decode(&req.signature).context("signature invalide (base64)")?;
    rsa_verify(&client_pubkey, session_signed_message(&req.client_id, req.timestamp, &req.encrypted_key).as_bytes(), &signature)?;
    rt.consume_nonce(&req.signature[..req.signature.len().min(64)]).await?;

    let wrapped = B64.decode(&req.encrypted_key).context("cle de session invalide (base64)")?;
    let key_bytes = rsa_decrypt(&rt.identity.private_key, &wrapped)?;
    if key_bytes.len() != 32 {
        anyhow::bail!("cle de session de taille inattendue");
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);

    let mut id_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut id_bytes);
    let session_id: String = id_bytes.iter().map(|b| format!("{b:02x}")).collect();

    let now = Utc::now();
    let mut sessions = rt.sessions.write().await;
    sessions.retain(|_, s| (now - s.last_used).num_seconds() < SESSION_IDLE_TTL_SECS);
    // Bounded per client: evict that client's *oldest* sessions first.
    let mut mine: Vec<(String, DateTime<Utc>)> = sessions.iter().filter(|(_, s)| s.client_id == req.client_id).map(|(k, s)| (k.clone(), s.created_at)).collect();
    if mine.len() >= MAX_SESSIONS_PER_CLIENT {
        mine.sort_by_key(|(_, created)| *created);
        for (k, _) in mine.iter().take(mine.len() + 1 - MAX_SESSIONS_PER_CLIENT) {
            sessions.remove(k);
        }
    }
    sessions.insert(session_id.clone(), Session { client_id: req.client_id, key, created_at: now, last_used: now });
    Ok(SessionResponse { ok: true, session_id })
}

#[derive(Deserialize)]
pub struct ApiRequest {
    pub client_id: String,
    pub session_id: String,
    pub timestamp: i64,
    /// Random single-use value; the server refuses to see the same one twice.
    pub nonce: String,
    /// Base64 RSA signature over `{timestamp}.{session_id}.{nonce}.{encrypted_body}`.
    pub signature: String,
    /// Base64-encoded JSON `EncryptedBlob` (AES-GCM under the session key).
    pub encrypted_body: String,
}

#[derive(Serialize, Deserialize)]
pub struct ApiResponse {
    pub encrypted_body: String,
}

fn api_signed_message(timestamp: i64, session_id: &str, nonce: &str, encrypted_body: &str) -> String {
    format!("{timestamp}.{session_id}.{nonce}.{encrypted_body}")
}

/// Verifies signature + freshness + replay + decrypts the body, returning the
/// plaintext JSON action payload and the session key (to encrypt the
/// response with). Deliberately a single choke point so every remote action
/// goes through the same checks - a handler can't forget to call this.
async fn authenticate_and_decrypt(rt: &RemoteRuntime, req: &ApiRequest) -> Result<(serde_json::Value, [u8; 32])> {
    check_skew(req.timestamp)?;

    let trusted = rt.trusted_snapshot().await;
    let client = trusted.iter().find(|c| c.id == req.client_id).ok_or_else(|| anyhow::anyhow!("client non jumele"))?;
    let client_pubkey = parse_public_key(&client.public_key_pem)?;

    let signature = B64.decode(&req.signature).context("signature invalide (base64)")?;
    let signed_message = api_signed_message(req.timestamp, &req.session_id, &req.nonce, &req.encrypted_body);
    rsa_verify(&client_pubkey, signed_message.as_bytes(), &signature)?;
    // Only after the signature checks out: otherwise anyone could burn
    // nonces (or fill the table) without being a paired client.
    rt.consume_nonce(&req.nonce).await?;

    let aes_key = {
        let mut sessions = rt.sessions.write().await;
        let s = sessions.get_mut(&req.session_id).filter(|s| s.client_id == req.client_id)
            .ok_or_else(|| anyhow::anyhow!("session inconnue ou expiree - ouvrez-en une nouvelle via /remote/session"))?;
        if (Utc::now() - s.last_used).num_seconds() >= SESSION_IDLE_TTL_SECS {
            sessions.remove(&req.session_id);
            anyhow::bail!("session inconnue ou expiree - ouvrez-en une nouvelle via /remote/session");
        }
        s.last_used = Utc::now();
        s.key
    };

    let blob_json = B64.decode(&req.encrypted_body).context("corps chiffre invalide (base64)")?;
    let blob: EncryptedBlob = serde_json::from_slice(&blob_json).context("enveloppe chiffree invalide")?;
    let plaintext = crate::secrets::decrypt(&aes_key, &blob)?;
    let action = serde_json::from_str(&plaintext).context("action JSON invalide")?;
    // Only reached once signature + nonce + session decrypt all succeeded.
    rt.touch_last_used(&req.client_id).await;
    Ok((action, aes_key))
}

fn encrypt_with(aes_key: &[u8; 32], value: &serde_json::Value) -> Result<String> {
    let blob = crate::secrets::encrypt(aes_key, &value.to_string())?;
    Ok(B64.encode(serde_json::to_vec(&blob)?))
}

fn decrypt_with(aes_key: &[u8; 32], encrypted_body: &str) -> Result<serde_json::Value> {
    let blob_json = B64.decode(encrypted_body)?;
    let blob: EncryptedBlob = serde_json::from_slice(&blob_json)?;
    let plain = crate::secrets::decrypt(aes_key, &blob)?;
    Ok(serde_json::from_str(&plain)?)
}

// ───────────────────────── blob store (corps volumineux) ─────────────────────────

fn blob_path(data_dir: &Path, reference: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(reference).map_err(|_| anyhow::anyhow!("reference de blob invalide"))?;
    Ok(blob_dir(data_dir).join(format!("{id}.bin")))
}

/// Best-effort janitor: abandoned uploads (client died mid-transfer) don't
/// accumulate forever.
async fn sweep_old_blobs(data_dir: &Path) {
    let Ok(mut rd) = tokio::fs::read_dir(blob_dir(data_dir)).await else { return };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let old = entry.metadata().await.ok().and_then(|m| m.modified().ok()).and_then(|m| m.elapsed().ok()).map(|e| e.as_secs() > BLOB_MAX_AGE_SECS).unwrap_or(false);
        if old {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

async fn blob_append(data_dir: &Path, reference: &str, offset: u64, data: &[u8]) -> Result<u64> {
    use tokio::io::AsyncWriteExt;
    let path = blob_path(data_dir, reference)?;
    tokio::fs::create_dir_all(blob_dir(data_dir)).await?;
    let current = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    if current != offset {
        anyhow::bail!("decalage de blob inattendu (attendu {current}, recu {offset})");
    }
    if current + data.len() as u64 > TUNNEL_BODY_MAX as u64 {
        let _ = tokio::fs::remove_file(&path).await;
        anyhow::bail!("corps trop volumineux (maximum {} Mo)", TUNNEL_BODY_MAX / (1024 * 1024));
    }
    let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
    f.write_all(data).await?;
    f.flush().await?;
    Ok(current + data.len() as u64)
}

async fn blob_read_all(data_dir: &Path, reference: &str) -> Result<Vec<u8>> {
    let path = blob_path(data_dir, reference)?;
    let bytes = tokio::fs::read(&path).await.context("blob introuvable")?;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(bytes)
}

async fn blob_stash(data_dir: &Path, bytes: &[u8]) -> Result<String> {
    let reference = uuid::Uuid::new_v4().to_string();
    let mut offset = 0u64;
    for chunk in bytes.chunks(BLOB_CHUNK) {
        offset = blob_append(data_dir, &reference, offset, chunk).await?;
    }
    if bytes.is_empty() {
        blob_append(data_dir, &reference, 0, &[]).await?;
    }
    Ok(reference)
}

// ───────────────────────── tunnel HTTP ─────────────────────────

fn tunnel_path_allowed(path: &str) -> bool {
    if !path.starts_with("/api/") || path.contains("..") || path.contains('\\') || path.contains("//") || path.to_ascii_lowercase().contains("%2e") || path.contains('\0') {
        return false;
    }
    let only_path = path.split('?').next().unwrap_or(path);
    TUNNEL_ALLOWED_PREFIXES.iter().any(|p| only_path == *p || only_path.starts_with(&format!("{p}/")))
}

async fn tunnel_http(rt: &RemoteRuntime, action: &serde_json::Value) -> Result<serde_json::Value> {
    let method = action["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
    let method = axum::http::Method::from_bytes(method.as_bytes()).map_err(|_| anyhow::anyhow!("methode HTTP invalide"))?;
    let path = action["path"].as_str().context("path manquant")?;
    if !tunnel_path_allowed(path) {
        anyhow::bail!("chemin non autorise a travers le tunnel : {path}");
    }
    let body_bytes: Vec<u8> = if let Some(r) = action["body_ref"].as_str() {
        blob_read_all(&rt.data_dir, r).await?
    } else if let Some(b) = action["body_b64"].as_str() {
        B64.decode(b).context("corps invalide (base64)")?
    } else {
        Vec::new()
    };

    let mut builder = axum::http::Request::builder().method(method).uri(path).header(TUNNEL_HEADER, "1");
    if let Some(ct) = action["content_type"].as_str() {
        builder = builder.header(axum::http::header::CONTENT_TYPE, ct);
    }
    let request = builder.body(axum::body::Body::from(body_bytes)).context("requete invalide")?;

    let response = rt.api_router.clone().oneshot(request).await.map_err(|e| anyhow::anyhow!("erreur interne: {e}"))?;
    let status = response.status().as_u16();
    let content_type = response.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let content_disposition = response.headers().get(axum::http::header::CONTENT_DISPOSITION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let bytes = axum::body::to_bytes(response.into_body(), TUNNEL_BODY_MAX).await.map_err(|e| anyhow::anyhow!("reponse trop volumineuse ou illisible: {e}"))?;

    let mut out = serde_json::json!({ "status": status, "content_type": content_type, "content_disposition": content_disposition, "size": bytes.len() });
    if bytes.len() <= INLINE_BODY_MAX {
        out["body_b64"] = serde_json::Value::String(B64.encode(&bytes));
    } else {
        out["body_ref"] = serde_json::Value::String(blob_stash(&rt.data_dir, &bytes).await?);
    }
    Ok(out)
}

/// Dispatches one decrypted action to the actual MCManager operation and
/// returns its (still-plaintext, to be encrypted by the caller) JSON
/// result. Mirrors the server REPL's command set (`list`, `status`,
/// `start`, `stop`, `restart`, `send`, `logs`) - same actions, remote
/// transport - plus the generic `http` tunnel and its blob helpers.
pub async fn dispatch_action(rt: &RemoteRuntime, action: serde_json::Value) -> serde_json::Value {
    let state = &rt.app_state;
    let name = action["action"].as_str().unwrap_or("");
    let server_id = action["server_id"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok());

    let result: Result<serde_json::Value> = async {
        match name {
            "ping" => Ok(serde_json::json!({ "ok": true, "version": crate::updater::CURRENT_VERSION })),
            "http" => tunnel_http(rt, &action).await,
            "blob_put" => {
                let reference = action["ref"].as_str().context("ref manquant")?;
                let offset = action["offset"].as_u64().unwrap_or(0);
                let data = B64.decode(action["data_b64"].as_str().context("data_b64 manquant")?).context("donnees invalides (base64)")?;
                if offset == 0 {
                    sweep_old_blobs(&rt.data_dir).await;
                }
                let size = blob_append(&rt.data_dir, reference, offset, &data).await?;
                Ok(serde_json::json!({ "ok": true, "size": size }))
            }
            "blob_get" => {
                let reference = action["ref"].as_str().context("ref manquant")?;
                let offset = action["offset"].as_u64().unwrap_or(0);
                let path = blob_path(&rt.data_dir, reference)?;
                let bytes = tokio::fs::read(&path).await.context("blob introuvable")?;
                let start = (offset as usize).min(bytes.len());
                let end = (start + BLOB_CHUNK).min(bytes.len());
                let done = end >= bytes.len();
                if done {
                    let _ = tokio::fs::remove_file(&path).await;
                }
                Ok(serde_json::json!({ "data_b64": B64.encode(&bytes[start..end]), "next": end, "done": done }))
            }
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
                tokio::time::sleep(Duration::from_secs(3)).await;
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
            "console" => {
                // Incremental console feed with long-polling: returns as soon
                // as there is at least one line newer than `since` (or after
                // `wait_ms`), so a remote console feels live without a
                // permanent socket - and without the remote needing to
                // accept anything but this one authenticated endpoint.
                let id = server_id.context("server_id manquant")?;
                let mut since = action["since"].as_u64();
                let wait = Duration::from_millis(action["wait_ms"].as_u64().unwrap_or(0).min(25_000));
                let deadline = Instant::now() + wait;
                loop {
                    {
                        let runtime = state.runtime.read().await;
                        if let Some(r) = runtime.get(&id) {
                            let total = r.seq;
                            let first = total - r.backlog.len() as u64;
                            if since.map(|s| s > total).unwrap_or(false) {
                                since = None; // counter went backwards: the remote process restarted
                            }
                            let from = since.map(|s| s.max(first)).unwrap_or(first);
                            if from < total {
                                let lines = r.backlog[(from - first) as usize..].to_vec();
                                return Ok(serde_json::json!({ "lines": lines, "next": total, "running": r.running }));
                            }
                            if since.is_none() || Instant::now() >= deadline {
                                return Ok(serde_json::json!({ "lines": [], "next": total, "running": r.running }));
                            }
                        } else if since.is_none() || Instant::now() >= deadline {
                            return Ok(serde_json::json!({ "lines": [], "next": 0, "running": false }));
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
            "import_server" => {
                // Receives a whole server (zipped) from a remote caller -
                // what powers "send a local server to a remote instance".
                // The archive arrives either inline (`zip_base64`, small) or
                // through the chunked blob store (`zip_ref`, the normal path
                // since v1.0.8 - no more giant single request).
                let name = action["name"].as_str().context("name manquant")?.to_string();
                let loader: crate::models::Loader = serde_json::from_value(action["loader"].clone()).context("loader invalide")?;
                let mc_version = action["mc_version"].as_str().context("mc_version manquant")?.to_string();
                let port = action["port"].as_u64().unwrap_or(25565) as u16;
                let zip_bytes = if let Some(r) = action["zip_ref"].as_str() {
                    blob_read_all(&rt.data_dir, r).await?
                } else {
                    B64.decode(action["zip_base64"].as_str().context("zip_ref ou zip_base64 manquant")?).context("archive invalide (base64)")?
                };

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
    let (action, aes_key) = authenticate_and_decrypt(rt, &req).await?;
    let result = dispatch_action(rt, action).await;
    let encrypted_body = encrypt_with(&aes_key, &result)?;
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

async fn http_session(axum::extract::State(rt): axum::extract::State<RemoteRuntime>, axum::Json(req): axum::Json<SessionRequest>) -> Result<axum::Json<SessionResponse>, axum::response::Response> {
    start_session(&rt, req).await.map(axum::Json).map_err(|e| bad_request(e))
}

async fn http_api(axum::extract::State(rt): axum::extract::State<RemoteRuntime>, axum::Json(req): axum::Json<ApiRequest>) -> Result<axum::Json<ApiResponse>, axum::response::Response> {
    handle_api_request(&rt, req).await.map(axum::Json).map_err(|e| bad_request(e))
}

fn bad_request(e: anyhow::Error) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}

/// The `/remote/*` routes alone. Mounted either on a dedicated port
/// (`serve`) or merged into the web server's own router (`mcmanager-server`
/// daemon: one port for the UI and for paired machines).
pub fn router(rt: RemoteRuntime) -> axum::Router {
    axum::Router::new()
        .route("/remote/info", axum::routing::get(http_info))
        .route("/remote/pair", axum::routing::post(http_pair))
        .route("/remote/session", axum::routing::post(http_session))
        .route("/remote/api", axum::routing::post(http_api).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)))
        .with_state(rt)
}

/// Binds `0.0.0.0:port` and serves the remote-control API until the
/// returned future is dropped/aborted. The caller (the server REPL)
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
    let resp = http.get(format!("http://{host}/remote/info")).timeout(Duration::from_secs(15)).send().await.context("instance distante injoignable")?;
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
    let resp = http.post(format!("http://{host}/remote/pair")).timeout(Duration::from_secs(30)).json(&body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("echec du jumelage : {}", error_text(resp).await);
    }
    let pair_resp: PairResponse = resp.json().await?;

    let mut targets = load_targets(data_dir).await;
    targets.retain(|t| t.label != label);
    targets.push(RemoteTarget { label: label.to_string(), host: host.to_string(), server_public_key_pem: pair_resp.server_public_key_pem });
    save_targets(data_dir, &targets).await?;
    Ok(())
}

/// Extracts the `{"error": "..."}` message the remote API answers with, or
/// falls back to the raw body.
async fn error_text(resp: reqwest::Response) -> String {
    let text = resp.text().await.unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"].as_str().map(|s| s.to_string())).unwrap_or(text)
}

struct ClientSession {
    session_id: String,
    key: [u8; 32],
    opened_at: Instant,
}

/// Cache of open sessions per remote instance, shared by every caller in
/// this process (the dashboard refresher, each tunnelled request, console
/// streams...). Before v1.0.8 a brand-new RSA session was negotiated for
/// *every* call - slow, and (server side) destructive for concurrent calls.
static CLIENT_SESSIONS: OnceLock<tokio::sync::Mutex<HashMap<String, ClientSession>>> = OnceLock::new();
/// Rotated well before the server's idle expiry so a long-lived cached
/// session never hits the edge of its lifetime in the middle of a call.
const CLIENT_SESSION_MAX_AGE: Duration = Duration::from_secs(10 * 60);

fn session_cache() -> &'static tokio::sync::Mutex<HashMap<String, ClientSession>> {
    CLIENT_SESSIONS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

fn cache_key(target: &RemoteTarget, identity: &RemoteIdentity) -> String {
    format!("{}|{}|{}", identity.fingerprint(), target.label, target.host)
}

async fn open_session(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget) -> Result<ClientSession> {
    let mut aes_key = [0u8; 32];
    OsRng.fill_bytes(&mut aes_key);
    let server_pubkey = parse_public_key(&target.server_public_key_pem)?;
    let wrapped = B64.encode(rsa_encrypt(&server_pubkey, &aes_key)?);
    let client_id = identity.fingerprint();
    let timestamp = Utc::now().timestamp();
    let signature = rsa_sign(&identity.private_key, session_signed_message(&client_id, timestamp, &wrapped).as_bytes())?;

    let session_body = serde_json::json!({ "client_id": client_id, "encrypted_key": wrapped, "timestamp": timestamp, "signature": B64.encode(signature) });
    let resp = http.post(format!("http://{}/remote/session", target.host)).timeout(Duration::from_secs(20)).json(&session_body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("echec d'etablissement de session : {}", error_text(resp).await);
    }
    let parsed: SessionResponse = resp.json().await.context("reponse de session invalide")?;
    Ok(ClientSession { session_id: parsed.session_id, key: aes_key, opened_at: Instant::now() })
}

async fn ensure_session(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, force_new: bool) -> Result<(String, [u8; 32])> {
    let key = cache_key(target, identity);
    let mut cache = session_cache().lock().await;
    if !force_new {
        if let Some(s) = cache.get(&key) {
            if s.opened_at.elapsed() < CLIENT_SESSION_MAX_AGE {
                return Ok((s.session_id.clone(), s.key));
            }
        }
    }
    let session = open_session(http, identity, target).await?;
    let out = (session.session_id.clone(), session.key);
    cache.insert(key, session);
    Ok(out)
}

async fn forget_session(identity: &RemoteIdentity, target: &RemoteTarget) {
    session_cache().lock().await.remove(&cache_key(target, identity));
}

async fn call_once(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, action: &serde_json::Value, timeout: Duration, force_new_session: bool) -> Result<serde_json::Value> {
    let (session_id, aes_key) = ensure_session(http, identity, target, force_new_session).await?;
    let client_id = identity.fingerprint();
    let encrypted_body = encrypt_with(&aes_key, action)?;
    let timestamp = Utc::now().timestamp();
    let mut nonce_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce: String = nonce_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let signature = rsa_sign(&identity.private_key, api_signed_message(timestamp, &session_id, &nonce, &encrypted_body).as_bytes())?;

    let req_body = serde_json::json!({
        "client_id": client_id, "session_id": session_id, "timestamp": timestamp, "nonce": nonce,
        "signature": B64.encode(signature), "encrypted_body": encrypted_body,
    });
    let resp = http.post(format!("http://{}/remote/api", target.host)).timeout(timeout).json(&req_body).send().await.context("instance distante injoignable")?;
    if !resp.status().is_success() {
        anyhow::bail!("erreur distante : {}", error_text(resp).await);
    }
    let api_resp: ApiResponse = resp.json().await?;
    decrypt_with(&aes_key, &api_resp.encrypted_body)
}

/// One remote action end to end: signed + encrypted request over a cached
/// session (opened on demand, transparently re-opened if the remote
/// forgot it - restart, idle expiry), decrypted response. Safe to call from
/// any number of tasks at once.
pub async fn client_call_timeout(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, action: serde_json::Value, timeout: Duration) -> Result<serde_json::Value> {
    match call_once(http, identity, target, &action, timeout, false).await {
        Err(e) if e.to_string().contains("session inconnue") => {
            forget_session(identity, target).await;
            call_once(http, identity, target, &action, timeout, true).await
        }
        other => other,
    }
}

pub async fn client_call(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, action: serde_json::Value) -> Result<serde_json::Value> {
    client_call_timeout(http, identity, target, action, Duration::from_secs(60)).await
}

/// Like `client_call` but turns an `{"error": ...}` answer into an `Err`.
pub async fn client_call_checked(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, action: serde_json::Value, timeout: Duration) -> Result<serde_json::Value> {
    let v = client_call_timeout(http, identity, target, action, timeout).await?;
    if let Some(e) = v["error"].as_str() {
        anyhow::bail!("{e}");
    }
    Ok(v)
}

/// Uploads `bytes` to the remote blob store in chunks; returns the
/// reference to pass as `body_ref` / `zip_ref`.
pub async fn client_blob_upload(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, bytes: &[u8]) -> Result<String> {
    let reference = uuid::Uuid::new_v4().to_string();
    let mut offset = 0u64;
    let mut chunks = bytes.chunks(BLOB_CHUNK).peekable();
    if chunks.peek().is_none() {
        client_call_checked(http, identity, target, serde_json::json!({ "action": "blob_put", "ref": reference, "offset": 0, "data_b64": "" }), Duration::from_secs(60)).await?;
    }
    for chunk in chunks {
        let action = serde_json::json!({ "action": "blob_put", "ref": reference, "offset": offset, "data_b64": B64.encode(chunk) });
        client_call_checked(http, identity, target, action, Duration::from_secs(180)).await?;
        offset += chunk.len() as u64;
    }
    Ok(reference)
}

async fn client_blob_download(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, reference: &str, expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected.min(TUNNEL_BODY_MAX));
    let mut offset = 0u64;
    loop {
        let r = client_call_checked(http, identity, target, serde_json::json!({ "action": "blob_get", "ref": reference, "offset": offset }), Duration::from_secs(180)).await?;
        let data = B64.decode(r["data_b64"].as_str().unwrap_or("")).context("morceau de blob invalide")?;
        out.extend_from_slice(&data);
        if r["done"].as_bool().unwrap_or(true) {
            break;
        }
        offset = r["next"].as_u64().unwrap_or(offset + data.len() as u64);
        if out.len() > TUNNEL_BODY_MAX {
            anyhow::bail!("reponse distante trop volumineuse");
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub struct TunnelResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
    pub body: Vec<u8>,
}

/// Performs one HTTP request *on the remote instance's own REST API*
/// (see "The tunnel" in the module docs). `path` includes the query string,
/// e.g. `/api/servers/<id>/files?path=plugins`.
pub async fn client_http(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, method: &str, path: &str, content_type: Option<&str>, body: Vec<u8>) -> Result<TunnelResponse> {
    let mut action = serde_json::json!({ "action": "http", "method": method, "path": path, "content_type": content_type });
    if body.len() > INLINE_BODY_MAX {
        action["body_ref"] = serde_json::Value::String(client_blob_upload(http, identity, target, &body).await?);
    } else if !body.is_empty() {
        action["body_b64"] = serde_json::Value::String(B64.encode(&body));
    }
    let r = client_call_checked(http, identity, target, action, Duration::from_secs(600)).await?;
    let size = r["size"].as_u64().unwrap_or(0) as usize;
    let body = if let Some(reference) = r["body_ref"].as_str() {
        client_blob_download(http, identity, target, reference, size).await?
    } else {
        B64.decode(r["body_b64"].as_str().unwrap_or("")).context("corps de reponse invalide")?
    };
    Ok(TunnelResponse {
        status: r["status"].as_u64().unwrap_or(502) as u16,
        content_type: r["content_type"].as_str().map(|s| s.to_string()),
        content_disposition: r["content_disposition"].as_str().map(|s| s.to_string()),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_keypair() -> (RsaPrivateKey, RsaPublicKey) {
        let sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap(); // small key: fast tests, crypto correctness doesn't depend on key size
        let pk = RsaPublicKey::from(&sk);
        (sk, pk)
    }

    async fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcmanager-remote-{tag}-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&d).await.unwrap();
        d
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

    #[test]
    fn tunnel_allowlist() {
        assert!(tunnel_path_allowed("/api/servers"));
        assert!(tunnel_path_allowed("/api/servers/abc/files?path=plugins"));
        assert!(tunnel_path_allowed("/api/marketplace/search?q=x"));
        assert!(!tunnel_path_allowed("/api/settings"));
        assert!(!tunnel_path_allowed("/api/ai/config"));
        assert!(!tunnel_path_allowed("/api/remote/targets"));
        assert!(!tunnel_path_allowed("/api/update/apply"));
        assert!(!tunnel_path_allowed("/api/serversX"));
        assert!(!tunnel_path_allowed("/api/servers/../settings"));
        assert!(!tunnel_path_allowed("/api/servers/%2e%2e/settings"));
        assert!(!tunnel_path_allowed("/remote/info"));
        assert!(!tunnel_path_allowed("/api//settings"));
    }

    /// A host with a real listener, plus helpers to pair any number of
    /// distinct client identities against it.
    struct TestHost {
        rt: RemoteRuntime,
        addr: String,
        dir: PathBuf,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestHost {
        async fn start() -> Self {
            let dir = temp_dir("host").await;
            let temp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = temp_listener.local_addr().unwrap().port();
            drop(temp_listener);
            let app_state = crate::state::build_state_for_test(&dir).await;
            let rt = RemoteRuntime::new(app_state, dir.clone()).await.unwrap();
            let rt2 = rt.clone();
            let task = tokio::spawn(async move {
                let _ = serve(rt2, port).await;
            });
            tokio::time::sleep(Duration::from_millis(150)).await;
            TestHost { rt, addr: format!("127.0.0.1:{port}"), dir, task }
        }

        async fn pair_new_client(&self, label: &str) -> (RemoteIdentity, RemoteTarget, PathBuf) {
            let client_dir = temp_dir("client").await;
            let identity = load_or_create_identity(&client_dir).await.unwrap();
            let (code, _) = self.rt.generate_pairing_code(PAIRING_CODE_DEFAULT_TTL_MIN).await;
            let http = reqwest::Client::new();
            client_pair(&http, &client_dir, &identity, &self.addr, label, &code).await.unwrap();
            let target = load_targets(&client_dir).await.into_iter().find(|t| t.label == label).unwrap();
            (identity, target, client_dir)
        }
    }

    impl Drop for TestHost {
        fn drop(&mut self) {
            self.task.abort();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn sample_entry(id: uuid::Uuid, folder: &Path) -> crate::models::ServerEntry {
        crate::models::ServerEntry {
            id,
            name: "test".into(),
            loader: crate::models::Loader::Paper,
            mc_version: "1.21.4".into(),
            loader_version: None,
            folder: folder.to_string_lossy().to_string(),
            jar_name: String::new(),
            java_path: "java".into(),
            xms_mb: 512, xmx_mb: 1024, port: 25599,
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
        }
    }

    /// Full round-trip of the actual wire protocol: pairing -> session
    /// establishment -> a signed+encrypted request -> verified decrypt.
    #[tokio::test]
    async fn full_pairing_and_request_flow() {
        let tmp = temp_dir("flow").await;
        let app_state = crate::state::build_state_for_test(&tmp).await;
        let rt = RemoteRuntime::new(app_state, tmp.clone()).await.unwrap();

        let client_sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let client_pk = RsaPublicKey::from(&client_sk);
        let client_pk_pem = client_pk.to_public_key_pem(LineEnding::LF).unwrap();

        let (code, _) = rt.generate_pairing_code(PAIRING_CODE_DEFAULT_TTL_MIN).await;
        let pair_resp = pair(&rt, PairRequest { client_id: "test-client".into(), label: "Test PC".into(), public_key_pem: client_pk_pem, code }).await.unwrap();
        assert_eq!(pair_resp.server_public_key_pem, rt.identity.public_pem());

        let mut aes_key = [0u8; 32];
        OsRng.fill_bytes(&mut aes_key);
        let server_pubkey = parse_public_key(&pair_resp.server_public_key_pem).unwrap();
        let wrapped = B64.encode(rsa_encrypt(&server_pubkey, &aes_key).unwrap());
        let ts = Utc::now().timestamp();
        let sig = rsa_sign(&client_sk, session_signed_message("test-client", ts, &wrapped).as_bytes()).unwrap();
        let session = start_session(&rt, SessionRequest { client_id: "test-client".into(), encrypted_key: wrapped, timestamp: ts, signature: B64.encode(sig) }).await.unwrap();

        let make_request = |nonce: &str| {
            let encrypted_body = encrypt_with(&aes_key, &serde_json::json!({ "action": "list" })).unwrap();
            let timestamp = Utc::now().timestamp();
            let signature = rsa_sign(&client_sk, api_signed_message(timestamp, &session.session_id, nonce, &encrypted_body).as_bytes()).unwrap();
            ApiRequest { client_id: "test-client".into(), session_id: session.session_id.clone(), timestamp, nonce: nonce.into(), signature: B64.encode(signature), encrypted_body }
        };

        let resp = handle_api_request(&rt, make_request("nonce-number-one")).await.unwrap();
        let value = decrypt_with(&aes_key, &resp.encrypted_body).unwrap();
        assert!(value["servers"].is_array());

        // Replay of the very same nonce must be refused, even though signature and freshness are still valid.
        let replay = handle_api_request(&rt, make_request("nonce-number-one")).await;
        assert!(replay.is_err(), "a reused nonce must be rejected");
        assert!(replay.err().unwrap().to_string().contains("rejeu"));

        // A tampered body breaks the signature.
        let mut tampered = make_request("nonce-number-two");
        tampered.encrypted_body.push('A');
        assert!(handle_api_request(&rt, tampered).await.is_err());

        tokio::fs::remove_dir_all(&tmp).await.ok();
    }

    #[tokio::test]
    async fn unpaired_client_is_rejected() {
        let tmp = temp_dir("unpaired").await;
        let app_state = crate::state::build_state_for_test(&tmp).await;
        let rt = RemoteRuntime::new(app_state, tmp.clone()).await.unwrap();

        let client_sk = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let fake_key = [0u8; 32];
        let encrypted_body = encrypt_with(&fake_key, &serde_json::json!({ "action": "list" })).unwrap();
        let timestamp = Utc::now().timestamp();
        let signature = rsa_sign(&client_sk, api_signed_message(timestamp, "sess", "nonce-never-paired", &encrypted_body).as_bytes()).unwrap();

        let result = handle_api_request(&rt, ApiRequest {
            client_id: "never-paired".into(), session_id: "sess".into(), timestamp, nonce: "nonce-never-paired".into(),
            signature: B64.encode(signature), encrypted_body,
        }).await;
        assert!(result.is_err(), "a client that never paired must be rejected");
        tokio::fs::remove_dir_all(&tmp).await.ok();
    }

    /// Exercises the actual network path end to end, including an unpaired
    /// "attacker" identity.
    #[tokio::test]
    async fn full_round_trip_over_real_http() {
        let host = TestHost::start().await;
        let http = reqwest::Client::new();

        let (fingerprint, _pubkey) = client_fetch_info(&http, &host.addr).await.unwrap();
        assert_eq!(fingerprint, host.rt.fingerprint());

        let (identity, target, client_dir) = host.pair_new_client("test-host").await;
        let result = client_call(&http, &identity, &target, serde_json::json!({ "action": "list" })).await.unwrap();
        assert!(result["servers"].is_array());
        let pong = client_call(&http, &identity, &target, serde_json::json!({ "action": "ping" })).await.unwrap();
        assert_eq!(pong["ok"], true);

        let attacker_dir = temp_dir("attacker").await;
        let attacker_identity = load_or_create_identity(&attacker_dir).await.unwrap();
        let fake_target = RemoteTarget { label: "x".into(), host: host.addr.clone(), server_public_key_pem: host.rt.identity.public_pem() };
        assert!(client_call(&http, &attacker_identity, &fake_target, serde_json::json!({ "action": "list" })).await.is_err(),
            "an unpaired identity must be rejected even over the real HTTP path");

        tokio::fs::remove_dir_all(&client_dir).await.ok();
        tokio::fs::remove_dir_all(&attacker_dir).await.ok();
    }

    /// The v1 bug: every call replaced the client's single session key, so
    /// simultaneous calls (two browser tabs, a refresher + a console...)
    /// broke each other. 24 parallel calls from one client plus 2 more
    /// clients at the same time must all succeed.
    #[tokio::test]
    async fn concurrent_calls_and_multiple_clients_do_not_interfere() {
        let host = TestHost::start().await;
        let http = reqwest::Client::new();
        let (id_a, target_a, dir_a) = host.pair_new_client("a").await;
        let (id_b, target_b, dir_b) = host.pair_new_client("b").await;
        let (id_c, target_c, dir_c) = host.pair_new_client("c").await;
        assert_ne!(id_a.fingerprint(), id_b.fingerprint());

        let (id_a, id_b, id_c) = (Arc::new(id_a), Arc::new(id_b), Arc::new(id_c));
        let mut tasks = Vec::new();
        for i in 0..24 {
            let (http, id, t) = (http.clone(), id_a.clone(), target_a.clone());
            tasks.push(tokio::spawn(async move { client_call(&http, &id, &t, serde_json::json!({ "action": "list", "i": i })).await }));
        }
        for _ in 0..4 {
            let (http2, id, t) = (http.clone(), id_b.clone(), target_b.clone());
            tasks.push(tokio::spawn(async move { client_call(&http2, &id, &t, serde_json::json!({ "action": "ping" })).await }));
            let (http3, id, t) = (http.clone(), id_c.clone(), target_c.clone());
            tasks.push(tokio::spawn(async move { client_call(&http3, &id, &t, serde_json::json!({ "action": "ping" })).await }));
        }
        for t in tasks {
            let r = t.await.unwrap();
            assert!(r.is_ok(), "concurrent call failed: {:?}", r.err());
        }

        // Three paired clients are listed, and revoking one leaves the others working.
        let clients = host.rt.clients_with_sessions().await;
        assert_eq!(clients.len(), 3);
        assert!(clients.iter().all(|(_, n)| *n >= 1));
        assert!(host.rt.revoke(&id_b.fingerprint()).await.unwrap());
        assert!(client_call(&http, &id_b, &target_b, serde_json::json!({ "action": "ping" })).await.is_err(), "a revoked client must be locked out immediately");
        assert!(client_call(&http, &id_a, &target_a, serde_json::json!({ "action": "ping" })).await.is_ok());
        assert!(client_call(&http, &id_c, &target_c, serde_json::json!({ "action": "ping" })).await.is_ok());

        for d in [dir_a, dir_b, dir_c] {
            tokio::fs::remove_dir_all(&d).await.ok();
        }
    }

    /// After the remote forgets its sessions (restart, idle expiry), the
    /// client silently opens a new one instead of failing.
    #[tokio::test]
    async fn client_recovers_when_remote_forgot_the_session() {
        let host = TestHost::start().await;
        let http = reqwest::Client::new();
        let (identity, target, dir) = host.pair_new_client("recover").await;
        assert!(client_call(&http, &identity, &target, serde_json::json!({ "action": "ping" })).await.is_ok());
        host.rt.sessions.write().await.clear();
        assert!(client_call(&http, &identity, &target, serde_json::json!({ "action": "ping" })).await.is_ok());
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    /// The tunnel: the remote's own REST API reached through the encrypted
    /// channel - JSON, a >2 MB upload (chunked blob path), a >2 MB download,
    /// and the allowlist refusing everything outside it.
    #[tokio::test]
    async fn http_tunnel_reaches_the_remote_api() {
        let host = TestHost::start().await;
        let http = reqwest::Client::new();
        let (identity, target, dir) = host.pair_new_client("tunnel").await;

        let server_dir = temp_dir("srv").await;
        let id = uuid::Uuid::new_v4();
        host.rt.app_state.servers.write().await.insert(id, sample_entry(id, &server_dir));

        let list = client_http(&http, &identity, &target, "GET", "/api/servers", None, vec![]).await.unwrap();
        assert_eq!(list.status, 200);
        let parsed: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);
        assert_eq!(parsed[0]["name"], "test");

        // > INLINE_BODY_MAX and > BLOB_CHUNK: exercises the chunked upload and download paths.
        let big_content: String = (0..(2 * 1024 * 1024 + 12345)).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let put = serde_json::json!({ "path": "big.txt", "content": big_content }).to_string().into_bytes();
        let r = client_http(&http, &identity, &target, "PUT", &format!("/api/servers/{id}/files/content"), Some("application/json"), put).await.unwrap();
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(tokio::fs::metadata(server_dir.join("big.txt")).await.unwrap().len() as usize, big_content.len());

        // The editor refuses > 2 MB files by design, so exercise the chunked *download* path with
        // an export (zip) of incompressible data instead: bigger than one blob chunk.
        let mut random = vec![0u8; 3 * 1024 * 1024];
        OsRng.fill_bytes(&mut random);
        tokio::fs::write(server_dir.join("random.bin"), &random).await.unwrap();
        let r = client_http(&http, &identity, &target, "GET", &format!("/api/servers/{id}/files/export?path="), None, vec![]).await.unwrap();
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body[..r.body.len().min(200)]));
        assert!(r.body.len() > 3 * 1024 * 1024, "zip should carry the 3 MB of random data ({} bytes)", r.body.len());
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(r.body)).unwrap();
        let mut entry = archive.by_name("random.bin").unwrap();
        let mut got = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut got).unwrap();
        assert_eq!(got, random, "bytes must survive the chunked, encrypted round trip intact");

        // Not on the allowlist: refused before reaching any handler.
        assert!(client_http(&http, &identity, &target, "GET", "/api/settings", None, vec![]).await.is_err());
        assert!(client_http(&http, &identity, &target, "GET", "/api/ai/config", None, vec![]).await.is_err());
        // A handler error is relayed as a normal HTTP status, not a transport failure.
        let missing = client_http(&http, &identity, &target, "GET", &format!("/api/servers/{}/status", uuid::Uuid::new_v4()), None, vec![]).await.unwrap();
        assert!(missing.status >= 400);

        tokio::fs::remove_dir_all(&server_dir).await.ok();
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    /// Incremental console feed: first call returns the backlog, later calls
    /// only newer lines, and a long-poll wakes up as soon as one arrives.
    #[tokio::test]
    async fn console_feed_is_incremental() {
        let host = TestHost::start().await;
        let http = reqwest::Client::new();
        let (identity, target, dir) = host.pair_new_client("console").await;
        let id = uuid::Uuid::new_v4();
        {
            let mut runtime = host.rt.app_state.runtime.write().await;
            let r = runtime.entry(id).or_insert_with(crate::state::ServerRuntime::default);
            r.push_line("one".into());
            r.push_line("two".into());
        }
        let first = client_call_checked(&http, &identity, &target, serde_json::json!({ "action": "console", "server_id": id }), Duration::from_secs(10)).await.unwrap();
        assert_eq!(first["lines"], serde_json::json!(["one", "two"]));
        let next = first["next"].as_u64().unwrap();

        let state = host.rt.app_state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            state.runtime.write().await.get_mut(&id).unwrap().push_line("three".into());
        });
        let started = Instant::now();
        let second = client_call_checked(&http, &identity, &target, serde_json::json!({ "action": "console", "server_id": id, "since": next, "wait_ms": 5000 }), Duration::from_secs(10)).await.unwrap();
        assert_eq!(second["lines"], serde_json::json!(["three"]));
        assert!(started.elapsed() < Duration::from_secs(4), "long-poll should wake as soon as a line arrives");

        let idle = client_call_checked(&http, &identity, &target, serde_json::json!({ "action": "console", "server_id": id, "since": second["next"], "wait_ms": 300 }), Duration::from_secs(10)).await.unwrap();
        assert_eq!(idle["lines"], serde_json::json!([]));
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}

#[allow(dead_code)]
fn _send_probe(rt: &RemoteRuntime, req: ApiRequest, a: serde_json::Value) {
    fn s<T: Send>(_: T) {}
    s(authenticate_and_decrypt(rt, &req));
    s(tunnel_http(rt, &a));
    s(dispatch_action(rt, a.clone()));
    s(handle_api_request(rt, req));
}
