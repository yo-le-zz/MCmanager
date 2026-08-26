//! `mcmanager-headless` - the CLI-only counterpart to the `mcmanager` web
//! app, for people who want to run MCManager on a remote server (a VPS, a
//! home Ubuntu box over SSH...) without ever binding a web port or needing
//! a browser at all. It reuses the exact same core (`mcmanager::*`) - same
//! `AppState`, same on-disk format under the same data directory - so
//! servers created here show up in the web UI (on another machine, or run
//! later on this one) and vice versa. Just not *both at once*: see
//! `mcmanager::state::acquire_instance_lock`.
//!
//! Runs as a persistent interactive shell (reads commands from stdin, one
//! per line) rather than a series of one-shot invocations, because server
//! process supervision lives in memory for the life of this process - a
//! `start` followed by exiting immediately would leave nobody watching the
//! child's console output or crash/restart state. Type `help` once running.
//!
//! Usage:
//!   mcmanager-headless                 interactive shell (default)
//!   mcmanager-headless --script FILE    run commands from a file, one per line, then exit

use std::io::Write as _;
use std::path::PathBuf;

use mcmanager::models::Loader;
use mcmanager::remote::{RemoteIdentity, RemoteRuntime};
use mcmanager::{debug, downloader, modrinth, process, remote, state, updater};
use uuid::Uuid;

/// Everything the REPL needs for the remote-control feature, held across
/// the whole session (not per-command) since pairing/session state and the
/// background server task both need to outlive a single line of input.
struct RemoteRepl {
    identity: RemoteIdentity,
    runtime: Option<RemoteRuntime>,
    server_task: Option<tokio::task::JoinHandle<()>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();

    println!("MCManager v{} - mode CLI (aucun serveur web ne sera lance)", updater::CURRENT_VERSION);
    let app_state = match state::build_state().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Erreur au demarrage : {e}");
            std::process::exit(1);
        }
    };
    println!("Dossier de donnees : {}", app_state.data_dir.display());
    println!("Tapez 'help' pour la liste des commandes, 'quit' pour quitter.\n");

    let identity = match remote::load_or_create_identity(&app_state.data_dir).await {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Avertissement : impossible de charger/creer l'identite de controle a distance : {e}");
            eprintln!("(les commandes 'remote' ne fonctionneront pas cette session)");
            // Still usable for everything else - remote control is opt-in,
            // its identity failing to load shouldn't block plain server management.
            RemoteIdentity::ephemeral()
        }
    };
    let mut remote_repl = RemoteRepl { identity, runtime: None, server_task: None };

    // Auto-start: servers the user has flagged (via `autostart add <id>`)
    // to launch as soon as the daemon comes up - e.g. after a reboot, so a
    // systemd unit with this binary can bring a server back without anyone
    // needing to type `start` by hand.
    let autostart_ids = load_autostart(&app_state.data_dir).await;
    for id in &autostart_ids {
        println!("Demarrage automatique de {id}...");
        if let Err(e) = process::start_server(&app_state, *id).await {
            eprintln!("  echec : {e}");
        }
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let lock_data_dir = app_state.data_dir.clone();

    let result = if let Some(pos) = args.iter().position(|a| a == "--script") {
        let Some(path) = args.get(pos + 1) else {
            eprintln!("--script necessite un chemin de fichier");
            std::process::exit(1);
        };
        let script_result = run_script(&app_state, &mut remote_repl, path).await;
        // --script normally exits once the file is done (documented,
        // scripting/automation use case). --daemon combined with it means
        // "run this setup (e.g. `remote enable`, `start <id>`), then stay
        // up" - the systemd unit uses exactly this combination.
        if script_result.is_ok() && args.iter().any(|a| a == "--daemon") {
            run_daemon(&app_state.data_dir).await
        } else {
            script_result
        }
    } else if args.iter().any(|a| a == "--daemon") {
        run_daemon(&app_state.data_dir).await
    } else {
        run_interactive(&app_state, &mut remote_repl).await
    };

    if let Some(task) = remote_repl.server_task.take() {
        task.abort();
    }
    let _ = tokio::fs::remove_file(lock_data_dir.join("mcmanager.lock")).await;
    result
}

/// For running under systemd (or any supervisor) with no attached
/// terminal: `run_interactive()`'s stdin-read loop would see an immediate
/// EOF on `/dev/null` and exit right away, which is wrong for a service
/// that's meant to stay up. This just waits for a shutdown signal instead
/// - the actual work (auto-started servers, `remote enable`d control) is
/// already running in background tasks by the time this is called.
async fn run_daemon(data_dir: &std::path::Path) -> anyhow::Result<()> {
    println!("Mode daemon - en attente (Ctrl+C ou SIGTERM pour arreter proprement).");
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
    println!("Arret demande, fermeture propre...");
    let _ = data_dir; // lock cleanup happens in the caller, same as every other exit path
    Ok(())
}

async fn run_interactive(state: &state::AppState, remote_repl: &mut RemoteRepl) -> anyhow::Result<()> {
    loop {
        print!("mcmanager> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        let n = std::io::stdin().read_line(&mut line).unwrap_or(0);
        if n == 0 {
            println!("Fin de l'entree, fermeture.");
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "quit" | "exit") {
            break;
        }
        if let Err(e) = dispatch(state, remote_repl, line).await {
            println!("Erreur : {e}");
        }
    }
    Ok(())
}

async fn run_script(state: &state::AppState, remote_repl: &mut RemoteRepl, path: &str) -> anyhow::Result<()> {
    let content = tokio::fs::read_to_string(path).await?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        println!("$ {line}");
        if let Err(e) = dispatch(state, remote_repl, line).await {
            println!("Erreur : {e}");
        }
    }
    Ok(())
}

async fn dispatch(state: &state::AppState, remote_repl: &mut RemoteRepl, line: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.as_slice() {
        ["help"] => { print_help(); Ok(()) }
        ["list"] => cmd_list(state).await,
        ["status", id] => cmd_status(state, id).await,
        ["start", id] => cmd_start(state, id).await,
        ["stop", id] => cmd_stop(state, id, false).await,
        ["stop", id, "--force"] => cmd_stop(state, id, true).await,
        ["restart", id] => { cmd_stop(state, id, false).await.ok(); tokio::time::sleep(std::time::Duration::from_secs(3)).await; cmd_start(state, id).await }
        ["logs", id] => cmd_logs(state, id, 40).await,
        ["logs", id, n] => cmd_logs(state, id, n.parse().unwrap_or(40)).await,
        ["send", id, rest @ ..] if !rest.is_empty() => process::send_command(state, parse_id(id)?, &rest.join(" ")).await,
        ["install", id, project] => cmd_install(state, id, project).await,
        ["debug", id] => cmd_debug(state, id).await,
        ["managed-sync", id] => cmd_managed_sync(state, id).await,
        ["create", rest @ ..] => cmd_create(state, rest).await,
        ["autostart", "add", id] => cmd_autostart_add(state, id).await,
        ["autostart", "remove", id] => cmd_autostart_remove(state, id).await,
        ["autostart", "list"] => cmd_autostart_list(state).await,
        ["remote", "fingerprint"] => { println!("Empreinte de cette instance : {}", remote_repl.identity.fingerprint()); Ok(()) }
        ["remote", "enable"] => cmd_remote_enable(state, remote_repl, 7778).await,
        ["remote", "enable", port] => cmd_remote_enable(state, remote_repl, port.parse().unwrap_or(7778)).await,
        ["remote", "disable"] => { if let Some(t) = remote_repl.server_task.take() { t.abort(); remote_repl.runtime = None; println!("Controle a distance desactive."); } else { println!("N'etait pas actif."); } Ok(()) }
        ["remote", "pairing-code"] => cmd_remote_pairing_code(remote_repl).await,
        ["remote", "clients"] => cmd_remote_clients(state).await,
        ["remote", "revoke", client_id] => cmd_remote_revoke(state, client_id).await,
        ["remote", "pair", host, label] => cmd_remote_pair(state, remote_repl, host, label).await,
        ["remote", "targets"] => cmd_remote_targets(state).await,
        ["remote", "list", label] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"list"})).await,
        ["remote", "status", label, id] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"status","server_id":id})).await,
        ["remote", "start", label, id] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"start","server_id":id})).await,
        ["remote", "stop", label, id] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"stop","server_id":id})).await,
        ["remote", "restart", label, id] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"restart","server_id":id})).await,
        ["remote", "logs", label, id] => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"logs","server_id":id})).await,
        ["remote", "send", label, id, cmd_rest @ ..] if !cmd_rest.is_empty() => cmd_remote_call(state, remote_repl, label, serde_json::json!({"action":"send","server_id":id,"command":cmd_rest.join(" ")})).await,
        _ => { println!("Commande inconnue : {line}\nTapez 'help' pour la liste des commandes."); Ok(()) }
    }
}

fn print_help() {
    println!(r#"Commandes disponibles :
  list                          liste les serveurs enregistres
  status <id>                   CPU / RAM / joueurs en ligne
  start <id>                    demarre un serveur
  stop <id> [--force]           arrete (proprement, ou --force pour tuer le processus)
  restart <id>                  arrete puis redemarre
  logs <id> [n]                 affiche les n dernieres lignes de la console (defaut 40)
  send <id> <commande...>       envoie une commande a la console du serveur (ex: send <id> say bonjour)
  install <id> <slug|id>        installe un mod/plugin Modrinth (derniere version compatible)
  managed-sync <id>             synchronise les mods/plugins "geres" de ce serveur
  debug <id>                    diagnostic automatique de crash (teste les mods/plugins un par un)
  create --name N --loader L --version V [--port P]
                                 cree un nouveau serveur (loader: vanilla|paper|purpur|spigot|fabric|quilt|forge|neoforge)
  autostart add|remove|list <id>
                                 serveurs a demarrer automatiquement quand ce daemon se lance (ex: apres un reboot)
  remote fingerprint            empreinte de l'identite RSA de cette instance
  remote enable [port]          active le controle a distance (0.0.0.0, port 7778 par defaut)
  remote disable                desactive le controle a distance
  remote pairing-code           genere un code de jumelage a usage unique (valide 10 min)
  remote clients                liste les clients autorises a se connecter a cette instance
  remote revoke <client_id>     revoque un client
  remote pair <host:port> <nom> jumelage avec une AUTRE instance pour la piloter (demande le code affiche la-bas)
  remote targets                liste les instances distantes jumelees
  remote list|status|start|stop|restart|logs <nom> [id]
                                 pilote une instance distante deja jumelee
  remote send <nom> <id> <commande...>
                                 envoie une commande console a un serveur distant
  help                          cette aide
  quit / exit                   quitter (Ctrl+D fonctionne aussi)
"#);
}

fn parse_id(s: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(s).map_err(|_| anyhow::anyhow!("id invalide : {s}"))
}

async fn cmd_list(state: &state::AppState) -> anyhow::Result<()> {
    let servers = state.servers.read().await;
    if servers.is_empty() {
        println!("Aucun serveur enregistre.");
        return Ok(());
    }
    let runtime = state.runtime.read().await;
    println!("{:<38} {:<22} {:<10} {:<8} {:<8}", "ID", "NOM", "LOADER", "VERSION", "ETAT");
    for (id, s) in servers.iter() {
        let running = runtime.get(id).map(|r| r.running).unwrap_or(false);
        println!("{:<38} {:<22} {:<10} {:<8} {:<8}", id, s.name, s.loader.as_str(), s.mc_version, if running { "actif" } else { "arrete" });
    }
    Ok(())
}

async fn cmd_status(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    let entry = state.servers.read().await.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    let runtime = state.runtime.read().await;
    let rt = runtime.get(&id);
    let running = rt.map(|r| r.running).unwrap_or(false);
    println!("Nom          : {}", entry.name);
    println!("En ligne     : {}", running);
    if running {
        if let Some(pid) = rt.and_then(|r| r.pid) {
            let (cpu, mem) = mcmanager::stats::process_stats(pid);
            println!("CPU          : {cpu:.1}%");
            println!("RAM          : {mem:.0} Mo");
        }
        if let Ok((online, max, _)) = mcmanager::stats::ping_server(entry.port).await {
            if let Some(online) = online {
                println!("Joueurs      : {online}/{}", max.unwrap_or(0));
            }
        }
    }
    Ok(())
}

async fn cmd_start(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    process::start_server(state, parse_id(id)?).await?;
    println!("Demarrage lance.");
    Ok(())
}

async fn cmd_stop(state: &state::AppState, id: &str, force: bool) -> anyhow::Result<()> {
    process::stop_server(state, parse_id(id)?, force).await?;
    println!("Arret demande.");
    Ok(())
}

async fn cmd_logs(state: &state::AppState, id: &str, n: usize) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    let runtime = state.runtime.read().await;
    let Some(rt) = runtime.get(&id) else {
        println!("(aucune sortie - le serveur n'a jamais ete demarre dans cette session)");
        return Ok(());
    };
    let start = rt.backlog.len().saturating_sub(n);
    for line in &rt.backlog[start..] {
        println!("{line}");
    }
    Ok(())
}

async fn cmd_install(state: &state::AppState, id: &str, project: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    let entry = state.servers.read().await.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    let version = modrinth::latest_matching_version(&state.http, project, entry.loader.modrinth_loader(), &entry.mc_version).await?
        .ok_or_else(|| anyhow::anyhow!("aucune version compatible trouvee pour ce serveur"))?;
    let dest = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    let filename = modrinth::download_version_file(&state.http, &version, &dest).await?;
    println!("Installe : {filename} ({})", version.version_number);
    Ok(())
}

async fn cmd_debug(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    println!("Diagnostic en cours (peut prendre plusieurs minutes)...");
    let report = debug::run_crash_diagnostic(state.clone(), id, 45).await?;
    for step in &report.steps {
        println!("  [{}] {} - {}", step.status, step.addon, step.detail);
    }
    println!("\n{}", report.summary);
    Ok(())
}

async fn cmd_managed_sync(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    let entry = state.servers.read().await.get(&id).cloned().ok_or_else(|| anyhow::anyhow!("serveur introuvable"))?;
    if entry.managed_addons.is_empty() {
        println!("Aucun mod/plugin gere pour ce serveur (ajoutez-en depuis l'interface web ou en editant servers.json).");
        return Ok(());
    }
    let dir = PathBuf::from(&entry.folder).join(entry.loader.addon_dir());
    tokio::fs::create_dir_all(&dir).await.ok();
    for managed in &entry.managed_addons {
        match modrinth::latest_matching_version(&state.http, &managed.project_id, entry.loader.modrinth_loader(), &entry.mc_version).await {
            Ok(Some(version)) => match modrinth::download_version_file(&state.http, &version, &dir).await {
                Ok(filename) => println!("  {} -> {filename}", managed.label),
                Err(e) => println!("  {} : echec ({e})", managed.label),
            },
            Ok(None) => println!("  {} : aucune version compatible", managed.label),
            Err(e) => println!("  {} : erreur ({e})", managed.label),
        }
    }
    Ok(())
}

async fn cmd_create(state: &state::AppState, args: &[&str]) -> anyhow::Result<()> {
    let mut name = None;
    let mut loader_str = None;
    let mut version = None;
    let mut port: u16 = 25565;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--name" => { name = args.get(i + 1).map(|s| s.to_string()); i += 2; }
            "--loader" => { loader_str = args.get(i + 1).map(|s| s.to_string()); i += 2; }
            "--version" => { version = args.get(i + 1).map(|s| s.to_string()); i += 2; }
            "--port" => { port = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(25565); i += 2; }
            _ => { i += 1; }
        }
    }
    let name = name.ok_or_else(|| anyhow::anyhow!("--name est requis"))?;
    let loader_str = loader_str.ok_or_else(|| anyhow::anyhow!("--loader est requis (vanilla|paper|purpur|spigot|fabric|quilt|forge|neoforge)"))?;
    let mc_version = version.ok_or_else(|| anyhow::anyhow!("--version est requis (ex: 1.21.11)"))?;
    let loader: Loader = serde_json::from_value(serde_json::Value::String(loader_str.clone()))
        .map_err(|_| anyhow::anyhow!("loader inconnu : {loader_str}"))?;

    let id = Uuid::new_v4();
    let folder = state.data_dir.join("servers").join(id.to_string());
    let java_path = state.config.read().await.java_path.clone();

    println!("Creation de '{name}' ({loader_str} {mc_version})... cela peut prendre une minute (telechargement).");
    let result = downloader::setup_server(&state.http, loader, &mc_version, None, &folder, &java_path).await?;

    let entry = mcmanager::models::ServerEntry {
        id, name, loader, mc_version,
        loader_version: result.loader_version,
        folder: folder.to_string_lossy().to_string(),
        jar_name: result.jar_name,
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
        aikar_flags: false,
        managed_addons: vec![],
        dynamic_server: false,
        created_at: chrono::Utc::now(),
    };
    tokio::fs::write(folder.join("eula.txt"), "eula=true\n").await.ok();
    let props_path = folder.join("server.properties");
    if !props_path.exists() {
        tokio::fs::write(&props_path, format!("server-port={}\nmotd=Serveur cree avec MCManager\nonline-mode=true\n", entry.port)).await.ok();
    }
    state.servers.write().await.insert(id, entry);
    state::save_servers(state).await?;
    println!("Cree avec l'ID : {id}");
    Ok(())
}

// ───────────────────────── autostart (config locale) ─────────────────────────

fn autostart_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("headless_autostart.json")
}

async fn load_autostart(data_dir: &std::path::Path) -> Vec<Uuid> {
    match tokio::fs::read_to_string(autostart_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save_autostart(data_dir: &std::path::Path, list: &[Uuid]) -> anyhow::Result<()> {
    tokio::fs::write(autostart_path(data_dir), serde_json::to_string_pretty(list)?).await?;
    Ok(())
}

async fn cmd_autostart_add(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    if !state.servers.read().await.contains_key(&id) {
        anyhow::bail!("serveur introuvable");
    }
    let mut list = load_autostart(&state.data_dir).await;
    if !list.contains(&id) {
        list.push(id);
        save_autostart(&state.data_dir, &list).await?;
    }
    println!("Demarrage automatique active pour {id}.");
    Ok(())
}

async fn cmd_autostart_remove(state: &state::AppState, id: &str) -> anyhow::Result<()> {
    let id = parse_id(id)?;
    let mut list = load_autostart(&state.data_dir).await;
    list.retain(|i| *i != id);
    save_autostart(&state.data_dir, &list).await?;
    println!("Demarrage automatique desactive pour {id}.");
    Ok(())
}

async fn cmd_autostart_list(state: &state::AppState) -> anyhow::Result<()> {
    let list = load_autostart(&state.data_dir).await;
    if list.is_empty() {
        println!("Aucun serveur en demarrage automatique.");
        return Ok(());
    }
    let servers = state.servers.read().await;
    for id in &list {
        let name = servers.get(id).map(|e| e.name.as_str()).unwrap_or("(introuvable)");
        println!("  {id}  {name}");
    }
    Ok(())
}

// ───────────────────────── controle a distance (cote expose) ─────────────────────────

async fn cmd_remote_enable(state: &state::AppState, remote_repl: &mut RemoteRepl, port: u16) -> anyhow::Result<()> {
    if remote_repl.server_task.is_some() {
        println!("Deja actif. Utilisez 'remote disable' d'abord pour changer de port.");
        return Ok(());
    }
    let rt = RemoteRuntime::new(state.clone(), state.data_dir.clone()).await?;
    let rt_for_task = rt.clone();
    let task = tokio::spawn(async move {
        if let Err(e) = remote::serve(rt_for_task, port).await {
            eprintln!("[remote] arrete : {e}");
        }
    });
    println!("Controle a distance active sur le port {port} (0.0.0.0 - toutes interfaces).");
    println!("Empreinte de cette instance : {}", rt.fingerprint());
    println!("Utilisez 'remote pairing-code' pour autoriser une machine a se connecter.");
    remote_repl.runtime = Some(rt);
    remote_repl.server_task = Some(task);
    Ok(())
}

async fn cmd_remote_pairing_code(remote_repl: &RemoteRepl) -> anyhow::Result<()> {
    let Some(rt) = &remote_repl.runtime else {
        anyhow::bail!("le controle a distance n'est pas actif ('remote enable' d'abord)");
    };
    let code = rt.generate_pairing_code().await;
    println!("Code de jumelage (valide 10 minutes, usage unique) : {code}");
    println!("Empreinte de cette instance (a verifier du cote client) : {}", rt.fingerprint());
    println!("Sur la machine qui doit piloter celle-ci : remote pair <cette-machine>:<port> <nom>");
    Ok(())
}

async fn cmd_remote_clients(state: &state::AppState) -> anyhow::Result<()> {
    let clients = remote::load_trusted(&state.data_dir).await;
    if clients.is_empty() {
        println!("Aucun client autorise.");
        return Ok(());
    }
    for c in &clients {
        println!("  {} ({}) - jumele le {}", c.label, c.id, c.paired_at.format("%Y-%m-%d %H:%M"));
    }
    Ok(())
}

async fn cmd_remote_revoke(state: &state::AppState, client_id: &str) -> anyhow::Result<()> {
    if remote::revoke_client(&state.data_dir, client_id).await? {
        println!("Client revoque.");
    } else {
        println!("Aucun client avec cet identifiant.");
    }
    Ok(())
}

// ───────────────────────── controle a distance (cote client) ─────────────────────────

async fn cmd_remote_pair(state: &state::AppState, remote_repl: &RemoteRepl, host: &str, label: &str) -> anyhow::Result<()> {
    println!("Contact de {host}...");
    let (fingerprint, _pubkey) = remote::client_fetch_info(&state.http, host).await?;
    println!("Empreinte annoncee par {host} : {fingerprint}");
    println!("Verifiez qu'elle correspond a celle affichee par 'remote pairing-code' sur cette machine distante.");
    print!("Code de jumelage recu de cette machine : ");
    std::io::stdout().flush().ok();
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    let code = code.trim();
    if code.is_empty() {
        println!("Jumelage annule (aucun code saisi).");
        return Ok(());
    }
    remote::client_pair(&state.http, &state.data_dir, &remote_repl.identity, host, label, code).await?;
    println!("Jumele avec succes sous le nom \"{label}\".");
    Ok(())
}

async fn cmd_remote_targets(state: &state::AppState) -> anyhow::Result<()> {
    let targets = remote::load_targets(&state.data_dir).await;
    if targets.is_empty() {
        println!("Aucune instance distante jumelee.");
        return Ok(());
    }
    for t in &targets {
        println!("  {} -> {}", t.label, t.host);
    }
    Ok(())
}

async fn cmd_remote_call(state: &state::AppState, remote_repl: &RemoteRepl, label: &str, action: serde_json::Value) -> anyhow::Result<()> {
    let targets = remote::load_targets(&state.data_dir).await;
    let target = targets.iter().find(|t| t.label == label).ok_or_else(|| anyhow::anyhow!("instance \"{label}\" inconnue - voir 'remote targets'"))?;
    let result = remote::client_call(&state.http, &remote_repl.identity, target, action).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
