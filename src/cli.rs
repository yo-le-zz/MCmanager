//! Headless CLI, mainly useful when MCManager runs on a remote machine
//! (a VPS, a home server reached over SSH...) where opening a browser isn't
//! convenient. It talks to an *already running* `mcmanager serve` instance
//! over its own REST API - it does not touch the JSON store directly, so it
//! is always consistent with whatever the web UI or another CLI call sees,
//! and safe to use even while the web server is handling other requests.
//!
//! Usage:
//!   mcmanager                      start the web server (default)
//!   mcmanager serve                same, explicit
//!   mcmanager cli list             list servers
//!   mcmanager cli status <id>      show CPU/RAM/players for a server
//!   mcmanager cli start <id>       start a server
//!   mcmanager cli stop <id>        stop a server
//!   mcmanager cli create --name NAME --loader paper --version 1.21.11
//!   mcmanager --help               show this help

use serde_json::Value;

fn base_url() -> String {
    let host = std::env::var("MCMANAGER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("MCMANAGER_PORT").unwrap_or_else(|_| "7777".to_string());
    format!("http://{host}:{port}/api")
}

pub fn print_help() {
    println!(r#"MCManager v{}

Usage:
  mcmanager                       demarre l'interface web (comportement par defaut)
  mcmanager serve                 idem, explicite
  mcmanager cli list              liste les serveurs enregistres
  mcmanager cli status <id>       statut (CPU/RAM/joueurs) d'un serveur
  mcmanager cli start <id>        demarre un serveur
  mcmanager cli stop <id>         arrete un serveur proprement
  mcmanager cli create --name N --loader paper --version 1.21.11 [--port P]
  mcmanager --version             affiche la version
  mcmanager --help                affiche cette aide

Ces commandes "cli" necessitent qu'une instance de MCManager tourne deja
(ex: via systemd) - elles parlent a son API sur MCMANAGER_HOST:MCMANAGER_PORT
(par defaut 127.0.0.1:7777), exactement comme le ferait l'interface web.
"#, crate::updater::CURRENT_VERSION);
}

/// Returns `Some(exit_code)` if a CLI subcommand was handled (and the
/// process should exit with that code instead of starting the web server),
/// or `None` if the caller should fall through to the normal web server.
pub async fn try_dispatch(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        None | Some("serve") => None,
        Some("--help") | Some("-h") | Some("help") => {
            print_help();
            Some(0)
        }
        Some("--version") | Some("-v") => {
            println!("mcmanager {}", crate::updater::CURRENT_VERSION);
            Some(0)
        }
        Some("cli") => Some(run_cli(&args[1..]).await),
        Some(other) => {
            eprintln!("Commande inconnue : {other}\n");
            print_help();
            Some(1)
        }
    }
}

async fn run_cli(args: &[String]) -> i32 {
    let client = reqwest::Client::new();
    let base = base_url();

    let result = match args.first().map(String::as_str) {
        Some("list") => cli_list(&client, &base).await,
        Some("status") => cli_status(&client, &base, args.get(1)).await,
        Some("start") => cli_action(&client, &base, args.get(1), "start").await,
        Some("stop") => cli_action(&client, &base, args.get(1), "stop").await,
        Some("create") => cli_create(&client, &base, &args[1..]).await,
        Some(other) => Err(format!("sous-commande cli inconnue : {other} (list|status|start|stop|create)")),
        None => Err("usage: mcmanager cli <list|status|start|stop|create> ...".to_string()),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Erreur: {e}");
            eprintln!("(MCManager est-il demarre ? essayez 'systemctl status mcmanager' ou lancez 'mcmanager serve')");
            1
        }
    }
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(body["error"].as_str().unwrap_or("erreur inconnue").to_string());
    }
    Ok(body)
}

async fn post_json(client: &reqwest::Client, url: &str, body: &Value) -> Result<Value, String> {
    let resp = client.post(url).json(body).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let out: Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(out["error"].as_str().unwrap_or("erreur inconnue").to_string());
    }
    Ok(out)
}

async fn cli_list(client: &reqwest::Client, base: &str) -> Result<(), String> {
    let servers = get_json(client, &format!("{base}/servers")).await?;
    let arr = servers.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        println!("Aucun serveur enregistre.");
        return Ok(());
    }
    println!("{:<38} {:<22} {:<10} {:<8}", "ID", "NOM", "LOADER", "VERSION");
    for s in arr {
        println!(
            "{:<38} {:<22} {:<10} {:<8}",
            s["id"].as_str().unwrap_or("?"),
            s["name"].as_str().unwrap_or("?"),
            s["loader"].as_str().unwrap_or("?"),
            s["mc_version"].as_str().unwrap_or("?"),
        );
    }
    Ok(())
}

async fn cli_status(client: &reqwest::Client, base: &str, id: Option<&String>) -> Result<(), String> {
    let id = id.ok_or("usage: mcmanager cli status <id>")?;
    let s = get_json(client, &format!("{base}/servers/{id}/status")).await?;
    println!("En ligne     : {}", s["running"].as_bool().unwrap_or(false));
    println!("CPU          : {:.1}%", s["cpu_percent"].as_f64().unwrap_or(0.0));
    println!("RAM          : {:.0} Mo", s["mem_mb"].as_f64().unwrap_or(0.0));
    if let Some(online) = s["players_online"].as_u64() {
        println!("Joueurs      : {}/{}", online, s["players_max"].as_u64().unwrap_or(0));
    }
    Ok(())
}

async fn cli_action(client: &reqwest::Client, base: &str, id: Option<&String>, action: &str) -> Result<(), String> {
    let id = id.ok_or_else(|| format!("usage: mcmanager cli {action} <id>"))?;
    post_json(client, &format!("{base}/servers/{id}/{action}"), &Value::Null).await?;
    println!("OK: {action} envoye pour {id}");
    Ok(())
}

async fn cli_create(client: &reqwest::Client, base: &str, args: &[String]) -> Result<(), String> {
    let mut name = None;
    let mut loader = None;
    let mut version = None;
    let mut port: u16 = 25565;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => { name = args.get(i + 1).cloned(); i += 2; }
            "--loader" => { loader = args.get(i + 1).cloned(); i += 2; }
            "--version" => { version = args.get(i + 1).cloned(); i += 2; }
            "--port" => { port = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(25565); i += 2; }
            _ => { i += 1; }
        }
    }
    let name = name.ok_or("--name est requis")?;
    let loader = loader.ok_or("--loader est requis (vanilla|paper|purpur|spigot|fabric|quilt|forge|neoforge)")?;
    let version = version.ok_or("--version est requis (ex: 1.21.11)")?;

    println!("Creation de '{name}' ({loader} {version})... cela peut prendre une minute (telechargement).");
    let body = serde_json::json!({
        "name": name, "loader": loader, "mc_version": version, "port": port, "eula_accepted": true,
    });
    let created = post_json(client, &format!("{base}/servers"), &body).await?;
    println!("Cree avec l'ID : {}", created["id"].as_str().unwrap_or("?"));
    Ok(())
}
