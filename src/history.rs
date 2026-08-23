//! Per-server boot/uptime/crash history, powering the "Statistiques" tab.
//! Distinct from `stats.rs` (live CPU/RAM/player-count snapshots of a
//! *running* process) - this is the persisted historical record: how many
//! times has this server been started, how long has it run in total, how
//! often has it crashed, and a scrollable log of recent sessions.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootRecord {
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub crashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerHistory {
    pub total_boots: u32,
    pub total_crashes: u32,
    pub total_uptime_secs: u64,
    /// Most recent first, capped at `MAX_RECORDS` - this is a UI log, not
    /// an audit trail, so unbounded growth isn't worth the disk/parse cost.
    pub records: Vec<BootRecord>,
}

const MAX_RECORDS: usize = 50;

fn history_path(data_dir: &Path, id: Uuid) -> PathBuf {
    data_dir.join("history").join(format!("{id}.json"))
}

async fn load(data_dir: &Path, id: Uuid) -> ServerHistory {
    match tokio::fs::read_to_string(history_path(data_dir, id)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => ServerHistory::default(),
    }
}

async fn save(data_dir: &Path, id: Uuid, hist: &ServerHistory) {
    let path = history_path(data_dir, id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    if let Ok(json) = serde_json::to_string_pretty(hist) {
        let _ = tokio::fs::write(&path, json).await;
    }
}

pub async fn get(data_dir: &Path, id: Uuid) -> ServerHistory {
    let mut hist = load(data_dir, id).await;
    hist.records.truncate(MAX_RECORDS);
    hist
}

/// Called from `process::start_server` right after the process actually
/// spawns. Pushes a new open-ended record (`stopped_at: None`).
pub async fn record_boot_start(data_dir: &Path, id: Uuid) {
    let mut hist = load(data_dir, id).await;
    hist.total_boots += 1;
    hist.records.insert(0, BootRecord { started_at: Utc::now(), stopped_at: None, crashed: false });
    hist.records.truncate(MAX_RECORDS);
    save(data_dir, id, &hist).await;
}

/// Called from the exit-handling branch in `process::spawn_watcher`. Closes
/// the most recent open record (if any - defensive, in case history was
/// cleared or the file didn't exist yet mid-session) and accumulates
/// uptime/crash totals.
pub async fn record_boot_end(data_dir: &Path, id: Uuid, crashed: bool) {
    let mut hist = load(data_dir, id).await;
    let now = Utc::now();
    if let Some(rec) = hist.records.iter_mut().find(|r| r.stopped_at.is_none()) {
        rec.stopped_at = Some(now);
        rec.crashed = crashed;
        let duration = (now - rec.started_at).num_seconds().max(0) as u64;
        hist.total_uptime_secs += duration;
    }
    if crashed {
        hist.total_crashes += 1;
    }
    save(data_dir, id, &hist).await;
}
