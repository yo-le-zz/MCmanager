use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

use crate::state::{AppState, ServerRuntime};
use crate::{files, process, stats};

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticStepResult {
    pub addon: String,
    pub status: String, // "ok" | "crash" | "timeout"
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    /// Whether the server's current addon set (as found, before any
    /// disabling) boots successfully. When true, nothing else runs - see
    /// the doc comment on `run_crash_diagnostic`.
    pub full_set_ok: bool,
    pub baseline_ok: bool,
    pub steps: Vec<DiagnosticStepResult>,
    pub culprits: Vec<String>,
    /// Set when no single addon crashes on its own, but a specific one
    /// reliably triggers a crash once added on top of the others already
    /// confirmed working - i.e. a suspected conflict *between* addons
    /// rather than a single broken one. `None` if phase 2 wasn't needed or
    /// didn't reproduce anything.
    pub combo_suspect: Option<String>,
    pub summary: String,
}

/// Automated crash triage ("mode debug"). Three phases, each one only run
/// if the previous one didn't already answer the question:
///
/// 0. **Sanity check**: boot the server exactly as it currently is. If that
///    works, there's nothing to diagnose - report it and stop, without
///    touching any addon's enabled/disabled state at all.
/// 1. **Isolation**: disable every mod/plugin, confirm the server boots
///    bare, then re-enable each one individually (everyone else staying
///    disabled) to find one that crashes the server *on its own*.
/// 2. **Combination**: if no single addon is at fault but the full set
///    still doesn't boot, re-enable addons cumulatively one at a time (in
///    their original order) until the boot fails - the one whose addition
///    triggered the failure is a suspected conflict with what's already
///    enabled, rather than a solo crash.
///
/// Always restores everybody's original enabled/disabled state before
/// returning, whichever phase it stopped at. Progress streams line-by-line
/// to the server's normal console (same WebSocket the Console tab uses).
pub async fn run_crash_diagnostic(state: AppState, id: Uuid, timeout_secs: u64) -> Result<DiagnosticReport> {
    let entry = {
        let servers = state.servers.read().await;
        servers.get(&id).cloned().context("serveur introuvable")?
    };
    {
        let runtime = state.runtime.read().await;
        if runtime.get(&id).map(|rt| rt.running).unwrap_or(false) {
            anyhow::bail!("arretez le serveur avant de lancer le diagnostic");
        }
    }

    let folder = PathBuf::from(&entry.folder);
    let addon_dir = entry.loader.addon_dir();
    let addons = files::list_addons(&folder, entry.loader)?;
    let original_enabled: Vec<(String, bool)> = addons.iter().map(|a| (a.file_name.clone(), a.enabled)).collect();
    let dir = folder.join(addon_dir);

    // A crashing addon with auto_restart on would otherwise fight this
    // function in an infinite restart loop using the exact broken addon
    // set we're trying to isolate. Neutralize it for the duration.
    let original_auto_restart = entry.auto_restart;
    if original_auto_restart {
        if let Some(e) = state.servers.write().await.get_mut(&id) {
            e.auto_restart = false;
        }
    }

    log_line(&state, id, "[Debug] Diagnostic de crash demarre.".to_string()).await;

    // ── Phase 0 : le serveur demarre-t-il seulement, tel quel ? ──
    log_line(&state, id, "[Debug] Verification avec la configuration actuelle...".to_string()).await;
    let (full_set_ok, full_set_detail) = try_boot(&state, id, timeout_secs).await;
    let mut steps = vec![DiagnosticStepResult {
        addon: "(configuration actuelle)".to_string(),
        status: status_str(full_set_ok, &full_set_detail),
        detail: full_set_detail.clone(),
    }];

    if full_set_ok {
        let summary = "Le serveur demarre normalement avec sa configuration actuelle - rien a diagnostiquer.".to_string();
        log_line(&state, id, format!("[Debug] {summary}")).await;
        if original_auto_restart {
            if let Some(e) = state.servers.write().await.get_mut(&id) {
                e.auto_restart = true;
            }
        }
        return Ok(DiagnosticReport { full_set_ok: true, baseline_ok: true, steps, culprits: vec![], combo_suspect: None, summary });
    }
    log_line(&state, id, format!("[Debug] Ca ne demarre pas ({full_set_detail}). Isolation en cours...")).await;

    for a in &addons {
        if a.enabled {
            files::toggle_addon(&folder, entry.loader, &a.file_name).ok();
        }
    }

    let mut culprits = Vec::new();
    let mut combo_suspect = None;

    log_line(&state, id, "[Debug] Test de base (aucun mod/plugin actif)...".to_string()).await;
    let (baseline_ok, baseline_detail) = try_boot(&state, id, timeout_secs).await;
    steps.push(DiagnosticStepResult {
        addon: "(aucun)".to_string(),
        status: status_str(baseline_ok, &baseline_detail),
        detail: baseline_detail.clone(),
    });

    let names: Vec<String> = original_enabled.iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(n, _)| n.trim_end_matches(".disabled").to_string())
        .collect();

    if !baseline_ok {
        log_line(&state, id, format!(
            "[Debug] Le serveur ne demarre pas meme sans mods/plugins ({baseline_detail}). \
             Le probleme n'est probablement pas lie a un addon - verifiez la version de Java, \
             la RAM allouee (Xms/Xmx) et le fichier jar du serveur."
        )).await;
    } else {
        log_line(&state, id, "[Debug] Base OK. Test des mods/plugins un par un...".to_string()).await;
        let total = names.len();

        for (i, name) in names.iter().enumerate() {
            log_line(&state, id, format!("[Debug] ({}/{total}) Test individuel de {name}...", i + 1)).await;

            let disabled_name = format!("{name}.disabled");
            if dir.join(&disabled_name).exists() {
                files::toggle_addon(&folder, entry.loader, &disabled_name).ok();
            }

            let (ok, detail) = try_boot(&state, id, timeout_secs).await;
            steps.push(DiagnosticStepResult { addon: name.clone(), status: status_str(ok, &detail), detail: detail.clone() });

            if ok {
                log_line(&state, id, format!("[Debug] {name} : OK.")).await;
            } else {
                culprits.push(name.clone());
                log_line(&state, id, format!("[Debug] \u{26A0} {name} semble provoquer un crash a lui seul ({detail}).")).await;
            }

            // Disable it again before testing the next one, regardless of
            // outcome, so each individual test stays isolated.
            if dir.join(name).exists() {
                files::toggle_addon(&folder, entry.loader, name).ok();
            }
        }

        // ── Phase 2 : pas de coupable solo, mais l'ensemble complet
        // plante quand meme -> chercher une combinaison en cumulant. ──
        if culprits.is_empty() {
            log_line(&state, id, "[Debug] Aucun coupable individuel. Recherche d'une combinaison problematique...".to_string()).await;
            let total = names.len();
            for (i, name) in names.iter().enumerate() {
                log_line(&state, id, format!("[Debug] ({}/{total}) Ajout cumulatif de {name}...", i + 1)).await;
                let disabled_name = format!("{name}.disabled");
                if dir.join(&disabled_name).exists() {
                    files::toggle_addon(&folder, entry.loader, &disabled_name).ok();
                }
                let (ok, detail) = try_boot(&state, id, timeout_secs).await;
                let already_enabled: Vec<&str> = names[..i].iter().map(|s| s.as_str()).collect();
                steps.push(DiagnosticStepResult {
                    addon: format!("{name} (+ {})", if already_enabled.is_empty() { "rien d'autre".to_string() } else { already_enabled.join(", ") }),
                    status: status_str(ok, &detail),
                    detail: detail.clone(),
                });
                if !ok {
                    combo_suspect = Some(name.clone());
                    log_line(&state, id, format!(
                        "[Debug] \u{26A0} Le crash apparait des l'ajout de {name} en presence de : {} ({detail}).",
                        if already_enabled.is_empty() { "aucun autre addon".to_string() } else { already_enabled.join(", ") }
                    )).await;
                    break;
                }
                log_line(&state, id, format!("[Debug] {name} : OK avec les precedents.")).await;
            }
            if combo_suspect.is_none() {
                log_line(&state, id, "[Debug] Reactivation complete sans reproduire le crash - probleme peut-etre intermittent.".to_string()).await;
            }
        }
    }

    // Restore everyone's original enabled/disabled state.
    for (name, was_enabled) in &original_enabled {
        let base = name.trim_end_matches(".disabled");
        let currently_enabled = dir.join(base).exists();
        if currently_enabled != *was_enabled {
            let current_name = if currently_enabled { base.to_string() } else { format!("{base}.disabled") };
            if dir.join(&current_name).exists() {
                files::toggle_addon(&folder, entry.loader, &current_name).ok();
            }
        }
    }
    if original_auto_restart {
        if let Some(e) = state.servers.write().await.get_mut(&id) {
            e.auto_restart = true;
        }
    }

    let summary = if !baseline_ok {
        format!("Le serveur ne demarre pas meme sans aucun mod/plugin - le probleme est ailleurs (Java, RAM, jar...). Detail : {baseline_detail}")
    } else if !culprits.is_empty() {
        format!("Mod(s)/plugin(s) suspect(s) (plantent seuls) : {}", culprits.join(", "))
    } else if let Some(combo) = &combo_suspect {
        format!("Aucun addon ne plante seul, mais {combo} semble entrer en conflit avec un ou plusieurs autres addons deja actifs (voir le detail des etapes).")
    } else {
        "Aucun addon ne plante seul, et la reactivation progressive de tous ne reproduit pas le crash observe au depart - le probleme est peut-etre intermittent (timing, ressources, port deja utilise) plutot que lie a un addon precis.".to_string()
    };
    log_line(&state, id, format!("[Debug] Diagnostic termine. {summary}")).await;

    Ok(DiagnosticReport { full_set_ok, baseline_ok, steps, culprits, combo_suspect, summary })
}

fn status_str(ok: bool, detail: &str) -> String {
    if ok {
        "ok".to_string()
    } else if detail.starts_with("timeout") {
        "timeout".to_string()
    } else {
        "crash".to_string()
    }
}

async fn log_line(state: &AppState, id: Uuid, line: String) {
    let mut runtime = state.runtime.write().await;
    let rt = runtime.entry(id).or_insert_with(ServerRuntime::default);
    rt.push_line(line);
}

/// Starts the server and waits up to `timeout_secs` for either a crash
/// (process exits on its own) or a successful boot (the status port
/// responds to a ping). Always leaves the server stopped afterwards so the
/// next test in the sequence starts from a clean slate.
async fn try_boot(state: &AppState, id: Uuid, timeout_secs: u64) -> (bool, String) {
    if let Err(e) = process::start_server(state, id).await {
        return (false, format!("echec du lancement : {e}"));
    }
    let port = state.servers.read().await.get(&id).map(|e| e.port).unwrap_or(25565);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let result = loop {
        if tokio::time::Instant::now() >= deadline {
            break (false, "timeout - le serveur n'a jamais repondu (bloque, ou juste lent a demarrer)".to_string());
        }
        let running = state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false);
        if !running {
            break (false, "le processus s'est arrete / a crashe".to_string());
        }
        if let Ok((Some(_), _, _)) = stats::ping_server(port).await {
            break (true, "demarre avec succes".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    };

    let still_running = state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false);
    if still_running {
        let _ = process::stop_server(state, id, false).await;
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false) {
                break;
            }
        }
        if state.runtime.read().await.get(&id).map(|rt| rt.running).unwrap_or(false) {
            let _ = process::stop_server(state, id, true).await; // graceful stop hung - force kill
        }
    }

    result
}
