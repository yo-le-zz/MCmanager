//! Reprise des serveurs Minecraft après un arrêt ou une mise à jour de MCManager.
//!
//! - Un **manifeste** (`running-servers.json`) liste les serveurs voulus « en marche » ; il est tenu à jour à chaque
//!   démarrage et à chaque arrêt voulu, donc il survit aussi à un plantage ou à une coupure de courant.
//! - À l'**arrêt** de MCManager (Ctrl+C, SIGTERM, mise à jour), les serveurs reçoivent un `stop` propre (le monde est
//!   sauvegardé) ; le manifeste n'est pas modifié par cet arrêt.
//! - Au **démarrage**, les serveurs du manifeste sont relancés l'un après l'autre (réglage « Relancer les serveurs au
//!   démarrage », activé par défaut).
//! - Fermer l'interface web n'a aucun effet sur les serveurs : ce sont des processus de MCManager, pas de la page.
//! - Après une mise à jour, `restart_process` arrête proprement les serveurs, relance MCManager lui-même, et la
//!   reprise les remet en marche.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

/// Pause entre deux démarrages (évite un pic de mémoire et de CPU quand beaucoup de serveurs reprennent).
pub const START_STAGGER: Duration = Duration::from_secs(3);
/// Attente maximale d'un arrêt propre avant de forcer.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    servers: Vec<Uuid>,
}

pub fn manifest_path(data_dir: &Path) -> PathBuf {
    data_dir.join("running-servers.json")
}

pub async fn load(data_dir: &Path) -> Vec<Uuid> {
    match tokio::fs::read_to_string(manifest_path(data_dir)).await {
        Ok(s) => serde_json::from_str::<Manifest>(&s).map(|m| m.servers).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save(data_dir: &Path, ids: &[Uuid]) {
    let body = serde_json::to_string_pretty(&Manifest { servers: ids.to_vec() }).unwrap_or_default();
    let tmp = data_dir.join("running-servers.json.tmp");
    // Écriture atomique : un arrêt brutal pendant l'écriture ne doit pas laisser un manifeste tronqué.
    if tokio::fs::write(&tmp, body).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, manifest_path(data_dir)).await;
    }
}

/// Le serveur doit être relancé au prochain démarrage de MCManager.
pub async fn add(state: &AppState, id: Uuid) {
    if state.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    let mut ids = load(&state.data_dir).await;
    if !ids.contains(&id) {
        ids.push(id);
        save(&state.data_dir, &ids).await;
    }
}

/// Arrêt voulu (ou plantage sans redémarrage automatique) : on ne le relancera pas. Sans effet pendant l'arrêt de MCManager.
pub async fn remove(state: &AppState, id: Uuid) {
    if state.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    let mut ids = load(&state.data_dir).await;
    if let Some(pos) = ids.iter().position(|i| *i == id) {
        ids.remove(pos);
        save(&state.data_dir, &ids).await;
    }
}

/// Serveurs à relancer : ceux du manifeste qui existent encore et ne tournent pas déjà, sans doublon, dans l'ordre.
pub fn plan(manifest: &[Uuid], existing: &HashSet<Uuid>, running: &HashSet<Uuid>) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    manifest.iter().copied().filter(|id| existing.contains(id) && !running.contains(id) && seen.insert(*id)).collect()
}

/// Relance les serveurs du manifeste avec la fonction donnée (injectable pour les tests). Renvoie les identifiants démarrés.
pub async fn resume_with<F, Fut>(state: &AppState, stagger: Duration, start: F) -> Vec<Uuid>
where
    F: Fn(Uuid) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    if !state.config.read().await.resume_servers {
        return Vec::new();
    }
    let manifest = load(&state.data_dir).await;
    let existing: HashSet<Uuid> = state.servers.read().await.keys().copied().collect();
    let running: HashSet<Uuid> = state.runtime.read().await.iter().filter(|(_, r)| r.running).map(|(i, _)| *i).collect();
    let mut started = Vec::new();
    for (n, id) in plan(&manifest, &existing, &running).into_iter().enumerate() {
        if n > 0 {
            tokio::time::sleep(stagger).await;
        }
        let name = state.servers.read().await.get(&id).map(|e| e.name.clone()).unwrap_or_default();
        match start(id).await {
            Ok(()) => {
                tracing::info!("Reprise du serveur « {name} » (il tournait avant l'arrêt de MCManager)");
                started.push(id);
            }
            Err(e) => tracing::warn!("Reprise de « {name} » impossible : {e}"),
        }
    }
    started
}

/// Lance la reprise en arrière-plan (au démarrage, une fois l'interface prête).
pub fn spawn_resume(state: AppState) {
    tokio::spawn(async move {
        // Laisse le temps au serveur web de s'ouvrir et au reste de s'initialiser.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let st = state.clone();
        resume_with(&state, START_STAGGER, move |id| {
            let st = st.clone();
            async move { crate::process::start_server(&st, id).await }
        })
        .await;
    });
}

/// Arrête proprement tous les serveurs en marche (`stop`), attend jusqu'à `timeout`, force les récalcitrants.
/// Le manifeste n'est pas touché : ces serveurs reprendront au prochain démarrage.
pub async fn graceful_shutdown(state: &AppState, timeout: Duration) {
    state.shutting_down.store(true, Ordering::SeqCst);
    let ids: Vec<Uuid> = state.runtime.read().await.iter().filter(|(_, r)| r.running).map(|(i, _)| *i).collect();
    if ids.is_empty() {
        return;
    }
    tracing::info!("Arrêt propre de {} serveur(s)…", ids.len());
    for id in &ids {
        let _ = crate::process::stop_server(state, *id, false).await;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let still: Vec<Uuid> = state.runtime.read().await.iter().filter(|(i, r)| r.running && ids.contains(i)).map(|(i, _)| *i).collect();
        if still.is_empty() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("{} serveur(s) ne se sont pas arrêtés en {} s : arrêt forcé", still.len(), timeout.as_secs());
            for id in still {
                let _ = crate::process::stop_server(state, id, true).await;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Lancé par l'ancien processus : le nouveau attend sa disparition avant de prendre le verrou d'instance.
pub const WAIT_PID_ENV: &str = "MCMANAGER_RESTART_WAIT_PID";

/// À appeler tôt au démarrage : si un redémarrage a été demandé, attend la fin de l'ancien processus (60 s au plus).
pub fn wait_for_previous_instance() {
    let Some(pid) = std::env::var(WAIT_PID_ENV).ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        let mut sys = sysinfo::System::new();
        sys.refresh_process(sysinfo::Pid::from_u32(pid));
        if sys.process(sysinfo::Pid::from_u32(pid)).is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Géré par un superviseur (systemd, Docker) : on le laisse relancer le processus plutôt que de le relancer nous-mêmes.
pub fn managed_by_supervisor() -> bool {
    std::env::var_os("INVOCATION_ID").is_some() || Path::new("/.dockerenv").exists()
}

/// Code de sortie « relancez-moi » (EX_TEMPFAIL) : `Restart=on-failure` (systemd) et `restart: unless-stopped` (Docker) le relancent.
pub const RESTART_EXIT_CODE: i32 = 75;

/// Arrête proprement les serveurs, libère le verrou, relance MCManager (mêmes arguments) puis quitte.
/// À utiliser après une mise à jour. Ne revient pas.
pub async fn restart_process(state: &AppState) -> ! {
    graceful_shutdown(state, STOP_TIMEOUT).await;
    let _ = tokio::fs::remove_file(state.data_dir.join("mcmanager.lock")).await;
    if managed_by_supervisor() {
        tracing::info!("Redémarrage demandé : sortie avec le code {RESTART_EXIT_CODE}, le superviseur relance MCManager");
        std::process::exit(RESTART_EXIT_CODE);
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mcmanager"));
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args().skip(1))
        .env(WAIT_PID_ENV, std::process::id().to_string())
        .env("MCMANAGER_NO_BROWSER", "1") // l'onglet est déjà ouvert : il se recharge tout seul
        .stdin(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // nouveau groupe : n'hérite pas du Ctrl+C du terminal
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0200 | 0x0000_0008); // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
    }
    match cmd.spawn() {
        Ok(_) => tracing::info!("Nouvelle instance lancée, arrêt de l'ancienne"),
        Err(e) => tracing::error!("Impossible de relancer MCManager : {e} — relancez-le manuellement"),
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-resume-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::new_v4()).collect()
    }

    #[test]
    fn plan_keeps_order_and_drops_missing_running_and_duplicates() {
        let v = ids(4);
        let existing: HashSet<Uuid> = [v[0], v[1], v[2]].into_iter().collect();
        let running: HashSet<Uuid> = [v[1]].into_iter().collect();
        let manifest = vec![v[2], v[0], v[1], v[3], v[2]];
        assert_eq!(plan(&manifest, &existing, &running), vec![v[2], v[0]]);
        assert!(plan(&[], &existing, &running).is_empty());
    }

    #[tokio::test]
    async fn manifest_follows_start_and_stop_but_not_the_shutdown() {
        let d = tmp();
        let state = crate::state::build_state_for_test(&d).await;
        let v = ids(2);
        assert!(load(&d).await.is_empty());
        add(&state, v[0]).await;
        add(&state, v[1]).await;
        add(&state, v[0]).await; // pas de doublon
        assert_eq!(load(&d).await, v);
        remove(&state, v[0]).await;
        assert_eq!(load(&d).await, vec![v[1]]);
        // Pendant l'arrêt de MCManager, les arrêts de serveurs ne retirent rien : ils reprendront au prochain démarrage.
        state.shutting_down.store(true, Ordering::SeqCst);
        remove(&state, v[1]).await;
        add(&state, v[0]).await;
        assert_eq!(load(&d).await, vec![v[1]]);
        assert!(!d.join("running-servers.json.tmp").exists());
        // Un manifeste corrompu n'empêche pas de démarrer.
        std::fs::write(manifest_path(&d), "{ pas du json").unwrap();
        assert!(load(&d).await.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn resume_starts_manifest_servers_in_order_and_honours_the_setting() {
        let d = tmp();
        let state = crate::state::build_state_for_test(&d).await;
        let v = ids(3);
        for (i, id) in v.iter().enumerate() {
            state.servers.write().await.insert(*id, crate::models::ServerEntry::test_entry(*id, &format!("S{i}"), &d.join(format!("s{i}"))));
        }
        for id in &v[..2] {
            add(&state, *id).await;
        }
        let order = Arc::new(Mutex::new(Vec::new()));
        let o = order.clone();
        let started = resume_with(&state, Duration::from_millis(1), move |id| {
            let o = o.clone();
            async move {
                o.lock().unwrap().push(id);
                if o.lock().unwrap().len() == 1 { Ok(()) } else { Err(anyhow::anyhow!("jar manquant")) }
            }
        })
        .await;
        assert_eq!(*order.lock().unwrap(), vec![v[0], v[1]], "ordre du manifeste, une erreur n'arrête pas les suivants");
        assert_eq!(started, vec![v[0]]);
        // Réglage désactivé : rien n'est relancé.
        state.config.write().await.resume_servers = false;
        let none = resume_with(&state, Duration::from_millis(1), |_| async { Ok(()) }).await;
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn graceful_shutdown_sends_stop_waits_and_keeps_the_manifest() {
        use std::process::Stdio;
        let d = tmp();
        let state = crate::state::build_state_for_test(&d).await;
        let (ok_id, stuck_id) = (Uuid::new_v4(), Uuid::new_v4());
        // « Serveur » obéissant : sort dès qu'il lit `stop`. « Serveur » bloqué : ignore stdin.
        for (id, script) in [(ok_id, "read l; [ \"$l\" = stop ] && exit 0"), (stuck_id, "while true; do sleep 1; done")] {
            let mut child = tokio::process::Command::new("sh").arg("-c").arg(script).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
            let mut rt = crate::state::ServerRuntime::default();
            rt.stdin = child.stdin.take();
            rt.pid = child.id();
            rt.child = Some(child);
            rt.running = true;
            state.runtime.write().await.insert(id, rt);
            state.servers.write().await.insert(id, crate::models::ServerEntry::test_entry(id, "S", &d));
            add(&state, id).await;
            // Mini « watcher » : constate la sortie comme le fait process.rs.
            let st = state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let mut rts = st.runtime.write().await;
                    let rt = rts.get_mut(&id).unwrap();
                    if rt.child.as_mut().unwrap().try_wait().ok().flatten().is_some() {
                        rt.running = false;
                        break;
                    }
                }
            });
        }
        let t0 = std::time::Instant::now();
        graceful_shutdown(&state, Duration::from_secs(2)).await;
        assert!(t0.elapsed() < Duration::from_secs(6));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(state.runtime.read().await.values().all(|r| !r.running), "le serveur obéissant s'arrête, le bloqué est forcé");
        assert!(state.shutting_down.load(Ordering::SeqCst));
        let m = load(&d).await;
        assert!(m.contains(&ok_id) && m.contains(&stuck_id), "le manifeste est intact : ils reprendront au prochain démarrage");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restart_helpers() {
        std::env::remove_var(WAIT_PID_ENV);
        wait_for_previous_instance(); // sans variable : retour immédiat
        assert_eq!(RESTART_EXIT_CODE, 75);
    }
}
