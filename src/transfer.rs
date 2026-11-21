//! Transferts entre machines jumelées : envoi reprenable de fichiers volumineux, déplacement d'un serveur Minecraft,
//! sauvegardes envoyées vers une autre machine (à la main ou en miroir automatique).
//!
//! Principes :
//!  - **En flux** : les fichiers sont lus et écrits par morceaux de 2 Mo (jamais chargés en mémoire).
//!  - **Reprenables** : la référence du blob distant est dérivée du fichier (chemin, taille, date) ; après une coupure, relancer
//!    la même opération demande à la machine distante combien d'octets elle a déjà et reprend à cet endroit.
//!  - **Vérifiés** : l'empreinte SHA-256 du fichier est comparée de bout en bout avant toute action irréversible.
//!  - **Sûrs** : un serveur n'est supprimé de la machine d'origine qu'après avoir été importé *et retrouvé* sur la machine
//!    de destination, et seulement si l'utilisateur l'a demandé (« déplacer » ; « copier » ne supprime rien).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::remote::{self, RemoteIdentity, RemoteTarget, BLOB_CHUNK};
use crate::state::AppState;

const CHUNK_RETRIES: usize = 4;

// ───────────────────────── outils purs ─────────────────────────

/// SHA-256 d'un fichier, lu par blocs (bloquant : à appeler depuis `spawn_blocking`).
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("lecture de {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Référence de blob stable pour un fichier donné : même fichier => même référence => transfert reprenable.
pub fn blob_ref_for(path: &Path, size: u64, mtime_secs: u64) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(format!("mcm-transfer|{}|{size}|{mtime_secs}", path.display()).as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    Uuid::from_bytes(b).to_string()
}

/// Nom de fichier de sauvegarde acceptable (pas de chemin, extension .zip).
pub fn valid_backup_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.ends_with(".zip") && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Nom de machine source utilisable comme nom de dossier.
pub fn clean_source(label: &str) -> String {
    let s: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).take(64).collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() { "machine".into() } else { s }
}

// ───────────────────────── sauvegardes reçues (côté destination) ─────────────────────────

pub fn incoming_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("incoming-backups")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IncomingBackup {
    pub source: String,
    pub name: String,
    pub size_bytes: u64,
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
}

pub fn list_incoming(data_dir: &Path) -> Vec<IncomingBackup> {
    let mut out = Vec::new();
    let Ok(sources) = std::fs::read_dir(incoming_dir(data_dir)) else { return out };
    for s in sources.flatten() {
        if !s.path().is_dir() {
            continue;
        }
        let source = s.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(s.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !valid_backup_name(&name) {
                continue;
            }
            let Ok(meta) = f.metadata() else { continue };
            out.push(IncomingBackup { source: source.clone(), name, size_bytes: meta.len(), modified: meta.modified().ok().map(chrono::DateTime::<chrono::Utc>::from) });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    out.truncate(500);
    out
}

pub fn incoming_path(data_dir: &Path, source: &str, name: &str) -> Result<PathBuf> {
    if !valid_backup_name(name) || clean_source(source) != source {
        anyhow::bail!("nom de sauvegarde ou de machine invalide");
    }
    Ok(incoming_dir(data_dir).join(source).join(name))
}

pub fn delete_incoming(data_dir: &Path, source: &str, name: &str) -> Result<bool> {
    let p = incoming_path(data_dir, source, name)?;
    if !p.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(p)?;
    Ok(true)
}

// ───────────────────────── miroirs de sauvegarde ─────────────────────────

fn mirrors_path(data_dir: &Path) -> PathBuf {
    data_dir.join("backup-mirrors.json")
}

/// serveur → machine vers laquelle chaque nouvelle sauvegarde est copiée automatiquement.
pub async fn load_mirrors(data_dir: &Path) -> HashMap<Uuid, String> {
    match tokio::fs::read_to_string(mirrors_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

pub async fn set_mirror(data_dir: &Path, server: Uuid, target: Option<String>) -> Result<()> {
    let mut m = load_mirrors(data_dir).await;
    match target {
        Some(t) if !t.trim().is_empty() => m.insert(server, t),
        _ => m.remove(&server),
    };
    tokio::fs::write(mirrors_path(data_dir), serde_json::to_string_pretty(&m)?).await?;
    Ok(())
}

/// Après chaque sauvegarde : copie vers la machine miroir configurée (en arrière-plan, sans jamais gêner la sauvegarde).
pub async fn mirror_backup(state: &AppState, server: Uuid, backup_name: &str) {
    let Some(target) = load_mirrors(&state.data_dir).await.remove(&server) else { return };
    let st = state.clone();
    let name = backup_name.to_string();
    tokio::spawn(async move {
        let spec = Spec::SendBackup { server_id: server, name, label: target };
        let job = new_job(&st, &spec).await;
        run_job(st, job, spec).await;
    });
}

// ───────────────────────── tâches ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub target: String,
    /// `running` | `done` | `error`
    pub state: String,
    pub sent: u64,
    pub total: u64,
    pub message: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Spec {
    /// Envoie un serveur local vers une machine ; `delete_source` = le déplacer (le supprimer ici une fois vérifié).
    MoveServer { server_id: Uuid, label: String, delete_source: bool },
    /// Rapatrie un serveur d'une machine distante.
    PullServer { label: String, remote_id: Uuid, delete_source: bool },
    SendBackup { server_id: Uuid, name: String, label: String },
    PullBackup { label: String, source: String, name: String, server_id: Uuid },
}

impl Spec {
    fn kind(&self) -> &'static str {
        match self {
            Spec::MoveServer { delete_source: true, .. } | Spec::PullServer { delete_source: true, .. } => "move",
            Spec::MoveServer { .. } | Spec::PullServer { .. } => "copy",
            Spec::SendBackup { .. } => "backup",
            Spec::PullBackup { .. } => "restore",
        }
    }
    fn label(&self) -> &str {
        match self {
            Spec::MoveServer { label, .. } | Spec::PullServer { label, .. } | Spec::SendBackup { label, .. } | Spec::PullBackup { label, .. } => label,
        }
    }
}

struct Registry {
    jobs: Vec<Job>,
    specs: HashMap<String, Spec>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry { jobs: Vec::new(), specs: HashMap::new() }))
}

pub fn list_jobs() -> Vec<Job> {
    let mut v = registry().lock().map(|r| r.jobs.clone()).unwrap_or_default();
    v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    v
}

fn update_job(id: &str, f: impl FnOnce(&mut Job)) {
    if let Ok(mut r) = registry().lock() {
        if let Some(j) = r.jobs.iter_mut().find(|j| j.id == id) {
            f(j);
        }
    }
}

async fn title_for(state: &AppState, spec: &Spec) -> String {
    let name_of = |id: Uuid| async move { state.servers.read().await.get(&id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string()) };
    match spec {
        Spec::MoveServer { server_id, .. } => name_of(*server_id).await,
        Spec::SendBackup { server_id, name, .. } => format!("{} — {name}", name_of(*server_id).await),
        Spec::PullBackup { name, .. } => name.clone(),
        Spec::PullServer { remote_id, .. } => remote_id.to_string(),
    }
}

/// Crée (ou réactive, pour une reprise) l'entrée de suivi d'une tâche.
pub async fn new_job(state: &AppState, spec: &Spec) -> String {
    let id = Uuid::new_v4().to_string();
    let job = Job { id: id.clone(), kind: spec.kind().into(), title: title_for(state, spec).await, target: spec.label().into(), state: "running".into(), sent: 0, total: 0, message: String::new(), started_at: chrono::Utc::now(), finished_at: None };
    if let Ok(mut r) = registry().lock() {
        r.jobs.push(job);
        r.specs.insert(id.clone(), spec.clone());
        // On ne garde que les 30 dernières tâches terminées.
        let finished: Vec<String> = r.jobs.iter().filter(|j| j.state != "running").map(|j| j.id.clone()).collect();
        if finished.len() > 30 {
            for old in &finished[..finished.len() - 30] {
                r.jobs.retain(|j| &j.id != old);
                r.specs.remove(old);
            }
        }
    }
    id
}

/// Relance une tâche en erreur : même spécification, mêmes fichiers temporaires => le transfert reprend où il s'était arrêté.
pub fn retry_spec(job_id: &str) -> Option<Spec> {
    let r = registry().lock().ok()?;
    let failed = r.jobs.iter().any(|j| j.id == job_id && j.state == "error");
    failed.then(|| r.specs.get(job_id).cloned()).flatten()
}

pub async fn run_job(state: AppState, job_id: String, spec: Spec) {
    update_job(&job_id, |j| {
        j.state = "running".into();
        j.message.clear();
        j.finished_at = None;
    });
    let result = match &spec {
        Spec::MoveServer { server_id, label, delete_source } => move_server(&state, &job_id, *server_id, label, *delete_source).await,
        Spec::PullServer { label, remote_id, delete_source } => pull_server(&state, &job_id, label, *remote_id, *delete_source).await,
        Spec::SendBackup { server_id, name, label } => send_backup(&state, &job_id, *server_id, name, label).await,
        Spec::PullBackup { label, source, name, server_id } => pull_backup(&state, &job_id, label, source, name, *server_id).await,
    };
    match result {
        Ok(msg) => update_job(&job_id, |j| {
            j.state = "done".into();
            j.message = msg;
            j.finished_at = Some(chrono::Utc::now());
            j.sent = j.total;
        }),
        Err(e) => {
            tracing::warn!("transfert {job_id} en erreur : {e:#}");
            update_job(&job_id, |j| {
                j.state = "error".into();
                j.message = format!("{e:#}");
                j.finished_at = Some(chrono::Utc::now());
            });
        }
    }
}

// ───────────────────────── envoi / réception de fichiers (client) ─────────────────────────

async fn target_and_identity(state: &AppState, label: &str) -> Result<(RemoteTarget, std::sync::Arc<RemoteIdentity>)> {
    let identity = crate::hub::identity(state).await?;
    let target = crate::hub::target_by_label(state, label).await.ok_or_else(|| anyhow::anyhow!("machine distante « {label} » inconnue"))?;
    Ok((target, identity))
}

/// Envoie `path` dans le stockage de blobs de la machine distante. Reprend là où la machine distante s'était arrêtée.
pub async fn upload_file(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, path: &Path, reference: &str, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let total = tokio::fs::metadata(path).await.context("fichier à envoyer introuvable")?.len();
    let stat = |hash: bool| serde_json::json!({ "action": "blob_stat", "ref": reference, "hash": hash });
    let mut offset = remote::client_call_checked(http, identity, target, stat(false), Duration::from_secs(60)).await?["size"].as_u64().unwrap_or(0);
    if offset > total {
        // Reste d'un autre fichier : on repart de zéro.
        remote::client_call_checked(http, identity, target, json!({ "action": "blob_drop", "ref": reference }), Duration::from_secs(60)).await?;
        offset = 0;
    }
    let mut f = tokio::fs::File::open(path).await?;
    f.seek(std::io::SeekFrom::Start(offset)).await?;
    progress(offset, total);
    let mut buf = vec![0u8; BLOB_CHUNK];
    while offset < total {
        let mut n = 0;
        while n < buf.len() {
            let k = f.read(&mut buf[n..]).await?;
            if k == 0 {
                break;
            }
            n += k;
        }
        if n == 0 {
            anyhow::bail!("le fichier a changé pendant l'envoi");
        }
        let mut attempt = 0;
        loop {
            attempt += 1;
            let action = json!({ "action": "blob_put", "ref": reference, "offset": offset, "data_b64": base64_encode(&buf[..n]) });
            match remote::client_call_checked(http, identity, target, action, Duration::from_secs(180)).await {
                Ok(_) => break,
                Err(e) if attempt < CHUNK_RETRIES => {
                    tracing::debug!("morceau à {offset} : {e} (essai {attempt})");
                    tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                    // Le morceau est peut-être bien arrivé malgré l'erreur : on se recale sur la machine distante.
                    let remote_size = remote::client_call_checked(http, identity, target, stat(false), Duration::from_secs(60)).await.ok().and_then(|r| r["size"].as_u64());
                    if remote_size == Some(offset + n as u64) {
                        break;
                    }
                }
                Err(e) => return Err(e).context(format!("envoi interrompu à {} Mo : relancez pour reprendre", offset / (1024 * 1024))),
            }
        }
        offset += n as u64;
        progress(offset, total);
    }
    // Vérification de bout en bout avant d'utiliser le fichier.
    let local = { let p = path.to_path_buf(); tokio::task::spawn_blocking(move || sha256_file(&p)).await?? };
    let remote_state = remote::client_call_checked(http, identity, target, stat(true), Duration::from_secs(600)).await?;
    if remote_state["size"].as_u64() != Some(total) || remote_state["sha256"].as_str() != Some(local.as_str()) {
        let _ = remote::client_call_checked(http, identity, target, json!({ "action": "blob_drop", "ref": reference }), Duration::from_secs(60)).await;
        anyhow::bail!("le fichier reçu par l'autre machine ne correspond pas (empreinte différente) : transfert annulé, relancez-le");
    }
    Ok(())
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Télécharge un blob distant vers `dest` (fichier `.part` conservé en cas de coupure, donc reprenable), puis vérifie l'empreinte.
pub async fn download_file(http: &reqwest::Client, identity: &RemoteIdentity, target: &RemoteTarget, reference: &str, dest: &Path, expected_size: u64, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<()> {
    use base64::Engine;
    use tokio::io::AsyncWriteExt;
    let part = dest.with_extension("part");
    if let Some(parent) = part.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if offset > expected_size {
        let _ = tokio::fs::remove_file(&part).await;
        offset = 0;
    }
    let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&part).await?;
    progress(offset, expected_size);
    while offset < expected_size {
        let mut attempt = 0;
        let r = loop {
            attempt += 1;
            match remote::client_call_checked(http, identity, target, json!({ "action": "blob_get", "ref": reference, "offset": offset, "keep": true }), Duration::from_secs(180)).await {
                Ok(r) => break r,
                Err(e) if attempt < CHUNK_RETRIES => {
                    tracing::debug!("lecture à {offset} : {e} (essai {attempt})");
                    tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                }
                Err(e) => return Err(e).context(format!("téléchargement interrompu à {} Mo : relancez pour reprendre", offset / (1024 * 1024))),
            }
        };
        let data = base64::engine::general_purpose::STANDARD.decode(r["data_b64"].as_str().unwrap_or("")).context("morceau invalide")?;
        if data.is_empty() {
            anyhow::bail!("l'autre machine a renvoyé un morceau vide");
        }
        f.write_all(&data).await?;
        offset += data.len() as u64;
        progress(offset, expected_size);
    }
    f.flush().await?;
    drop(f);
    let local = { let p = part.clone(); tokio::task::spawn_blocking(move || sha256_file(&p)).await?? };
    let remote_state = remote::client_call_checked(http, identity, target, json!({ "action": "blob_stat", "ref": reference, "hash": true }), Duration::from_secs(600)).await?;
    if remote_state["sha256"].as_str() != Some(local.as_str()) {
        let _ = tokio::fs::remove_file(&part).await;
        anyhow::bail!("le fichier téléchargé est corrompu (empreinte différente) : relancez le transfert");
    }
    tokio::fs::rename(&part, dest).await?;
    let _ = remote::client_call_checked(http, identity, target, json!({ "action": "blob_drop", "ref": reference }), Duration::from_secs(60)).await;
    Ok(())
}

fn progress_for(job_id: &str) -> impl Fn(u64, u64) + Sync + '_ {
    move |sent, total| update_job(job_id, |j| {
        j.sent = sent;
        j.total = total;
    })
}

fn transfers_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("transfers")
}

fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

// ───────────────────────── opérations ─────────────────────────

async fn move_server(state: &AppState, job_id: &str, server_id: Uuid, label: &str, delete_source: bool) -> Result<String> {
    let entry = state.servers.read().await.get(&server_id).cloned().context("serveur introuvable")?;
    if state.runtime.read().await.get(&server_id).map(|r| r.running).unwrap_or(false) {
        anyhow::bail!("arrêtez le serveur avant de le déplacer ou de le copier (un monde en cours d'écriture donnerait une archive incohérente)");
    }
    let (target, identity) = target_and_identity(state, label).await?;
    // L'archive est conservée entre deux essais : c'est ce qui rend la reprise possible.
    tokio::fs::create_dir_all(transfers_dir(state)).await?;
    let zip_path = transfers_dir(state).join(format!("{job_id}.zip"));
    if !zip_path.exists() {
        let root = PathBuf::from(&entry.folder);
        let tmp = tokio::task::spawn_blocking(move || crate::files::export_zip(&root, "")).await??;
        if tokio::fs::rename(&tmp, &zip_path).await.is_err() {
            tokio::fs::copy(&tmp, &zip_path).await?;
            let _ = tokio::fs::remove_file(&tmp).await;
        }
    }
    let meta = tokio::fs::metadata(&zip_path).await?;
    let reference = blob_ref_for(&zip_path, meta.len(), mtime_secs(&meta));
    upload_file(&state.http, &identity, &target, &zip_path, &reference, &progress_for(job_id)).await?;

    let action = json!({
        "action": "import_server", "name": entry.name, "loader": entry.loader, "mc_version": entry.mc_version, "port": entry.port, "zip_ref": reference,
        "settings": { "xms_mb": entry.xms_mb, "xmx_mb": entry.xmx_mb, "jar_name": entry.jar_name, "extra_args": entry.extra_args, "aikar_flags": entry.aikar_flags,
                      "auto_restart": entry.auto_restart, "auto_restart_delay_secs": entry.auto_restart_delay_secs, "auto_backup_minutes": entry.auto_backup_minutes,
                      "backup_retention": entry.backup_retention, "loader_version": entry.loader_version },
    });
    let result = remote::client_call_checked(&state.http, &identity, &target, action, Duration::from_secs(900)).await?;
    let remote_id = result["server_id"].as_str().context("l'autre machine n'a pas confirmé la création du serveur")?.to_string();
    // On vérifie que le serveur existe bel et bien là-bas AVANT de toucher à l'original.
    let list = remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "list" }), Duration::from_secs(60)).await?;
    let found = list["servers"].as_array().map(|a| a.iter().any(|s| s["id"].as_str() == Some(remote_id.as_str()))).unwrap_or(false);
    if !found {
        anyhow::bail!("le serveur importé est introuvable sur l'autre machine : l'original est conservé");
    }
    let _ = tokio::fs::remove_file(&zip_path).await;
    crate::hub::refresh_target(state, &target).await;
    if delete_source {
        let warnings = crate::api::remove_server(state, server_id).await?;
        let extra = if warnings.is_empty() { String::new() } else { format!(" (avertissements : {})", warnings.join(" ; ")) };
        Ok(format!("Serveur déplacé vers « {label} » ; l'original a été supprimé ici{extra}."))
    } else {
        Ok(format!("Serveur copié vers « {label} » ; l'original est conservé ici."))
    }
}

async fn pull_server(state: &AppState, job_id: &str, label: &str, remote_id: Uuid, delete_source: bool) -> Result<String> {
    let (target, identity) = target_and_identity(state, label).await?;
    let info = remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "export_server", "server_id": remote_id.to_string() }), Duration::from_secs(900)).await?;
    let reference = info["ref"].as_str().context("export refusé")?.to_string();
    let size = info["size"].as_u64().context("taille inconnue")?;
    tokio::fs::create_dir_all(transfers_dir(state)).await?;
    let dest = transfers_dir(state).join(format!("{job_id}.zip"));
    download_file(&state.http, &identity, &target, &reference, &dest, size, &progress_for(job_id)).await?;

    let e = &info["entry"];
    let id = Uuid::new_v4();
    let folder = state.data_dir.join("servers").join(id.to_string());
    tokio::fs::create_dir_all(&folder).await?;
    let (f2, d2) = (folder.clone(), dest.clone());
    tokio::task::spawn_blocking(move || crate::files::import_zip_file(&f2, "", &d2)).await??;
    let _ = tokio::fs::remove_file(&dest).await;
    let java_path = state.config.read().await.java_path.clone();
    let s = &e["settings"];
    let entry = crate::models::ServerEntry {
        id, name: e["name"].as_str().unwrap_or("Serveur importé").to_string(),
        loader: serde_json::from_value(e["loader"].clone()).context("loader inconnu")?,
        mc_version: e["mc_version"].as_str().unwrap_or("").to_string(),
        loader_version: s["loader_version"].as_str().map(String::from),
        folder: folder.to_string_lossy().to_string(),
        jar_name: s["jar_name"].as_str().unwrap_or("").to_string(),
        java_path, xms_mb: s["xms_mb"].as_u64().unwrap_or(1024) as u32, xmx_mb: s["xmx_mb"].as_u64().unwrap_or(2048) as u32,
        port: e["port"].as_u64().unwrap_or(25565) as u16,
        extra_args: s["extra_args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        eula_accepted: true, auto_backup_minutes: s["auto_backup_minutes"].as_u64().map(|v| v as u32), backup_retention: s["backup_retention"].as_u64().map(|v| v as u32),
        auto_restart: s["auto_restart"].as_bool().unwrap_or(false), auto_restart_delay_secs: s["auto_restart_delay_secs"].as_u64().unwrap_or(5) as u32,
        scheduled_restart_minutes: None, stop_when_empty_minutes: None, dynamic_server: false,
        aikar_flags: s["aikar_flags"].as_bool().unwrap_or(false), managed_addons: vec![], created_at: chrono::Utc::now(),
    };
    state.servers.write().await.insert(id, entry);
    crate::state::save_servers(state).await?;
    if delete_source {
        remote::client_http(&state.http, &identity, &target, "DELETE", &format!("/api/servers/{remote_id}"), None, Vec::new()).await.context("suppression du serveur sur l'autre machine")?;
        crate::hub::refresh_target(state, &target).await;
        Ok(format!("Serveur rapatrié depuis « {label} » ; l'original a été supprimé là-bas."))
    } else {
        Ok(format!("Serveur copié depuis « {label} » ; l'original est conservé là-bas."))
    }
}

async fn send_backup(state: &AppState, job_id: &str, server_id: Uuid, name: &str, label: &str) -> Result<String> {
    if !valid_backup_name(name) {
        anyhow::bail!("nom de sauvegarde invalide");
    }
    let path = crate::backup::backups_dir(&state.data_dir, &server_id).join(name);
    if !path.is_file() {
        anyhow::bail!("sauvegarde introuvable");
    }
    if state.backup_progress.read().await.contains_key(&server_id) {
        anyhow::bail!("une sauvegarde est en cours pour ce serveur : réessayez quand elle est terminée");
    }
    let server_name = state.servers.read().await.get(&server_id).map(|e| e.name.clone()).unwrap_or_default();
    let (target, identity) = target_and_identity(state, label).await?;
    let meta = tokio::fs::metadata(&path).await?;
    let reference = blob_ref_for(&path, meta.len(), mtime_secs(&meta));
    upload_file(&state.http, &identity, &target, &path, &reference, &progress_for(job_id)).await?;
    // Sur l'autre machine, le dossier d'accueil porte le nom de CETTE machine (celle qui envoie).
    let me = std::env::var("MCMANAGER_MACHINE_NAME").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| sysinfo::System::host_name().unwrap_or_else(|| "machine".into()));
    let file_name = format!("{}-{name}", clean_source(&server_name));
    remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "backup_receive", "ref": reference, "source": clean_source(&me), "name": file_name }), Duration::from_secs(300)).await?;
    Ok(format!("Sauvegarde envoyée vers « {label} »."))
}

async fn pull_backup(state: &AppState, job_id: &str, label: &str, source: &str, name: &str, server_id: Uuid) -> Result<String> {
    if !state.servers.read().await.contains_key(&server_id) {
        anyhow::bail!("serveur local introuvable");
    }
    let (target, identity) = target_and_identity(state, label).await?;
    let info = remote::client_call_checked(&state.http, &identity, &target, json!({ "action": "backup_incoming_fetch", "source": source, "name": name }), Duration::from_secs(600)).await?;
    let reference = info["ref"].as_str().context("sauvegarde indisponible")?.to_string();
    let size = info["size"].as_u64().unwrap_or(0);
    let dir = crate::backup::backups_dir(&state.data_dir, &server_id);
    tokio::fs::create_dir_all(&dir).await?;
    // Nom local distinct : une sauvegarde récupérée ne remplace jamais une sauvegarde existante.
    let local_name = format!("recuperee-{name}");
    download_file(&state.http, &identity, &target, &reference, &dir.join(&local_name), size, &progress_for(job_id)).await?;
    Ok(format!("Sauvegarde récupérée : « {local_name} » (visible dans l'onglet Sauvegardes, restaurable)."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-transfer-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sha256_of_a_file_and_stable_blob_references() {
        let d = tmp();
        let p = d.join("a.bin");
        std::fs::write(&p, b"test").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08");
        let r1 = blob_ref_for(&p, 4, 100);
        assert_eq!(r1, blob_ref_for(&p, 4, 100), "même fichier => même référence (reprise possible)");
        assert_ne!(r1, blob_ref_for(&p, 5, 100));
        assert_ne!(r1, blob_ref_for(&p, 4, 101));
        assert!(Uuid::parse_str(&r1).is_ok(), "la référence doit être un UUID valide pour le stockage distant");
        assert!(sha256_file(&d.join("absent")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_are_sanitised() {
        for ok in ["backup-2026-11-14.zip", "Survie_1.zip"] {
            assert!(valid_backup_name(ok), "{ok}");
        }
        for bad in ["", "a.txt", "../x.zip", "a/b.zip", ".caché.zip", "a b.zip", "é.zip", &format!("{}.zip", "x".repeat(130))] {
            assert!(!valid_backup_name(bad), "{bad}");
        }
        assert_eq!(clean_source("Mon VPS (Paris)"), "Mon_VPS__Paris_");
        assert!(!clean_source("../../etc").contains('/'));
        assert_eq!(clean_source(""), "machine");
        assert_eq!(clean_source("..."), "machine");
    }

    #[test]
    fn incoming_backups_are_listed_and_deleted_safely() {
        let d = tmp();
        let dir = incoming_dir(&d).join("vps-paris");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Survie-a.zip"), b"zip").unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        let l = list_incoming(&d);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].source.as_str(), l[0].name.as_str(), l[0].size_bytes), ("vps-paris", "Survie-a.zip", 3));
        assert!(incoming_path(&d, "vps-paris", "../x.zip").is_err());
        assert!(incoming_path(&d, "../autre", "a.zip").is_err());
        assert!(!delete_incoming(&d, "vps-paris", "absent.zip").unwrap());
        assert!(delete_incoming(&d, "vps-paris", "Survie-a.zip").unwrap());
        assert!(list_incoming(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn mirrors_round_trip() {
        let d = tmp();
        let s = Uuid::new_v4();
        assert!(load_mirrors(&d).await.is_empty());
        set_mirror(&d, s, Some("vps".into())).await.unwrap();
        assert_eq!(load_mirrors(&d).await.get(&s).map(String::as_str), Some("vps"));
        set_mirror(&d, s, None).await.unwrap();
        assert!(load_mirrors(&d).await.is_empty());
        set_mirror(&d, s, Some("  ".into())).await.unwrap();
        assert!(load_mirrors(&d).await.is_empty(), "une cible vide désactive le miroir");
        let _ = std::fs::remove_dir_all(&d);
    }
}
