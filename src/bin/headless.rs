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
use mcmanager::{debug, downloader, modrinth, process, state, updater};
use uuid::Uuid;

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

    let args: Vec<String> = std::env::args().skip(1).collect();
    let lock_data_dir = app_state.data_dir.clone();

    let result = if let Some(pos) = args.iter().position(|a| a == "--script") {
        let Some(path) = args.get(pos + 1) else {
            eprintln!("--script necessite un chemin de fichier");
            std::process::exit(1);
        };
        run_script(&app_state, path).await
    } else {
        run_interactive(&app_state).await
    };

    let _ = tokio::fs::remove_file(lock_data_dir.join("mcmanager.lock")).await;
    result
}

async fn run_interactive(state: &state::AppState) -> anyhow::Result<()> {
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
        if let Err(e) = dispatch(state, line).await {
            println!("Erreur : {e}");
        }
    }
    Ok(())
}

async fn run_script(state: &state::AppState, path: &str) -> anyhow::Result<()> {
    let content = tokio::fs::read_to_string(path).await?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        println!("$ {line}");
        if let Err(e) = dispatch(state, line).await {
            println!("Erreur : {e}");
        }
    }
    Ok(())
}

async fn dispatch(state: &state::AppState, line: &str) -> anyhow::Result<()> {
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
